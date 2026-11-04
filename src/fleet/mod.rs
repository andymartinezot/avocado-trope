//! Fleet view: every agent Herdr knows about, on one screen.
//!
//! `zoe fleet` asks Herdr who is running (`herdr agent list`, a few ms), and
//! follows each agent's transcript to fill in what Herdr doesn't know: model,
//! context, the last tool, failures, files, whether it is stuck.
//!
//! The cost is kept off the screen and off the disk. A transcript is parsed
//! once and then only its appended bytes are read ([`watch`]); that work runs
//! on a background task that publishes finished rows, so the UI only ever
//! draws, and only when a row changed ([`view`]).

pub mod view;
pub mod watch;

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::sync::watch as latest;

/// How long Herdr gets to answer one poll.
const HERDR_TIMEOUT: Duration = Duration::from_secs(5);

/// One agent as `herdr agent list` reports it. Everything but the pane is
/// optional: older Herdr versions report less, and a missing field should cost
/// a blank cell, not the whole view.
#[derive(Debug, Clone, Deserialize)]
pub struct Entry {
    pub pane_id: String,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub agent_status: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub terminal_title_stripped: Option<String>,
    #[serde(default)]
    pub foreground_cwd: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub agent_session: Option<EntrySession>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EntrySession {
    #[serde(default)]
    pub value: Option<String>,
}

/// One workspace as `herdr workspace list` reports it.
#[derive(Debug, Clone, Deserialize)]
struct Workspace {
    workspace_id: String,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    number: Option<u32>,
}

/// One line of the fleet: an agent and what its transcript says.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Row {
    pub pane: String,
    /// `claude`, `codex`, …
    pub agent: String,
    /// Herdr's word: `working`, `blocked`, `done`, `idle`, `unknown`.
    pub status: String,
    pub title: String,
    pub workspace: String,
    /// Orders workspaces the way Herdr's sidebar does.
    pub workspace_order: u32,
    pub path: String,
    pub model: Option<String>,
    pub context_tokens: u64,
    pub context_window: Option<u64>,
    pub last_tool: Option<String>,
    pub tools: usize,
    pub failed: usize,
    pub subagents: usize,
    pub subagents_running: usize,
    pub files: usize,
    /// Seconds since the transcript last moved.
    pub age_secs: Option<i64>,
    /// What is holding it up, when something is.
    pub stuck: Option<String>,
    pub session: Option<String>,
    /// A session file to open instead of looking `session` up (demo rows).
    pub session_file: Option<String>,
}

impl Row {
    /// Lower needs you sooner.
    pub fn urgency(&self) -> u8 {
        match (self.status.as_str(), self.stuck.is_some()) {
            ("blocked", _) => 0,
            (_, true) => 1,
            ("working", _) => 2,
            ("done", _) => 3,
            ("idle", _) => 4,
            _ => 5,
        }
    }

    /// Worth a look: waiting on you, stuck, or its context nearly full.
    pub fn needs_attention(&self) -> bool {
        let window = crate::settings::effective_window(self.context_window, 0);
        self.urgency() <= 1
            || crate::settings::pressure(self.context_tokens, window)
                == crate::settings::Pressure::Critical
    }
}

/// What the poller publishes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub rows: Vec<Row>,
    /// Why the last poll failed; the rows are then the last good ones.
    pub error: Option<String>,
    /// Sessions whose first read is still running.
    pub loading: usize,
}

/// Parse `herdr agent list` output.
pub fn parse_agents(json: &str) -> Result<Vec<Entry>, String> {
    #[derive(Deserialize)]
    struct Reply {
        result: Agents,
    }
    #[derive(Deserialize)]
    struct Agents {
        agents: Vec<Entry>,
    }
    serde_json::from_str::<Reply>(json)
        .map(|r| r.result.agents)
        .map_err(|e| format!("unexpected reply from herdr: {e}"))
}

fn parse_workspaces(json: &str) -> Vec<Workspace> {
    #[derive(Deserialize)]
    struct Reply {
        result: Spaces,
    }
    #[derive(Deserialize)]
    struct Spaces {
        workspaces: Vec<Workspace>,
    }
    serde_json::from_str::<Reply>(json)
        .map(|r| r.result.workspaces)
        .unwrap_or_default()
}

/// The Herdr binary: the one Herdr says it is, else `herdr` on PATH.
pub fn herdr_bin() -> String {
    std::env::var("HERDR_BIN_PATH")
        .ok()
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| "herdr".to_string())
}

/// Run `herdr <args>` and return stdout. Fixed argv, no shell; killed if it
/// outlives [`HERDR_TIMEOUT`], so a wedged Herdr can't wedge the poller.
pub async fn herdr(args: &[&str]) -> Result<String, String> {
    let run = Command::new(herdr_bin())
        .args(args)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(HERDR_TIMEOUT, run).await {
        Err(_) => Err("herdr did not answer".to_string()),
        Ok(Err(e)) => Err(format!("cannot run herdr: {e}")),
        Ok(Ok(out)) if out.status.success() => {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        }
        Ok(Ok(out)) => Err(format!(
            "herdr failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}

/// Where rows come from.
pub enum Source {
    /// A running Herdr.
    Herdr,
    /// Made-up rows, one frame per poll, for recordings and tests. A frame is
    /// the JSON list of [`Row`]s.
    Demo { frames: Vec<Vec<Row>>, next: usize },
}

impl Source {
    /// Load a demo file: `{"frames": [[row, …], …]}`.
    pub fn demo(path: &std::path::Path) -> Result<Source, String> {
        #[derive(Deserialize)]
        struct File {
            frames: Vec<Vec<Row>>,
        }
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let file: File = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        if file.frames.is_empty() {
            return Err("the demo file has no frames".to_string());
        }
        Ok(Source::Demo {
            frames: file.frames,
            next: 0,
        })
    }
}

/// Panes that newly need attention between two snapshots.
fn newly_attention<'a>(before: &[Row], now: &'a [Row]) -> Vec<&'a Row> {
    now.iter()
        .filter(|r| r.urgency() <= 1)
        .filter(|r| !before.iter().any(|b| b.pane == r.pane && b.urgency() <= 1))
        .collect()
}

async fn notify(row: &Row) {
    let why = match (&row.stuck, row.status.as_str()) {
        (_, "blocked") => "waiting on you".to_string(),
        (Some(s), _) => format!("stuck on {s}"),
        _ => return,
    };
    let body = format!("{} {why}", row.title);
    let _ = herdr(&[
        "notification",
        "show",
        "avocado-trope",
        "--body",
        &body,
        "--sound",
        "none",
    ])
    .await;
}

/// The background task: poll, fold, publish. Ends when the UI goes away.
pub async fn poll(
    mut source: Source,
    interval: Duration,
    notifying: bool,
    tx: latest::Sender<Snapshot>,
) {
    let mut watchers = Some(watch::Watchers::default());
    let mut spaces: Vec<Workspace> = Vec::new();
    let mut polls = 0u64;
    let mut seen_first = false;
    let mut before: Vec<Row> = Vec::new();
    loop {
        if tx.is_closed() {
            return;
        }
        let snapshot = match &mut source {
            Source::Demo { frames, next } => {
                let rows = frames[*next % frames.len()].clone();
                *next += 1;
                Snapshot {
                    rows,
                    error: None,
                    loading: 0,
                }
            }
            Source::Herdr => {
                // Workspace names rarely change; ask now and then.
                if polls.is_multiple_of(15)
                    && let Ok(json) = herdr(&["workspace", "list"]).await
                {
                    spaces = parse_workspaces(&json);
                }
                polls += 1;
                match herdr(&["agent", "list"])
                    .await
                    .and_then(|j| parse_agents(&j))
                {
                    Err(error) => {
                        let rows = tx.borrow().rows.clone();
                        Snapshot {
                            rows,
                            error: Some(error),
                            loading: 0,
                        }
                    }
                    Ok(entries) => {
                        let mut w = watchers.take().unwrap_or_default();
                        let space = spaces
                            .iter()
                            .map(|s| {
                                (
                                    s.workspace_id.clone(),
                                    (
                                        s.label.clone().unwrap_or_default(),
                                        s.number.unwrap_or(u32::MAX),
                                    ),
                                )
                            })
                            .collect();
                        let progress = tx.clone();
                        let done = tokio::task::spawn_blocking(move || {
                            let rows =
                                w.update(&entries, &space, chrono::Utc::now(), |rows, loading| {
                                    // A first read can take a while on a big
                                    // transcript: show what's ready as it lands.
                                    progress.send_replace(Snapshot {
                                        rows: rows.to_vec(),
                                        error: None,
                                        loading,
                                    });
                                });
                            (w, rows)
                        })
                        .await;
                        match done {
                            Ok((w, rows)) => {
                                watchers = Some(w);
                                Snapshot {
                                    rows,
                                    error: None,
                                    loading: 0,
                                }
                            }
                            Err(e) => Snapshot {
                                rows: Vec::new(),
                                error: Some(format!("reading sessions failed: {e}")),
                                loading: 0,
                            },
                        }
                    }
                }
            }
        };
        if notifying && seen_first {
            for row in newly_attention(&before, &snapshot.rows) {
                notify(row).await;
            }
        }
        seen_first = true;
        before = snapshot.rows.clone();
        tx.send_if_modified(|current| {
            let changed = *current != snapshot;
            if changed {
                *current = snapshot;
            }
            changed
        });
        tokio::time::sleep(interval).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(pane: &str, status: &str, stuck: bool) -> Row {
        Row {
            pane: pane.into(),
            status: status.into(),
            stuck: stuck.then(|| "Bash".to_string()),
            ..Row::default()
        }
    }

    #[test]
    fn reads_herdr_agent_list() {
        let json = r#"{"id":"x","result":{"agents":[
            {"agent":"claude","agent_session":{"value":"abc"},"agent_status":"working",
             "pane_id":"w2:p1","workspace_id":"w2","foreground_cwd":"/tmp/x","new_field":1},
            {"pane_id":"w2:p2"}]}}"#;
        let agents = parse_agents(json).unwrap();
        assert_eq!(agents.len(), 2);
        assert_eq!(
            agents[0].agent_session.as_ref().unwrap().value.as_deref(),
            Some("abc")
        );
        assert_eq!(agents[1].agent, None);
    }

    #[test]
    fn a_herdr_error_is_an_error_not_an_empty_fleet() {
        assert!(parse_agents(r#"{"error":{"code":"x"}}"#).is_err());
        assert!(parse_agents("not json").is_err());
    }

    #[test]
    fn waiting_on_you_outranks_stuck_outranks_working() {
        assert!(row("a", "blocked", false).urgency() < row("b", "working", true).urgency());
        assert!(row("a", "working", true).urgency() < row("b", "working", false).urgency());
        assert!(row("a", "working", false).urgency() < row("b", "idle", false).urgency());
    }

    #[test]
    fn notifies_once_per_transition() {
        let idle = [row("a", "working", false)];
        let stuck = [row("a", "working", true)];
        assert_eq!(newly_attention(&idle, &stuck).len(), 1);
        assert_eq!(newly_attention(&stuck, &stuck).len(), 0);
        assert_eq!(newly_attention(&stuck, &idle).len(), 0);
    }
}
