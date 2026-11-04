//! `zoe compare <a> <b>`: two sessions side by side, replaying on one clock.
//!
//! Each side is an ordinary [`App`] drawn into half the screen, started from
//! its own first record at the same speed, so at any moment both show the same
//! time since they began. That is what makes "which one got there first, and
//! at what cost" readable.
//!
//! Transport and display keys go to both sides so they stay in step; selecting
//! and panning go to the focused side (`w` swaps it, a click focuses).

use std::io::stdout;

use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
};
use crossterm::execute;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use tokio::sync::mpsc;
use tokio::time::{Duration, Instant};

use crate::handler;
use crate::state::App;
use crate::state::session::ToolState;
use crate::tailer::{TailRequest, UiEvent};
use crate::ui;

/// One side: its app and the tailer feeding it.
pub struct Side {
    pub app: App,
    pub tail: mpsc::Sender<TailRequest>,
    pub events: mpsc::Receiver<UiEvent>,
}

/// Which side a key goes to.
#[derive(Debug, PartialEq, Eq)]
enum Route {
    Both,
    Focused,
}

fn route(key: &crossterm::event::KeyEvent, editing: bool) -> Route {
    if editing {
        return Route::Both;
    }
    match key.code {
        KeyCode::Char(' ' | '[' | ']' | 'g' | 'G' | 's' | 'S' | 'b' | 'B' | 'e' | 'E' | '/')
        | KeyCode::End => Route::Both,
        _ => Route::Focused,
    }
}

/// `elapsed · tools · failed · output`, the numbers a comparison is for.
fn summary(app: &App) -> String {
    let s = &app.session;
    let failed = s
        .agents
        .values()
        .flat_map(|a| a.tool_calls())
        .filter(|t| t.state == ToolState::Err)
        .count();
    let output: u64 = s.agents.values().map(|a| a.output_tokens).sum();
    let start = s.agent("main").and_then(|a| a.first_ts);
    let end = s.agents.values().filter_map(|a| a.last_ts).max();
    let elapsed = match (start, end) {
        (Some(a), Some(b)) => {
            let secs = (b - a).num_seconds().max(0);
            format!("{}m {:02}s · ", secs / 60, secs % 60)
        }
        _ => String::new(),
    };
    format!(
        "{elapsed}{} agents · {} tools · {failed} failed · {} out",
        s.agent_count(),
        s.tool_count(),
        ui::nodes::fmt_tokens(output)
    )
}

fn draw(frame: &mut Frame, a: &mut App, b: &mut App, left_focused: bool) -> (Rect, Rect) {
    let area = frame.area();
    let [bar, body] = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(area);
    let [l, gap, r] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(body);
    let [bar_l, _, bar_r] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Fill(1),
    ])
    .areas(bar);

    let look = crate::theme::current();
    let palette = look.palette;
    let bg = Style::default().bg(palette.surface);
    for (rect, app, focused) in [(bar_l, &*a, left_focused), (bar_r, &*b, !left_focused)] {
        let mark = if focused { "▸ " } else { "  " };
        let line = Line::from(vec![
            Span::styled(mark, bg.fg(palette.accent).add_modifier(Modifier::BOLD)),
            Span::styled(summary(app), bg.fg(palette.text)),
        ]);
        frame.render_widget(Paragraph::new(line).style(bg), rect);
    }
    ui::draw_in(frame, l, a);
    ui::draw_in(frame, r, b);
    let divider = Style::default().bg(palette.canvas_bg).fg(palette.muted);
    let buf = frame.buffer_mut();
    for y in gap.y..gap.y + gap.height {
        buf[(gap.x, y)].set_symbol("│").set_style(divider);
    }
    (l, r)
}

/// Run the comparison to completion.
pub async fn run(mut left: Side, mut right: Side) -> anyhow::Result<()> {
    let mut terminal = ratatui::init();
    execute!(stdout(), EnableMouseCapture)?;
    crate::tui::install_panic_hook();

    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        use futures::StreamExt;
        let mut reader = crossterm::event::EventStream::new();
        while let Some(Ok(event)) = reader.next().await {
            if event_tx.send(event).is_err() {
                break;
            }
        }
    });

    // Real time on both sides: skipping idle gaps would pace each side by its
    // own gaps and break the shared clock.
    left.app.timeline.compress_gaps = false;
    right.app.timeline.compress_gaps = false;

    let mut tick = tokio::time::interval(Duration::from_millis(16));
    let mut last_tick = Instant::now();
    let mut last_status = Instant::now();
    let mut left_focused = true;
    let mut split = 0u16;

    let result = loop {
        let now = Instant::now();
        let elapsed = now - last_tick;
        last_tick = now;
        for app in [&mut left.app, &mut right.app] {
            let _ = app.flow.tick_auto_pan(elapsed);
            app.flow.tick_animation(elapsed);
            app.tick_camera(elapsed);
            app.tick_timeline(elapsed);
        }
        if now - last_status >= Duration::from_secs(1) {
            left.app.status_tick();
            right.app.status_tick();
            last_status = now;
        }

        if let Err(e) = terminal.draw(|f| {
            let (_, r) = draw(f, &mut left.app, &mut right.app, left_focused);
            split = r.x;
        }) {
            break Err::<(), anyhow::Error>(e.into());
        }

        tokio::select! {
            _ = tick.tick() => {}
            Some(ev) = left.events.recv() => left.app.handle_ui_event(ev),
            Some(ev) = right.events.recv() => right.app.handle_ui_event(ev),
            Some(ev) = event_rx.recv() => {
                if input(&ev, &mut left.app, &mut right.app, &mut left_focused, split) {
                    break Ok(());
                }
            }
        }
        while let Ok(ev) = left.events.try_recv() {
            left.app.handle_ui_event(ev);
        }
        while let Ok(ev) = right.events.try_recv() {
            right.app.handle_ui_event(ev);
        }
    };

    drop(left.tail);
    drop(right.tail);
    let _ = execute!(stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

/// Apply one input event. Returns whether to quit.
fn input(event: &Event, a: &mut App, b: &mut App, left_focused: &mut bool, split: u16) -> bool {
    match event {
        Event::Key(key) if key.kind != KeyEventKind::Release => {
            let editing = a.log_editing || b.log_editing;
            if !editing {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Char('Q') => return true,
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return true;
                    }
                    KeyCode::Char('w') | KeyCode::Char('W') => {
                        *left_focused = !*left_focused;
                        return false;
                    }
                    // One step for both sides, not one each.
                    KeyCode::Char('t') | KeyCode::Char('T') => {
                        let look = crate::theme::cycle();
                        a.flow.theme = look.flow_theme();
                        b.flow.theme = look.flow_theme();
                        crate::prefs::save_theme(look.id);
                        return false;
                    }
                    _ => {}
                }
            }
            match route(key, editing) {
                Route::Both => {
                    let quit = handler::handle_event(event, a);
                    handler::handle_event(event, b) || quit
                }
                Route::Focused if *left_focused => handler::handle_event(event, a),
                Route::Focused => handler::handle_event(event, b),
            }
        }
        Event::Mouse(m) => {
            let on_left = m.column < split;
            if matches!(m.kind, crossterm::event::MouseEventKind::Down(_)) {
                *left_focused = on_left;
            }
            if on_left {
                handler::handle_event(event, a)
            } else {
                handler::handle_event(event, b)
            }
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn transport_goes_to_both_selection_to_one() {
        assert_eq!(route(&key(KeyCode::Char(' ')), false), Route::Both);
        assert_eq!(route(&key(KeyCode::Char(']')), false), Route::Both);
        assert_eq!(route(&key(KeyCode::Char('e')), false), Route::Both);
        assert_eq!(route(&key(KeyCode::Tab), false), Route::Focused);
        assert_eq!(route(&key(KeyCode::Char('h')), false), Route::Focused);
        // A search is typed into both, or the two filters would drift apart.
        assert_eq!(route(&key(KeyCode::Char('h')), true), Route::Both);
    }

    #[test]
    fn w_swaps_the_focus_and_q_quits() {
        let mut a = App::new("a".into(), crate::state::Mode::Replay);
        let mut b = App::new("b".into(), crate::state::Mode::Replay);
        let mut focus = true;
        let w = Event::Key(key(KeyCode::Char('w')));
        assert!(!input(&w, &mut a, &mut b, &mut focus, 40));
        assert!(!focus, "w moves the focus");
        let q = Event::Key(key(KeyCode::Char('q')));
        assert!(input(&q, &mut a, &mut b, &mut focus, 40));
    }
}
