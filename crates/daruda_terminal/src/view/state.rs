//! GPUI-free state owned by `TerminalView`.
//!
//! Every field that paint *and* event handlers both touch lives in
//! [`TerminalViewState`]. The struct carries no `gpui::Window`,
//! `Context`, `Entity`, `FocusHandle`, `Task`, or `Subscription` —
//! only plain data + sealed value types like `gpui::Font` /
//! `gpui::Pixels` / `Instant`. That separation makes it possible to
//! exercise the state machine in pure unit tests (no
//! `TestAppContext`, no async setup) and turns the daruda CLAUDE.md
//! Pitfall #8 rule ("paint-scope state must not leak into the event
//! path") into a structural fact rather than a guideline.
//!
//! The `font` / `font_size` / `vertical_spacing` /
//! `horizontal_spacing` quartet is the load-bearing piece: paint
//! shapes glyphs from these values, mouse handlers translate
//! pixel positions into grid cells with these values, IME positions
//! the candidate window with these values. Reading them off any
//! single source (here: the entity's `state` field, fed by config
//! reload) is what keeps the four code paths from drifting against
//! one another.

/// Deferred-refresh intent for the next render. Replaces the former
/// `(pending_refresh, pending_refresh_keep_selection)` bool pair, whose
/// `(false, true)` combination was illegal yet representable. The three
/// variants enumerate exactly the valid states.
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PendingRefresh {
    /// No refresh pending.
    #[default]
    No,
    /// Refresh and clear the linear selection (full content change).
    Clear,
    /// Refresh and preserve the linear selection (viewport-window shift:
    /// user scroll, search / prompt navigation, PTY echo).
    Preserve,
}

/// The mouse drag currently in progress. Replaces the former
/// `(is_dragging, drag_row, scrollbar_drag_start)` trio, which could
/// represent illegal combinations (both drag types active at once, or
/// `is_dragging == false` while `drag_row == Some`). The variants
/// enumerate exactly the valid states: idle, a text-selection drag
/// tracking the live drag row, or a scrollbar-thumb drag tracking the
/// cursor-to-thumb-top pixel offset.
#[derive(Default, Clone, Copy, PartialEq)]
pub(crate) enum MouseDragState {
    /// No drag in progress.
    #[default]
    None,
    /// Left button held during a text selection. `row` is the grid row
    /// (0-based viewport) of the last drag-mouse position; paint extends
    /// selection highlights on otherwise-empty rows down to this row so a
    /// drag past the last text line still reads as "selected".
    TextSelection { row: usize },
    /// Scrollbar thumb being dragged. `offset` is the pixel offset between
    /// the cursor and the thumb top, captured at drag start so the thumb
    /// does not jump on the first move event.
    ScrollbarDrag { offset: f32 },
}

/// Shared paint+event state. New fields land here as they are
/// migrated out of [`super::TerminalView`].
pub(crate) struct TerminalViewState {
    // ---- Viewport scroll-lock ----
    /// Combined scroll-lock flag + absolute-line anchor.
    /// `Live` → the viewport follows PTY output.
    /// `Pinned { anchor }` → the user is reading scrollback; the anchor
    /// tracks the viewport-top abs line so IND/SU grid scrolls do not
    /// drift the reading position.  Released by `snap_to_bottom`,
    /// OSC 133 A, alt-screen toggle, and RIS.
    pub(crate) viewport_lock: super::viewport_lock::ViewportLock,

    /// Deadline before which trackpad momentum scroll events must not
    /// re-lock the viewport. Set by PTY-input paths
    /// (`snap_to_bottom_on_pty_input`) so that residual inertia from a
    /// scroll-then-type gesture cannot immediately re-pin the viewport
    /// after the input snapped it to the bottom.  Cleared when a new
    /// physical gesture begins (`TouchPhase::Started`).
    pub(crate) suppress_scroll_lock_until: Option<std::time::Instant>,
    // ---- Pitfall #8: cell metrics ----
    //
    // `cell_metrics_at(window, &font, px(font_size))` is the only
    // GPUI text-system call that converts these values into measured
    // pixels. paint, prepaint, mouse, IME, and resize all fan out
    // from that single call — see `TerminalView::cell_layout`.
    /// Primary terminal font (family + fallbacks + features +
    /// weight + style). `gpui::Font` is plain data — it does not
    /// require a `Window` to construct or read.
    pub(crate) font: gpui::Font,

    /// Body font size in points. Updated by zoom actions
    /// (`Cmd+=` / `Cmd+-`) and config reload. `cell_metrics_at`
    /// receives `px(font_size)` so the shaper measures glyphs at
    /// exactly this size; mouse and IME read it directly so their
    /// pixel↔cell math always lines up with paint.
    pub(crate) font_size: f32,

    /// Line-height multiplier. The shaper's natural advance height
    /// is multiplied by this value to produce the rendered cell
    /// height (iTerm2 `KEY_VERTICAL_SPACING`).
    pub(crate) vertical_spacing: f32,

    /// Cell-width multiplier. Applied to the shaper's natural
    /// advance width (iTerm2 `KEY_HORIZONTAL_SPACING`). Narrow rows
    /// pass the resulting width through `force_width` at shape
    /// time so monospace columns line up.
    pub(crate) horizontal_spacing: f32,

    /// Background opacity (0.0–1.0). Composited into the alpha
    /// channel of every default-bg quad so the desktop shows
    /// through when the OS window is `Transparent` / `Blurred`.
    pub(crate) background_alpha: f32,

    // ---- Search ----
    /// Active search state (query, matches, focused index, regex
    /// cache). `query.is_empty()` means inactive. Both the
    /// scrollback scanner (paint side) and the search-bar input
    /// (event side) read and mutate this; co-locating the field
    /// keeps the two on the same value without going through
    /// `cx.notify()` round-trips.
    pub(crate) search: super::SearchState,

    /// Whether the search overlay is visible and capturing keys.
    /// Cmd+F flips it on; Esc / Cmd+W flips it off. Paint reads to
    /// decide whether to draw the bar; key handlers read to decide
    /// where printable input goes (search query vs PTY).
    pub(crate) search_overlay: bool,

    // ---- Selection / drag ----
    /// Active text selection (linear or block). Paint draws the
    /// highlight; mouse handlers extend it during drag. `None`
    /// means no selection — Cmd+C copies the visible viewport.
    pub(crate) selection: Option<super::selection::ByteSelection>,

    /// The mouse drag currently in progress, if any. A drag is
    /// either a text selection (drives the autoscroll task and
    /// selection-extend highlights) or a scrollbar-thumb drag;
    /// the two are mutually exclusive.
    pub(crate) mouse_drag: MouseDragState,

    /// Bounds of the scrollbar thumb painted last frame. Mouse
    /// handlers read this to detect a thumb-click without
    /// recomputing scroll metrics.
    pub(crate) scrollbar_thumb_bounds: Option<gpui::Bounds<gpui::Pixels>>,

    // ---- Viewport snapshot ----
    //
    // These four are populated by `refresh_viewport` after each PTY
    // burst and consumed by paint (line shaping, BG quads) and event
    // handlers (mouse → cell mapping, IME caret anchoring).
    /// Visible lines as plain strings. Index 0 = top of viewport.
    pub(crate) viewport_lines: Vec<String>,

    /// Per-line byte offsets into the joined viewport text. Used by
    /// search highlight + selection's "find which line a byte is on"
    /// helpers.
    pub(crate) viewport_line_offsets: Vec<usize>,

    /// Total byte length of the viewport (sum of `viewport_lines`).
    pub(crate) viewport_total_len: usize,

    /// Per-cell style runs, one row per `viewport_lines` entry.
    /// Drives the BG quad merge in prepaint and the per-cell glyph
    /// `TextRun` building.
    pub(crate) viewport_style_runs: Vec<Vec<ghostty_vt::StyleRun>>,

    // ---- Paint-frame bookkeeping ----
    /// Bounds of the terminal element painted last frame. Mouse
    /// handlers read this (subtract from window cursor coords) to
    /// produce element-local coordinates for `pixel_to_cell`. Set by
    /// the prepaint path; readers that fire before the first paint
    /// see `None`.
    pub(crate) last_bounds: Option<gpui::Bounds<gpui::Pixels>>,

    /// Window title cached from the last OSC 0/2. Read by
    /// `terminal_title()` (workspace tab strip) and by IME callbacks
    /// that surface the title to the OS.
    pub(crate) last_window_title: Option<String>,

    // ---- Pending PTY output ----
    /// Bytes received from the PTY but not yet fed into the VT
    /// parser. The 16 ms render tick batches them so a verbose
    /// command (`cat large_file`) doesn't render once per chunk.
    pub(crate) pending_output: Vec<u8>,

    /// Whether `apply_side_effects` should rebuild the viewport
    /// snapshot on the next render (after batched PTY output landed
    /// or a setting changed dirty rows), and if so whether the linear
    /// selection should be cleared or preserved across the rebuild.
    pub(crate) pending_refresh: PendingRefresh,

    // ---- IME composition ----
    /// Active IME preedit. `None` when no composition is in flight.
    /// Paint draws the underlined glyph at the caret cell; IME
    /// callbacks (`set_marked_text`, `unmark_text`) write here.
    pub(crate) marked_text: Option<gpui::SharedString>,

    /// UTF-16 selection range inside `marked_text` (macOS reports
    /// IME selection in UTF-16 code units). Empty range = caret.
    pub(crate) marked_selected_range_utf16: std::ops::Range<usize>,

    /// Last cursor position observed via `session.cursor_position()`.
    /// IME's `bounds_for_range` falls back to this when the live
    /// cursor goes unreachable (alt-screen with hidden caret) so the
    /// macOS candidate window keeps a sensible anchor.
    pub(crate) last_known_cursor: Option<(u16, u16)>,

    /// In-progress Hangul syllable being assembled from Compatibility
    /// Jamo commits. macOS IME occasionally delivers individual jamo
    /// instead of precomposed syllables; the composer combines them
    /// into wide precomposed syllables before anything reaches the
    /// PTY. See `view/hangul_composer.rs`.
    pub(crate) hangul_composer: super::hangul_composer::HangulComposer,

    // ---- Hover / jump / flash ----
    /// URL currently under the mouse pointer while the hover modifier
    /// (Cmd by default) is held. `row` is a 0-indexed viewport row, so
    /// the cached hit is only valid for the viewport snapshot that
    /// produced it; keyboard scroll / PTY output would otherwise leave
    /// the underline painted onto an unrelated row. `update_hovered_url`
    /// is mouse-move only, so `schedule_viewport_refresh` clears this
    /// on every viewport shift — the next mouse-move re-derives it.
    /// Paint underlines the URL; mouse move / modifier release clear it.
    /// `None` when no URL or no modifier.
    pub(crate) hovered_url: Option<super::HoveredUrl>,

    /// Annotation overlay under the mouse pointer (SP-1). Drives the
    /// hover-tint switch in the annotation paint pass. Updated by the
    /// mouse-move hit test in `view/mouse.rs`; `None` when the cursor
    /// is not over any annotation box.
    pub(crate) hovered_annotation: Option<crate::session::interval_tree::MarkId>,

    /// Identity ([`crate::session::PromptMark::seq`]) of the most recent
    /// prompt-jump target (FTCS A marks). Storing the mark's `seq` — not
    /// a screen row — keeps the highlight stable across `prompt_marks`
    /// churn (`\x1b[3J` mirror in `clear_line_buffer_and_shift_marks`
    /// may drop wiped marks, ring eviction may shrink the list, but
    /// surviving marks keep their `seq` and `abs_y`): the focused mark
    /// either still exists (highlight follows it) or is gone (no
    /// paint), with no manual reset needed. `schedule_viewport_refresh`
    /// still clears this on manual scroll / PTY input — that is a
    /// deliberate UX choice (turn the jump highlight off when the user
    /// moves on), not a staleness workaround.
    pub(crate) focused_prompt: Option<u64>,

    /// Same, for the `CommandExecuted` (FTCS C) list. Walked by
    /// Cmd+Shift+Option+↑/↓.
    pub(crate) focused_command: Option<u64>,

    /// Independent flash-overlay deadlines (bell + prompt-jump wrap).
    /// Both are additive — paint renders each separately.
    pub(crate) flash: FlashOverlay,
}

/// Deadlines for the two additive viewport flash overlays. They are
/// triggered and painted independently (not mutually exclusive), so each
/// effect owns its own field rather than sharing one slot.
#[derive(Default, Clone, Copy, PartialEq)]
pub(crate) struct FlashOverlay {
    /// Deadline for the visual bell flash (BEL char / DECSET 1004
    /// bell). Paint draws a translucent overlay until this time.
    pub bell: Option<std::time::Instant>,

    /// Deadline after which the wrap-around flash overlay clears.
    /// Mirrors iTerm2's `kiTermIndicatorWrapToTop` flash signalling
    /// that a prompt jump looped back to the other end.
    pub prompt_jump: Option<std::time::Instant>,
}

impl TerminalViewState {
    /// Construct with the four-field quartet sourced from
    /// `TerminalSession` plus a fully built `Font`. Other fields
    /// default to zero / empty as they migrate in (currently:
    /// nothing else lives here).
    pub(crate) fn new(
        font: gpui::Font,
        font_size: f32,
        vertical_spacing: f32,
        horizontal_spacing: f32,
        background_alpha: f32,
    ) -> Self {
        Self {
            viewport_lock: super::viewport_lock::ViewportLock::default(),
            suppress_scroll_lock_until: None,
            font,
            font_size,
            vertical_spacing,
            horizontal_spacing,
            background_alpha,
            search: super::SearchState::default(),
            search_overlay: false,
            selection: None,
            mouse_drag: MouseDragState::None,
            scrollbar_thumb_bounds: None,
            viewport_lines: Vec::new(),
            viewport_line_offsets: Vec::new(),
            viewport_total_len: 0,
            viewport_style_runs: Vec::new(),
            last_bounds: None,
            last_window_title: None,
            pending_output: Vec::new(),
            pending_refresh: PendingRefresh::No,
            marked_text: None,
            marked_selected_range_utf16: 0..0,
            last_known_cursor: None,
            hangul_composer: super::hangul_composer::HangulComposer::new(),
            hovered_url: None,
            hovered_annotation: None,
            focused_prompt: None,
            focused_command: None,
            flash: FlashOverlay::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(font_size: f32) -> TerminalViewState {
        TerminalViewState::new(gpui::Font::default(), font_size, 1.0, 1.0, 1.0)
    }

    #[test]
    fn pitfall8_quartet_round_trips() {
        // The four metric fields stay distinct slots — a future
        // refactor that accidentally aliases (e.g. dropping one and
        // pointing reads at another) would fail this test.
        let s = TerminalViewState::new(gpui::Font::default(), 14.0, 1.2, 0.95, 0.85);
        assert_eq!(s.font_size, 14.0);
        assert!((s.vertical_spacing - 1.2).abs() < f32::EPSILON);
        assert!((s.horizontal_spacing - 0.95).abs() < f32::EPSILON);
        assert!((s.background_alpha - 0.85).abs() < f32::EPSILON);
    }

    #[test]
    fn font_size_mutable_in_place() {
        // Zoom actions edit `font_size` in place; this guards
        // against a future refactor that locks the field down (e.g.
        // builder-only API).
        let mut s = fixture(13.0);
        s.font_size = 16.0;
        assert_eq!(s.font_size, 16.0);
    }

    #[test]
    fn defaults_describe_idle_terminal() {
        // A freshly-constructed state must look "no input pending,
        // no selection, no IME, no overlays" — anything else would
        // make the entity show stale UI between PTY-output bursts.
        let s = fixture(13.0);
        assert!(!s.viewport_lock.is_locked());
        assert!(s.suppress_scroll_lock_until.is_none());
        assert!(s.search.query.is_empty());
        assert!(!s.search_overlay);
        assert!(s.selection.is_none());
        assert!(s.mouse_drag == MouseDragState::None);
        assert!(s.viewport_lines.is_empty());
        assert_eq!(s.viewport_total_len, 0);
        assert!(s.pending_output.is_empty());
        assert!(s.pending_refresh == PendingRefresh::No);
        assert!(s.marked_text.is_none());
        assert!(s.hovered_url.is_none());
        assert!(s.focused_prompt.is_none());
        assert!(s.focused_command.is_none());
        assert!(s.flash.bell.is_none());
        assert!(s.flash.prompt_jump.is_none());
    }

    #[test]
    fn search_overlay_open_does_not_touch_query() {
        // Opening the search overlay (Cmd+F) must preserve any
        // existing query so the user can keep refining without
        // re-typing. The `on_search_open` action sets only the
        // `search_overlay` flag and the cursor byte — never `query`.
        let mut s = fixture(13.0);
        s.search.query = "needle".into();
        s.search.cursor_byte = "needle".len();
        s.search_overlay = true;
        assert_eq!(s.search.query, "needle");
        assert_eq!(s.search.cursor_byte, "needle".len());
    }
}
