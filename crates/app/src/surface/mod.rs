//! App-shell identity, shortcuts, and display text.
//!
//! Split by change trigger so each file stays editable in isolation:
//!   * [`constants`] — app name + `TERM_PROGRAM` value (rename trigger).
//!   * [`keybindings`] — every `KeyBinding` shortcut string
//!     (key-remap trigger).
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
pub mod strings;
pub mod timestamp;
