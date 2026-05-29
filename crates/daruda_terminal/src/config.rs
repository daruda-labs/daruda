use ghostty_vt::Rgb;

#[derive(Clone, Copy, Debug)]
pub struct TerminalConfig {
    pub cols: u16,
    pub rows: u16,
    pub default_fg: Rgb,
    pub default_bg: Rgb,
    pub update_window_title: bool,
    /// Parse OSC 7 (`file://host/path`) sequences emitted by the shell to
    /// track the current working directory. Disable to skip the work and
    /// to opt out of cwd-aware features (new tab/pane inheritance, header
    /// path display).
    pub track_cwd: bool,
    /// Flash the viewport briefly when the shell/app emits BEL (0x07).
    /// Off by default — many shells BEL on invalid keys (e.g. readline
    /// at end of line) and the flash becomes noise.
    pub visual_bell: bool,
    /// Scroll behaviour for prompt/command jump when the target mark
    /// is already visible. `AlwaysTop` (default) scrolls so the mark
    /// lands at the top of the viewport every press — matches the
    /// iTerm2 "command mark" variant and keeps keystrokes useful even
    /// when all marks fit on screen. `LeaveInPlace` behaves like
    /// iTerm2's `scrollLineNumberRangeIntoView` (no-op when already
    /// visible), handy when you want a quieter viewport.
    pub prompt_jump_scroll: PromptJumpScroll,
    /// Terminal body font size in points (iTerm2 `KEY_NORMAL_FONT` size).
    pub font_size: f32,
    /// Line height multiplier, independent of `font_size`. Mirrors
    /// iTerm2 `KEY_VERTICAL_SPACING` (range 0.5–2.0, default 1.0).
    /// Final cell height = base_line_height * vertical_spacing.
    pub vertical_spacing: f32,
    /// Cell width multiplier. Mirrors iTerm2 `KEY_HORIZONTAL_SPACING`
    /// (range 0.5–2.0, default 1.0). Applied via `force_width` on
    /// shape_line for narrow rows; wide (CJK / emoji) rows keep
    /// natural advances.
    pub horizontal_spacing: f32,
    /// ANSI palette override (indices 0–15). `None` = ghostty default
    /// palette. When set, fed as OSC 4 sequences on session init.
    pub palette: Option<[[u8; 3]; 16]>,
    /// Maximum rows ghostty_vt allocates for the scrollback buffer.
    /// Matches `daruda_config::ScrollbackConfig::max_rows`.
    pub max_scrollback: usize,
    /// Terminal background opacity (0.0–1.0, default 1.0 = fully opaque).
    /// Applied as alpha to every background quad so the desktop shows
    /// through — mirrors iTerm2's `transparencyAlpha = 1.0 - transparency`.
    /// The OS window must be in `Transparent` or `Blurred` mode for this
    /// to have any effect; `build_window_options` handles that automatically.
    pub background_alpha: f32,
    /// Cap on the bytes the OSC 1337 path is allowed to buffer. The
    /// generic OSC aggregator's `PARSE_TAIL_LIMIT` is too small for
    /// `Copy=…:<base64>` clipboard payloads, so OSC 1337 sequences
    /// get a separate, larger budget that mirrors
    /// `daruda_config::ClipboardConfig::streaming_max_bytes`.
    /// 0 disables OSC 1337 large-payload handling (falls back to
    /// the generic limit).
    pub osc1337_max_bytes: usize,
    /// macOS-native cursor/edit shortcuts (`Cmd/Opt + arrow/delete`).
    /// When `true`, those keystrokes are remapped to the equivalent
    /// readline bytes before reaching the PTY — iTerm2's "Natural Text
    /// Editing" preset. See `view::keybindings`. Mirrors
    /// `daruda_config::ShellConfig::natural_text_editing`.
    pub natural_text_editing: bool,
}

/// Points. Monaco 13 on first launch — a notch above iTerm2's
/// factory `Monaco 12` (still legible on a 2× retina display while
/// leaving Cmd+- headroom) and below the 16px GPUI `1rem` default.
pub const DEFAULT_FONT_SIZE: f32 = 13.0;
/// iTerm2 default for `KEY_VERTICAL_SPACING` / `KEY_HORIZONTAL_SPACING`.
pub const DEFAULT_SPACING: f32 = 1.0;
/// iTerm2 slider range for vertical/horizontal spacing.
pub const SPACING_MIN: f32 = 0.5;
pub const SPACING_MAX: f32 = 2.0;
/// Clamp floor/ceil on font_size — matches typical terminal UX.
pub const FONT_SIZE_MIN: f32 = 6.0;
pub const FONT_SIZE_MAX: f32 = 72.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptJumpScroll {
    /// Always scroll so the target mark sits at the top of the viewport.
    AlwaysTop,
    /// Skip the scroll when the target mark is already visible.
    LeaveInPlace,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            cols: 80,
            rows: 24,
            default_fg: Rgb {
                r: 0xFF,
                g: 0xFF,
                b: 0xFF,
            },
            default_bg: Rgb {
                r: 0x00,
                g: 0x00,
                b: 0x00,
            },
            update_window_title: true,
            track_cwd: true,
            visual_bell: false,
            prompt_jump_scroll: PromptJumpScroll::AlwaysTop,
            font_size: DEFAULT_FONT_SIZE,
            vertical_spacing: DEFAULT_SPACING,
            horizontal_spacing: DEFAULT_SPACING,
            palette: None,
            max_scrollback: 10_000,
            background_alpha: 1.0,
            osc1337_max_bytes: 10 * 1024 * 1024,
            natural_text_editing: true,
        }
    }
}

impl TerminalConfig {
    /// Clamp font settings to iTerm2-style sane ranges. Called by the
    /// runtime config loader and the zoom actions so a bad input can
    /// never push the grid into 0-size rows.
    pub fn clamp_font_settings(&mut self) {
        self.font_size = self.font_size.clamp(FONT_SIZE_MIN, FONT_SIZE_MAX);
        self.vertical_spacing = self.vertical_spacing.clamp(SPACING_MIN, SPACING_MAX);
        self.horizontal_spacing = self.horizontal_spacing.clamp(SPACING_MIN, SPACING_MAX);
    }
}
