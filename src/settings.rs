//! Launch-time settings read by the portable core: the context window to
//! assume and when a quiet tool call counts as stuck. Set once by the native
//! frontend, read everywhere; plain atomics, like the active theme.

use std::sync::atomic::{AtomicU64, Ordering};

/// Claude's standard window. A prompt past it proves a larger one.
const STANDARD_WINDOW: u64 = 200_000;
/// What a prompt past [`STANDARD_WINDOW`] implies.
const LARGE_WINDOW: u64 = 1_000_000;

static CONTEXT_WINDOW: AtomicU64 = AtomicU64::new(0);
static STUCK_AFTER_SECS: AtomicU64 = AtomicU64::new(300);

/// Assume this window for agents whose format doesn't state one. 0 clears it.
pub fn set_context_window(tokens: u64) {
    CONTEXT_WINDOW.store(tokens, Ordering::Relaxed);
}

/// Flag a tool call as stuck after this many seconds without an end.
pub fn set_stuck_after_secs(secs: u64) {
    STUCK_AFTER_SECS.store(secs.max(1), Ordering::Relaxed);
}

/// Seconds a tool call may stay open before it counts as stuck.
pub fn stuck_after_secs() -> u64 {
    STUCK_AFTER_SECS.load(Ordering::Relaxed)
}

/// The window to measure an agent against: what the format states, else what
/// the user set, else what its peak proves. `None` means unknown, and unknown
/// stays unknown: neither a model id nor a transcript says how large a Claude
/// window is, and guessing 200k would raise false alarms on 1M sessions.
pub fn effective_window(stated: Option<u64>, peak: u64) -> Option<u64> {
    stated
        .filter(|w| *w > 0)
        .or_else(|| match CONTEXT_WINDOW.load(Ordering::Relaxed) {
            0 => (peak > STANDARD_WINDOW).then_some(LARGE_WINDOW),
            w => Some(w),
        })
}

/// How full a context window is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pressure {
    Normal,
    /// 70% or more: compaction is getting close.
    High,
    /// 90% or more.
    Critical,
}

/// Percentage of `window` in use, capped at 100.
pub fn percent(used: u64, window: u64) -> u64 {
    if window == 0 {
        return 0;
    }
    (used.saturating_mul(100) / window).min(100)
}

/// The pressure of `used` against a known window; `Normal` when unknown.
pub fn pressure(used: u64, window: Option<u64>) -> Pressure {
    match window.map(|w| percent(used, w)) {
        Some(p) if p >= 90 => Pressure::Critical,
        Some(p) if p >= 70 => Pressure::High,
        _ => Pressure::Normal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stated_window_wins_and_unknown_stays_unknown() {
        assert_eq!(effective_window(Some(475_000), 900_000), Some(475_000));
        assert_eq!(effective_window(None, 150_000), None);
        assert_eq!(effective_window(None, 200_000), None);
        assert_eq!(effective_window(None, 200_001), Some(1_000_000));
    }

    #[test]
    fn pressure_steps() {
        assert_eq!(pressure(69, Some(100)), Pressure::Normal);
        assert_eq!(pressure(70, Some(100)), Pressure::High);
        assert_eq!(pressure(90, Some(100)), Pressure::Critical);
        assert_eq!(pressure(10_000, None), Pressure::Normal);
        assert_eq!(percent(5, 0), 0);
        assert_eq!(percent(u64::MAX, 10), 100);
    }
}
