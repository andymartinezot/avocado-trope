// The mark, in the docs.rs sidebar and its tab. Absolute URLs because rustdoc
// does not copy local files into the generated output — docs.rs would serve a
// dead link to a path that only exists in this checkout.
#![doc(
    html_logo_url = "https://raw.githubusercontent.com/furkankly/zoetrope/main/assets/icon.svg",
    html_favicon_url = "https://raw.githubusercontent.com/furkankly/zoetrope/main/assets/icon.svg"
)]
//! avocado-trope: watch Claude Code and Codex agents as a live flow graph.
//! See the README for usage.
//!
//! ## The library
//!
//! The portable core behind both the native frontend
//! (the `avocado-trope` terminal app, `src/main.rs`) and the browser frontend (the
//! `zoetrope-web` crate at `web/wasm/`, which depends on this one with default
//! features off).
//!
//! Portable everywhere: the domain [`state`] (model + unified replay/live
//! [`timeline`](state::Timeline) + flow-graph projection), the [`ui`] rendering,
//! the [`fact`] vocabulary every transcript format is reduced to, and the
//! [`provider`] providers that do the reducing. The wire types and pure replay
//! assembly live in [`tailer`]; its live file-tailing + the terminal loop ([`tui`]) and input
//! ([`handler`]) are native-only (they pull tokio/crossterm/fs) and `cfg`-gated
//! behind the `native` feature.

pub mod fact;
pub mod provider;
pub mod settings;
pub mod state;
pub mod tailer;
pub mod theme;
pub mod ui;

// The native frontend: terminal loop + crossterm input.
//
// Frontend plumbing, with no stability promise. These are public because the
// `avocado-trope` binary is a separate crate that links this one, not because they are an
// interface to build on: they may change shape in any release. The parts meant
// to be depended on are the domain and the vocabulary above.
#[cfg(feature = "native")]
pub mod autopilot;
#[cfg(feature = "native")]
pub mod compare;
#[cfg(feature = "native")]
pub mod fleet;
#[cfg(feature = "native")]
pub mod handler;
#[cfg(feature = "native")]
pub mod prefs;
#[cfg(feature = "native")]
pub mod tui;
