//! The session at a glance: the left sidebar (title, model, time, agents,
//! tools, context, last prompt) and the multi-row session log in the timeline
//! panel (the latest tool calls and prompts across every agent).
//!
//! Both read the session model as folded up to the playhead, so scrubbing back
//! rewinds them with the graph.

use chrono::{DateTime, Utc};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Padding, Paragraph, Wrap};

use crate::fact::AgentKind;
use crate::state::App;
use crate::state::session::{AgentInfo, AgentStatus, ToolState};
use crate::ui::nodes::fmt_tokens;
use crate::ui::truncate;

/// Sidebar width in cells.
pub(crate) const SIDEBAR_WIDTH: u16 = 32;

/// Compact elapsed time: `42s`, `3m 20s`, `1h 05m`.
fn fmt_duration(d: chrono::Duration) -> String {
    let s = d.num_seconds().max(0);
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m {:02}s", s / 60, s % 60)
    } else {
        format!("{}h {:02}m", s / 3600, (s % 3600) / 60)
    }
}

/// Context in use, with the share of the window when the format states it.
pub(crate) fn fmt_context(used: u64, window: Option<u64>) -> String {
    match window.filter(|w| *w > 0) {
        Some(w) => format!(
            "{} / {} · {}%",
            fmt_tokens(used),
            fmt_tokens(w),
            (used.saturating_mul(100) / w).min(100)
        ),
        None => fmt_tokens(used),
    }
}

fn agent_label(a: &AgentInfo) -> &str {
    a.agent_type.as_deref().unwrap_or(a.kind.default_label())
}

/// The left sidebar. Draws nothing when `area` is too small to be useful.
pub(crate) fn render_sidebar(frame: &mut Frame, area: Rect, app: &App) {
    if area.width < 16 || area.height < 8 {
        return;
    }
    let look = crate::theme::current();
    let palette = app.flow.theme.palette();
    let bg = Style::default().bg(palette.surface);
    let label = bg.fg(palette.subtle);
    let value = bg.fg(palette.text);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(bg.fg(palette.muted))
        .style(bg)
        .padding(Padding::horizontal(1))
        .title(Line::from(Span::styled(
            " session ",
            bg.fg(palette.subtle).add_modifier(Modifier::BOLD),
        )));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let w = inner.width as usize;

    let session = &app.session;
    let main = session.agent("main");

    let mut lines: Vec<Line> = Vec::new();

    let title = app
        .session_info
        .title
        .as_deref()
        .or_else(|| session.first_prompt())
        .unwrap_or("session");
    lines.push(Line::from(Span::styled(
        truncate(title, w * 2),
        value.add_modifier(Modifier::BOLD),
    )));
    if let Some(model) = main.and_then(|a| a.model.as_deref()) {
        lines.push(Line::from(Span::styled(
            crate::theme::short_model(model),
            bg.fg(look.role(AgentKind::Main))
                .add_modifier(Modifier::BOLD),
        )));
    }
    lines.push(Line::from(""));

    let row = |k: &str, v: String, style: Style| {
        Line::from(vec![
            Span::styled(format!("{k:<11}"), label),
            Span::styled(truncate(&v, w.saturating_sub(11)), style),
        ])
    };

    // Elapsed: the root agent's first record to now. At a live edge "now" is
    // the wall clock, so the figure keeps ticking through a long tool call;
    // anywhere else it is the playhead, so scrubbing back rewinds it.
    let last: Option<DateTime<Utc>> = session.agents.values().filter_map(|a| a.last_ts).max();
    let end = app.timeline.now_reference().or(last);
    if let (Some(start), Some(end)) = (main.and_then(|a| a.first_ts), end) {
        lines.push(row("elapsed", fmt_duration(end - start), value));
    }

    // Agents by state.
    let (mut running, mut idle, mut done, mut failed) = (0, 0, 0, 0);
    for a in session.agents.values() {
        match a.status {
            AgentStatus::Running => running += 1,
            AgentStatus::Idle | AgentStatus::Stopped => idle += 1,
            AgentStatus::Done => done += 1,
            AgentStatus::Failed => failed += 1,
        }
    }
    lines.push(Line::from(vec![
        Span::styled(format!("{:<11}", "agents"), label),
        Span::styled(format!("{} ", session.agent_count()), value),
        Span::styled(
            format!("{} {running} ", AgentStatus::Running.glyph()),
            bg.fg(look.status(AgentStatus::Running)),
        ),
        Span::styled(
            format!("{} {idle} ", AgentStatus::Idle.glyph()),
            bg.fg(look.status(AgentStatus::Idle)),
        ),
        Span::styled(
            format!("{} {done} ", AgentStatus::Done.glyph()),
            bg.fg(look.status(AgentStatus::Done)),
        ),
        Span::styled(
            format!("{} {failed}", AgentStatus::Failed.glyph()),
            bg.fg(look.status(AgentStatus::Failed)),
        ),
    ]));

    // Tool calls, and how many came back as errors.
    let errors = session
        .agents
        .values()
        .flat_map(|a| a.tool_calls())
        .filter(|t| t.state == ToolState::Err)
        .count();
    let mut tools = vec![
        Span::styled(format!("{:<11}", "tools"), label),
        Span::styled(format!("{} ", session.tool_count()), value),
    ];
    if errors > 0 {
        tools.push(Span::styled(
            format!("{} {errors} failed", AgentStatus::Failed.glyph()),
            bg.fg(palette.error),
        ));
    }
    lines.push(Line::from(tools));

    if let Some(a) = main.filter(|a| a.context_tokens > 0) {
        lines.push(row(
            "context",
            fmt_context(a.context_tokens, a.context_window),
            value,
        ));
    }
    let output: u64 = session.agents.values().map(|a| a.output_tokens).sum();
    if output > 0 {
        lines.push(row("output", format!("{} tok", fmt_tokens(output)), value));
    }
    // What the provider counted (file edits, queued prompts, …).
    for (name, n) in &app.session_info.tallies {
        lines.push(row(name, n.to_string(), value));
    }

    // The prompt the session is working on.
    if let Some(prompt) = session.prompts.back() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled("last prompt", label)));
        lines.push(Line::from(Span::styled(
            format!("» {}", prompt.excerpt),
            bg.fg(palette.accent),
        )));
    }

    frame.render_widget(
        Paragraph::new(lines).style(bg).wrap(Wrap { trim: false }),
        inner,
    );
}

/// One row of the session log.
struct LogRow<'a> {
    ts: Option<DateTime<Utc>>,
    agent: Option<&'a AgentInfo>,
    /// Tool name, or `None` for a prompt.
    tool: Option<&'a str>,
    text: &'a str,
    state: Option<ToolState>,
}

/// The latest `rows.height` events across every agent, newest at the bottom:
/// `time  agent  tool  summary  mark`. Prompts read `◆ text`.
pub(crate) fn render_session_log(frame: &mut Frame, rows: Rect, app: &App) {
    if rows.width < 20 || rows.height == 0 {
        return;
    }
    let look = crate::theme::current();
    let palette = app.flow.theme.palette();
    let bg = Style::default().bg(palette.surface);
    let n = rows.height as usize;

    let mut events: Vec<LogRow> = Vec::new();
    for a in app.session.agents.values() {
        for t in a.tool_calls() {
            events.push(LogRow {
                ts: t.ts,
                agent: Some(a),
                tool: Some(&t.name),
                text: t.summary.as_deref().unwrap_or(""),
                state: Some(t.state),
            });
        }
    }
    for p in app.session.prompts.iter() {
        events.push(LogRow {
            ts: p.ts,
            agent: None,
            tool: None,
            text: &p.excerpt,
            state: None,
        });
    }
    // Undated events sort first and fall off the top.
    events.sort_by_key(|e| e.ts);
    let start = events.len().saturating_sub(n);

    let r = Rect::new(
        rows.x.saturating_add(1),
        rows.y,
        rows.width.saturating_sub(2),
        rows.height,
    );
    let agent_w = 14usize;
    let tool_w = 13usize;
    let mut lines: Vec<Line> = Vec::new();
    for e in &events[start..] {
        let time =
            e.ts.map(|t| {
                t.with_timezone(&chrono::Local)
                    .format("%H:%M:%S")
                    .to_string()
            })
            .unwrap_or_else(|| "--:--:--".to_string());
        let mut spans = vec![Span::styled(format!("{time}  "), bg.fg(palette.subtle))];
        match (e.tool, e.agent) {
            (Some(tool), Some(a)) => {
                spans.push(Span::styled(
                    format!("{:<agent_w$}", truncate(agent_label(a), agent_w - 1)),
                    bg.fg(look.role(a.kind)).add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::styled(
                    format!("{:<tool_w$}", truncate(tool, tool_w - 1)),
                    bg.fg(palette.text),
                ));
                let mark = match e.state {
                    Some(ToolState::Ok) => (" ✓", look.status(AgentStatus::Done)),
                    Some(ToolState::Err) => (" ✗", palette.error),
                    _ => (" ◐", look.status(AgentStatus::Running)),
                };
                let used = 10 + agent_w + tool_w + 2;
                let tw = (r.width as usize).saturating_sub(used);
                // Pad by display columns, not chars, so a wide-char summary
                // doesn't push the mark off the row.
                let text = truncate(e.text, tw);
                let pad = tw.saturating_sub(unicode_width::UnicodeWidthStr::width(text.as_str()));
                spans.push(Span::styled(
                    format!("{text}{}", " ".repeat(pad)),
                    bg.fg(palette.subtle),
                ));
                spans.push(Span::styled(mark.0, bg.fg(mark.1)));
            }
            _ => {
                spans.push(Span::styled(
                    "◆ ",
                    bg.fg(palette.accent).add_modifier(Modifier::BOLD),
                ));
                let tw = (r.width as usize).saturating_sub(12);
                spans.push(Span::styled(truncate(e.text, tw), bg.fg(palette.text)));
            }
        }
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines).style(bg), r);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(fmt_duration(chrono::Duration::seconds(42)), "42s");
        assert_eq!(fmt_duration(chrono::Duration::seconds(200)), "3m 20s");
        assert_eq!(fmt_duration(chrono::Duration::seconds(3900)), "1h 05m");
    }

    #[test]
    fn context_with_and_without_window() {
        assert_eq!(fmt_context(84_000, None), "84.0k");
        assert_eq!(fmt_context(95_000, Some(475_000)), "95.0k / 475.0k · 20%");
    }
}
