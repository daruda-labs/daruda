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
    viewport_lines: &[String],
    col: u16,
    row: u16,
    cell_width: f32,
    origin_left: Pixels,
) -> Pixels {
    let row_idx = row.saturating_sub(1) as usize;
    if let Some(Some(line)) = line_layouts.get(row_idx)
        && let Some(text) = viewport_lines.get(row_idx)
    {
        let byte = byte_index_for_column_in_line(text, col);
        let byte = super::text_edit::clamp_to_char_boundary(line.text.as_str(), byte);
        return origin_left + line.x_for_index(byte.min(line.text.len()));
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
/// text because GPUI's shaper drops `force_width` when any glyph is
/// wide, so glyphs rendered beyond that point sit at their natural
/// advances rather than a multiple of the monospace cell.
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::px;

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
