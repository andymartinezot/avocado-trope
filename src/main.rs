//! zoetrope — visualize coding-agent sessions as a live flow graph.
//!
//! CLI (hand-rolled over `std::env::args`, no clap):
//!
//! ```text
//! zoe                       follow the current project's live session
//! zoe <file.jsonl>          replay a recording, played from the start
//! zoe <id>                  replay a session by id (or a unique prefix)
//! zoe <dir>                 follow another project's live session
//! zoe <file> --follow       follow a file's live edge instead of replaying
//! zoe <file> --speed N      playback speed multiplier (default 8.0)
//! zoe --provider <name> ... force the transcript format instead of detecting it
//! zoe --theme <name> ...    color theme (also $ZOE_THEME, or Herdr's theme)
//! zoe inspect <file|id|dir> headless: print the session tree + info
//! zoe inspect --json|--markdown <target>   the same as data, for scripts and PRs
//! zoe fleet                 every agent Herdr knows about, on one screen
//! zoe compare <a> <b>       two sessions side by side on one clock
//! ```

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};
use tokio::sync::mpsc;

use zoetrope::provider::{Provider, ReadMode, Target, open};
use zoetrope::state::session::SessionModel;
use zoetrope::state::{App, Mode};
use zoetrope::tailer::{TailRequest, UiEvent};
use zoetrope::{tailer, tui};

/// Channel capacity for the bounded request/event channels.
const CHANNEL_CAP: usize = 32;

/// Parsed CLI invocation.
///
/// One TUI command (`View`) over the unified timeline engine, plus the headless
/// `Inspect`. The launch only sets defaults: *what* to open and *where the
/// playhead starts*. Everything (scrub, follow, go-live, pause) is available
/// once running, regardless of how it launched.
#[derive(Debug, Clone)]
pub enum Cli {
    /// View a session in the TUI. `target`: a session file (replay it from the
    /// start), a project dir (follow its live session), a session id, or
    /// `None` (the current project). `follow` starts at the live edge instead
    /// of replaying. `provider` forces the format instead of detecting it.
    View {
        target: Option<String>,
        follow: bool,
        speed: f64,
        provider: Option<Provider>,
        /// `--theme`: wins over `$ZOE_THEME` and Herdr's theme.
        theme: Option<String>,
    },
    /// Headless: parse and print the session tree + info; no TUI. The target
    /// resolves like `View`'s: a file, a session id, or a project directory.
    Inspect {
        target: String,
        provider: Option<Provider>,
        format: Format,
    },
    /// Every Herdr agent on one screen.
    Fleet {
        theme: Option<String>,
        /// Seconds between polls.
        interval: u64,
        notify: bool,
        /// Print one snapshot as JSON and exit.
        json: bool,
        /// Show made-up rows from this file instead of asking Herdr.
        demo: Option<PathBuf>,
    },
    /// Two sessions side by side.
    Compare {
        a: String,
        b: String,
        speed: f64,
        theme: Option<String>,
    },
}

/// How `inspect` prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Text,
    Json,
    Markdown,
}

/// Default replay speed multiplier.
const DEFAULT_REPLAY_SPEED: f64 = 8.0;

const USAGE: &str = "\
zoetrope — visualize coding-agent sessions as a flow graph

USAGE:
    zoe                     follow the current project's live session
    zoe <file.jsonl>        replay a recording, played from the start
    zoe <id>                replay a session by id, or a unique prefix of one
    zoe <dir>               follow another project's live session
    zoe <file> --follow     follow a file's live edge instead of replaying
    zoe <file> --speed N    playback speed (default 8.0)
    zoe --provider <name>   force the format (claude, codex) instead of detecting it
    zoe --theme <name>      color theme: dracula (default), catppuccin, tokyo-night,
                            gruvbox, nord, classic, terminal. Also $ZOE_THEME. Press t
                            to switch; the choice is remembered. Inside Herdr it
                            follows Herdr's theme until you pick one.
    zoe --theme list        print the theme names
    zoe inspect <file|id>   headless: print the session tree + info
        --json, --markdown  the same as data; contains prompts, paths and errors
    zoe fleet               every agent in Herdr on one screen
        --interval N        seconds between polls (default 2)
        --notify            Herdr notification when an agent starts needing you
        --json              print one snapshot and exit
    zoe compare <a> <b>     two sessions side by side, replaying on one clock
    --stuck-after N         flag a tool call open this many minutes (default 5)
    --context-window N      tokens to assume when the transcript doesn't say
    zoe --version           print the version and exit

Once open, scrub/follow/pause/go-live are available no matter how you launched.";

/// Parse `std::env::args` into a [`Cli`]. Returns a usage error on bad input.
fn parse_cli(args: impl Iterator<Item = String>) -> Result<Cli> {
    // Skip argv[0].
    let mut args = args.skip(1).peekable();

    // Flags every command takes, taken out first so each command's own loop
    // stays simple.
    let mut rest = Vec::new();
    let mut theme = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--theme" => {
                let v = args
                    .next()
                    .ok_or_else(|| anyhow!("--theme requires a name\n\n{USAGE}"))?;
                if v == "list" {
                    println!("{}", zoetrope::theme::names().replace(", ", "\n"));
                    std::process::exit(0);
                }
                if zoetrope::theme::find(&v).is_none() {
                    bail!(
                        "unknown theme {v:?}; choose one of: {}",
                        zoetrope::theme::names()
                    );
                }
                theme = Some(v);
            }
            "--stuck-after" => {
                let v = args
                    .next()
                    .ok_or_else(|| anyhow!("--stuck-after requires minutes\n\n{USAGE}"))?;
                let minutes: u64 = v
                    .parse()
                    .ok()
                    .filter(|m| (1..=10_000).contains(m))
                    .ok_or_else(|| {
                        anyhow!("--stuck-after must be a number of minutes, got {v:?}")
                    })?;
                zoetrope::settings::set_stuck_after_secs(minutes * 60);
            }
            "--context-window" => {
                let v = args
                    .next()
                    .ok_or_else(|| anyhow!("--context-window requires tokens\n\n{USAGE}"))?;
                let tokens: u64 =
                    v.parse().ok().filter(|t| *t > 0).ok_or_else(|| {
                        anyhow!("--context-window must be a token count, got {v:?}")
                    })?;
                zoetrope::settings::set_context_window(tokens);
            }
            _ => rest.push(arg),
        }
    }
    let mut args = rest.into_iter().peekable();
    let provider_flag =
        |args: &mut std::iter::Peekable<std::vec::IntoIter<String>>| -> Result<Option<Provider>> {
            let v = args
                .next()
                .ok_or_else(|| anyhow!("--provider requires a name\n\n{USAGE}"))?;
            Provider::parse(&v)
                .map(Some)
                .ok_or_else(|| anyhow!("unknown provider {v:?}; known: claude, codex"))
        };

    if args.peek().map(String::as_str) == Some("fleet") {
        args.next();
        let (mut interval, mut notify, mut json, mut demo) = (2u64, false, false, None);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--interval" => {
                    let v = args
                        .next()
                        .ok_or_else(|| anyhow!("--interval requires seconds\n\n{USAGE}"))?;
                    interval = v
                        .parse()
                        .ok()
                        .filter(|s| (1..=3600).contains(s))
                        .ok_or_else(|| anyhow!("--interval must be seconds, got {v:?}"))?;
                }
                "--notify" => notify = true,
                "--json" => json = true,
                "--demo" => {
                    let v = args
                        .next()
                        .ok_or_else(|| anyhow!("--demo requires a file\n\n{USAGE}"))?;
                    demo = Some(PathBuf::from(v));
                }
                other => bail!("unknown fleet argument {other:?}\n\n{USAGE}"),
            }
        }
        return Ok(Cli::Fleet {
            theme,
            interval,
            notify,
            json,
            demo,
        });
    }

    if args.peek().map(String::as_str) == Some("compare") {
        args.next();
        let mut targets = Vec::new();
        let mut speed = DEFAULT_REPLAY_SPEED;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--speed" => {
                    let v = args
                        .next()
                        .ok_or_else(|| anyhow!("--speed requires a number\n\n{USAGE}"))?;
                    speed = v
                        .parse::<f64>()
                        .ok()
                        .filter(|s| s.is_finite() && *s > 0.0)
                        .ok_or_else(|| anyhow!("--speed must be a positive number, got {v:?}"))?;
                }
                other if other.starts_with('-') => bail!("unknown flag {other:?}\n\n{USAGE}"),
                _ => targets.push(arg),
            }
        }
        let [a, b]: [String; 2] = targets
            .try_into()
            .map_err(|_| anyhow!("compare takes two sessions\n\n{USAGE}"))?;
        return Ok(Cli::Compare { a, b, speed, theme });
    }

    // `inspect <file>` is the one distinct (headless) subcommand.
    if args.peek().map(String::as_str) == Some("inspect") {
        args.next();
        let mut target: Option<String> = None;
        let mut provider = None;
        let mut format = Format::Text;
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--provider" => provider = provider_flag(&mut args)?,
                "--json" => format = Format::Json,
                "--markdown" => format = Format::Markdown,
                other if other.starts_with('-') => bail!("unknown flag {other:?}\n\n{USAGE}"),
                _ => {
                    if target.is_some() {
                        bail!("inspect takes a single target argument\n\n{USAGE}");
                    }
                    target = Some(arg);
                }
            }
        }
        let target = target.ok_or_else(|| {
            anyhow!("inspect requires a file, a session id or a directory\n\n{USAGE}")
        })?;
        return Ok(Cli::Inspect {
            target,
            provider,
            format,
        });
    }

    // Otherwise: an optional positional target + flags.
    let mut target: Option<String> = None;
    let mut follow = false;
    let mut speed = DEFAULT_REPLAY_SPEED;
    let mut provider = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            // Packaging depends on this: the Homebrew formula's `test do`
            // block runs `zoe --version`, and it has to exit 0.
            "-V" | "--version" => {
                println!("zoe {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--follow" => follow = true,
            "--speed" => {
                let v = args
                    .next()
                    .ok_or_else(|| anyhow!("--speed requires a number\n\n{USAGE}"))?;
                speed = v
                    .parse::<f64>()
                    .with_context(|| format!("invalid --speed value: {v:?}"))?;
                if !(speed.is_finite() && speed > 0.0) {
                    bail!("--speed must be a positive number, got {v:?}");
                }
            }
            "--provider" => provider = provider_flag(&mut args)?,
            other if other.starts_with('-') => {
                bail!("unknown flag {other:?}\n\n{USAGE}");
            }
            _ => {
                if target.is_some() {
                    bail!("expected a single target argument\n\n{USAGE}");
                }
                target = Some(arg);
            }
        }
    }

    Ok(Cli::View {
        target,
        follow,
        speed,
        provider,
        theme,
    })
}

/// Fully parse a session, every file of it, into a [`SessionModel`] and its
/// [`SessionInfo`](zoetrope::state::SessionInfo). Shared by `inspect`; the
/// live/replay path uses the tailer instead.
fn parse_session_fully(
    target: &Target,
    only: Option<Provider>,
) -> Result<(SessionModel, zoetrope::state::SessionInfo)> {
    let session = open(target, only)?;
    let mut model = SessionModel::new(session.id.clone());
    let mut info = zoetrope::state::SessionInfo::default();
    let p = session.provider;
    let mut apply = |mut statement: zoetrope::fact::Statement| {
        for f in statement.take_session_meta() {
            info.apply(&f);
        }
        for f in &statement.facts {
            model.apply_fact(f);
        }
    };
    // Order is not critical — the model is fold-order independent — so files
    // go in as the session lists them, root first.
    for f in session.every_file() {
        let text = std::fs::read_to_string(&f.path)
            .with_context(|| format!("reading {}", f.path.display()))?;
        match f.read {
            ReadMode::Tail => {
                let mut stream = p.stream_for(f);
                for statement in text.lines().filter_map(|l| stream.push(l)) {
                    apply(statement);
                }
            }
            ReadMode::Whole => {
                if let Some(statement) = p.sidecar(f, &text) {
                    apply(statement);
                }
            }
        }
    }

    // Group nodes have no direct completion signal — roll them up from their
    // children once everything is folded.
    model.recompute_group_status();

    // Interactive agents (main, forks) have no completion signal: derive
    // their liveness against the wall clock — `inspect` is a point-in-time
    // view, so a recently active session shows `running`, a long-quiet one `idle`.
    model.recompute_liveness(Some(chrono::Utc::now()));

    Ok((model, info))
}

/// Run the `inspect` subcommand: fully parse the session and print a tree to
/// stdout. Returns an error (non-zero exit) on an unreadable file. This is the
/// headless smoke test — no TTY required.
async fn run_inspect(target: String, provider: Option<Provider>, format: Format) -> Result<()> {
    use zoetrope::state::{export, render};
    let target = resolve_target(target)?;
    let (model, info) = parse_session_fully(&target, provider)?;
    match format {
        Format::Text => print!("{}", render::report(&model, &info)),
        Format::Json => println!("{:#}", export::json(&model, &info)),
        Format::Markdown => print!("{}", export::markdown(&model, &info)),
    }
    Ok(())
}

/// Run `zoe fleet`: a poller publishing rows, and a screen drawing them.
async fn run_fleet(
    theme: Option<String>,
    interval: u64,
    notify: bool,
    json: bool,
    demo: Option<PathBuf>,
) -> Result<()> {
    use zoetrope::fleet::{self, Snapshot, Source};
    let source = match &demo {
        Some(path) => Source::demo(path).map_err(|e| anyhow!("{}: {e}", path.display()))?,
        None => Source::Herdr,
    };
    let (tx, mut rx) = tokio::sync::watch::channel(Snapshot::default());
    let poller = fleet::poll(source, std::time::Duration::from_secs(interval), notify, tx);
    if json {
        tokio::spawn(poller);
        // The first complete snapshot: no session still being read.
        loop {
            rx.changed().await.context("the poller stopped")?;
            let snap = rx.borrow_and_update().clone();
            if let Some(e) = snap.error {
                bail!(e);
            }
            if snap.loading == 0 {
                println!("{}", serde_json::to_string_pretty(&snap.rows)?);
                return Ok(());
            }
        }
    }
    pick_theme(theme);
    tokio::spawn(poller);
    fleet::view::run(rx, demo.is_some()).await?;
    // Don't wait on a transcript read that's still running: nobody needs it.
    std::process::exit(0);
}

/// Start following one session: its app, and a tailer feeding it.
async fn start_session(
    target: Target,
    provider: Option<Provider>,
    mode: Mode,
    replay: bool,
    speed: f64,
    session_id: String,
) -> Result<zoetrope::compare::Side> {
    let (tail_tx, tail_rx) = mpsc::channel::<TailRequest>(CHANNEL_CAP);
    let (ui_tx, ui_rx) = mpsc::channel::<UiEvent>(CHANNEL_CAP);
    tail_tx
        .send(TailRequest::Watch(target))
        .await
        .map_err(|_| anyhow!("tailer channel closed before start"))?;
    tokio::spawn(async move {
        if let Err(e) = tailer::run(tail_rx, ui_tx.clone(), replay, speed, provider).await {
            let _ = ui_tx.send(UiEvent::Error(e.to_string())).await;
        }
    });
    Ok(zoetrope::compare::Side {
        app: App::new(session_id, mode),
        tail: tail_tx,
        events: ui_rx,
        ready: false,
    })
}

/// Run `zoe compare`.
async fn run_compare(a: String, b: String, speed: f64, theme: Option<String>) -> Result<()> {
    pick_theme(theme);
    load_herdr_keys();
    let mut sides = Vec::new();
    for arg in [a, b] {
        let target = resolve_target(arg)?;
        if matches!(target, Target::Here(_)) {
            bail!("compare takes session files or ids, not directories");
        }
        let id = open(&target, None)?.id;
        sides.push(start_session(target, None, Mode::Replay, true, speed, id).await?);
    }
    let right = sides.pop().context("two sides")?;
    let left = sides.pop().context("two sides")?;
    zoetrope::compare::run(left, right).await
}

/// Resolve a `View` invocation into (session id, target, mode, feeder, speed),
/// then spawn the tailer and run the TUI.
///
/// A **file** or an **id** bulk-loads + tails (`replay` feeder); `--follow`
/// only changes the start position (head vs beginning) via the mode. A
/// **dir** (or no target → the current project) discovers the newest session
/// there, any provider, and live-tails it.
async fn run_tui(cli: Cli) -> Result<()> {
    let Cli::View {
        target,
        follow,
        speed,
        provider,
        theme,
    } = cli
    else {
        unreachable!("inspect handled in main");
    };

    let target = match target {
        Some(t) => resolve_target(t)?,
        None => Target::Here(std::env::current_dir().context("resolving current directory")?),
    };
    let (session_id, mode, replay, speed) = match &target {
        // A concrete file, or a stored session by id → bulk-load + tail. Paced
        // from the start unless `--follow` asks to ride the (possibly still
        // growing) edge.
        Target::Path(_) | Target::Id(_) => {
            let session_id = open(&target, provider)?.id;
            let mode = if follow { Mode::Live } else { Mode::Replay };
            (session_id, mode, true, speed)
        }
        // A directory (or none → cwd) → live: discover the newest session of
        // that project and follow it. (It need not exist yet; the tailer
        // waits.) The id is best-effort so stale events filter; the tailer
        // re-discovers and may switch.
        Target::Here(_) => {
            let session_id = open(&target, provider).map(|s| s.id).unwrap_or_default();
            (session_id, Mode::Live, false, DEFAULT_REPLAY_SPEED)
        }
    };

    pick_theme(theme);
    load_herdr_keys();
    // A session's id is fixed at launch here; the tailer may re-discover.
    let side = start_session(target, provider, mode, replay, speed, session_id).await?;
    tui::run(side.app, side.tail, side.events).await
}

/// What a positional argument means: an existing file is a session's file, an
/// existing directory is a project to follow, anything shaped like a path
/// that does not exist is a typo, and the rest is a session id or a prefix
/// of one. Shared by `zoe <target>` and `zoe inspect <target>`.
fn resolve_target(arg: String) -> Result<Target> {
    let path = PathBuf::from(&arg);
    if path.is_file() {
        return Ok(Target::Path(path));
    }
    if path.is_dir() {
        return Ok(Target::Here(path));
    }
    if path.components().count() > 1 || path.extension().is_some() {
        bail!("not found: {}", path.display());
    }
    Ok(Target::Id(arg))
}

/// Choose the launch theme: `--theme`, else `$ZOE_THEME`, else the theme you
/// last switched to with `t`, else Herdr's theme when running inside a Herdr
/// pane, else the default. A name we don't know
/// (an env value, or a Herdr theme with no counterpart) keeps the default.
fn pick_theme(flag: Option<String>) {
    let env = std::env::var("ZOE_THEME").ok().filter(|v| !v.is_empty());
    let herdr = || {
        std::env::var_os("HERDR_PANE_ID")?;
        herdr_theme(&read_capped(&herdr_config_path()?)?)
    };
    if let Some(name) = flag
        .or(env)
        .or_else(zoetrope::prefs::saved_theme)
        .or_else(herdr)
    {
        zoetrope::theme::set_by_name(&name);
    }
}

/// Show the Herdr keys bound to this app, when running inside Herdr.
fn load_herdr_keys() {
    if std::env::var_os("HERDR_PANE_ID").is_none() {
        return;
    }
    if let Some(text) = herdr_config_path().and_then(|p| read_capped(&p)) {
        zoetrope::settings::set_herdr_keys(herdr_keys(&text));
    }
}

/// `prefix+shift+a` with the prefix spelled out, as a person presses it:
/// `Ctrl+Space Shift+A`.
fn pretty_key(key: &str, prefix: &str) -> String {
    let word = |part: &str| {
        let mut c = part.chars();
        c.next().map_or(String::new(), |f| {
            f.to_uppercase().collect::<String>() + c.as_str()
        })
    };
    let chord = |k: &str| k.split('+').map(word).collect::<Vec<_>>().join("+");
    match key.strip_prefix("prefix+") {
        Some(rest) => format!("{} {}", chord(prefix), chord(rest)),
        None => chord(key),
    }
}

/// The keys in a Herdr config that open this app, as (key, what it opens).
/// A line scan, like [`herdr_theme`]: `key` and `command` are plain strings in
/// `[[keys.command]]` tables.
fn herdr_keys(config: &str) -> Vec<(String, String)> {
    let value = |line: &str, name: &str| {
        let rest = line.strip_prefix(name)?.trim_start().strip_prefix('=')?;
        Some(
            rest.split(" #")
                .next()?
                .trim()
                .trim_matches('"')
                .to_string(),
        )
    };
    let mut prefix = "ctrl+b".to_string();
    let mut key = None;
    let mut out = Vec::new();
    for line in config.lines().map(str::trim) {
        if let Some(p) = value(line, "prefix") {
            prefix = p;
        } else if line == "[[keys.command]]" {
            key = None;
        } else if let Some(k) = value(line, "key") {
            key = Some(k);
        } else if let Some(cmd) = value(line, "command")
            && cmd.contains("avocado-trope")
            && let Some(k) = key.take()
        {
            let what = if cmd.contains("fleet") {
                "fleet"
            } else if cmd.contains("zoomed") || cmd.contains("--placement tab") {
                "full"
            } else if cmd.contains("--direction right") {
                "right"
            } else if cmd.contains("--direction down") {
                "below"
            } else {
                "split"
            };
            out.push((k, what.to_string()));
        }
    }
    out.into_iter()
        .map(|(k, what)| (pretty_key(&k, &prefix), what))
        .collect()
}

/// Herdr's config file: `$HERDR_CONFIG_PATH`, else `<config dir>/herdr/config.toml`.
fn herdr_config_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("HERDR_CONFIG_PATH") {
        return Some(PathBuf::from(p));
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("herdr").join("config.toml"))
}

/// The first 64 KiB of a file as text: a config is tiny, and a path that
/// turns out to be a FIFO or a device must not hang or grow the read.
fn read_capped(path: &std::path::Path) -> Option<String> {
    use std::io::Read;
    let mut text = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(64 * 1024)
        .read_to_string(&mut text)
        .ok()?;
    Some(text)
}

/// `name` under `[theme]` in a Herdr config. A line scan rather than a TOML
/// parser: the key is a plain `name = "…"` string in one known table.
fn herdr_theme(config: &str) -> Option<String> {
    let mut in_theme = false;
    for line in config.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_theme = line == "[theme]";
            continue;
        }
        if !in_theme {
            continue;
        }
        // Only the `name` key itself: `name_x = …` is some other key.
        let Some(value) = line
            .strip_prefix("name")
            .map(str::trim_start)
            .and_then(|v| v.strip_prefix('='))
        else {
            continue;
        };
        let value = value
            .split('#')
            .next()
            .unwrap_or("")
            .trim()
            .trim_matches(['"', '\'']);
        return (!value.is_empty()).then(|| value.to_string());
    }
    None
}

#[tokio::main]
async fn main() -> Result<()> {
    // Answer and exit before the TUI touches the terminal, so this works over a
    // pipe — assets/build.sh asks for it and has no tty to spare. The tape's
    // Sleep has to be at least this long or the recording cuts mid-gesture, and
    // that number used to be copied into the tape by hand.
    if std::env::var("ZOETROPE_DEMO").as_deref() == Ok("duration") {
        println!("{:.2}", zoetrope::autopilot::tour_secs());
        return Ok(());
    }

    let cli = parse_cli(std::env::args())?;
    match cli {
        Cli::Inspect {
            target,
            provider,
            format,
        } => run_inspect(target, provider, format).await,
        Cli::Fleet {
            theme,
            interval,
            notify,
            json,
            demo,
        } => run_fleet(theme, interval, notify, json, demo).await,
        Cli::Compare { a, b, speed, theme } => run_compare(a, b, speed, theme).await,
        other => run_tui(other).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_herdr_theme_name() {
        let cfg = "onboarding = false\n[ui]\nname = \"no\"\n[theme]\nname = \"tokyo-night\" # mine\nauto_switch = false\n[ui.toast]\n";
        assert_eq!(herdr_theme(cfg).as_deref(), Some("tokyo-night"));
        assert_eq!(herdr_theme("[theme.custom]\nname = \"x\"\n"), None);
        // Other keys that start with `name` don't stop the scan.
        assert_eq!(
            herdr_theme("[theme]\nname_override = true\nnames = 1\nname = \"nord\"\n").as_deref(),
            Some("nord")
        );
        assert_eq!(herdr_theme(""), None);
    }

    #[test]
    fn reads_the_herdr_keys_that_open_this_app() {
        let cfg = r#"
[keys]
prefix = "ctrl+space"
focus_pane_up = "prefix+i"

[[keys.command]]
key = "prefix+e"
type = "shell"
command = "herdr plugin action invoke show-explorer --plugin herdr-sidebar"

[[keys.command]]
key = "prefix+shift+a"
type = "shell"
command = "herdr plugin pane open --plugin andymartinezot.avocado-trope --entrypoint graph --placement zoomed"

[[keys.command]]
key = "prefix+shift+e"
command = "herdr plugin pane open --plugin andymartinezot.avocado-trope --entrypoint fleet --placement zoomed"
"#;
        let keys = herdr_keys(cfg);
        assert_eq!(
            keys,
            vec![
                ("Ctrl+Space Shift+A".to_string(), "full".to_string()),
                ("Ctrl+Space Shift+E".to_string(), "fleet".to_string()),
            ]
        );
        assert!(herdr_keys("").is_empty());
    }

    #[test]
    fn new_commands_parse() {
        assert!(matches!(
            cli(&["fleet", "--interval", "5", "--notify"]).unwrap(),
            Cli::Fleet {
                interval: 5,
                notify: true,
                ..
            }
        ));
        assert!(cli(&["fleet", "--interval", "0"]).is_err());
        assert!(cli(&["fleet", "stray"]).is_err());
        assert!(matches!(
            cli(&["compare", "a.jsonl", "b.jsonl", "--speed", "4"]).unwrap(),
            Cli::Compare { .. }
        ));
        assert!(cli(&["compare", "a.jsonl"]).is_err());
        assert!(matches!(
            cli(&["inspect", "--json", "x.jsonl"]).unwrap(),
            Cli::Inspect {
                format: Format::Json,
                ..
            }
        ));
        assert!(cli(&["x.jsonl", "--stuck-after", "0"]).is_err());
        assert!(cli(&["x.jsonl", "--context-window", "nope"]).is_err());
        // Global flags work before the subcommand too.
        assert!(matches!(
            cli(&["--theme", "nord", "fleet"]).unwrap(),
            Cli::Fleet { .. }
        ));
    }

    #[test]
    fn theme_flag_is_validated() {
        assert!(cli(&["--theme", "nord"]).is_ok());
        assert!(cli(&["--theme", "nope"]).is_err());
        assert!(cli(&["--theme"]).is_err());
    }
    use zoetrope::state::session::AgentStatus;

    fn cli(args: &[&str]) -> Result<Cli> {
        // parse_cli skips argv[0], so prepend a fake program name.
        let mut v = vec!["zoe".to_string()];
        v.extend(args.iter().map(|s| s.to_string()));
        parse_cli(v.into_iter())
    }

    #[test]
    fn bare_invocation_is_live_for_cwd() {
        match cli(&[]).unwrap() {
            Cli::View {
                target: None,
                follow: false,
                ..
            } => {}
            other => panic!("expected View{{target:None}}, got {other:?}"),
        }
    }

    #[test]
    fn positional_argument_is_the_target() {
        match cli(&["/tmp/foo"]).unwrap() {
            Cli::View {
                target: Some(p), ..
            } => assert_eq!(p, "/tmp/foo"),
            other => panic!("got {other:?}"),
        }
        // A session file, a directory or an id: told apart at run time.
        match cli(&["01a03eb1"]).unwrap() {
            Cli::View {
                target: Some(p), ..
            } => assert_eq!(p, "01a03eb1"),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn default_speed_and_no_follow() {
        match cli(&["s.jsonl"]).unwrap() {
            Cli::View {
                speed,
                follow,
                provider,
                ..
            } => {
                assert_eq!(speed, DEFAULT_REPLAY_SPEED);
                assert!(!follow);
                assert_eq!(provider, None);
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn speed_follow_and_provider_flags_in_any_order() {
        match cli(&["s.jsonl", "--speed", "4", "--follow", "--provider", "codex"]).unwrap() {
            Cli::View {
                target: Some(p),
                follow,
                speed,
                provider,
                ..
            } => {
                assert_eq!(p, "s.jsonl");
                assert_eq!(speed, 4.0);
                assert!(follow);
                assert_eq!(provider, Some(Provider::Codex));
            }
            other => panic!("got {other:?}"),
        }
        // Flags before the path, too.
        match cli(&["--provider", "Claude", "--speed", "2.5", "s.jsonl"]).unwrap() {
            Cli::View {
                target: Some(p),
                speed,
                provider,
                ..
            } => {
                assert_eq!(p, "s.jsonl");
                assert_eq!(speed, 2.5);
                assert_eq!(provider, Some(Provider::Claude));
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn rejects_bad_speed_and_unknown_provider() {
        assert!(cli(&["s.jsonl", "--speed", "nope"]).is_err());
        assert!(cli(&["s.jsonl", "--speed", "0"]).is_err());
        assert!(cli(&["s.jsonl", "--speed", "-3"]).is_err());
        assert!(cli(&["s.jsonl", "--speed"]).is_err());
        assert!(cli(&["s.jsonl", "--provider", "gemini"]).is_err());
        assert!(cli(&["s.jsonl", "--provider"]).is_err());
    }

    #[test]
    fn rejects_extra_positional_and_unknown_flags() {
        assert!(cli(&["a.jsonl", "b.jsonl"]).is_err());
        assert!(cli(&["--bogus"]).is_err());
    }

    #[test]
    fn inspect_takes_one_target_and_a_provider() {
        match cli(&["inspect", "s.jsonl"]).unwrap() {
            Cli::Inspect {
                target, provider, ..
            } => {
                assert_eq!(target, "s.jsonl");
                assert_eq!(provider, None);
            }
            other => panic!("got {other:?}"),
        }
        // An id resolves for inspect the way it does for the TUI.
        match cli(&["inspect", "01a03eb1"]).unwrap() {
            Cli::Inspect { target, .. } => assert_eq!(target, "01a03eb1"),
            other => panic!("got {other:?}"),
        }
        match cli(&["inspect", "--provider", "codex", "s.jsonl"]).unwrap() {
            Cli::Inspect { provider, .. } => assert_eq!(provider, Some(Provider::Codex)),
            other => panic!("got {other:?}"),
        }
        assert!(cli(&["inspect"]).is_err());
        assert!(cli(&["inspect", "a", "b"]).is_err());
    }

    #[test]
    fn a_missing_path_is_a_typo_and_a_bare_word_is_an_id() {
        assert!(resolve_target("./nope.jsonl".into()).is_err());
        assert!(resolve_target("nope.jsonl".into()).is_err());
        assert!(
            matches!(resolve_target("01a03eb1".into()), Ok(Target::Id(id)) if id == "01a03eb1")
        );
        assert!(matches!(resolve_target(".".into()), Ok(Target::Here(_))));
    }

    #[test]
    fn parse_session_fully_marks_quiet_main_idle() {
        // Inspect is a point-in-time view: a transcript whose last activity is
        // far in the past reports main as Idle (interactive agents never claim
        // completion — the format has no end marker to prove it).
        let dir = std::env::temp_dir().join(format!("zoetrope-fullparse-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let tmp = dir.join("77777777-7777-7777-7777-777777777777.jsonl");
        std::fs::write(
            &tmp,
            b"{\"type\":\"user\",\"uuid\":\"u\",\"parentUuid\":null,\"timestamp\":\"2026-06-05T13:51:00.000Z\",\"message\":{\"role\":\"user\",\"content\":\"hi\"}}\n",
        )
        .unwrap();

        let (model, _info) = parse_session_fully(&Target::Path(tmp.clone()), None).expect("parses");
        let main = model.agent(zoetrope::state::session::MAIN_ID).unwrap();
        assert_eq!(main.status, AgentStatus::Idle);
        // The root's name comes from the provider now, not the model.
        assert_eq!(main.agent_type.as_deref(), Some("claude"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
