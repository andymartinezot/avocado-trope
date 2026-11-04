//! The one thing this app writes: the theme you last switched to, in
//! `<config dir>/avocado-trope/theme`. Everything else is read-only.

use std::path::PathBuf;

fn path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("avocado-trope").join("theme"))
}

/// The remembered theme name, if any.
pub fn saved_theme() -> Option<String> {
    let text = std::fs::read_to_string(path()?).ok()?;
    let name = text.lines().next()?.trim();
    (!name.is_empty()).then(|| name.to_string())
}

/// Remember a theme. Best effort: a read-only home must not disturb the UI.
pub fn save_theme(id: &str) {
    let Some(path) = path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, format!("{id}\n"));
}
