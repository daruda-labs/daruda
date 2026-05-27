//! Safe Rust wrapper over `libghostty-vt` (Zig).
//!
//! ## FFI safety contract
//!
//! Every `unsafe` block in this crate upholds one of three invariants:
//!
//! 1. **Terminal pointer.** [`Terminal`] holds a [`NonNull<c_void>`]
//!    returned by `ghostty_vt_terminal_new`. The pointer is non-null
//!    by `NonNull` invariant, exclusively owned (no `Clone`), valid
//!    for the lifetime of the wrapper, and freed exactly once in
//!    [`Drop`]. Every `&self` / `&mut self` method passes
//!    `self.ptr.as_ptr()` to a C function that does **not** retain
//!    the pointer beyond the call.
//! 2. **Borrowed Rust slice → C.** `bytes.as_ptr()` + `bytes.len()`
//!    describe a valid byte slice the C side reads but does not
//!    retain.
//! 3. **C-allocated buffer → Rust.** `ghostty_vt_*` allocator calls
//!    return a `(ptr, len)` pair valid until the matching
//!    `ghostty_vt_bytes_free`. We null-check before dereferencing,
//!    construct a slice via [`std::slice::from_raw_parts`] for the
//!    advertised length, and pair every successful allocation with
//!    exactly one free.

use std::ffi::c_void;
use std::fmt;
use std::ptr::NonNull;

#[derive(Debug)]
pub enum Error {
    CreateFailed,
    FeedFailed(i32),
    ScrollFailed(i32),
    DumpFailed,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::CreateFailed => write!(f, "terminal create failed"),
            Error::FeedFailed(code) => write!(f, "terminal feed failed: {code}"),
            Error::ScrollFailed(code) => write!(f, "terminal scroll failed: {code}"),
            Error::DumpFailed => write!(f, "terminal dump failed"),
        }
    }
}

impl std::error::Error for Error {}

pub struct Terminal {
    ptr: NonNull<c_void>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellStyle {
    pub fg: Rgb,
    pub bg: Rgb,
    pub flags: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StyleRun {
    pub start_col: u16,
    pub end_col: u16,
    pub fg: Rgb,
    pub bg: Rgb,
    pub flags: u8,
}

/// How a row was terminated. `Hard` means a literal newline (or no
/// trailing content), `Soft` means DECAWM auto-wrap (the next row is a
/// wrap-continuation of this one).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WrapKind {
    Hard,
    Soft,
}

/// One-shot grid event drained by [`Terminal::take_grid_events`].
///
/// These signal screen-affecting transitions that consumers (terminal
/// view, selection layer, etc.) need to react to once per occurrence:
///
/// - [`GridEvent::AltScreenToggle`]: alt-screen entered/exited via
///   DECSET/DECRST 47, 1047, or 1049. `entered = true` means the
///   alternate screen is now active.
/// - [`GridEvent::Ris`]: hard reset (ESC c). Implies alt-screen is also
///   left if it was active — that exit is reported as a separate
///   `AltScreenToggle { entered: false }` immediately before the `Ris`
///   event.
///
/// Tag bytes on the FFI wire must stay in sync with `GRID_EVENT_*` in
/// `crates/ghostty_vt_sys/zig/lib.zig`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GridEvent {
    AltScreenToggle { entered: bool },
    Ris,
}

const GRID_EVENT_TAG_ALT_SCREEN_ENTER: u8 = 0;
const GRID_EVENT_TAG_ALT_SCREEN_EXIT: u8 = 1;
const GRID_EVENT_TAG_RIS: u8 = 2;
const GRID_EVENT_RECORD_BYTES: usize = 2;

#[derive(Clone, Copy, Debug, Default)]
pub struct KeyModifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub super_key: bool,
}

impl KeyModifiers {
    fn bits(self) -> u16 {
        let mut bits = 0u16;
        if self.shift {
            bits |= 0x0001;
        }
        if self.control {
            bits |= 0x0002;
        }
        if self.alt {
            bits |= 0x0004;
        }
        if self.super_key {
            bits |= 0x0008;
        }
        bits
    }
}

/// Terminal key-mode flags passed to the key encoder.
///
/// Bit 0: `cursor_key_application` — DECCKM (mode 1). When set, arrow
///        keys emit `\x1bOA`–`\x1bOD` (application) instead of `\x1b[A`–`\x1b[D`.
/// Bit 1: `keypad_key_application` — DECNKM (mode 66). When set, numeric
///        keypad keys emit application sequences instead of ASCII digits.
#[derive(Clone, Copy, Debug, Default)]
pub struct KeyModeFlags {
    pub cursor_key_application: bool,
    pub keypad_key_application: bool,
}

impl KeyModeFlags {
    fn bits(self) -> u8 {
        let mut bits = 0u8;
        if self.cursor_key_application {
            bits |= 0x01;
        }
        if self.keypad_key_application {
            bits |= 0x02;
        }
        bits
    }
}

pub fn encode_key_named(
    name: &str,
    modifiers: KeyModifiers,
    mode_flags: KeyModeFlags,
) -> Option<Vec<u8>> {
    if name.is_empty() {
        return None;
    }

    // SAFETY: `name` is a valid `&str`, so `name.as_ptr()` + `name.len()`
    // describe a UTF-8 byte slice the C function only reads (invariant #2).
    let bytes = unsafe {
        ghostty_vt_sys::ghostty_vt_encode_key_named(
            name.as_ptr(),
            name.len(),
            modifiers.bits(),
            mode_flags.bits(),
        )
    };
    if bytes.ptr.is_null() || bytes.len == 0 {
        return None;
    }

    // SAFETY: `bytes.ptr` is non-null and `bytes.len` non-zero (just
    // checked); the buffer is owned by ghostty_vt and stays valid until
    // `ghostty_vt_bytes_free` (invariant #3).
    let slice = unsafe { std::slice::from_raw_parts(bytes.ptr, bytes.len) };
    let out = slice.to_vec();
    // SAFETY: `bytes` was returned from the matching `ghostty_vt_encode_key_named`
    // call above and has not been freed yet (invariant #3).
    unsafe { ghostty_vt_sys::ghostty_vt_bytes_free(bytes) };
    Some(out)
}

/// Default scrollback row limit — matches ghostty's built-in default.
pub const DEFAULT_MAX_SCROLLBACK: usize = 10_000;

impl Terminal {
    pub fn new(cols: u16, rows: u16) -> Result<Self, Error> {
        Self::with_scrollback(cols, rows, DEFAULT_MAX_SCROLLBACK)
    }

    pub fn with_scrollback(cols: u16, rows: u16, max_scrollback: usize) -> Result<Self, Error> {
        // SAFETY: pure constructor — no preconditions. May return null
        // on allocation failure, which we convert to `Error::CreateFailed`.
        let ptr = unsafe { ghostty_vt_sys::ghostty_vt_terminal_new(cols, rows, max_scrollback) };
        let ptr = NonNull::new(ptr).ok_or(Error::CreateFailed)?;
        Ok(Self { ptr })
    }

    pub fn set_default_colors(&mut self, fg: Rgb, bg: Rgb) {
        // SAFETY: `self.ptr` upholds invariant #1; `Rgb` fields are
        // plain `u8` passed by value.
        unsafe {
            ghostty_vt_sys::ghostty_vt_terminal_set_default_colors(
                self.ptr.as_ptr(),
                fg.r,
                fg.g,
                fg.b,
                bg.r,
                bg.g,
                bg.b,
            )
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Result<(), Error> {
        // SAFETY: `self.ptr` upholds invariant #1; `bytes` is a valid
        // borrowed Rust slice (invariant #2). The C side reads only
        // `bytes.len()` bytes and does not retain the pointer.
        let rc = unsafe {
            ghostty_vt_sys::ghostty_vt_terminal_feed(self.ptr.as_ptr(), bytes.as_ptr(), bytes.len())
        };
        if rc == 0 {
            Ok(())
        } else {
            Err(Error::FeedFailed(rc))
        }
    }

    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), Error> {
        // SAFETY: `self.ptr` upholds invariant #1.
        let rc =
            unsafe { ghostty_vt_sys::ghostty_vt_terminal_resize(self.ptr.as_ptr(), cols, rows) };
        if rc == 0 {
            Ok(())
        } else {
            Err(Error::ScrollFailed(rc))
        }
    }

    pub fn dump_viewport(&self) -> Result<String, Error> {
        // SAFETY: `self.ptr` upholds invariant #1.
        let bytes = unsafe { ghostty_vt_sys::ghostty_vt_terminal_dump_viewport(self.ptr.as_ptr()) };
        if bytes.ptr.is_null() {
            return Err(Error::DumpFailed);
        }

        // SAFETY: `bytes.ptr` is non-null (just checked) and `bytes.len`
        // is the C-reported length; valid until the paired free below
        // (invariant #3).
        let slice = unsafe { std::slice::from_raw_parts(bytes.ptr, bytes.len) };
        let s = String::from_utf8_lossy(slice).into_owned();
        // SAFETY: `bytes` was returned by the matching dump call and
        // has not been freed yet (invariant #3).
        unsafe { ghostty_vt_sys::ghostty_vt_bytes_free(bytes) };
        Ok(s)
    }

    pub fn dump_viewport_row(&self, row: u16) -> Result<String, Error> {
        // SAFETY: `self.ptr` upholds invariant #1.
        let bytes = unsafe {
            ghostty_vt_sys::ghostty_vt_terminal_dump_viewport_row(self.ptr.as_ptr(), row)
        };
        if bytes.ptr.is_null() {
            return Err(Error::DumpFailed);
        }

        // SAFETY: invariant #3 — non-null buffer with C-reported length,
        // valid until the paired free below.
        let slice = unsafe { std::slice::from_raw_parts(bytes.ptr, bytes.len) };
        let s = String::from_utf8_lossy(slice).into_owned();
        // SAFETY: paired free for the buffer obtained above (invariant #3).
        unsafe { ghostty_vt_sys::ghostty_vt_bytes_free(bytes) };
        Ok(s)
    }

    /// Dump one row from the `screen` coordinate space (scrollback +
    /// active area). `y` is 0-based; use `total_rows` as an exclusive
    /// upper bound.
    pub fn dump_screen_row(&self, y: u32) -> Result<String, Error> {
        // SAFETY: `self.ptr` upholds invariant #1.
        let bytes =
            unsafe { ghostty_vt_sys::ghostty_vt_terminal_dump_screen_row(self.ptr.as_ptr(), y) };
        if bytes.ptr.is_null() {
            return Err(Error::DumpFailed);
        }
        // SAFETY: invariant #3 — non-null buffer with C-reported length.
        let slice = unsafe { std::slice::from_raw_parts(bytes.ptr, bytes.len) };
        let s = String::from_utf8_lossy(slice).into_owned();
        // SAFETY: paired free (invariant #3).
        unsafe { ghostty_vt_sys::ghostty_vt_bytes_free(bytes) };
        Ok(s)
    }

    pub fn dump_viewport_row_cell_styles(&self, row: u16) -> Result<Vec<CellStyle>, Error> {
        // SAFETY: `self.ptr` upholds invariant #1.
        let bytes = unsafe {
            ghostty_vt_sys::ghostty_vt_terminal_dump_viewport_row_cell_styles(
                self.ptr.as_ptr(),
                row,
            )
        };
        if bytes.ptr.is_null() {
            return Err(Error::DumpFailed);
        }
        if bytes.len == 0 {
            // SAFETY: paired free of an empty (but non-null) buffer
            // (invariant #3).
            unsafe { ghostty_vt_sys::ghostty_vt_bytes_free(bytes) };
            return Ok(Vec::new());
        }
        if bytes.len % 8 != 0 {
            // SAFETY: paired free before propagating the parse error
            // (invariant #3).
            unsafe { ghostty_vt_sys::ghostty_vt_bytes_free(bytes) };
            return Err(Error::DumpFailed);
        }

        // SAFETY: non-null buffer with C-reported length, multiple of 8
        // bytes per cell-style record (invariant #3).
        let slice = unsafe { std::slice::from_raw_parts(bytes.ptr, bytes.len) };
        let mut out = Vec::with_capacity(bytes.len / 8);
        for chunk in slice.chunks_exact(8) {
            out.push(CellStyle {
                fg: Rgb {
                    r: chunk[0],
                    g: chunk[1],
                    b: chunk[2],
                },
                bg: Rgb {
                    r: chunk[3],
                    g: chunk[4],
                    b: chunk[5],
                },
                flags: chunk[6],
            });
        }

        // SAFETY: paired free for the buffer parsed above (invariant #3).
        unsafe { ghostty_vt_sys::ghostty_vt_bytes_free(bytes) };
        Ok(out)
    }

    pub fn dump_viewport_row_style_runs(&self, row: u16) -> Result<Vec<StyleRun>, Error> {
        // SAFETY: `self.ptr` upholds invariant #1.
        let bytes = unsafe {
            ghostty_vt_sys::ghostty_vt_terminal_dump_viewport_row_style_runs(self.ptr.as_ptr(), row)
        };
        parse_style_runs(bytes)
    }

    /// Screen-coordinate variant of `dump_viewport_row_style_runs`. `y`
    /// is 0-based and may address any row in scrollback or the active
    /// viewport. Used by the LineBuffer capture path to fetch styles
    /// for a row that has just scrolled out of the viewport.
    pub fn dump_screen_row_style_runs(&self, y: u32) -> Result<Vec<StyleRun>, Error> {
        // SAFETY: `self.ptr` upholds invariant #1.
        let bytes = unsafe {
            ghostty_vt_sys::ghostty_vt_terminal_dump_screen_row_style_runs(self.ptr.as_ptr(), y)
        };
        parse_style_runs(bytes)
    }

    /// Per-cell OSC 8 hyperlink IDs for a row in screen coordinates.
    /// `y` covers `[0, total_rows())`. Returns one `u16` per cell —
    /// `0` indicates the cell has no hyperlink. Used by the LineBuffer
    /// capture path so OSC 8 link IDs survive a row scrolling out of
    /// the viewport.
    pub fn dump_screen_row_url_ids(&self, y: u32) -> Result<Vec<u16>, Error> {
        // SAFETY: `self.ptr` upholds invariant #1.
        let bytes = unsafe {
            ghostty_vt_sys::ghostty_vt_terminal_dump_screen_row_url_ids(self.ptr.as_ptr(), y)
        };
        if bytes.ptr.is_null() {
            return Err(Error::DumpFailed);
        }
        if bytes.len == 0 {
            // SAFETY: paired free of an empty (but non-null) buffer
            // (invariant #3).
            unsafe { ghostty_vt_sys::ghostty_vt_bytes_free(bytes) };
            return Ok(Vec::new());
        }
        if bytes.len % 2 != 0 {
            // SAFETY: paired free before propagating the parse error
            // (invariant #3).
            unsafe { ghostty_vt_sys::ghostty_vt_bytes_free(bytes) };
            return Err(Error::DumpFailed);
        }
        // SAFETY: non-null buffer with C-reported length, multiple of
        // 2 bytes per u16 (invariant #3).
        let slice = unsafe { std::slice::from_raw_parts(bytes.ptr, bytes.len) };
        let mut out = Vec::with_capacity(bytes.len / 2);
        for chunk in slice.chunks_exact(2) {
            out.push(u16::from_ne_bytes([chunk[0], chunk[1]]));
        }
        // SAFETY: paired free for the buffer parsed above (invariant #3).
        unsafe { ghostty_vt_sys::ghostty_vt_bytes_free(bytes) };
        Ok(out)
    }

    /// Returns the wrap kind for the row at absolute screen coordinate
    /// `y`. `WrapKind::Hard` means the row ended with a hard newline;
    /// `WrapKind::Soft` means DECAWM auto-wrap. Used to merge
    /// wrap-continuations during LineBuffer capture.
    pub fn row_wrap_kind(&self, y: u32) -> WrapKind {
        // SAFETY: `self.ptr` upholds invariant #1.
        let code =
            unsafe { ghostty_vt_sys::ghostty_vt_terminal_row_wrap_kind(self.ptr.as_ptr(), y) };
        match code {
            1 => WrapKind::Soft,
            _ => WrapKind::Hard,
        }
    }

    pub fn take_dirty_viewport_rows(&mut self, rows: u16) -> Result<Vec<u16>, Error> {
        // SAFETY: `self.ptr` upholds invariant #1.
        let bytes = unsafe {
            ghostty_vt_sys::ghostty_vt_terminal_take_dirty_viewport_rows(self.ptr.as_ptr(), rows)
        };
        if bytes.ptr.is_null() || bytes.len == 0 {
            return Ok(Vec::new());
        }
        if bytes.len % 2 != 0 {
            // SAFETY: paired free before propagating the parse error
            // (invariant #3).
            unsafe { ghostty_vt_sys::ghostty_vt_bytes_free(bytes) };
            return Err(Error::DumpFailed);
        }

        // SAFETY: non-null buffer with C-reported length, multiple of
        // 2 bytes per `u16` row index (invariant #3).
        let slice = unsafe { std::slice::from_raw_parts(bytes.ptr, bytes.len) };
        let mut out = Vec::with_capacity(bytes.len / 2);
        for chunk in slice.chunks_exact(2) {
            out.push(u16::from_le_bytes([chunk[0], chunk[1]]));
        }
        // SAFETY: paired free for the buffer parsed above (invariant #3).
        unsafe { ghostty_vt_sys::ghostty_vt_bytes_free(bytes) };
        Ok(out)
    }

    /// Drain pending [`GridEvent`]s recorded by alt-screen mode changes
    /// and hard resets since the last call. Unknown tag bytes are
    /// skipped silently so future event kinds added on the Zig side do
    /// not crash older callers.
    ///
    /// Callers are expected to drain this regularly (e.g. after each
    /// frame's worth of feed). The internal buffer on the Zig side
    /// otherwise grows with every alt-screen toggle and RIS.
    pub fn take_grid_events(&mut self) -> Vec<GridEvent> {
        // SAFETY: `self.ptr` upholds invariant #1.
        let bytes =
            unsafe { ghostty_vt_sys::ghostty_vt_terminal_take_grid_events(self.ptr.as_ptr()) };
        // Zig contract (`ghostty_vt_terminal_take_grid_events`): `ptr`
        // is null iff `len` is 0, so a single null check covers both
        // the "no events" and the allocation-failure paths.
        if bytes.ptr.is_null() {
            return Vec::new();
        }
        // Truncate any partial trailing record so the loop never reads
        // past the buffer; record size is constant 2 bytes.
        let usable = bytes.len - (bytes.len % GRID_EVENT_RECORD_BYTES);
        // SAFETY: non-null buffer with C-reported length; `usable` is
        // a multiple of `GRID_EVENT_RECORD_BYTES` and `<= bytes.len`
        // (invariant #3).
        let slice = unsafe { std::slice::from_raw_parts(bytes.ptr, usable) };
        let mut out = Vec::with_capacity(usable / GRID_EVENT_RECORD_BYTES);
        for chunk in slice.chunks_exact(GRID_EVENT_RECORD_BYTES) {
            match chunk[0] {
                GRID_EVENT_TAG_ALT_SCREEN_ENTER => {
                    out.push(GridEvent::AltScreenToggle { entered: true });
                }
                GRID_EVENT_TAG_ALT_SCREEN_EXIT => {
                    out.push(GridEvent::AltScreenToggle { entered: false });
                }
                GRID_EVENT_TAG_RIS => out.push(GridEvent::Ris),
                _ => {}
            }
        }
        // SAFETY: paired free for the buffer parsed above (invariant #3).
        unsafe { ghostty_vt_sys::ghostty_vt_bytes_free(bytes) };
        out
    }

    /// Returns cursor style code (DECSCUSR value).
    /// 0=default(block), 1=blinking block, 2=steady block,
    /// 3=blinking underline, 4=steady underline, 5=blinking bar, 6=steady bar
    pub fn cursor_style(&self) -> u8 {
        // SAFETY: `self.ptr` upholds invariant #1.
        unsafe { ghostty_vt_sys::ghostty_vt_terminal_cursor_style(self.ptr.as_ptr()) }
    }

    /// Returns whether the cursor is currently visible per DECTCEM
    /// (CSI ?25). Full-screen TUIs (Claude Code, vim, less) send
    /// `?25 l` to hide the hardware caret while they draw their own;
    /// honouring this keeps daruda from painting a duplicate caret
    /// on top of the app's UI.
    pub fn cursor_visible(&self) -> bool {
        // SAFETY: `self.ptr` upholds invariant #1.
        unsafe { ghostty_vt_sys::ghostty_vt_terminal_cursor_visible(self.ptr.as_ptr()) }
    }

    /// Returns true if a bell (BEL) was received since last call.
    pub fn take_bell(&mut self) -> bool {
        // SAFETY: `self.ptr` upholds invariant #1.
        unsafe { ghostty_vt_sys::ghostty_vt_terminal_take_bell(self.ptr.as_ptr()) }
    }

    pub fn total_rows(&self) -> u32 {
        // SAFETY: `self.ptr` upholds invariant #1.
        unsafe { ghostty_vt_sys::ghostty_vt_terminal_total_rows(self.ptr.as_ptr()) }
    }

    pub fn viewport_row_offset(&self) -> u32 {
        // SAFETY: `self.ptr` upholds invariant #1.
        unsafe { ghostty_vt_sys::ghostty_vt_terminal_viewport_row_offset(self.ptr.as_ptr()) }
    }

    /// Rows that scrolled off the active area into scrollback history since
    /// the previous call. Unlike [`viewport_row_offset`], this stays correct
    /// after the scrollback ring saturates (it is backed by a tracked pin, so
    /// pruning does not perturb the delta), giving callers a monotonic
    /// per-feed scroll count for mirroring scrolled-off rows into their own
    /// scrollback. Returns the full current scrollback depth on the first
    /// call (and after a reset that drops the watermark).
    ///
    /// [`viewport_row_offset`]: Self::viewport_row_offset
    pub fn take_scrolled_rows(&mut self) -> u32 {
        // SAFETY: `self.ptr` upholds invariant #1.
        unsafe { ghostty_vt_sys::ghostty_vt_terminal_take_scrolled_rows(self.ptr.as_ptr()) }
    }

    pub fn take_viewport_scroll_delta(&mut self) -> i32 {
        // SAFETY: `self.ptr` upholds invariant #1.
        unsafe { ghostty_vt_sys::ghostty_vt_terminal_take_viewport_scroll_delta(self.ptr.as_ptr()) }
    }

    pub fn cursor_position(&self) -> Option<(u16, u16)> {
        let mut col: u16 = 0;
        let mut row: u16 = 0;
        // SAFETY: `self.ptr` upholds invariant #1; `&mut col` / `&mut row`
        // are valid Rust references to stack-local `u16`s — the C side
        // writes through them once and does not retain the pointers.
        let ok = unsafe {
            ghostty_vt_sys::ghostty_vt_terminal_cursor_position(
                self.ptr.as_ptr(),
                &mut col as *mut u16,
                &mut row as *mut u16,
            )
        };
        ok.then_some((col, row))
    }

    pub fn hyperlink_at(&self, col: u16, row: u16) -> Option<String> {
        // SAFETY: `self.ptr` upholds invariant #1.
        let bytes = unsafe {
            ghostty_vt_sys::ghostty_vt_terminal_hyperlink_at(self.ptr.as_ptr(), col, row)
        };
        if bytes.ptr.is_null() || bytes.len == 0 {
            return None;
        }

        // SAFETY: invariant #3 — non-null buffer with C-reported length.
        let slice = unsafe { std::slice::from_raw_parts(bytes.ptr, bytes.len) };
        let s = String::from_utf8_lossy(slice).into_owned();
        // SAFETY: paired free for the buffer above (invariant #3).
        unsafe { ghostty_vt_sys::ghostty_vt_bytes_free(bytes) };
        Some(s)
    }

    pub fn scroll_viewport(&mut self, delta_lines: i32) -> Result<(), Error> {
        // SAFETY: `self.ptr` upholds invariant #1.
        let rc = unsafe {
            ghostty_vt_sys::ghostty_vt_terminal_scroll_viewport(self.ptr.as_ptr(), delta_lines)
        };
        if rc == 0 {
            Ok(())
        } else {
            Err(Error::ScrollFailed(rc))
        }
    }

    pub fn scroll_viewport_top(&mut self) -> Result<(), Error> {
        // SAFETY: `self.ptr` upholds invariant #1.
        let rc =
            unsafe { ghostty_vt_sys::ghostty_vt_terminal_scroll_viewport_top(self.ptr.as_ptr()) };
        if rc == 0 {
            Ok(())
        } else {
            Err(Error::ScrollFailed(rc))
        }
    }

    pub fn scroll_viewport_bottom(&mut self) -> Result<(), Error> {
        // SAFETY: `self.ptr` upholds invariant #1.
        let rc = unsafe {
            ghostty_vt_sys::ghostty_vt_terminal_scroll_viewport_bottom(self.ptr.as_ptr())
        };
        if rc == 0 {
            Ok(())
        } else {
            Err(Error::ScrollFailed(rc))
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // SAFETY: `self.ptr` upholds invariant #1; `Drop` runs exactly
        // once because `Terminal` is not `Clone` and the field is
        // private. The pointer is invalidated immediately after.
        unsafe { ghostty_vt_sys::ghostty_vt_terminal_free(self.ptr.as_ptr()) }
    }
}

/// Parse a `(ptr, len)` buffer of packed 12-byte `StyleRun` records (as
/// produced by the Zig `dump_*_row_style_runs` exports) into a
/// `Vec<StyleRun>`. Always frees the input buffer before returning.
fn parse_style_runs(bytes: ghostty_vt_sys::ghostty_vt_bytes_t) -> Result<Vec<StyleRun>, Error> {
    if bytes.ptr.is_null() {
        return Err(Error::DumpFailed);
    }
    if bytes.len == 0 {
        // SAFETY: paired free of an empty (but non-null) buffer
        // (invariant #3).
        unsafe { ghostty_vt_sys::ghostty_vt_bytes_free(bytes) };
        return Ok(Vec::new());
    }
    if !bytes.len.is_multiple_of(12) {
        // SAFETY: paired free before propagating the parse error
        // (invariant #3).
        unsafe { ghostty_vt_sys::ghostty_vt_bytes_free(bytes) };
        return Err(Error::DumpFailed);
    }

    // SAFETY: non-null buffer with C-reported length, multiple of
    // 12 bytes per style-run record (invariant #3).
    let slice = unsafe { std::slice::from_raw_parts(bytes.ptr, bytes.len) };
    let mut out = Vec::with_capacity(bytes.len / 12);
    for chunk in slice.chunks_exact(12) {
        out.push(StyleRun {
            start_col: u16::from_ne_bytes([chunk[0], chunk[1]]),
            end_col: u16::from_ne_bytes([chunk[2], chunk[3]]),
            fg: Rgb {
                r: chunk[4],
                g: chunk[5],
                b: chunk[6],
            },
            bg: Rgb {
                r: chunk[7],
                g: chunk[8],
                b: chunk[9],
            },
            flags: chunk[10],
        });
    }

    // SAFETY: paired free for the buffer parsed above (invariant #3).
    unsafe { ghostty_vt_sys::ghostty_vt_bytes_free(bytes) };
    Ok(out)
}

pub fn terminal_new(cols: u16, rows: u16) -> Result<Terminal, Error> {
    Terminal::new(cols, rows)
}

pub fn terminal_new_with_scrollback(
    cols: u16,
    rows: u16,
    max_scrollback: usize,
) -> Result<Terminal, Error> {
    Terminal::with_scrollback(cols, rows, max_scrollback)
}
