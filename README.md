<p align="center"><img src="docs/avocado-trope-icon.svg" width="128" alt="avocado-trope icon: a cut avocado whose pit is a small flow graph"></p>

<h1 align="center">avocado-trope</h1>

<p align="center">
  <a href="https://github.com/andymartinezot/avocado-trope/releases/latest"><img src="https://img.shields.io/github/v/release/andymartinezot/avocado-trope" alt="Latest release"></a>
  <a href="https://github.com/furkankly/zoetrope"><img src="https://img.shields.io/badge/fork%20of-zoetrope-d7af00" alt="Fork of zoetrope"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue" alt="MIT license"></a>
  <a href="https://github.com/andymartinezot/avocado-trope"><img src="https://img.shields.io/github/stars/andymartinezot/avocado-trope?style=social" alt="GitHub stars"></a>
</p>

**Watch your Claude Code and Codex agents: one session as a live flow graph, or every agent in [herdr](https://herdr.dev) on one screen.**
See who is working, who is stuck, what failed and how full each context is, without opening each pane.

> **This is a fork.** avocado-trope is built on [zoetrope](https://github.com/furkankly/zoetrope) by
> [furkankly](https://github.com/furkankly), which does the hard part: reading the transcripts and
> drawing the graph. The intention here is to improve on it: a clearer look, more of what a session
> records on screen, and tools for running many agents at once. All credit for the original goes
> to its author. See [What this fork adds](#what-this-fork-adds). It's a standalone repository
> rather than a GitHub fork only because herdr's plugin marketplace skips forks.

| | |
|---|---|
| **Session graph** | The main agent, the agents it spawned and their tools, live or replayed, with a summary sidebar and a session log |
| **Fleet** | Every herdr agent on one screen: status, model, context, last tool, failures, files. Jump to a pane or open its graph |
| **Warnings** | Stuck tool calls, agents waiting on you, and context windows filling up |
| **Compare** | Two sessions side by side on one clock |
| **Export** | A session as JSON or Markdown, for scripts and pull requests |
| **Agents** | Claude Code and Codex |
| **Runs on** | macOS and Linux. The graph needs only a terminal; the fleet needs herdr 0.9.3+ |

---

## Contents

- [What this fork adds](#what-this-fork-adds)
- [Demo](#demo)
- [Quick start](#quick-start)
- [The session graph](#the-session-graph)
- [Fleet](#fleet)
- [Compare two sessions](#compare-two-sessions)
- [Export a session](#export-a-session)
- [Themes](#themes)
- [How it works](#how-it-works)
- [CLI reference](#cli-reference)
- [Safety](#safety)
- [State and privacy](#state-and-privacy)
- [Troubleshooting](#troubleshooting)
- [Develop](#develop)
- [Credits](#credits)

---

## What this fork adds

zoetrope reads a session's transcript and draws it as a graph you can replay and scrub. Everything
below is added on top of that.

| Area | Added here |
|---|---|
| Look | Cards colored by role, the model on every card, a header with a legend, 7 themes |
| Session detail | Context in use, a session sidebar, a multi-row session log with search, the session's own name as its title |
| Failures | The error text of failed calls, and a lens that shows only failures |
| Files | Which files each agent changed, and which it created |
| Warnings | Stuck tool calls, and context use colored as a known window fills |
| Many agents | The fleet view, with herdr notifications |
| Analysis | Side-by-side comparison, JSON and Markdown export |

The browser build from zoetrope is still in the repository, but this fork doesn't work on it: the
new keys and screens are terminal only.

---

## Demo

![The session graph replaying a session: sidebar, session log, the failure lens, search, the detail panel and themes](docs/avocado-trope-demo.gif)

In the recording:

1. A session replays. The sidebar sums it up and the log lists each tool call as it lands.
2. `e` shows only failures, with what each one reported.
3. `/` searches the log.
4. Selecting an agent opens its detail panel.
5. `t` switches theme.

![The fleet: agents grouped by workspace, filtering, sorting and opening a graph](docs/avocado-trope-fleet.gif)

Agents change state as it runs: one starts waiting on you, one gets stuck, one's context nears its
limit. Then `a` keeps only who needs you, `s` changes the sort, `/` filters, and `g` opens a graph.

![Two sessions replaying side by side](docs/avocado-trope-compare.gif)

Two sessions replay on one clock, with their totals on top.

All recordings use the demo sessions in [`assets/`](assets), so they never show real work.

---

## Quick start

### With herdr

```sh
herdr plugin install andymartinezot/avocado-trope/herdr-plugin
herdr integration install claude    # so herdr knows each pane's session id
herdr integration install codex     # optional, for Codex
```

The install downloads a checksum-verified binary for macOS or Linux (arm64 or amd64). If it can't,
it builds from source when [Rust](https://rustup.rs) is installed.

> **What you're trusting:** release binaries are built by GitHub Actions from this repository
> (actions pinned to commit SHAs). The install downloads over HTTPS and checks the sha256 published
> with the release, which catches corrupted downloads; the trust root is GitHub and this repository.
> Prefer to check first? Read [`herdr-plugin/herdr/fetch-or-build.sh`](herdr-plugin/herdr/fetch-or-build.sh).

Then add keys to `~/.config/herdr/config.toml`, using keys that are free in your setup:

```toml
[[keys.command]]
key = "prefix+shift+a"
type = "shell"
command = "herdr plugin pane open --plugin andymartinezot.avocado-trope --entrypoint graph --placement zoomed"
description = "avocado-trope: session graph"

[[keys.command]]
key = "prefix+shift+e"
type = "shell"
command = "herdr plugin pane open --plugin andymartinezot.avocado-trope --entrypoint fleet --placement zoomed"
description = "avocado-trope: every agent"
```

Run `herdr server reload-config` after adding them. `q` closes either screen.

### Without herdr

The session graph, compare and export need only a terminal:

```sh
curl -fsSL https://raw.githubusercontent.com/andymartinezot/avocado-trope/main/scripts/install.sh | sh
```

This installs the latest release into `~/.local/bin` (set `INSTALL_DIR` to change that), after
checking its checksum. Or download a binary from
[Releases](https://github.com/andymartinezot/avocado-trope/releases), or build from source with
`cargo install --git https://github.com/andymartinezot/avocado-trope`.

```sh
avocado-trope                 # follow the live session of the current folder
avocado-trope <id>            # replay a session by id, or a unique prefix of one
avocado-trope <file.jsonl>    # replay a session file
```

---

## The session graph

| Part | Shows |
|---|---|
| **Header** | The session's name, the main model, and a legend |
| **Cards** | Border color is the role, the glyph on it is the state. Model, description, last tool, context and output tokens |
| **Sidebar** | Elapsed time, agents by state, tool calls and failures, context, output, files changed, stuck agents, your last prompt, and the main keys. Inside herdr it also lists the keys you bound to avocado-trope, read from your config |
| **Session log** | The latest tool calls and prompts across all agents, in windows 30 rows tall or more |
| **Timeline** | Scrub anywhere. The sidebar and the log rewind with it |

| Key | Action |
|---|---|
| `space` | Pause or resume |
| `[` `]` | Previous or next prompt |
| `g` / `End` | Go live |
| `s` | Skip idle gaps on or off |
| `tab`, arrows, click | Select an agent and open its detail panel |
| `o` / `f` / `r` | Overview / follow the active agent / rearrange |
| `e` | Log: failures only, showing what each one reported |
| `/` | Log: search. `enter` keeps the filter, `esc` clears it |
| `b` / `B` | Sidebar on or off / pin it in narrow windows |
| `t` | Next theme |
| `i` / `?` | Session details / all keys |
| `q` | Quit |

### Warnings

- **Stuck:** an agent with a tool call open and no new records for 5 minutes (`--stuck-after`) gets
  a `⚠` on its card, naming the tool. A call that spawned an agent doesn't count, because it stays
  open for that agent's whole life. Neither does one left open for more than a day: that's an
  interrupted session, not a hang.
- **Context:** the figure turns yellow at 70% of the window and red at 90%.

The percentage needs the window size. Codex records it. Claude Code doesn't, and a model id doesn't
tell you, so avocado-trope won't guess: it shows the token count alone until you set
`--context-window`, or until a session passes 200k, which proves a 1M window.

---

## Fleet

```sh
avocado-trope fleet
```

One row per agent, grouped by workspace in herdr's order. Inside a workspace, whoever needs you comes
first: waiting on you, then stuck, working, done and idle.

| Key | Action |
|---|---|
| `↑` `↓` `j` `k` | Move. A click selects, a second click jumps |
| `enter` | Jump to the agent's pane in herdr |
| `g` | Open its session graph. `q` comes back |
| `a` | Only agents that need you |
| `s` | Sort by urgency, last activity or context |
| `/` | Filter by name, workspace or folder |
| `t` / `?` / `q` | Next theme / help / quit |

Columns drop out as the window narrows, least useful first. With `--notify`, herdr shows a
notification when an agent starts waiting on you or gets stuck, once per change. `avocado-trope fleet --json`
prints one snapshot for scripts.

In the fleet, only an agent herdr reports as working can be stuck, and one with no open call counts
too when its transcript has been quiet for the `--stuck-after` time. That can be a long think rather
than a hang, so treat `⚠` as "take a look".

### Cost

Measured on a laptop, in a 150×45 terminal, polling every 2 seconds:

| Fleet | First read | CPU | Memory | Key response |
|---|---|---|---|---|
| 10 real agents, about 100 MB of transcripts | 0.8 s | 0.4% | under 100 MB, flat | 2 ms |
| 100 agents, 1 GB of transcripts, quiet | 3.5 s | 1.1% | 139 MB, flat | 2 ms |
| 100 agents, 1 GB, 25 of them writing 890 lines/s | 3.5 s | 1.4% | 376 MB, grows with the data | 2 ms |

The screen works during the first read: rows appear as each session finishes.

---

## Compare two sessions

```sh
avocado-trope compare <session-a> <session-b> --speed 8
```

Both start from their first record at the same speed, and wait until both are loaded. At any moment
each side shows the same time since it began, so you can see which got there first and at what cost.
Useful for the same task on two models. The bar on top shows each side's elapsed time, agents, tools,
failures and output.

`space`, `[` `]`, `g`, `b`, `e` and `/` act on both sides. Selecting and panning act on the focused
side, marked `▸`: `w` swaps it, or click. `s` is off here, since skipping idle gaps would break the
shared clock.

---

## Export a session

```sh
avocado-trope inspect <id>                       # readable tree
avocado-trope inspect --json <id>                # agents, tools, failures, files, prompts
avocado-trope inspect --markdown <id> > run.md   # a summary to paste into a pull request
```

Exports hold what the session holds, including prompts, file paths and error text, which can contain
secrets. Read them before sharing.

---

## Themes

`dracula` (default), `catppuccin`, `tokyo-night`, `gruvbox`, `nord`, `classic` (the original zoetrope
look) and `terminal` (your terminal's own colors, which also suits light themes).

Press `t` to switch; your choice is remembered. Otherwise the first of these that applies wins:

1. `--theme <name>`
2. `$AVOCADO_TROPE_THEME`
3. The theme you last picked with `t`
4. herdr's theme, when running inside a herdr pane
5. `dracula`

Names from other tools work too (`catppuccin-mocha`, `tokyo-night-storm`), and light variants map to
`terminal`. `avocado-trope --theme list` prints the names.

---

## How it works

Claude Code and Codex write a transcript for every session. Each format enters through one boundary,
[`src/provider/`](src/provider), which turns records into format-free facts ([`src/fact.rs`](src/fact.rs)).
A fold builds the session model from those facts, and the graph, sidebar, log, fleet and exports all
read that model.

The fleet asks herdr who is running (`herdr agent list`, a few milliseconds), parses each transcript
once and then reads only what was appended. That work runs on a background task that publishes
finished rows, and the screen redraws only when a row changes.

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) and [`docs/DESIGN.md`](docs/DESIGN.md), both from
zoetrope, for the design of the core.

---

## CLI reference

| Command | Does |
|---|---|
| `avocado-trope` | Follow the current folder's live session |
| `avocado-trope <file\|id\|dir>` | Replay a file or session, or follow another folder |
| `avocado-trope fleet` | Every herdr agent on one screen |
| `avocado-trope compare <a> <b>` | Two sessions side by side |
| `avocado-trope inspect <file\|id\|dir>` | Print a session, no TUI |

| Flag | Applies to | Does |
|---|---|---|
| `--follow` | `avocado-trope` | Start at the live edge instead of the beginning |
| `--speed N` | `avocado-trope`, `compare` | Replay speed (default 8) |
| `--provider <name>` | `avocado-trope`, `inspect` | Force `claude` or `codex` |
| `--theme <name\|list>` | all | Color theme, or print the names |
| `--stuck-after N` | `avocado-trope`, `fleet`, `compare` | Minutes before an open tool call is stuck (default 5) |
| `--context-window N` | all | Window in tokens, for agents whose format doesn't state one |
| `--interval N` | `fleet` | Seconds between polls (default 2) |
| `--notify` | `fleet` | Notify through herdr when an agent starts needing you |
| `--json` | `fleet`, `inspect` | Print JSON and exit |
| `--markdown` | `inspect` | Print Markdown and exit |
| `--demo <file>` | `fleet` | Made-up rows from a file, for recordings |

---

## Safety

- **Never touches your sessions:** transcripts are only read. It can't type into an agent or change
  a pane. `enter` only moves focus.
- **No network:** nothing in the program opens a connection, and there is no telemetry.
- **Only starts two programs:** `avocado-trope fleet` runs `herdr`, and `g` runs `avocado-trope` itself. Both use fixed
  arguments with no shell, and herdr calls are killed after 5 seconds. Session text never reaches a
  shell; a notification's text is passed as a single argument.
- **Cleans what it shows:** transcripts can hold anything, so control characters are dropped before
  they reach your terminal.
- **Verified downloads:** both installers check the release's sha256 and won't install a mismatch.

---

## State and privacy

avocado-trope reads `~/.claude/projects`, `~/.codex/sessions` and, inside herdr, herdr's config: the
theme, and the keys that open avocado-trope.

The only file it writes is `~/.config/avocado-trope/theme` (or `$XDG_CONFIG_HOME/avocado-trope`),
holding the name of the theme you picked. Session content stays on your machine. Prompts and tool
output do appear on screen, so mind what you share while it's open.

---

## Troubleshooting

| Symptom | Fix |
|---|---|
| The fleet says it can't run herdr | Install herdr, or set `HERDR_BIN_PATH` to it |
| A fleet row has no model or context | herdr doesn't know that pane's session yet. Run `herdr integration install claude`, then restart that agent |
| No context percentage | Claude Code doesn't record the window. Pass `--context-window 1000000` |
| An agent shows `⚠` but is fine | A long command or a long think. Raise `--stuck-after` |
| The session log is one line | The window is under 30 rows. `e` and `/` still open a taller log from 20 rows |
| The sidebar is missing | The window is under 110 columns, or a detail panel is open. `B` pins it down to 60 |
| The theme ignores herdr | You picked one with `t`, which wins. Delete `~/.config/avocado-trope/theme` |
| A linked clone shows the old look | Its panes run `target/release/avocado-trope` from that clone: run `cargo build --release` |

---

## Develop

```sh
cargo test
cargo clippy --all-targets
cargo fmt
cargo run -- assets/claude/demo.jsonl
cargo run -- fleet --demo assets/fleet/demo.json   # made-up agents, no herdr needed
UPDATE_GOLDEN=1 cargo test                         # after an intended change to parsing
vhs scripts/demo/fleet.tape                        # re-record a GIF (brew install vhs)
```

To work on the plugin from a clone, run `herdr plugin link herdr-plugin`: its panes then use your
`target/release` build.

Releases are cut by pushing a `vX.Y.Z` tag that matches `version` in both `Cargo.toml` and
`herdr-plugin/herdr-plugin.toml`. GitHub Actions then publishes the binaries and `SHA256SUMS`.

Commits follow [Conventional Commits](https://www.conventionalcommits.org). To bring in upstream
work: `git remote add upstream https://github.com/furkankly/zoetrope`, then `git fetch upstream` and
merge.

---

## Credits

avocado-trope exists because of [zoetrope](https://github.com/furkankly/zoetrope) by
[furkankly](https://github.com/furkankly), and the [rataflow](https://github.com/furkankly/rataflow)
and [ratatui](https://ratatui.rs) libraries it stands on. If you only want the session graph, or
want it in your browser, use the [original](https://zoetrope.furkankly.dev).

## License

MIT, like zoetrope. See [LICENSE](LICENSE).
