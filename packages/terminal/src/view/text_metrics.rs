//! Text/cell measurement helpers.
//!
//! Pixel ↔ column ↔ byte conversion utilities. Used by every overlay
//! that needs to align with the terminal grid (search highlights, URL
//! hover underline, search-bar click-to-position, IME positioning).
//!
//! **Cell-range pixel positioning rule** — see
//! [`shaped_pixel_range_for_cols`]. Always use this helper when
//! drawing an overlay tied to grid columns; calling
//! `cell_width * col` directly drifts on rows containing wide /
//! CJK / emoji glyphs because GPUI's shaper only forces monospace
//! widths when the row is pure narrow text.

use gpui::Pixels;

/// Cell metrics at an explicit font size — used by overlays (search
/// bar input) that render text at a different size from the terminal
/// grid. Reading the inherited size for those would give a
/// mismatched cell width and drift any pixel→column math.
pub(crate) fn cell_metrics_at(
    window: &mut gpui::Window,
    font: &gpui::Font,
    font_size: gpui::Pixels,
) -> Option<(f32, f32)> {
    let mut style = window.text_style();
    style.font_family = font.family.clone();
    style.font_features = crate::default_terminal_font_features();
    style.font_fallbacks = font.fallbacks.clone();
    style.font_size = font_size.into();

    let run = style.to_run(1);
    let lines = window
        .text_system()
        .shape_text(
            gpui::SharedString::from("M"),
            font_size,
            &[run],
            None,
            Some(1),
        )
        .ok()?;
    let line = lines.first()?;

    let cell_width = f32::from(line.width()).max(1.0);
    // Use the font's natural metrics (`ascent + descent`) rather than
    // `style.line_height` — GPUI's default style puts `line_height`
    // at `phi()` (1.618 × font_size), roughly 40% taller than a
    // terminal wants. iTerm2's `PTYFontInfo` computes
    // `charHeight = ceil(ascender - descender + leading)`; the same
    // value falls out of `LineLayout.ascent + descent` here, so
    // `vertical_spacing = 1.0` lands on iTerm2's baseline cell
    // height instead of drifting ~6px per row taller.
    let cell_height = f32::from(line.ascent() + line.descent()).max(1.0);
    Some((cell_width, cell_height))
}

/// Map a horizontal pixel offset (from the first glyph of `text`) to
/// a byte index in `text`, by shaping the whole string at the exact
/// font + size used for rendering and asking the shaper for its
/// inverse mapping.
///
/// **Why not `offset_px / cell_width`?** `cell_metrics_at` measures
/// a single monospace glyph (`"M"`), which matches only narrow ASCII.
/// CJK / emoji / wide glyphs render at ~2× that advance, so a
/// cell-width division counts each wide char as two cells and the
/// mapping drifts further from the click point with every wide
/// glyph encountered.
///
/// Returns `None` if shaping fails or the text is empty — callers
/// should treat empty text as "cursor at 0" and shaping failure as
/// "cursor at end" (same fallback the click handler uses when the
/// panel bounds are unknown).
pub(crate) fn byte_index_for_x_in_text(
    window: &mut gpui::Window,
    font: &gpui::Font,
    font_size: Pixels,
    text: &str,
    offset_px: Pixels,
) -> Option<usize> {
    if text.is_empty() {
        return Some(0);
    }
    let mut style = window.text_style();
    style.font_family = font.family.clone();
    style.font_features = crate::default_terminal_font_features();
    style.font_fallbacks = font.fallbacks.clone();
    style.font_size = font_size.into();

    let run = style.to_run(text.len());
    let lines = window
        .text_system()
        .shape_text(
            gpui::SharedString::from(text.to_string()),
            font_size,
            &[run],
            None,
            Some(1),
        )
        .ok()?;
    let line = lines.first()?;

    let clamped = offset_px.max(gpui::px(0.0)).min(line.width());
    Some(
        line.unwrapped_layout
            .closest_index_for_x(clamped)
            .min(text.len()),
    )
}

/// Pixel x of the **left edge** of the grid cell at `(col, row)`,
/// using the cached ShapedLine for that row when available and
/// falling back to `cell_width * (col - 1)` otherwise.
///
/// **Why not just `cell_width * (col - 1)`?** Rows containing wide
/// (CJK / emoji) or zero-width combining glyphs don't honour GPUI's
/// `force_width`, so cell-width arithmetic drifts by up to 1 cell
/// per wide glyph that appears **before** the cursor. The fallback
/// path is used when the row hasn't been shaped yet (e.g. the very
/// first `bounds_for_range` call before any paint).
pub(crate) fn cell_left_x_for_col(
    line_layouts: &[Option<gpui::ShapedLine>],
    col: u16,
    row: u16,
    cell_width: f32,
    origin_left: Pixels,
) -> Pixels {
    let row_idx = row.saturating_sub(1) as usize;
    // The shaped line's `text` is the row text (built from `viewport_lines`
    // and recleared on every dirty update), so `shaped_x_for_col` derives
    // both the byte index and the pixel from that one string — keeping the
    // preedit anchor consistent with the cursor and correct when the cursor
    // sits past a trailing space the row dump trimmed.
    if let Some(Some(line)) = line_layouts.get(row_idx) {
        return shaped_x_for_col(line, col, cell_width, origin_left);
    }
    origin_left + gpui::px(cell_width * col.saturating_sub(1) as f32)
}

/// Resolve an inclusive 1-indexed column range (`start_col..=end_col`)
/// to a horizontal pixel span on the already-shaped line.
///
/// **Always use this helper** when drawing an overlay tied to grid
/// columns (search highlight, URL hover underline, focused mark
/// stripe, …). Calling `cell_width * col` instead drifts by 1+ cells
/// on rows that contain wide / CJK / emoji glyphs because GPUI's
/// shaper only honours `force_width` when the row is pure narrow text.
///
/// Returns `None` when the resolved byte range is empty.
pub(crate) fn shaped_pixel_range_for_cols(
    shaped: &gpui::ShapedLine,
    line_text: &str,
    start_col: u16,
    end_col: u16,
) -> Option<(Pixels, Pixels)> {
    let text = shaped.text.as_str();
    let byte_start = super::text_edit::clamp_to_char_boundary(
        text,
        byte_index_for_column_in_line(line_text, start_col),
    );
    let byte_end = super::text_edit::clamp_to_char_boundary(
        text,
        byte_index_for_column_in_line(line_text, end_col.saturating_add(1)),
    );
    if byte_end <= byte_start {
        return None;
    }
    Some((shaped.x_for_index(byte_start), shaped.x_for_index(byte_end)))
}

/// Pixel x of a byte offset inside `text`, shaping `text` at the exact
/// font + size used for rendering. Used for IME preedit positioning —
/// `cell_width * cell_offset` drifts on mixed narrow / wide preedit
/// text because `element::prepaint` shapes any wide row with
/// `force_width = None`, so glyphs rendered beyond that point sit at their
/// natural advances rather than a multiple of the monospace cell.
pub(crate) fn x_for_byte_in_text(
    window: &mut gpui::Window,
    font: &gpui::Font,
    font_size: Pixels,
    text: &str,
    byte: usize,
) -> Option<Pixels> {
    if text.is_empty() || byte == 0 {
        return Some(gpui::px(0.0));
    }
    let mut style = window.text_style();
    style.font_family = font.family.clone();
    style.font_features = crate::default_terminal_font_features();
    style.font_fallbacks = font.fallbacks.clone();
    style.font_size = font_size.into();

    let run = style.to_run(text.len());
    let shaped = window.text_system().shape_line(
        gpui::SharedString::from(text.to_string()),
        font_size,
        &[run],
        None,
    );
    let byte = super::text_edit::clamp_to_char_boundary(text, byte.min(text.len()));
    Some(shaped.x_for_index(byte))
}

/// Map a 1-indexed display column to a UTF-8 byte index in `line`.
/// Wide chars (CJK, emoji) advance by their `unicode_width` so the
/// returned byte still lands on the start of the cell the column
/// names. Returns `line.len()` when the column is past the last
/// printable cell.
pub(crate) fn byte_index_for_column_in_line(line: &str, col: u16) -> usize {
    use unicode_width::UnicodeWidthChar as _;

    let col = col.max(1) as usize;
    if col == 1 {
        return 0;
    }

    let mut current_col = 1usize;
    for (byte_index, ch) in line.char_indices() {
        let width = ch.width().unwrap_or(0);
        if width == 0 {
            continue;
        }

        if current_col == col {
            return byte_index;
        }

        let next_col = current_col.saturating_add(width);
        if col < next_col {
            return byte_index;
        }

        current_col = next_col;
    }

    line.len()
}

/// Inverse of [`byte_index_for_column_in_line`]: map a UTF-8 byte
/// offset back to its 1-indexed display column. Bytes past the end of
/// `line` clamp to "one column past the last printable cell" so the
/// selection start at "after the last char" survives without
/// underflowing.
pub(crate) fn column_for_byte_in_line(line: &str, byte: usize) -> u16 {
    use unicode_width::UnicodeWidthChar as _;
    let target = byte.min(line.len());
    let mut col: u32 = 1;
    for (bi, ch) in line.char_indices() {
        if bi >= target {
            return col.min(u16::MAX as u32) as u16;
        }
        col = col.saturating_add(ch.width().unwrap_or(0) as u32);
    }
    col.min(u16::MAX as u32) as u16
}

/// Pixel x coordinate of the grid's right edge: the column just past
/// the last cell. Used by the selection highlight and drag-empty-row
/// fill so multi-row extensions stop at the grid boundary instead of
/// the element's `bounds.right()` (which may include outer padding).
///
/// iTerm2's selection rectangles are clipped to
/// `PTYTextView.liveRect`; Alacritty's renderer multiplies
/// `cell_width × columns`. Both match this formula.
pub(crate) fn grid_right_x(origin_left: Pixels, cell_width: f32, cols: u16) -> Pixels {
    origin_left + gpui::px(cell_width * cols as f32)
}

/// Total display columns occupied by `line` — the sum of per-char widths,
/// the same width model [`byte_index_for_column_in_line`] walks. Zero-width
/// combining marks contribute 0.
fn display_columns_in_line(line: &str) -> usize {
    use unicode_width::UnicodeWidthChar as _;
    line.chars().map(|c| c.width().unwrap_or(0)).sum()
}

/// Left-edge pixel x (from `origin_left`) of 1-indexed `col` within a shaped
/// row, accounting for trailing grid cells the row-text dump trimmed.
///
/// For columns inside the shaped text the shaper's `x_for_index` is
/// authoritative — `element::prepaint` shapes wide/CJK rows with
/// `force_width = None`, so glyphs keep their natural advance and the linear
/// `(col-1) × cell_width` drifts by up to a cell per wide glyph.
///
/// But the cursor (and the IME preedit anchored to it) can legitimately sit
/// *past* the last shaped glyph: a TUI such as Claude Code's input box
/// advances over a just-typed trailing space without writing a `0x20`, and
/// `encodeUtf8` trims that empty cell, so the shaped row is a column short.
/// Clamping `x_for_index` to the text length then paints the cursor/preedit
/// on top of the space, hiding it until a following glyph makes the space
/// non-trailing. Extend by `cell_width` per column beyond the shaped text so
/// the anchor lands on its true column. `col == shaped_cols + 1` is the
/// normal end-of-line position and still routes through the shaper.
/// Columns a 1-indexed `col` sits **past** the shaped text's last cell —
/// i.e. how many trimmed trailing grid cells the pixel mapping must bridge
/// with `cell_width`. `0` for any column inside the text *and* for the
/// normal end-of-line position (`shaped_cols + 1`); both route through the
/// shaper. This is the exact arithmetic the 6900398 CJK-cursor fix dropped,
/// so it lives in one tested place — see `shaped_x_for_col`.
fn cols_past_shaped_text(shaped_cols: usize, col: usize) -> usize {
    col.saturating_sub(shaped_cols + 1)
}

fn shaped_x_for_col(
    line: &gpui::ShapedLine,
    col: u16,
    cell_width: f32,
    origin_left: Pixels,
) -> Pixels {
    let text = line.text.as_str();
    let shaped_cols = display_columns_in_line(text);
    let col = col.max(1) as usize;
    match cols_past_shaped_text(shaped_cols, col) {
        0 => {
            let byte = byte_index_for_column_in_line(text, col as u16);
            let byte = super::text_edit::clamp_to_char_boundary(text, byte);
            origin_left + line.x_for_index(byte.min(text.len()))
        }
        extra => origin_left + line.x_for_index(text.len()) + gpui::px(cell_width * extra as f32),
    }
}

/// Left-edge pixel x of the cursor block for a given 1-indexed VT column.
/// Delegates to [`shaped_x_for_col`] so the trailing-cell handling stays in
/// one place and is not accidentally "simplified" to the linear form.
pub(crate) fn cursor_x_for_col(
    line: &gpui::ShapedLine,
    col: u16,
    origin_left: Pixels,
    cell_width: f32,
) -> Pixels {
    shaped_x_for_col(line, col, cell_width, origin_left)
}

/// Width of the cursor block in pixels for the glyph at 1-indexed `col`.
///
/// Measures the actual shaped advance of the character under the cursor so
/// the block covers exactly one character cell even on lines where
/// `force_width` is disabled (CJK / wide chars). Falls back to `cell_width`
/// when the cursor is past the last character (end-of-line position).
pub(crate) fn cursor_width_for_col(line: &gpui::ShapedLine, col: u16, cell_width: f32) -> f32 {
    let byte_index = byte_index_for_column_in_line(line.text.as_str(), col);
    let text = line.text.as_str();
    if byte_index < text.len() {
        let char_end = text
            .get(byte_index..)
            .and_then(|s| s.chars().next())
            .map(|c| byte_index + c.len_utf8())
            .unwrap_or(byte_index + 1)
            .min(text.len());
        let w = f32::from(line.x_for_index(char_end)) - f32::from(line.x_for_index(byte_index));
        if w > 0.5 { w } else { cell_width }
    } else {
        cell_width
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

    #[test]
    fn display_columns_counts_wide_chars_as_two() {
        assert_eq!(display_columns_in_line(""), 0);
        assert_eq!(display_columns_in_line("abc"), 3);
        assert_eq!(display_columns_in_line("한"), 2);
        // Claude Code input row: ❯ + NBSP + "한글" = 1 + 1 + 2 + 2.
        assert_eq!(display_columns_in_line("\u{276f}\u{a0}한글"), 6);
    }

    #[test]
    fn cols_past_shaped_text_bridges_trimmed_trailing_cells() {
        // Regression guard for the 6900398 CJK-cursor fix: switching the
        // cursor/preedit anchor from `cell_width × (col-1)` to the shaper's
        // `x_for_index` clamped any column past the shaped text onto the
        // last glyph, so a TUI cursor parked past a trimmed trailing space
        // painted on top of the space and hid it until the next keystroke.
        //
        // Row "❯ 한글" shapes to 6 display columns (the trailing space the
        // user typed is an empty cell that `encodeUtf8` trims).
        let shaped_cols = 6;
        // Columns inside the text and the end-of-line position route through
        // the shaper unchanged.
        assert_eq!(cols_past_shaped_text(shaped_cols, 1), 0);
        assert_eq!(cols_past_shaped_text(shaped_cols, 6), 0);
        assert_eq!(cols_past_shaped_text(shaped_cols, 7), 0);
        // Cursor (and IME preedit) at column 8 sits one cell past the shaped
        // text — bridge exactly one `cell_width` so it lands on its true
        // column instead of clamping onto the space.
        assert_eq!(cols_past_shaped_text(shaped_cols, 8), 1);
        assert_eq!(cols_past_shaped_text(shaped_cols, 10), 3);
    }

    #[test]
    fn grid_right_x_basic() {
        assert_eq!(grid_right_x(px(0.0), 8.0, 80), px(640.0));
    }

    #[test]
    fn grid_right_x_offsets_origin() {
        assert_eq!(grid_right_x(px(10.0), 8.0, 80), px(650.0));
    }

    #[test]
    fn grid_right_x_zero_cols_returns_origin() {
        assert_eq!(grid_right_x(px(50.0), 8.0, 0), px(50.0));
    }

    #[test]
    fn grid_right_x_handles_fractional_cell_width() {
        // Monaco 13pt in GPUI measures ~7.8px / cell. Integer cols
        // should still yield a clean sum rather than floor/ceil drift.
        let x = grid_right_x(px(0.0), 7.8, 80);
        assert!((f32::from(x) - 624.0).abs() < 0.01);
    }
}
