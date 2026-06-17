pub mod ansi;
mod config;
pub mod coords;
mod font;
pub mod pty;
pub mod session;
pub mod ux;
pub mod vt_codes;
pub mod vt_limits;

pub mod view;

pub use config::{
    DEFAULT_FONT_SIZE, DEFAULT_INSET_X, DEFAULT_INSET_Y, DEFAULT_SPACING, FONT_SIZE_MAX,
    FONT_SIZE_MIN, INSET_MAX, INSET_MIN, PromptJumpScroll, SPACING_MAX, SPACING_MIN,
    TerminalConfig, TerminalDims,
};
pub use font::{default_terminal_font, default_terminal_font_features, terminal_font_with_family};
pub use ghostty_vt::Error as VtError;
pub use session::{CommandHistoryEntry, TerminalSession};
pub use view::TerminalViewEvent;
pub use vt_codes::{AttentionKind, NotificationRequest};

#[cfg(test)]
mod tests;
