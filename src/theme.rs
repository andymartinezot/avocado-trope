//! Color themes: one [`Look`] per theme, picked once at launch and switchable
//! at runtime (`t`).
//!
//! A look is the rataflow [`Palette`] (the surfaces, text and feedback colors
//! every widget already reads) plus [`Roles`], the colors this app adds on
//! top: one per agent kind for card borders and titles, and the status colors.
//! Border color says *who* an agent is; the status glyph and its color say
//! *what state* it is in. Selection keeps its own accent so it never reads as
//! a role or a state.
//!
//! Portable (native + wasm): no IO here. Reading `--theme`, `$ZOE_THEME` or
//! Herdr's config happens in the native frontend, which hands a name to
//! [`set_by_name`].

use std::sync::atomic::{AtomicUsize, Ordering};

use rataflow::{Palette, Theme};
use ratatui::style::Color;

use crate::fact::AgentKind;
use crate::state::session::AgentStatus;

/// Colors this app adds on top of the rataflow palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Roles {
    /// The root agent's card.
    pub main: Color,
    /// Spawned agents' cards.
    pub subagent: Color,
    /// Workflow (group) cards.
    pub group: Color,
    /// A running agent: glyph, status word and its edge.
    pub running: Color,
    /// A finished agent's ✓.
    pub done: Color,
    /// Attention short of failure: a stuck agent, a filling context.
    pub warn: Color,
}

/// A complete theme.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Look {
    /// Name used by `--theme`, `$ZOE_THEME` and the status bar.
    pub id: &'static str,
    pub palette: Palette,
    pub roles: Roles,
}

impl Look {
    /// Border/title color for an agent kind.
    pub fn role(&self, kind: AgentKind) -> Color {
        match kind {
            AgentKind::Main => self.roles.main,
            AgentKind::Subagent => self.roles.subagent,
            AgentKind::Group => self.roles.group,
        }
    }

    /// Presence color for a status, paired with `AgentStatus::glyph()`.
    pub fn status(&self, status: AgentStatus) -> Color {
        match status {
            AgentStatus::Running => self.roles.running,
            AgentStatus::Idle => self.palette.subtle,
            AgentStatus::Done => self.roles.done,
            AgentStatus::Failed => self.palette.error,
            // Terminal but neither success nor failure: a neutral mark.
            AgentStatus::Stopped => self.palette.muted,
        }
    }

    /// Color for context use at a given pressure.
    pub fn pressure(&self, pressure: crate::settings::Pressure) -> Color {
        use crate::settings::Pressure;
        match pressure {
            Pressure::Normal => self.palette.subtle,
            Pressure::High => self.roles.warn,
            Pressure::Critical => self.palette.error,
        }
    }

    /// The rataflow theme for the canvas.
    pub fn flow_theme(&self) -> Theme {
        Theme::Custom(self.palette)
    }
}

const fn hex(rgb: u32) -> Color {
    Color::Rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// Dracula: <https://draculatheme.com/contribute#color-palette>.
const DRACULA: Look = Look {
    id: "dracula",
    palette: Palette {
        canvas_bg: hex(0x21222c),
        surface: hex(0x282a36),
        muted: hex(0x44475a),
        subtle: hex(0x6272a4),
        accent: hex(0xff79c6),
        text: hex(0xf8f8f2),
        success: hex(0x50fa7b),
        error: hex(0xff5555),
    },
    roles: Roles {
        main: hex(0xffb86c),
        subagent: hex(0x8be9fd),
        group: hex(0xbd93f9),
        running: hex(0x50fa7b),
        done: hex(0xbd93f9),
        warn: hex(0xf1fa8c),
    },
};

/// Catppuccin Mocha: <https://catppuccin.com/palette>.
const CATPPUCCIN: Look = Look {
    id: "catppuccin",
    palette: Palette {
        canvas_bg: hex(0x181825),
        surface: hex(0x1e1e2e),
        muted: hex(0x45475a),
        subtle: hex(0x7f849c),
        accent: hex(0xf5c2e7),
        text: hex(0xcdd6f4),
        success: hex(0xa6e3a1),
        error: hex(0xf38ba8),
    },
    roles: Roles {
        main: hex(0xfab387),
        subagent: hex(0x89b4fa),
        group: hex(0xcba6f7),
        running: hex(0xa6e3a1),
        done: hex(0xcba6f7),
        warn: hex(0xf9e2af),
    },
};

/// Tokyo Night: <https://github.com/folke/tokyonight.nvim>.
const TOKYO_NIGHT: Look = Look {
    id: "tokyo-night",
    palette: Palette {
        canvas_bg: hex(0x16161e),
        surface: hex(0x1a1b26),
        muted: hex(0x3b4261),
        subtle: hex(0x565f89),
        accent: hex(0xe0af68),
        text: hex(0xc0caf5),
        success: hex(0x9ece6a),
        error: hex(0xf7768e),
    },
    roles: Roles {
        main: hex(0xff9e64),
        subagent: hex(0x7aa2f7),
        group: hex(0xbb9af7),
        running: hex(0x9ece6a),
        done: hex(0xbb9af7),
        warn: hex(0xe0af68),
    },
};

/// Gruvbox dark: <https://github.com/morhetz/gruvbox>.
const GRUVBOX: Look = Look {
    id: "gruvbox",
    palette: Palette {
        canvas_bg: hex(0x1d2021),
        surface: hex(0x282828),
        muted: hex(0x504945),
        subtle: hex(0x928374),
        accent: hex(0xfabd2f),
        text: hex(0xebdbb2),
        success: hex(0xb8bb26),
        error: hex(0xfb4934),
    },
    roles: Roles {
        main: hex(0xfe8019),
        subagent: hex(0x83a598),
        group: hex(0xd3869b),
        running: hex(0xb8bb26),
        done: hex(0xd3869b),
        warn: hex(0xfabd2f),
    },
};

/// Nord: <https://www.nordtheme.com/docs/colors-and-palettes>.
const NORD: Look = Look {
    id: "nord",
    palette: Palette {
        canvas_bg: hex(0x242933),
        surface: hex(0x2e3440),
        muted: hex(0x434c5e),
        subtle: hex(0x616e88),
        accent: hex(0xebcb8b),
        text: hex(0xeceff4),
        success: hex(0xa3be8c),
        error: hex(0xbf616a),
    },
    roles: Roles {
        main: hex(0xd08770),
        subagent: hex(0x88c0d0),
        group: hex(0xb48ead),
        running: hex(0xa3be8c),
        done: hex(0xb48ead),
        warn: hex(0xebcb8b),
    },
};

/// The original zoetrope look: stock dark base with a gold accent and
/// uncolored cards.
const CLASSIC: Look = Look {
    id: "classic",
    palette: Palette {
        accent: Color::Indexed(178),
        ..Palette::DARK
    },
    roles: Roles {
        main: Palette::DARK.muted,
        subagent: Palette::DARK.muted,
        group: Palette::DARK.muted,
        running: Palette::DARK.success,
        done: Color::Indexed(178),
        warn: Color::Indexed(214),
    },
};

/// The terminal's own 16 ANSI colors, so the app follows whatever theme the
/// terminal (light or dark) is set to.
const TERMINAL: Look = Look {
    id: "terminal",
    palette: Palette {
        canvas_bg: Color::Reset,
        surface: Color::Reset,
        muted: Color::DarkGray,
        subtle: Color::DarkGray,
        accent: Color::Blue,
        text: Color::Reset,
        success: Color::Green,
        error: Color::Red,
    },
    roles: Roles {
        main: Color::Yellow,
        subagent: Color::Cyan,
        group: Color::Magenta,
        running: Color::Green,
        done: Color::Magenta,
        warn: Color::Yellow,
    },
};

/// Every theme, in `t` cycling order. The first is the default.
pub const LOOKS: &[Look] = &[
    DRACULA,
    CATPPUCCIN,
    TOKYO_NIGHT,
    GRUVBOX,
    NORD,
    CLASSIC,
    TERMINAL,
];

static CURRENT: AtomicUsize = AtomicUsize::new(0);

/// The active theme.
pub fn current() -> &'static Look {
    &LOOKS[CURRENT.load(Ordering::Relaxed) % LOOKS.len()]
}

/// Resolve a theme name to an index into [`LOOKS`].
///
/// Accepts the ids above and the names other tools use for the same themes
/// (Herdr's `tokyo-night-storm`, `catppuccin-mocha`, `gruvbox-dark`, …), so a
/// name read from another app's config usually just works. Light variants
/// (`catppuccin-latte`, `tokyo-night-day`, `gruvbox-light`, …) map to
/// `terminal`, which follows a light terminal palette.
pub fn find(name: &str) -> Option<usize> {
    let name = name.trim().to_ascii_lowercase().replace(['_', ' '], "-");
    let light = ["latte", "day", "light", "dawn"]
        .iter()
        .any(|w| name.split('-').any(|part| part == *w));
    let id = if light {
        "terminal"
    } else if name.starts_with("tokyo") {
        "tokyo-night"
    } else if name.starts_with("catppuccin") {
        "catppuccin"
    } else if name.starts_with("gruvbox") {
        "gruvbox"
    } else if name == "zoetrope" || name == "default" {
        "classic"
    } else {
        name.as_str()
    };
    LOOKS.iter().position(|l| l.id == id)
}

/// Switch to a theme by name. Returns `false` (and changes nothing) for an
/// unknown name.
pub fn set_by_name(name: &str) -> bool {
    match find(name) {
        Some(i) => {
            CURRENT.store(i, Ordering::Relaxed);
            true
        }
        None => false,
    }
}

/// Advance to the next theme and return it.
pub fn cycle() -> &'static Look {
    let next = (CURRENT.load(Ordering::Relaxed) + 1) % LOOKS.len();
    CURRENT.store(next, Ordering::Relaxed);
    current()
}

/// Comma-separated theme ids, for usage and error messages.
pub fn names() -> String {
    LOOKS.iter().map(|l| l.id).collect::<Vec<_>>().join(", ")
}

/// Short display name for a model id: `claude-opus-4-1-20250805` → `Opus 4.1`,
/// `claude-sonnet-4-5` → `Sonnet 4.5`, `gpt-5-codex` → `GPT-5 Codex`.
/// Unrecognised ids pass through unchanged.
pub fn short_model(model: &str) -> String {
    let m = model.trim();
    if let Some(rest) = m.strip_prefix("claude-") {
        let mut family = None;
        let mut version: Vec<&str> = Vec::new();
        for part in rest.split('-') {
            if part.chars().all(|c| c.is_ascii_digit()) {
                // A date stamp (8 digits) ends the version.
                if part.len() >= 6 {
                    break;
                }
                version.push(part);
            } else if family.is_none() {
                family = Some(part);
            }
        }
        if let Some(f) = family {
            let mut name = String::new();
            let mut chars = f.chars();
            if let Some(c) = chars.next() {
                name.extend(c.to_uppercase());
                name.push_str(chars.as_str());
            }
            if !version.is_empty() {
                name.push(' ');
                name.push_str(&version.join("."));
            }
            return name;
        }
    }
    if let Some(rest) = m.strip_prefix("gpt-") {
        let mut parts = rest.splitn(2, '-');
        let ver = parts.next().unwrap_or_default();
        return match parts.next() {
            Some(tail) => {
                let words: Vec<String> = tail
                    .split('-')
                    .map(|w| {
                        let mut c = w.chars();
                        match c.next() {
                            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                            None => String::new(),
                        }
                    })
                    .collect();
                format!("GPT-{ver} {}", words.join(" "))
            }
            None => format!("GPT-{ver}"),
        };
    }
    m.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_ids_and_aliases() {
        assert_eq!(find("dracula"), Some(0));
        assert_eq!(LOOKS[find("Tokyo Night Storm").unwrap()].id, "tokyo-night");
        assert_eq!(LOOKS[find("catppuccin-mocha").unwrap()].id, "catppuccin");
        assert_eq!(LOOKS[find("gruvbox_dark").unwrap()].id, "gruvbox");
        assert_eq!(LOOKS[find("catppuccin-latte").unwrap()].id, "terminal");
        assert_eq!(LOOKS[find("gruvbox-light").unwrap()].id, "terminal");
        assert_eq!(LOOKS[find("zoetrope").unwrap()].id, "classic");
        assert_eq!(find("solarized"), None);
    }

    #[test]
    fn ids_are_unique() {
        for (i, a) in LOOKS.iter().enumerate() {
            assert_eq!(find(a.id), Some(i), "{} must resolve to itself", a.id);
        }
    }

    #[test]
    fn short_model_names() {
        assert_eq!(short_model("claude-opus-4-1-20250805"), "Opus 4.1");
        assert_eq!(short_model("claude-sonnet-4-5"), "Sonnet 4.5");
        assert_eq!(short_model("claude-3-5-haiku-20241022"), "Haiku 3.5");
        assert_eq!(short_model("claude-opus-5-5"), "Opus 5.5");
        assert_eq!(short_model("gpt-5-codex"), "GPT-5 Codex");
        assert_eq!(short_model("gpt-5-codex-mini"), "GPT-5 Codex Mini");
        assert_eq!(short_model("gpt-5.1-codex"), "GPT-5.1 Codex");
        assert_eq!(short_model("gpt-5"), "GPT-5");
        assert_eq!(short_model("o3"), "o3");
    }
}
