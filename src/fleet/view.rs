//! The fleet screen: one row per agent, grouped by workspace.
//!
//! Draws only when something changed (a new snapshot, a key, a click), so an
//! idle fleet costs nothing between polls. `g` hands the terminal to a child
//! `avocado-trope` on the selected session; input is read on a thread that can be paused
//! for it, since two readers on one terminal would each lose keys.

use std::collections::BTreeMap;
use std::io::stdout;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    MouseButton, MouseEventKind,
};
use crossterm::execute;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use tokio::sync::{mpsc, watch as latest};

use super::{Row, Snapshot, herdr};
use crate::settings;
use crate::ui::nodes::fmt_tokens;
use crate::ui::truncate;

/// How rows are ordered inside a workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    /// Whoever needs you first.
    Urgency,
    /// Most recently active first.
    Activity,
    /// Fullest context first.
    Context,
}

impl Sort {
    fn next(self) -> Sort {
        match self {
            Sort::Urgency => Sort::Activity,
            Sort::Activity => Sort::Context,
            Sort::Context => Sort::Urgency,
        }
    }

    fn word(self) -> &'static str {
        match self {
            Sort::Urgency => "urgency",
            Sort::Activity => "activity",
            Sort::Context => "context",
        }
    }
}

/// One screen line of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Item {
    Heading(String, usize),
    Agent(usize),
}

/// Everything the screen shows.
pub struct View {
    pub snapshot: Snapshot,
    /// The selected pane. By id, so re-sorting doesn't move the selection to
    /// another agent under your cursor.
    selected: Option<String>,
    scroll: usize,
    sort: Sort,
    attention_only: bool,
    query: String,
    editing: bool,
    show_help: bool,
    message: Option<String>,
    /// Which list item each drawn line holds: (screen row, pane).
    hits: Vec<(u16, String)>,
    /// Lines the list had room for at the last draw.
    height: usize,
}

impl View {
    pub fn new() -> Self {
        View {
            snapshot: Snapshot::default(),
            selected: None,
            scroll: 0,
            sort: Sort::Urgency,
            attention_only: false,
            query: String::new(),
            editing: false,
            show_help: false,
            message: None,
            hits: Vec::new(),
            height: 0,
        }
    }

    fn visible(&self, r: &Row) -> bool {
        let query = self.query.to_lowercase();
        (!self.attention_only || r.needs_attention())
            && (query.is_empty()
                || format!(
                    "{} {} {} {} {}",
                    r.title, r.workspace, r.path, r.agent, r.status
                )
                .to_lowercase()
                .contains(&query))
    }

    /// The list: workspaces in Herdr's order, rows sorted inside each.
    fn items(&self) -> Vec<Item> {
        let rows = &self.snapshot.rows;
        let mut groups: BTreeMap<(u32, &str), Vec<usize>> = BTreeMap::new();
        for (i, r) in rows.iter().enumerate() {
            if self.visible(r) {
                groups
                    .entry((r.workspace_order, r.workspace.as_str()))
                    .or_default()
                    .push(i);
            }
        }
        let mut out = Vec::new();
        for ((_, name), mut members) in groups {
            members.sort_by_key(|&i| {
                let r = &rows[i];
                let key: (i64, i64) = match self.sort {
                    // Pane order inside a level: rows that swapped places on
                    // every poll couldn't be read.
                    Sort::Urgency => (r.urgency() as i64, 0),
                    Sort::Activity => (r.age_secs.unwrap_or(i64::MAX), 0),
                    Sort::Context => (-(r.context_tokens.min(i64::MAX as u64) as i64), 0),
                };
                (key, r.pane.clone())
            });
            out.push(Item::Heading(name.to_string(), members.len()));
            out.extend(members.into_iter().map(Item::Agent));
        }
        out
    }

    fn selected_row(&self) -> Option<&Row> {
        let pane = self.selected.as_ref()?;
        self.snapshot.rows.iter().find(|r| &r.pane == pane)
    }

    /// Keep the selection on a visible row.
    fn settle_selection(&mut self, items: &[Item]) {
        let rows = &self.snapshot.rows;
        let pane_at = |item: &Item| match item {
            Item::Agent(i) => Some(rows[*i].pane.clone()),
            Item::Heading(..) => None,
        };
        let present = self
            .selected
            .as_ref()
            .is_some_and(|p| items.iter().filter_map(pane_at).any(|q| &q == p));
        if !present {
            self.selected = items.iter().find_map(pane_at);
        }
    }

    fn step(&mut self, delta: i32) {
        let items = self.items();
        let rows = &self.snapshot.rows;
        let panes: Vec<&String> = items
            .iter()
            .filter_map(|it| match it {
                Item::Agent(i) => Some(&rows[*i].pane),
                Item::Heading(..) => None,
            })
            .collect();
        if panes.is_empty() {
            return;
        }
        let at = self
            .selected
            .as_ref()
            .and_then(|s| panes.iter().position(|p| *p == s))
            .unwrap_or(0) as i32;
        let next = (at + delta).clamp(0, panes.len() as i32 - 1) as usize;
        self.selected = Some(panes[next].clone());
    }
}

impl Default for View {
    fn default() -> Self {
        Self::new()
    }
}

/// What a key or click asks the loop to do.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Redraw,
    Quit,
    /// Bring this pane to the front in Herdr.
    Focus(String),
    /// Open this row's graph.
    Graph,
}

/// Apply one input event.
pub fn handle(view: &mut View, event: &Event) -> Action {
    match event {
        Event::Resize(..) => Action::Redraw,
        Event::Mouse(m) => match m.kind {
            MouseEventKind::ScrollDown => {
                view.step(1);
                Action::Redraw
            }
            MouseEventKind::ScrollUp => {
                view.step(-1);
                Action::Redraw
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let hit = view
                    .hits
                    .iter()
                    .find(|(y, _)| *y == m.row)
                    .map(|(_, p)| p.clone());
                match hit {
                    // A click selects; a click on the selection jumps to it.
                    Some(p) if view.selected.as_ref() == Some(&p) => Action::Focus(p),
                    Some(p) => {
                        view.selected = Some(p);
                        Action::Redraw
                    }
                    None => Action::None,
                }
            }
            _ => Action::None,
        },
        Event::Key(key) if key.kind != KeyEventKind::Release => {
            view.message = None;
            if view.editing {
                match key.code {
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Action::Quit;
                    }
                    KeyCode::Esc => {
                        view.query.clear();
                        view.editing = false;
                    }
                    KeyCode::Enter => view.editing = false,
                    KeyCode::Backspace => {
                        view.query.pop();
                    }
                    KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        view.query.push(c);
                    }
                    _ => {}
                }
                return Action::Redraw;
            }
            match key.code {
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::Quit,
                KeyCode::Char('q') => Action::Quit,
                KeyCode::Esc if view.show_help => {
                    view.show_help = false;
                    Action::Redraw
                }
                KeyCode::Esc => {
                    view.query.clear();
                    view.attention_only = false;
                    Action::Redraw
                }
                KeyCode::Char('?') => {
                    view.show_help = !view.show_help;
                    Action::Redraw
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    view.step(1);
                    Action::Redraw
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    view.step(-1);
                    Action::Redraw
                }
                KeyCode::PageDown => {
                    view.step(view.height.max(2) as i32 - 1);
                    Action::Redraw
                }
                KeyCode::PageUp => {
                    view.step(1 - view.height.max(2) as i32);
                    Action::Redraw
                }
                KeyCode::Char('s') => {
                    view.sort = view.sort.next();
                    Action::Redraw
                }
                KeyCode::Char('a') => {
                    view.attention_only = !view.attention_only;
                    Action::Redraw
                }
                KeyCode::Char('/') => {
                    view.editing = true;
                    Action::Redraw
                }
                KeyCode::Char('t') => {
                    let look = crate::theme::cycle();
                    #[cfg(not(test))]
                    crate::prefs::save_theme(look.id);
                    let _ = look;
                    Action::Redraw
                }
                KeyCode::Enter => view
                    .selected_row()
                    .map_or(Action::None, |r| Action::Focus(r.pane.clone())),
                KeyCode::Char('g') => Action::Graph,
                _ => Action::None,
            }
        }
        _ => Action::None,
    }
}

fn status_glyph(r: &Row) -> &'static str {
    if r.stuck.is_some() {
        return "⚠";
    }
    match r.status.as_str() {
        "working" => "●",
        "blocked" => "◆",
        "done" => "✓",
        "idle" => "◌",
        _ => "?",
    }
}

/// `42s`, `7m`, `3h`, `2d`.
pub(crate) fn fmt_age(secs: i64) -> String {
    match secs {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3600),
        s => format!("{}d", s / 86_400),
    }
}

/// Column widths for a window width; narrower windows lose the least useful.
struct Columns {
    model: usize,
    context: usize,
    tool: usize,
    counts: bool,
    path: usize,
}

fn columns(width: usize) -> Columns {
    Columns {
        model: if width >= 70 { 15 } else { 0 },
        context: if width >= 60 { 15 } else { 0 },
        tool: if width >= 96 { 20 } else { 0 },
        counts: width >= 112,
        path: if width >= 150 { 28 } else { 0 },
    }
}

fn pad(text: &str, width: usize) -> String {
    let t = truncate(text, width.saturating_sub(1));
    let w = unicode_width::UnicodeWidthStr::width(t.as_str());
    format!("{t}{}", " ".repeat(width.saturating_sub(w)))
}

fn row_line<'a>(r: &Row, selected: bool, width: usize, frame_tick: bool) -> Line<'a> {
    let look = crate::theme::current();
    let palette = look.palette;
    let base = if selected {
        Style::default().bg(palette.muted)
    } else {
        Style::default().bg(palette.canvas_bg)
    };
    let cols = columns(width);
    let glyph_color = if r.stuck.is_some() {
        look.roles.warn
    } else {
        match r.status.as_str() {
            "working" => look.roles.running,
            "blocked" => palette.accent,
            "done" => look.roles.done,
            _ => palette.subtle,
        }
    };
    let glyph = if r.status == "working" && r.stuck.is_none() && frame_tick {
        "○"
    } else {
        status_glyph(r)
    };
    let fixed = 3
        + cols.model
        + cols.context
        + cols.tool
        + if cols.counts { 12 } else { 0 }
        + 6
        + cols.path;
    let title_w = width.saturating_sub(fixed).max(8);
    let title = if r.title.is_empty() {
        r.agent.as_str()
    } else {
        r.title.as_str()
    };
    let mut spans = vec![
        Span::styled(format!(" {glyph} "), base.fg(glyph_color)),
        Span::styled(
            pad(title, title_w),
            base.fg(palette.text).add_modifier(Modifier::BOLD),
        ),
    ];
    if cols.model > 0 {
        let model = r
            .model
            .as_deref()
            .map(crate::theme::short_model)
            .unwrap_or_else(|| r.agent.clone());
        spans.push(Span::styled(
            pad(&model, cols.model),
            base.fg(look.roles.main),
        ));
    }
    if cols.context > 0 {
        let window = settings::effective_window(r.context_window, 0);
        let text = match (r.context_tokens, window) {
            (0, _) => String::new(),
            (t, Some(w)) => format!("{:>3}% {}", settings::percent(t, w), fmt_tokens(t)),
            (t, None) => format!("     {}", fmt_tokens(t)),
        };
        let pressure = settings::pressure(r.context_tokens, window);
        spans.push(Span::styled(
            pad(&text, cols.context),
            base.fg(look.pressure(pressure)),
        ));
    }
    if cols.tool > 0 {
        let text = match (&r.stuck, &r.last_tool) {
            (Some(s), _) => format!("stuck: {s}"),
            (None, Some(t)) => format!("⚒ {} {t}", r.tools),
            (None, None) => String::new(),
        };
        let color = if r.stuck.is_some() {
            look.roles.warn
        } else {
            palette.subtle
        };
        spans.push(Span::styled(pad(&text, cols.tool), base.fg(color)));
    }
    if cols.counts {
        let failed = if r.failed > 0 {
            format!("✗ {}", r.failed)
        } else {
            String::new()
        };
        let files = if r.files > 0 {
            format!("✎ {}", r.files)
        } else {
            String::new()
        };
        spans.push(Span::styled(pad(&failed, 6), base.fg(palette.error)));
        spans.push(Span::styled(pad(&files, 6), base.fg(palette.subtle)));
    }
    let age = r.age_secs.map(fmt_age).unwrap_or_default();
    spans.push(Span::styled(format!("{age:>5} "), base.fg(palette.subtle)));
    if cols.path > 0 {
        spans.push(Span::styled(
            pad(&r.path, cols.path),
            base.fg(palette.subtle),
        ));
    }
    Line::from(spans)
}

/// Draw the whole screen.
pub fn draw(frame: &mut Frame, view: &mut View, pulse: bool) {
    let look = crate::theme::current();
    let palette = look.palette;
    let area = frame.area();
    let bg = Style::default().bg(palette.canvas_bg);
    frame.render_widget(Block::default().style(bg), area);
    let [head, list, detail, foot] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);

    let items = view.items();
    view.settle_selection(&items);
    let height = list.height as usize;
    view.height = height;

    let rows = &view.snapshot.rows;
    let count = |f: &dyn Fn(&Row) -> bool| rows.iter().filter(|r| f(r)).count();
    let mut top = vec![
        Span::styled(
            " avocado-trope ",
            Style::default()
                .bg(palette.accent)
                .fg(palette.canvas_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  fleet · {} agents", rows.len()),
            bg.fg(palette.text),
        ),
    ];
    for (glyph, n, color, word) in [
        (
            "◆",
            count(&|r| r.status == "blocked"),
            palette.accent,
            "waiting",
        ),
        ("⚠", count(&|r| r.stuck.is_some()), look.roles.warn, "stuck"),
        (
            "●",
            count(&|r| r.status == "working"),
            look.roles.running,
            "working",
        ),
        ("◌", count(&|r| r.status == "idle"), palette.subtle, "idle"),
    ] {
        if n > 0 {
            top.push(Span::styled(format!("   {glyph} {n}"), bg.fg(color)));
            top.push(Span::styled(format!(" {word}"), bg.fg(palette.subtle)));
        }
    }
    if view.snapshot.loading > 0 {
        top.push(Span::styled(
            format!("   reading {} sessions…", view.snapshot.loading),
            bg.fg(palette.subtle),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(top)).style(bg), head);

    // Keep the selection in view.
    if let Some(pos) = items.iter().position(|it| match it {
        Item::Agent(i) => Some(&rows[*i].pane) == view.selected.as_ref(),
        Item::Heading(..) => false,
    }) {
        if pos < view.scroll {
            // Bring its heading along when the list is scrolled back to the top.
            view.scroll = if pos <= 1 { 0 } else { pos };
        } else if pos >= view.scroll + height {
            view.scroll = pos + 1 - height;
        }
    }
    view.scroll = view.scroll.min(items.len().saturating_sub(height));

    let width = list.width as usize;
    let mut lines = Vec::with_capacity(height);
    view.hits.clear();
    for (y, item) in items.iter().skip(view.scroll).take(height).enumerate() {
        match item {
            Item::Heading(name, n) => {
                let label = if name.is_empty() { "agents" } else { name };
                let text = format!(" {label} ({n}) ");
                let used = unicode_width::UnicodeWidthStr::width(text.as_str());
                lines.push(Line::from(vec![
                    Span::styled(text, bg.fg(look.roles.group).add_modifier(Modifier::BOLD)),
                    Span::styled("─".repeat(width.saturating_sub(used)), bg.fg(palette.muted)),
                ]));
            }
            Item::Agent(i) => {
                let r = &rows[*i];
                let selected = view.selected.as_ref() == Some(&r.pane);
                view.hits.push((list.y + y as u16, r.pane.clone()));
                lines.push(row_line(r, selected, width, pulse));
            }
        }
    }
    if items.is_empty() {
        let text = if rows.is_empty() {
            " no agents running in Herdr"
        } else {
            " nothing matches the filter"
        };
        lines.push(Line::from(Span::styled(text, bg.fg(palette.subtle))));
    }
    frame.render_widget(Paragraph::new(lines).style(bg), list);

    // One line about the selection.
    let info = match (&view.message, &view.snapshot.error, view.selected_row()) {
        (Some(m), _, _) => Span::styled(format!(" {m}"), bg.fg(palette.accent)),
        (None, Some(e), _) => Span::styled(format!(" ⚠ {e}"), bg.fg(palette.error)),
        (None, None, Some(r)) => {
            let subs = if r.subagents > 0 {
                format!(
                    " · {} subagents ({} running)",
                    r.subagents, r.subagents_running
                )
            } else {
                String::new()
            };
            Span::styled(
                format!(" {} · {}{subs}", r.pane, r.path),
                bg.fg(palette.subtle),
            )
        }
        _ => Span::raw(""),
    };
    frame.render_widget(Paragraph::new(Line::from(info)).style(bg), detail);

    let mut hints = vec![Span::styled(
        format!(
            " enter jump · g graph · a attention · s sort: {} · / filter · t {} · ? help · q quit",
            view.sort.word(),
            look.id
        ),
        Style::default().bg(palette.surface).fg(palette.subtle),
    )];
    if view.editing || !view.query.is_empty() {
        let cursor = if view.editing { "▏" } else { "" };
        hints.insert(
            0,
            Span::styled(
                format!(" /{}{cursor} ", view.query),
                Style::default().bg(palette.surface).fg(palette.accent),
            ),
        );
    }
    frame.render_widget(
        Paragraph::new(Line::from(hints)).style(Style::default().bg(palette.surface)),
        foot,
    );

    if view.show_help {
        draw_help(frame, area);
    }
}

fn draw_help(frame: &mut Frame, area: Rect) {
    use ratatui::widgets::{Borders, Clear};
    let look = crate::theme::current();
    let palette = look.palette;
    let bg = Style::default().bg(palette.surface);
    let w = area.width.min(58);
    let h = area.height.min(13);
    let popup = Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    );
    frame.render_widget(Clear, popup);
    let key = bg.fg(palette.accent).add_modifier(Modifier::BOLD);
    let txt = bg.fg(palette.text);
    let rows = [
        ("↑↓ j k", "move · click selects, click again jumps"),
        ("enter", "jump to the agent's pane in Herdr"),
        ("g", "open its session graph (q comes back)"),
        ("a", "only agents that need you"),
        ("s", "sort: urgency, activity, context"),
        ("/", "filter by name, workspace, path"),
        ("t", "next theme"),
        ("esc", "clear filters"),
        ("q", "quit"),
    ];
    let mut lines = vec![Line::from("")];
    lines.extend(rows.iter().map(|(k, v)| {
        Line::from(vec![
            Span::styled(format!(" {k:<8}"), key),
            Span::styled(*v, txt),
        ])
    }));
    frame.render_widget(
        Paragraph::new(lines).style(bg).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(bg.fg(palette.accent))
                .style(bg)
                .title(" fleet "),
        ),
        popup,
    );
}

/// Run a child `avocado-trope` on this row's session, sharing the terminal.
fn open_graph(row: &Row) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut cmd = std::process::Command::new(exe);
    // The child starts fresh: hand it what this run was told.
    cmd.args(["--theme", crate::theme::current().id]);
    cmd.args([
        "--stuck-after",
        &(settings::stuck_after_secs() / 60).max(1).to_string(),
    ]);
    if let Some(tokens) = settings::context_window() {
        cmd.args(["--context-window", &tokens.to_string()]);
    }
    if let Some(file) = &row.session_file {
        cmd.arg(file);
    } else if let Some(id) = &row.session {
        cmd.arg("--follow");
        if crate::provider::Provider::parse(&row.agent).is_some() {
            cmd.args(["--provider", &row.agent]);
        }
        cmd.arg(id);
    } else {
        return Err("this agent has no session yet".to_string());
    }
    let status = cmd.status().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        // Its message went to the alternate screen with it.
        Err(format!("couldn't open a graph for {}", row.pane))
    }
}

/// The interactive loop.
pub async fn run(mut rx: latest::Receiver<Snapshot>, demo: bool) -> anyhow::Result<()> {
    let mut terminal = ratatui::init();
    execute!(stdout(), EnableMouseCapture)?;
    crate::tui::install_panic_hook();

    // Input thread. `poll` with a timeout rather than a blocking read, so it
    // can be told to leave the terminal alone while a child owns it.
    let paused = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let (key_tx, mut key_rx) = mpsc::unbounded_channel();
    {
        let (paused, stop) = (paused.clone(), stop.clone());
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if paused.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                }
                if event::poll(Duration::from_millis(50)).unwrap_or(false)
                    && let Ok(ev) = event::read()
                    && key_tx.send(ev).is_err()
                {
                    return;
                }
            }
        });
    }

    let mut view = View::new();
    view.snapshot = rx.borrow().clone();
    let mut pulse = false;
    let mut beat = tokio::time::interval(Duration::from_millis(500));
    // After a long stretch with nobody working, one pulse, not a burst of them.
    beat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let result = loop {
        let any_working = view.snapshot.rows.iter().any(|r| r.status == "working");
        if let Err(e) = terminal.draw(|f| draw(f, &mut view, pulse)) {
            break Err(e.into());
        }
        // Sleep until something can change what's on screen.
        let action = loop {
            tokio::select! {
                changed = rx.changed() => {
                    if changed.is_err() {
                        break Action::Quit;
                    }
                    view.snapshot = rx.borrow_and_update().clone();
                    break Action::Redraw;
                }
                Some(ev) = key_rx.recv() => {
                    let a = handle(&mut view, &ev);
                    if a != Action::None {
                        break a;
                    }
                }
                _ = beat.tick(), if any_working => {
                    pulse = !pulse;
                    break Action::Redraw;
                }
            }
        };
        match action {
            Action::Quit => break Ok(()),
            Action::None | Action::Redraw => {}
            Action::Focus(_) if demo => view.message = Some("jumps to the pane in Herdr".into()),
            Action::Focus(pane) => {
                view.message = match herdr(&["agent", "focus", &pane]).await {
                    Ok(_) => Some(format!("jumped to {pane}")),
                    Err(e) => Some(e),
                };
            }
            Action::Graph => {
                let Some(row) = view.selected_row().cloned() else {
                    continue;
                };
                paused.store(true, Ordering::Relaxed);
                // Let the input thread leave its poll before the child starts.
                tokio::time::sleep(Duration::from_millis(80)).await;
                let _ = execute!(stdout(), DisableMouseCapture);
                ratatui::restore();
                let outcome = tokio::task::spawn_blocking(move || open_graph(&row)).await;
                terminal = ratatui::init();
                let _ = execute!(stdout(), EnableMouseCapture);
                if let Err(e) = terminal.clear() {
                    break Err(e.into());
                }
                paused.store(false, Ordering::Relaxed);
                if let Ok(Err(e)) = outcome {
                    view.message = Some(e);
                }
            }
        }
    };
    stop.store(true, Ordering::Relaxed);
    let _ = execute!(stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, MouseEvent};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn row(pane: &str, space: &str, status: &str, age: i64) -> Row {
        Row {
            pane: pane.into(),
            workspace: space.into(),
            status: status.into(),
            title: format!("agent {pane}"),
            age_secs: Some(age),
            ..Row::default()
        }
    }

    fn key(c: char) -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE))
    }

    fn view(rows: Vec<Row>) -> View {
        let mut v = View::new();
        v.snapshot.rows = rows;
        v
    }

    fn render(v: &mut View, w: u16, h: u16) -> String {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| draw(f, v, false)).unwrap();
        let buf = t.backend().buffer().clone();
        (0..h)
            .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn groups_by_workspace_and_puts_who_needs_you_first() {
        let mut v = view(vec![
            row("a", "one", "idle", 1),
            row("b", "one", "blocked", 500),
            row("c", "two", "working", 2),
        ]);
        let out = render(&mut v, 120, 12);
        let at = |s: &str| out.find(s).unwrap_or_else(|| panic!("{s} missing:\n{out}"));
        assert!(at("agent b") < at("agent a"), "blocked sorts above idle");
        assert!(at("one (2)") < at("two (1)"));
    }

    #[test]
    fn the_selection_follows_the_agent_through_a_resort() {
        let mut v = view(vec![row("a", "w", "idle", 1), row("b", "w", "idle", 2)]);
        render(&mut v, 120, 12);
        assert_eq!(v.selected.as_deref(), Some("a"));
        // `a` starts working elsewhere in the order; the cursor stays on it.
        v.snapshot.rows[1].status = "blocked".into();
        render(&mut v, 120, 12);
        assert_eq!(v.selected.as_deref(), Some("a"));
    }

    #[test]
    fn a_vanished_agent_moves_the_selection() {
        let mut v = view(vec![row("a", "w", "idle", 1), row("b", "w", "idle", 2)]);
        render(&mut v, 120, 12);
        v.snapshot.rows.remove(0);
        render(&mut v, 120, 12);
        assert_eq!(v.selected.as_deref(), Some("b"));
    }

    #[test]
    fn attention_filter_and_search() {
        let mut v = view(vec![row("a", "w", "idle", 1), row("b", "w", "blocked", 2)]);
        handle(&mut v, &key('a'));
        let out = render(&mut v, 120, 12);
        assert!(out.contains("agent b") && !out.contains("agent a"));
        handle(
            &mut v,
            &Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        );

        handle(&mut v, &key('/'));
        for c in "agent a".chars() {
            handle(&mut v, &key(c));
        }
        // Keys typed into the search aren't commands: `a` didn't toggle.
        assert!(!v.attention_only);
        let out = render(&mut v, 120, 12);
        assert!(out.contains("agent a") && !out.contains("agent b"));
    }

    #[test]
    fn scrolling_keeps_a_long_fleet_in_view() {
        let rows: Vec<Row> = (0..60)
            .map(|i| row(&format!("p{i:02}"), "w", "idle", i))
            .collect();
        let mut v = view(rows);
        render(&mut v, 120, 14);
        for _ in 0..59 {
            handle(&mut v, &key('j'));
        }
        let out = render(&mut v, 120, 14);
        assert!(out.contains("agent p59"), "{out}");
        handle(
            &mut v,
            &Event::Key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE)),
        );
        render(&mut v, 120, 14);
        assert!(v.selected.as_deref() != Some("p59"));
    }

    #[test]
    fn a_click_selects_then_jumps() {
        let mut v = view(vec![row("a", "w", "idle", 1), row("b", "w", "idle", 2)]);
        render(&mut v, 120, 12);
        let (y, pane) = v.hits[1].clone();
        let click = Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 3,
            row: y,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(handle(&mut v, &click), Action::Redraw);
        assert_eq!(handle(&mut v, &click), Action::Focus(pane));
    }

    #[test]
    fn renders_at_any_size_without_panicking() {
        for (w, h) in [
            (1, 1),
            (10, 3),
            (40, 6),
            (61, 8),
            (97, 10),
            (151, 40),
            (400, 100),
        ] {
            let mut v = view(vec![Row {
                model: Some("claude-opus-5-5".into()),
                context_tokens: 950_000,
                context_window: Some(1_000_000),
                last_tool: Some("Bash".into()),
                stuck: Some("Bash".into()),
                title: "日本語のタイトル ".repeat(10),
                ..row("a", "ws", "working", 90_000)
            }]);
            v.show_help = true;
            render(&mut v, w, h);
        }
    }

    #[test]
    fn ages() {
        assert_eq!(fmt_age(42), "42s");
        assert_eq!(fmt_age(420), "7m");
        assert_eq!(fmt_age(7200), "2h");
        assert_eq!(fmt_age(172_800), "2d");
    }
}
