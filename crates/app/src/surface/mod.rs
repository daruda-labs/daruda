//! App-shell identity, shortcuts, and display text.
//!
//! Split by change trigger so each file stays editable in isolation:
//!   * [`constants`] — app name + `TERM_PROGRAM` value (rename trigger).
//!   * [`keybindings`] — every `KeyBinding` shortcut string
//!     (key-remap trigger).
//!   * [`shortcut_display`] — those same strings rendered for a reader
//!     (key-remap trigger, but a formatting change, so kept separate from
//!     the declarative table).
//!   * [`strings`] — menu and dialog labels (localisation trigger).
//!   * [`timestamp`] — wall-clock timestamp shapes (localisation trigger).
//!
//! Terminal-protocol constants (escape sequences, VT codes) live in
//! `daruda_terminal::{ansi, vt_codes, vt_limits}`. Terminal-view
//! display constants (search overlay, theme) live in
//! `daruda_terminal::ux`. This module is only for the app chrome.

pub mod action_map;
pub mod constants;
pub mod keybindings;
pub mod shortcut_display;
pub mod strings;
pub mod timestamp;
