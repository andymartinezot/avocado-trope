//! Following many sessions cheaply.
//!
//! Each session is parsed once. After that a poll stats its files and reads
//! only what was appended, through the same byte tailer the graph uses, so a
//! quiet session costs a `stat` per file and a busy one costs its new lines.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};

use super::{Entry, Row};
use crate::provider::{self, Provider, ReadMode, Session, SessionFile, Stream, Target};
use crate::state::SessionInfo;
use crate::state::session::{AgentKind, AgentStatus, SessionModel, ToolState};
use crate::tailer::bytes::{ReadResult, TailState, read_appended};

/// How often a session is checked for new subagent files.
const RESCAN: Duration = Duration::from_secs(5);
/// How long to wait before looking again for a session with no transcript yet.
const RETRY_OPEN: Duration = Duration::from_secs(10);

struct Tail {
    state: TailState,
    stream: Stream,
}

/// One followed session.
pub struct Watched {
    session: Session,
    tails: HashMap<PathBuf, Tail>,
    whole_read: HashSet<PathBuf>,
    model: SessionModel,
    info: SessionInfo,
    last_rescan: Instant,
    /// A file was rewritten during this pump.
    restarted: bool,
}

impl Watched {
    /// Start following a session. Nothing is read until [`pump`](Self::pump).
    pub fn new(session: Session) -> Self {
        let model = SessionModel::new(session.id.clone());
        Watched {
            session,
            tails: HashMap::new(),
            whole_read: HashSet::new(),
            model,
            info: SessionInfo::default(),
            last_rescan: Instant::now(),
            restarted: false,
        }
    }

    fn apply(model: &mut SessionModel, info: &mut SessionInfo, mut s: crate::fact::Statement) {
        for f in s.take_session_meta() {
            info.apply(&f);
        }
        for f in &s.facts {
            model.apply_fact(f);
        }
    }

    fn pump_file(&mut self, file: &SessionFile, provider: Provider) -> bool {
        match file.read {
            ReadMode::Whole => {
                if !self.whole_read.insert(file.path.clone()) {
                    return false;
                }
                let Ok(text) = std::fs::read_to_string(&file.path) else {
                    self.whole_read.remove(&file.path);
                    return false;
                };
                match provider.sidecar(file, &text) {
                    Some(s) => {
                        Self::apply(&mut self.model, &mut self.info, s);
                        true
                    }
                    None => false,
                }
            }
            ReadMode::Tail => {
                let tail = self.tails.entry(file.path.clone()).or_insert_with(|| Tail {
                    state: TailState::default(),
                    stream: provider.stream_for(file),
                });
                match read_appended(&file.path, &mut tail.state) {
                    ReadResult::Lines(lines) => {
                        for line in &lines {
                            if let Some(s) = tail.stream.push(line) {
                                Self::apply(&mut self.model, &mut self.info, s);
                            }
                        }
                        !lines.is_empty()
                    }
                    ReadResult::Reset => {
                        // Rewritten from the top: start this session over.
                        self.tails.clear();
                        self.whole_read.clear();
                        self.model = SessionModel::new(self.session.id.clone());
                        self.info = SessionInfo::default();
                        self.restarted = true;
                        true
                    }
                    ReadResult::NoChange | ReadResult::Missing => false,
                }
            }
        }
    }

    /// Read whatever is new. Returns whether anything was.
    pub fn pump(&mut self) -> bool {
        let provider = self.session.provider;
        if self.last_rescan.elapsed() >= RESCAN {
            self.session.rescan();
            self.last_rescan = Instant::now();
        }
        let files: Vec<SessionFile> = self.session.every_file().cloned().collect();
        let mut any = false;
        for file in &files {
            any |= self.pump_file(file, provider);
        }
        // Reread now, so a row never shows the emptied model.
        if std::mem::take(&mut self.restarted) {
            for file in &files {
                self.pump_file(file, provider);
            }
        }
        any
    }

    /// Fill a row from what the transcript says at `now`.
    fn fill(&mut self, row: &mut Row, now: DateTime<Utc>) {
        self.model.recompute_liveness(Some(now));
        self.model.recompute_stuck(Some(now));
        let model = &self.model;
        let main = model.agent("main");
        row.model = main.and_then(|a| a.model.clone());
        if let Some(a) = main {
            row.context_tokens = a.context_tokens;
            row.context_window =
                crate::settings::effective_window(a.context_window, a.context_peak);
            row.last_tool = a.last_tool().map(str::to_string);
        }
        if let Some(title) = self.info.title.as_deref().or_else(|| model.first_prompt())
            && row.title.is_empty()
        {
            row.title = title.to_string();
        }
        let mut files: HashSet<&String> = HashSet::new();
        let (mut tools, mut failed, mut subs, mut subs_running) = (0, 0, 0, 0);
        let mut last = None;
        let mut stuck = None;
        for a in model.agents.values() {
            tools += a.tool_calls().len();
            failed += a.tool_calls().filter(|t| t.state == ToolState::Err).count();
            files.extend(a.files.keys());
            last = last.max(a.last_ts);
            if a.kind == AgentKind::Subagent {
                subs += 1;
                subs_running += usize::from(a.status == AgentStatus::Running);
            }
            stuck = stuck.or_else(|| a.stuck.clone());
        }
        row.tools = tools;
        row.failed = failed;
        row.subagents = subs;
        row.subagents_running = subs_running;
        row.files = files.len();
        row.age_secs = last.map(|t| (now - t).num_seconds().max(0));
        // Only a working pane can be stuck: Herdr knows an idle one has stopped,
        // whatever its transcript left open. Working yet quiet counts too.
        let working = row.status == "working";
        let quiet = row
            .age_secs
            .is_some_and(|a| a >= crate::settings::stuck_after_secs() as i64);
        row.stuck = working
            .then(|| stuck.or_else(|| quiet.then(|| "no output".to_string())))
            .flatten();
    }
}

/// Every followed session, by session id.
#[derive(Default)]
pub struct Watchers {
    sessions: HashMap<String, Watched>,
    /// Sessions with no transcript yet, and when to look again.
    missing: HashMap<String, Instant>,
}

/// `~/code/x` for a path under the home directory.
fn short_path(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path == home => "~".to_string(),
        Ok(home) if !home.is_empty() && path.starts_with(&format!("{home}/")) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_string(),
    }
}

fn base_row(e: &Entry, spaces: &HashMap<String, (String, u32)>) -> Row {
    let (workspace, workspace_order) = e
        .workspace_id
        .as_ref()
        .and_then(|id| spaces.get(id))
        .cloned()
        .unwrap_or_else(|| (e.workspace_id.clone().unwrap_or_default(), u32::MAX));
    Row {
        pane: e.pane_id.clone(),
        agent: e.agent.clone().unwrap_or_default(),
        status: e.agent_status.clone().unwrap_or_else(|| "unknown".into()),
        title: e.name.clone().unwrap_or_default(),
        workspace,
        workspace_order,
        path: short_path(
            e.foreground_cwd
                .as_deref()
                .or(e.cwd.as_deref())
                .unwrap_or(""),
        ),
        session: e.agent_session.as_ref().and_then(|s| s.value.clone()),
        ..Row::default()
    }
}

impl Watchers {
    fn open(&mut self, id: &str, agent: Option<&str>) -> bool {
        if self.sessions.contains_key(id) {
            return true;
        }
        if self
            .missing
            .get(id)
            .is_some_and(|t| t.elapsed() < RETRY_OPEN)
        {
            return false;
        }
        let only = agent.and_then(Provider::parse);
        match provider::open(&Target::Id(id.to_string()), only) {
            Ok(session) => {
                self.missing.remove(id);
                self.sessions.insert(id.to_string(), Watched::new(session));
                true
            }
            Err(_) => {
                self.missing.insert(id.to_string(), Instant::now());
                false
            }
        }
    }

    /// Bring every session in `entries` up to date and build the rows.
    /// `progress` gets the rows so far after each session's first read.
    pub fn update(
        &mut self,
        entries: &[Entry],
        spaces: &HashMap<String, (String, u32)>,
        now: DateTime<Utc>,
        mut progress: impl FnMut(&[Row], usize),
    ) -> Vec<Row> {
        let mut rows: Vec<Row> = entries.iter().map(|e| base_row(e, spaces)).collect();
        let unread = |w: &Self, r: &Row| {
            r.session.as_ref().is_some_and(|id| {
                w.sessions.get(id).is_none_or(|s| s.tails.is_empty()) && !w.missing.contains_key(id)
            })
        };
        let mut loading = rows.iter().filter(|r| unread(self, r)).count();
        if loading > 0 {
            // A session still being read shouldn't blank the ones already known.
            for row in rows.iter_mut() {
                if let Some(w) = row
                    .session
                    .as_ref()
                    .and_then(|id| self.sessions.get_mut(id))
                    && !w.tails.is_empty()
                {
                    w.fill(row, now);
                }
            }
            progress(&rows, loading);
        }
        for i in 0..rows.len() {
            let Some(id) = rows[i].session.clone() else {
                continue;
            };
            if !self.open(&id, entries[i].agent.as_deref()) {
                continue;
            }
            let Some(w) = self.sessions.get_mut(&id) else {
                continue;
            };
            let first = w.tails.is_empty();
            w.pump();
            w.fill(&mut rows[i], now);
            if first {
                loading = loading.saturating_sub(1);
                if loading > 0 {
                    progress(&rows, loading);
                }
            }
        }
        for (row, e) in rows.iter_mut().zip(entries) {
            if row.title.is_empty() {
                row.title = e.terminal_title_stripped.clone().unwrap_or_default();
            }
        }
        // Stop following what Herdr no longer reports.
        let live: HashSet<&String> = rows.iter().filter_map(|r| r.session.as_ref()).collect();
        self.sessions.retain(|id, _| live.contains(id));
        self.missing.retain(|id, _| live.contains(id));
        rows
    }

    /// How many sessions are being followed.
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const USER: &str = r#"{"type":"user","uuid":"u1","parentUuid":null,"timestamp":"2026-06-05T10:00:00.000Z","sessionId":"s","isSidechain":false,"message":{"role":"user","content":"hello"}}"#;
    const TOOL: &str = r#"{"type":"assistant","uuid":"a1","parentUuid":"u1","timestamp":"2026-06-05T10:00:05.000Z","sessionId":"s","isSidechain":false,"requestId":"r1","message":{"role":"assistant","model":"claude-opus-4-8","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"sleep 999"}}],"usage":{"input_tokens":10,"output_tokens":20,"cache_read_input_tokens":5}}}"#;

    fn watched(dir: &std::path::Path) -> (Watched, PathBuf) {
        let path = dir.join("11111111-2222-4333-8444-555555555555.jsonl");
        std::fs::write(&path, format!("{USER}\n")).unwrap();
        let session = provider::open(&Target::Path(path.clone()), Some(Provider::Claude)).unwrap();
        (Watched::new(session), path)
    }

    fn tempdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("zoe-fleet-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn reads_only_what_was_appended() {
        let dir = tempdir("append");
        let (mut w, path) = watched(&dir);
        assert!(w.pump(), "the first read finds the first line");
        assert!(!w.pump(), "an unchanged file reads nothing");

        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        writeln!(f, "{TOOL}").unwrap();
        assert!(w.pump(), "appended lines are picked up");
        assert_eq!(w.model.tool_count(), 1);
        assert!(!w.pump());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_rewritten_file_starts_over() {
        let dir = tempdir("reset");
        let (mut w, path) = watched(&dir);
        std::fs::write(&path, format!("{USER}\n{TOOL}\n")).unwrap();
        w.pump();
        assert_eq!(w.model.tool_count(), 1);
        std::fs::write(&path, format!("{USER}\n")).unwrap();
        w.pump();
        assert_eq!(w.model.tool_count(), 0, "the old tool call is gone");
        assert!(
            w.model.first_prompt().is_some(),
            "and the file is read again at once"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_row_reports_the_tool_holding_it_up() {
        let dir = tempdir("row");
        let (mut w, path) = watched(&dir);
        std::fs::write(&path, format!("{USER}\n{TOOL}\n")).unwrap();
        w.pump();
        let mut row = Row {
            status: "working".into(),
            ..Row::default()
        };
        // Ten minutes after the call started, with no result.
        let later: DateTime<Utc> = "2026-06-05T10:10:05Z".parse().unwrap();
        w.fill(&mut row, later);
        assert_eq!(row.stuck.as_deref(), Some("Bash"));
        assert_eq!(row.model.as_deref(), Some("claude-opus-4-8"));
        assert_eq!(row.context_tokens, 15);
        assert_eq!(row.tools, 1);

        // Herdr says the pane stopped: an open call in the transcript is not
        // an alarm.
        let mut idle = Row {
            status: "idle".into(),
            ..Row::default()
        };
        w.fill(&mut idle, later);
        assert_eq!(idle.stuck, None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_pane_with_no_session_still_gets_a_row() {
        let e: Entry =
            serde_json::from_str(r#"{"pane_id":"w1:p1","agent_status":"idle"}"#).unwrap();
        let mut w = Watchers::default();
        let rows = w.update(&[e], &HashMap::new(), Utc::now(), |_, _| {});
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, "idle");
        assert!(w.is_empty());
    }

    #[test]
    fn home_is_shortened() {
        if let Ok(home) = std::env::var("HOME") {
            assert_eq!(short_path(&format!("{home}/code")), "~/code");
            assert_eq!(
                short_path(&format!("{home}ice/code")),
                format!("{home}ice/code")
            );
        }
        assert_eq!(short_path("/opt/x"), "/opt/x");
    }
}
