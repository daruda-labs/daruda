//! Unicode box-drawing + block-element glyph helpers.
//!
//! Two families, both drawn as GPU quads instead of font glyphs:
//!
//! 1. **Line-drawing** (`─ │ ┌ ┐ …`) — directional bitmask + stroke-
//!    weight scale, drawn as thin rectangles along cell midpoints.
//!    Using the shaper gives gappy / off-center results in most
//!    monospace fonts.
//! 2. **Block elements** (`█ ▐ ▌ ▀ ▄ ▖ ▗ ▘ ▙ ▚ ▛ ▜ ▝ ▞ ▟ ▁..▇ ▉..▏
//!    ▔ ▕ ░ ▒ ▓`) — one or two fractional-cell rectangles filled
//!    at full alpha (solid) or 25–75% alpha (shade). Bypasses the
//!    font's own block glyphs whose vertical metrics drift between
//!    fonts — e.g. Monaco renders `█` at full cell height but
//!    `▐` at cap-height, so text-art and progress bars visibly
//!    misalign when drawn by the shaper.
//!
//! Both families share `line_has_box_drawing` as the cheap per-row
//! precheck and `box_drawing_quads_for_char` as the unified quad
//! builder, so the paint loop only grows one match arm per family.

use gpui::{Bounds, Hsla, PaintQuad, Pixels, fill, point, px};

pub(crate) const BOX_DIR_LEFT: u8 = 0x01;
pub(crate) const BOX_DIR_RIGHT: u8 = 0x02;
pub(crate) const BOX_DIR_UP: u8 = 0x04;
pub(crate) const BOX_DIR_DOWN: u8 = 0x08;

/// Cheap precheck used to skip the expensive per-char box-drawing
/// loop. Returns `true` if the line has any glyph that should bypass
/// the font shaper (line-drawing, block element, or Powerline).
pub(crate) fn line_has_box_drawing(line: &str) -> bool {
    line.chars().any(|ch| {
        box_drawing_mask(ch).is_some()
            || block_glyph_for_char(ch).is_some()
            || powerline_for_char(ch).is_some()
    })
}

/// Returns the (direction bitmask, stroke-weight scale) for a box
/// drawing glyph. `None` for non-box characters.
pub(crate) fn box_drawing_mask(ch: char) -> Option<(u8, f32)> {
    let light = 1.0;
    let heavy = 1.35;
    let double = 1.15;

    let mask = match ch {
        '─' | '━' | '═' => BOX_DIR_LEFT | BOX_DIR_RIGHT,
        '│' | '┃' | '║' => BOX_DIR_UP | BOX_DIR_DOWN,
        '┌' | '┏' | '╔' | '╭' => BOX_DIR_RIGHT | BOX_DIR_DOWN,
        '┐' | '┓' | '╗' | '╮' => BOX_DIR_LEFT | BOX_DIR_DOWN,
        '└' | '┗' | '╚' | '╰' => BOX_DIR_RIGHT | BOX_DIR_UP,
        '┘' | '┛' | '╝' | '╯' => BOX_DIR_LEFT | BOX_DIR_UP,
        '├' | '┣' | '╠' => BOX_DIR_RIGHT | BOX_DIR_UP | BOX_DIR_DOWN,
        '┤' | '┫' | '╣' => BOX_DIR_LEFT | BOX_DIR_UP | BOX_DIR_DOWN,
        '┬' | '┳' | '╦' => BOX_DIR_LEFT | BOX_DIR_RIGHT | BOX_DIR_DOWN,
        '┴' | '┻' | '╩' => BOX_DIR_LEFT | BOX_DIR_RIGHT | BOX_DIR_UP,
        '┼' | '╋' | '╬' => BOX_DIR_LEFT | BOX_DIR_RIGHT | BOX_DIR_UP | BOX_DIR_DOWN,
        _ => return None,
    };

    let scale = match ch {
        '━' | '┃' | '┏' | '┓' | '┗' | '┛' | '┣' | '┫' | '┳' | '┻' | '╋' => {
            heavy
        }
        '═' | '║' | '╔' | '╗' | '╚' | '╝' | '╠' | '╣' | '╦' | '╩' | '╬' => {
            double
        }
        _ => light,
    };

    Some((mask, scale))
}

pub(super) fn box_drawing_quads_for_char(
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    cell_width: f32,
    color: Hsla,
    ch: char,
) -> Vec<PaintQuad> {
    // Block-element path: rectangular fills covering fractional
    // portions of the cell. Drawn on top of whatever the shaper
    // produced — opaque fills cover the font glyph completely;
    // shade fills intentionally blend.
    if let Some(block) = block_glyph_for_char(ch) {
        return block_fill_quads(bounds, line_height, cell_width, color, block);
    }

    // Powerline path: filled/thin triangles via scanline approx.
    if let Some(pl) = powerline_for_char(ch) {
        return powerline_quads(bounds, line_height, cell_width, color, pl);
    }

    // Line-drawing path: thin midpoint strokes.
    let Some((mask, scale)) = box_drawing_mask(ch) else {
        return Vec::new();
    };

    let x0 = bounds.left();
    let x1 = x0 + px(cell_width);
    let y0 = bounds.top();
    let y1 = y0 + line_height;

    let mid_x = x0 + px(cell_width * 0.5);
    let mid_y = y0 + line_height * 0.5;

    let thickness = px(((f32::from(line_height) / 12.0).max(1.0) * scale).max(1.0));
    let half_t = thickness * 0.5;

    let has_left = mask & BOX_DIR_LEFT != 0;
    let has_right = mask & BOX_DIR_RIGHT != 0;
    let has_up = mask & BOX_DIR_UP != 0;
    let has_down = mask & BOX_DIR_DOWN != 0;

    let mut quads = Vec::new();

    if has_left || has_right {
        let (start_x, end_x) = if has_left && has_right {
            (x0, x1)
        } else if has_left {
            (x0, mid_x)
        } else {
            (mid_x, x1)
        };
        quads.push(fill(
            Bounds::from_corners(point(start_x, mid_y - half_t), point(end_x, mid_y + half_t)),
            color,
        ));
    }

    if has_up || has_down {
        let (start_y, end_y) = if has_up && has_down {
            (y0, y1)
        } else if has_up {
            (y0, mid_y)
        } else {
            (mid_y, y1)
        };

        quads.push(fill(
            Bounds::from_corners(point(mid_x - half_t, start_y), point(mid_x + half_t, end_y)),
            color,
        ));
    }

    quads
}

// ---------------------------------------------------------------------------
// Block elements (U+2580..U+259F) — fractional-cell fills
// ---------------------------------------------------------------------------

/// Axis-aligned rectangle in **fractional cell coordinates**. Both
/// axes run `0.0` (top / left) to `1.0` (bottom / right). Every
/// entry in `BLOCK_GLYPHS` uses the same unit system so extending
/// the table with a new glyph is a one-line change.
pub(crate) type CellRect = (f32, f32, f32, f32);

/// One block glyph's paint recipe: the rectangles to fill plus the
/// alpha (1.0 = opaque block, <1.0 = shade / dither substitute).
#[derive(Clone, Copy, Debug)]
pub(crate) struct BlockGlyph {
    pub(crate) fills: &'static [CellRect],
    pub(crate) alpha: f32,
}

/// Lookup table — extend by adding a new match arm with the
/// Unicode code point and fractional-cell rectangles. Keep ordering
/// by code point so a reviewer can spot gaps at a glance.
pub(crate) fn block_glyph_for_char(ch: char) -> Option<BlockGlyph> {
    // Fractions reused across the table. Inlined literals would
    // work too, but named constants make each row self-documenting.
    const E1: f32 = 1.0 / 8.0;
    const E2: f32 = 2.0 / 8.0;
    const E3: f32 = 3.0 / 8.0;
    const H: f32 = 0.5;
    const E5: f32 = 5.0 / 8.0;
    const E6: f32 = 6.0 / 8.0;
    const E7: f32 = 7.0 / 8.0;
    const T1: f32 = 1.0 / 3.0; // thirds (1/3, 2/3 blocks)
    const T2: f32 = 2.0 / 3.0;

    // Each arm: `(&[(x0, y0, x1, y1), …], alpha)` — `None` falls
    // through to the caller (line-drawing path or font glyph).
    let (fills, alpha): (&'static [CellRect], f32) = match ch {
        // U+2580 ▀  Upper half
        '\u{2580}' => (&[(0.0, 0.0, 1.0, H)], 1.0),
        // U+2581..2587 ▁ ▂ ▃ ▄ ▅ ▆ ▇  Lower N/8 fill
        '\u{2581}' => (&[(0.0, E7, 1.0, 1.0)], 1.0),
        '\u{2582}' => (&[(0.0, E6, 1.0, 1.0)], 1.0),
        '\u{2583}' => (&[(0.0, E5, 1.0, 1.0)], 1.0),
        '\u{2584}' => (&[(0.0, H, 1.0, 1.0)], 1.0), // lower half (4/8)
        '\u{2585}' => (&[(0.0, E3, 1.0, 1.0)], 1.0),
        '\u{2586}' => (&[(0.0, E2, 1.0, 1.0)], 1.0),
        '\u{2587}' => (&[(0.0, E1, 1.0, 1.0)], 1.0),
        // U+2588 █  Full block
        '\u{2588}' => (&[(0.0, 0.0, 1.0, 1.0)], 1.0),
        // U+2589..258F ▉ ▊ ▋ ▌ ▍ ▎ ▏  Left N/8 fill
        '\u{2589}' => (&[(0.0, 0.0, E7, 1.0)], 1.0),
        '\u{258A}' => (&[(0.0, 0.0, E6, 1.0)], 1.0),
        '\u{258B}' => (&[(0.0, 0.0, E5, 1.0)], 1.0),
        '\u{258C}' => (&[(0.0, 0.0, H, 1.0)], 1.0), // left half (4/8)
        '\u{258D}' => (&[(0.0, 0.0, E3, 1.0)], 1.0),
        '\u{258E}' => (&[(0.0, 0.0, E2, 1.0)], 1.0),
        '\u{258F}' => (&[(0.0, 0.0, E1, 1.0)], 1.0),
        // U+2590 ▐  Right half
        '\u{2590}' => (&[(H, 0.0, 1.0, 1.0)], 1.0),
        // U+2591..2593 ░ ▒ ▓  Shade (light / medium / dark)
        '\u{2591}' => (&[(0.0, 0.0, 1.0, 1.0)], 0.25),
        '\u{2592}' => (&[(0.0, 0.0, 1.0, 1.0)], 0.50),
        '\u{2593}' => (&[(0.0, 0.0, 1.0, 1.0)], 0.75),
        // U+2594 ▔  Upper 1/8
        '\u{2594}' => (&[(0.0, 0.0, 1.0, E1)], 1.0),
        // U+2595 ▕  Right 1/8
        '\u{2595}' => (&[(E7, 0.0, 1.0, 1.0)], 1.0),
        // U+2596..259F  Quadrant glyphs — 2×2 sub-cells
        '\u{2596}' => (&[(0.0, H, H, 1.0)], 1.0), // ▖ lower-left
        '\u{2597}' => (&[(H, H, 1.0, 1.0)], 1.0), // ▗ lower-right
        '\u{2598}' => (&[(0.0, 0.0, H, H)], 1.0), // ▘ upper-left
        '\u{2599}' => (&[(0.0, 0.0, H, 1.0), (H, H, 1.0, 1.0)], 1.0), // ▙ UL+LL+LR
        '\u{259A}' => (&[(0.0, 0.0, H, H), (H, H, 1.0, 1.0)], 1.0), // ▚ UL+LR (diag)
        '\u{259B}' => (&[(0.0, 0.0, 1.0, H), (0.0, H, H, 1.0)], 1.0), // ▛ UL+UR+LL
        '\u{259C}' => (&[(0.0, 0.0, 1.0, H), (H, H, 1.0, 1.0)], 1.0), // ▜ UL+UR+LR
        '\u{259D}' => (&[(H, 0.0, 1.0, H)], 1.0), // ▝ upper-right
        '\u{259E}' => (&[(H, 0.0, 1.0, H), (0.0, H, H, 1.0)], 1.0), // ▞ UR+LL (anti-diag)
        '\u{259F}' => (&[(H, 0.0, 1.0, H), (0.0, H, 1.0, 1.0)], 1.0), // ▟ UR+LL+LR
        // U+1FB82..1FB8B  1/3 and 2/3 blocks (Legacy Computing Supplement)
        // Used by tqdm, Rich, modern progress bars and charts.
        '\u{1FB82}' => (&[(0.0, 0.0, 1.0, T1)], 1.0), // 🮂 Upper 1/3
        '\u{1FB83}' => (&[(0.0, 0.0, 1.0, T2)], 1.0), // 🮃 Upper 2/3
        '\u{1FB84}' => (&[(0.0, T1, 1.0, 1.0)], 1.0), // 🮄 Lower 2/3
        '\u{1FB85}' => (&[(0.0, T2, 1.0, 1.0)], 1.0), // 🮅 Lower 1/3
        '\u{1FB87}' => (&[(T2, 0.0, 1.0, 1.0)], 1.0), // 🮇 Right 1/3
        '\u{1FB88}' => (&[(T1, 0.0, 1.0, 1.0)], 1.0), // 🮈 Right 2/3
        '\u{1FB89}' => (&[(0.0, 0.0, T2, 1.0)], 1.0), // 🮉 Left 2/3
        '\u{1FB8A}' => (&[(0.0, 0.0, T1, 1.0)], 1.0), // 🮊 Left 1/3
        // Add new glyphs here — append a `'\u{…}' => (&[…], 1.0 or <1.0)`
        // arm. Unit tests in the module root cover the geometry
        // pattern so a regression on an existing row fires first.
        _ => return None,
    };
    Some(BlockGlyph { fills, alpha })
}

/// Convert fractional-cell rectangles into absolute-pixel fill
/// quads. Each edge is `round()`-snapped to an integer pixel
/// (Alacritty's convention) so adjacent `█` cells produce a seamless
/// bar instead of 1px gaps / overlaps at fractional screen scales.
fn block_fill_quads(
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    cell_width: f32,
    fg: Hsla,
    glyph: BlockGlyph,
) -> Vec<PaintQuad> {
    let color = if glyph.alpha >= 1.0 {
        fg
    } else {
        Hsla {
            a: fg.a * glyph.alpha,
            ..fg
        }
    };

    let x0 = f32::from(bounds.left());
    let y0 = f32::from(bounds.top());
    let cw = cell_width;
    let ch = f32::from(line_height);

    glyph
        .fills
        .iter()
        .map(|(fx0, fy0, fx1, fy1)| {
            let x_left = (x0 + cw * fx0).round();
            let x_right = (x0 + cw * fx1).round();
            let y_top = (y0 + ch * fy0).round();
            let y_bottom = (y0 + ch * fy1).round();
            // Guarantee at least 1px visible on very small cells.
            let x_right = x_right.max(x_left + 1.0);
            let y_bottom = y_bottom.max(y_top + 1.0);
            fill(
                Bounds::from_corners(
                    point(px(x_left), px(y_top)),
                    point(px(x_right), px(y_bottom)),
                ),
                color,
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Powerline symbols (U+E0B0..E0B3) — triangle / chevron fills
// ---------------------------------------------------------------------------

/// Powerline separator shape. Solid = filled triangle covering the
/// full cell. Thin = 1–2px wide diagonal stroke (chevron outline).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PowerlineShape {
    /// Filled right-pointing triangle  (base on left edge, point on right).
    SolidRight,
    /// Thin right-pointing chevron  (diagonal stroke only).
    ThinRight,
    /// Filled left-pointing triangle  (base on right edge, point on left).
    SolidLeft,
    /// Thin left-pointing chevron .
    ThinLeft,
}

pub(crate) fn powerline_for_char(ch: char) -> Option<PowerlineShape> {
    match ch {
        '\u{E0B0}' => Some(PowerlineShape::SolidRight),
        '\u{E0B1}' => Some(PowerlineShape::ThinRight),
        '\u{E0B2}' => Some(PowerlineShape::SolidLeft),
        '\u{E0B3}' => Some(PowerlineShape::ThinLeft),
        _ => None,
    }
}

/// Scanline-approximate a Powerline triangle/chevron. For a cell of
/// height H, emits H thin horizontal quads whose widths trace the
/// triangle edge. Solid fills paint all scanlines from the base
/// edge; thin strokes paint only a 2px band along the diagonal.
fn powerline_quads(
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    cell_width: f32,
    fg: Hsla,
    shape: PowerlineShape,
) -> Vec<PaintQuad> {
    let x0 = f32::from(bounds.left());
    let y0 = f32::from(bounds.top());
    let cw = cell_width;
    let ch = f32::from(line_height);
    let rows = ch.round().max(1.0) as usize;
    let mid = ch * 0.5;
    let stroke = 2.0_f32; // thin chevron stroke width in pixels

    let mut quads = Vec::with_capacity(rows);
    for i in 0..rows {
        let y = i as f32;
        // progress: 0.0 at top/bottom edges, 1.0 at vertical midpoint.
        let progress = if y < mid {
            y / mid
        } else {
            (ch - y - 1.0).max(0.0) / mid
        };
        let w = (cw * progress).round().max(0.0);

        let (x_left, x_right) = match shape {
            PowerlineShape::SolidRight => (x0, x0 + w),
            PowerlineShape::SolidLeft => (x0 + cw - w, x0 + cw),
            PowerlineShape::ThinRight => {
                let edge = w;
                let lo = (edge - stroke).max(0.0);
                (x0 + lo, x0 + (lo + stroke).min(cw))
            }
            PowerlineShape::ThinLeft => {
                let edge = cw - w;
                (x0 + edge, x0 + (edge + stroke).min(cw))
            }
        };

        if x_right > x_left {
            quads.push(fill(
                Bounds::from_corners(
                    point(px(x_left.round()), px((y0 + y).round())),
                    point(px(x_right.round()), px((y0 + y + 1.0).round())),
                ),
                fg,
            ));
        }
    }
    quads
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_block_fills_whole_cell() {
        let g = block_glyph_for_char('█').unwrap();
        assert_eq!(g.fills, &[(0.0, 0.0, 1.0, 1.0)]);
        assert_eq!(g.alpha, 1.0);
    }

    #[test]
    fn right_half_and_full_block_share_cell_height() {
        // The user-reported bug: █ and ▐ must have identical vertical
        // extent. In fractional cell coords both fills span y 0..=1,
        // so the post-round() pixel rects agree by construction.
        let full = block_glyph_for_char('█').unwrap();
        let right_half = block_glyph_for_char('▐').unwrap();
        let (_, ya, _, yb) = full.fills[0];
        let (_, yc, _, yd) = right_half.fills[0];
        assert_eq!((ya, yb), (yc, yd));
    }

    #[test]
    fn left_and_right_half_cover_full_cell_together() {
        // ▌ + ▐ side-by-side must paint the same area as █.
        let left = block_glyph_for_char('▌').unwrap().fills[0];
        let right = block_glyph_for_char('▐').unwrap().fills[0];
        assert_eq!(
            left.2, right.0,
            "left half right-edge meets right half left-edge"
        );
        assert_eq!((left.0, left.1, right.2, right.3), (0.0, 0.0, 1.0, 1.0));
    }

    #[test]
    fn lower_fill_series_increases_by_eighths() {
        // ▁▂▃▄▅▆▇ — each successive char grows the fill by 1/8.
        let heights: Vec<f32> = ['▁', '▂', '▃', '▄', '▅', '▆', '▇']
            .iter()
            .map(|ch| {
                let g = block_glyph_for_char(*ch).unwrap();
                let (_, y0, _, y1) = g.fills[0];
                y1 - y0
            })
            .collect();
        for (i, h) in heights.iter().enumerate() {
            let expected = (i + 1) as f32 / 8.0;
            assert!(
                (h - expected).abs() < 1e-6,
                "{} should be {}/8 tall, got {}",
                i + 1,
                i + 1,
                h
            );
        }
    }

    #[test]
    fn left_fill_series_increases_by_eighths() {
        let widths: Vec<f32> = ['▏', '▎', '▍', '▌', '▋', '▊', '▉']
            .iter()
            .map(|ch| {
                let g = block_glyph_for_char(*ch).unwrap();
                let (x0, _, x1, _) = g.fills[0];
                x1 - x0
            })
            .collect();
        for (i, w) in widths.iter().enumerate() {
            let expected = (i + 1) as f32 / 8.0;
            assert!((w - expected).abs() < 1e-6, "{} slot mismatch: {}", i, w);
        }
    }

    #[test]
    fn shade_chars_use_alpha_not_fractional_fill() {
        // Shades always fill the whole cell but with reduced alpha.
        // This is the visual convention — a dither pattern would be
        // indistinguishable on a retina display and doubles the
        // quad count.
        for (ch, expected) in [('░', 0.25), ('▒', 0.50), ('▓', 0.75)] {
            let g = block_glyph_for_char(ch).unwrap();
            assert_eq!(g.fills, &[(0.0, 0.0, 1.0, 1.0)]);
            assert!((g.alpha - expected).abs() < 1e-6, "{} alpha", ch);
        }
    }

    #[test]
    fn diagonal_quadrants_cover_opposite_corners() {
        // ▚ (UL + LR) and ▞ (UR + LL) together cover the full cell.
        let diag = block_glyph_for_char('▚').unwrap();
        let anti = block_glyph_for_char('▞').unwrap();
        assert_eq!(diag.fills.len(), 2);
        assert_eq!(anti.fills.len(), 2);
        // UL quadrant present in ▚
        assert!(diag.fills.contains(&(0.0, 0.0, 0.5, 0.5)));
        // LR quadrant present in ▚
        assert!(diag.fills.contains(&(0.5, 0.5, 1.0, 1.0)));
    }

    #[test]
    fn three_quadrant_chars_have_three_cell_corners() {
        // ▙ ▛ ▜ ▟ each cover 3 of 4 quadrants → two fills that
        // together occupy 3/4 of the cell area.
        for ch in ['▙', '▛', '▜', '▟'] {
            let g = block_glyph_for_char(ch).unwrap();
            let area: f32 = g
                .fills
                .iter()
                .map(|(x0, y0, x1, y1)| (x1 - x0) * (y1 - y0))
                .sum();
            assert!(
                (area - 0.75).abs() < 1e-6,
                "{} should cover 3/4: {}",
                ch,
                area
            );
        }
    }

    #[test]
    fn non_block_chars_return_none() {
        assert!(block_glyph_for_char('A').is_none());
        assert!(block_glyph_for_char(' ').is_none());
        assert!(
            block_glyph_for_char('─').is_none(),
            "line-drawing is a separate family"
        );
    }

    // ----- 1/3, 2/3 blocks -----------------------------------------

    #[test]
    fn third_blocks_cover_correct_fractions() {
        let u1 = block_glyph_for_char('\u{1FB82}').unwrap(); // Upper 1/3
        let u2 = block_glyph_for_char('\u{1FB83}').unwrap(); // Upper 2/3
        let l2 = block_glyph_for_char('\u{1FB84}').unwrap(); // Lower 2/3
        let l1 = block_glyph_for_char('\u{1FB85}').unwrap(); // Lower 1/3
        // Height fractions.
        let height = |g: BlockGlyph| g.fills[0].3 - g.fills[0].1;
        assert!((height(u1) - 1.0 / 3.0).abs() < 1e-6);
        assert!((height(u2) - 2.0 / 3.0).abs() < 1e-6);
        assert!((height(l2) - 2.0 / 3.0).abs() < 1e-6);
        assert!((height(l1) - 1.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn third_blocks_horizontal_variants() {
        let r1 = block_glyph_for_char('\u{1FB87}').unwrap(); // Right 1/3
        let r2 = block_glyph_for_char('\u{1FB88}').unwrap(); // Right 2/3
        let l2 = block_glyph_for_char('\u{1FB89}').unwrap(); // Left 2/3
        let l1 = block_glyph_for_char('\u{1FB8A}').unwrap(); // Left 1/3
        let width = |g: BlockGlyph| g.fills[0].2 - g.fills[0].0;
        assert!((width(r1) - 1.0 / 3.0).abs() < 1e-6);
        assert!((width(r2) - 2.0 / 3.0).abs() < 1e-6);
        assert!((width(l2) - 2.0 / 3.0).abs() < 1e-6);
        assert!((width(l1) - 1.0 / 3.0).abs() < 1e-6);
    }

    // ----- Powerline -------------------------------------------------

    #[test]
    fn powerline_chars_are_recognized() {
        assert!(powerline_for_char('\u{E0B0}').is_some()); // solid right
        assert!(powerline_for_char('\u{E0B1}').is_some()); // thin right
        assert!(powerline_for_char('\u{E0B2}').is_some()); // solid left
        assert!(powerline_for_char('\u{E0B3}').is_some()); // thin left
        assert!(powerline_for_char('A').is_none());
    }

    #[test]
    fn powerline_solid_right_emits_nonzero_quads() {
        let bounds = Bounds::from_corners(point(px(0.0), px(0.0)), point(px(8.0), px(16.0)));
        let quads = powerline_quads(
            bounds,
            px(16.0),
            8.0,
            gpui::white(),
            PowerlineShape::SolidRight,
        );
        // Scanline per pixel row → 16 quads. Top and bottom rows
        // may be zero-width (clipped), so at least 14.
        assert!(quads.len() >= 14, "got {} quads", quads.len());
    }

    #[test]
    fn powerline_solid_left_mirrors_right() {
        let bounds = Bounds::from_corners(point(px(0.0), px(0.0)), point(px(8.0), px(16.0)));
        let right = powerline_quads(
            bounds,
            px(16.0),
            8.0,
            gpui::white(),
            PowerlineShape::SolidRight,
        );
        let left = powerline_quads(
            bounds,
            px(16.0),
            8.0,
            gpui::white(),
            PowerlineShape::SolidLeft,
        );
        assert_eq!(right.len(), left.len());
        // Midpoint row should have full cell_width in both directions.
        let mid = right.len() / 2;
        let rw = f32::from(right[mid].bounds.size.width);
        let lw = f32::from(left[mid].bounds.size.width);
        assert!((rw - lw).abs() < 2.0, "mid-row width should be similar");
    }

    // ----- line_has_box_drawing + precheck ---------------------------

    #[test]
    fn line_has_box_drawing_detects_all_families() {
        assert!(line_has_box_drawing("hello █ world"));
        assert!(line_has_box_drawing("▐▐▐▐"));
        assert!(line_has_box_drawing("░▒▓"));
        assert!(line_has_box_drawing("progress ▏ bar"));
        assert!(!line_has_box_drawing("plain ASCII"));
        assert!(line_has_box_drawing("mix ─ and █"), "line + block");
        assert!(line_has_box_drawing("powerline \u{E0B0} sep"), "powerline");
        assert!(line_has_box_drawing("third 🮂 block"), "1/3 block");
    }

    #[test]
    fn block_fill_quads_round_to_pixel_boundaries() {
        // A 7.8px cell would produce fractional quad edges without
        // rounding. The helper must snap every edge to integer
        // pixels so adjacent `█` cells abut cleanly.
        let bounds = Bounds::from_corners(point(px(0.0), px(0.0)), point(px(7.8), px(15.6)));
        let glyph = block_glyph_for_char('█').unwrap();
        let quads = block_fill_quads(bounds, px(15.6), 7.8, gpui::white(), glyph);
        assert_eq!(quads.len(), 1);
        let b = quads[0].bounds;
        let left: f32 = b.left().into();
        let top: f32 = b.top().into();
        let right: f32 = b.right().into();
        let bottom: f32 = b.bottom().into();
        assert_eq!(left, left.round(), "left snapped");
        assert_eq!(top, top.round(), "top snapped");
        assert_eq!(right, right.round(), "right snapped");
        assert_eq!(bottom, bottom.round(), "bottom snapped");
    }
}
