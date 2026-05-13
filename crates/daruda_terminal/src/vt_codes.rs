//! Numeric parameter codes used by CSI / OSC / FTCS sequences.
//!
//! Every raw integer the parser or emitter cares about has a name here.
//! Escape sequence **bytes** live in [`crate::ansi`]; buffer size
//! **limits** live in [`crate::vt_limits`].

// ============================================================================
// CSI — DEC private modes (`CSI ? Ps h/l`)
// ============================================================================

// ============================================================================
// Alt-screen mode codes (observed, not tracked by CsiMode)
// ============================================================================

/// Legacy alt-screen mode (`CSI ? 47 h/l`).
pub const CSI_ALT_SCREEN_LEGACY: u32 = 47;
/// Alt-screen mode without cursor save/restore (`CSI ? 1047 h/l`).
pub const CSI_ALT_SCREEN: u32 = 1047;
/// Alt-screen mode with cursor save on enter and restore on exit (`CSI ? 1049 h/l`).
/// This is the variant used by most modern TUI applications.
pub const CSI_ALT_SCREEN_SAVE_CURSOR: u32 = 1049;

/// DEC private modes we track. Any `Ps` not listed here is forwarded
/// unchanged to ghostty_vt.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum CsiMode {
    /// Application Cursor Keys (`DECCKM`, DEC private mode 1). When set,
    /// arrow keys emit `\x1bOA`–`\x1bOD` instead of `\x1b[A`–`\x1b[D`.
    DecCkm = 1,
    /// X10 compatibility mouse reporting (button press only).
    MouseX10 = 1000,
    /// Button-event mouse reporting (press + release + drag).
    MouseButtonEvent = 1002,
    /// Any-event mouse reporting (every motion).
    MouseAnyEvent = 1003,
    /// Focus in/out event reporting — `ESC[I` on focus, `ESC[O` on blur.
    FocusEvent = 1004,
    /// UTF-8 mouse coordinate encoding — extends Normal mode range to col/row 2015.
    MouseUtf8 = 1005,
    /// SGR-encoded mouse coordinates — unlimited range.
    MouseSgr = 1006,
    /// Alternate scroll mode — in alt-screen, wheel scroll emits arrow-key sequences.
    AlternateScroll = 1007,
    /// Application Keypad Mode (`DECNKM`, DEC private mode 66). When set,
    /// numeric keypad keys emit application sequences instead of ASCII digits.
    DecNkm = 66,
    /// Bracketed paste — wrap pasted text in `ESC[200~ / ESC[201~`.
    BracketedPaste = 2004,
    /// Synchronized Output (BSynchronized, DEC private mode 2026). Apps set
    /// this before drawing to prevent partial-frame tearing.
    SynchronizedOutput = 2026,
}

impl CsiMode {
    pub fn from_raw(n: u32) -> Option<Self> {
        Some(match n {
            1 => Self::DecCkm,
            66 => Self::DecNkm,
            1000 => Self::MouseX10,
            1002 => Self::MouseButtonEvent,
            1003 => Self::MouseAnyEvent,
            1004 => Self::FocusEvent,
            1005 => Self::MouseUtf8,
            1006 => Self::MouseSgr,
            1007 => Self::AlternateScroll,
            2004 => Self::BracketedPaste,
            2026 => Self::SynchronizedOutput,
            _ => return None,
        })
    }
}

// ============================================================================
// OSC — Operating System Command `Ps` codes
// ============================================================================

/// Window + icon title (xterm OSC 0 sets both).
pub const OSC_TITLE_ICON_AND_WINDOW: u32 = 0;
/// Window title only (most shells emit this one).
pub const OSC_TITLE_WINDOW: u32 = 2;
/// Current working directory (`file://hostname/path`).
pub const OSC_CWD: u32 = 7;
/// Default foreground color — query/set.
pub const OSC_DEFAULT_FG: u32 = 10;
/// Default background color — query/set.
pub const OSC_DEFAULT_BG: u32 = 11;
/// Notification (xterm / iTerm2 "Show Growl Notification"). Body-only
/// payload; the host supplies the title.
pub const OSC_NOTIFICATION: u32 = 9;
/// Clipboard set/get — base64 payload with selection prefix.
pub const OSC_CLIPBOARD: u32 = 52;
/// FinalTerm / FTCS shell-integration marks.
pub const OSC_FTCS: u32 = 133;
/// rxvt-extended structured notification (`notify ; <title> ; <body>`).
pub const OSC_NOTIFY_RXVT: u32 = 777;
/// iTerm2 proprietary commands. Payload is `key=value` (or `Copy=:base64`,
/// `RequestAttention=once`, etc.).
pub const OSC_ITERM2: u32 = 1337;

// ============================================================================
// OSC 1337 — iTerm2 proprietary subcommands
// ============================================================================

/// `OSC 1337 ; RequestAttention=<value>` — dock bounce / window alert.
/// Mirrors iTerm2's public proprietary escape-codes contract.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AttentionKind {
    /// `yes` — continuous attention until the window receives focus
    /// (NSCriticalRequest).
    Critical,
    /// `no` — cancel a pending attention request.
    Cancel,
    /// `once` — one-shot bounce (NSInformationalRequest).
    Once,
}

impl AttentionKind {
    pub fn from_value(s: &str) -> Option<Self> {
        match s.trim() {
            "yes" => Some(Self::Critical),
            "no" => Some(Self::Cancel),
            // iTerm2's `fireworks` is an in-window visual flourish that
            // has no AppKit analogue; downgrade to a single bounce so
            // the user-visible signal is preserved.
            "once" | "fireworks" => Some(Self::Once),
            _ => None,
        }
    }
}

// ============================================================================
// OSC 9 / OSC 777 — system notifications
// ============================================================================

/// One desktop-notification request emitted by the shell. Each variant
/// carries only the channel-specific data; the host (Workspace) decides
/// whether to gate by config and supplies any missing pieces (e.g. the
/// app name as title for OSC 9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationRequest {
    /// `OSC 9 ; <body>` — body only. Title defaults to the app name.
    Osc9 { body: String },
    /// `OSC 777 ; notify ; <title> ; <body>` — rxvt-extended.
    Osc777 { title: String, body: String },
}

// ============================================================================
// FTCS — FinalTerm subcommands (OSC 133 ; <letter> …)
// ============================================================================

/// Subcommand letter emitted by the shell after `OSC 133 ;`.
/// Reference: iTerm2 `VT100Terminal.m:4520-4616`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FtcsCommand {
    /// `A` — prompt drawing started.
    PromptStart,
    /// `B` — user command input begins.
    CommandStart,
    /// `C` — command has been sent to the shell and is executing.
    CommandExecuted,
    /// `D` — command finished, optionally with an exit code (`D;N`).
    CommandFinished,
    /// `E` — semantic text block (command output) begins.
    SemanticTextStart,
    /// `F` — semantic text block ends.
    SemanticTextEnd,
}

impl FtcsCommand {
    pub fn from_letter(b: u8) -> Option<Self> {
        Some(match b {
            b'A' => Self::PromptStart,
            b'B' => Self::CommandStart,
            b'C' => Self::CommandExecuted,
            b'D' => Self::CommandFinished,
            b'E' => Self::SemanticTextStart,
            b'F' => Self::SemanticTextEnd,
            _ => return None,
        })
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csi_mode_round_trip() {
        assert_eq!(CsiMode::from_raw(1), Some(CsiMode::DecCkm));
        assert_eq!(CsiMode::from_raw(66), Some(CsiMode::DecNkm));
        assert_eq!(CsiMode::from_raw(1006), Some(CsiMode::MouseSgr));
        assert_eq!(CsiMode::from_raw(2004), Some(CsiMode::BracketedPaste));
        assert_eq!(CsiMode::from_raw(2026), Some(CsiMode::SynchronizedOutput));
        assert_eq!(CsiMode::from_raw(9999), None);
    }

    #[test]
    fn attention_kind_parses_iterm2_values() {
        assert_eq!(
            AttentionKind::from_value("yes"),
            Some(AttentionKind::Critical)
        );
        assert_eq!(AttentionKind::from_value("no"), Some(AttentionKind::Cancel));
        assert_eq!(AttentionKind::from_value("once"), Some(AttentionKind::Once));
        assert_eq!(
            AttentionKind::from_value("fireworks"),
            Some(AttentionKind::Once)
        );
        assert_eq!(AttentionKind::from_value("nope"), None);
    }

    #[test]
    fn ftcs_letters_cover_abcdef() {
        for (b, expect) in [
            (b'A', FtcsCommand::PromptStart),
            (b'B', FtcsCommand::CommandStart),
            (b'C', FtcsCommand::CommandExecuted),
            (b'D', FtcsCommand::CommandFinished),
            (b'E', FtcsCommand::SemanticTextStart),
            (b'F', FtcsCommand::SemanticTextEnd),
        ] {
            assert_eq!(FtcsCommand::from_letter(b), Some(expect));
        }
        assert!(FtcsCommand::from_letter(b'Z').is_none());
    }
}
