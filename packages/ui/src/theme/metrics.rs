//! Shared numeric scales. Components own the recipes built from these values.

pub const PAD_STANDARD: f32 = 8.0;
pub const PAD_LG: f32 = 10.0;
pub const PAD_XL: f32 = 14.0;
pub const PAD_SM: f32 = 6.0;
pub const PAD_XS: f32 = 4.0;
pub const GAP_STANDARD: f32 = 6.0;
pub const GAP_SM: f32 = 4.0;
pub const GAP_LG: f32 = 8.0;
pub const GAP_XS: f32 = 2.0;
pub const FONT_SIZE_SM: f32 = 11.0;
pub const FONT_SIZE_MD: f32 = 12.0;
pub const FONT_SIZE_XS: f32 = 10.0;
pub const FONT_SIZE_LG: f32 = 13.0;
pub const RADIUS_SM: f32 = 4.0;
pub const RADIUS_XS: f32 = 2.0;
pub const RADIUS_MD: f32 = 6.0;
pub const RADIUS_LG: f32 = 8.0;

/// Standard chrome artwork and its square interaction target (px).
pub const CONTROL_ICON_SIZE: f32 = 16.0;
pub const CONTROL_TARGET_SIZE: f32 = 24.0;
pub const BUTTON_STANDARD_PAD_X: f32 = 12.0;
pub const BUTTON_STANDARD_PAD_Y: f32 = 3.0;
pub const BUTTON_COMPACT_HEIGHT: f32 = 20.0;
pub const BUTTON_COMPACT_PAD_X: f32 = 4.0;
pub const BUTTON_COMPACT_PAD_Y: f32 = 1.0;
pub const BUTTON_COMPACT_LINE_HEIGHT: f32 = 16.0;

/// Field geometry is pixel-based and independent of the vendor size tier.
pub const INPUT_STANDARD_HEIGHT: f32 = 28.0;
pub const INPUT_FORM_HEIGHT: f32 = 36.0;
pub const INPUT_PAD_X: f32 = 9.0;
pub const INPUT_LINE_HEIGHT: f32 = 20.0;
pub const INPUT_BORDER_WIDTH: f32 = 1.0;
pub const INPUT_STANDARD_PAD_Y: f32 =
    (INPUT_STANDARD_HEIGHT - INPUT_LINE_HEIGHT) / 2.0 - INPUT_BORDER_WIDTH;
pub const INPUT_FORM_PAD_Y: f32 =
    (INPUT_FORM_HEIGHT - INPUT_LINE_HEIGHT) / 2.0 - INPUT_BORDER_WIDTH;
pub const SEARCH_CLEAR_OFFSET_Y: f32 = (INPUT_LINE_HEIGHT - CONTROL_TARGET_SIZE) / 2.0;
pub const POPOVER_WINDOW_MARGIN: f32 = 12.0;
pub const POPOVER_PANEL_PAD: f32 = 12.0;
pub const POPOVER_DEFAULT_CHROME: f32 = 2.0 * (POPOVER_PANEL_PAD + INPUT_BORDER_WIDTH);
pub const POPOVER_HEADER_BUDGET: f32 = 48.0;
/// Clears the scrollbar's full clickable strip, not just its visible thumb.
pub const POPOVER_ACTION_GUTTER: f32 = 16.0;
pub const DIFF_SURFACE_ROW_ALPHA: f32 = 0.12;
pub const DIFF_SURFACE_WORD_ALPHA: f32 = 0.24;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scales_are_positive_and_strictly_increasing() {
        for scale in [
            &[PAD_XS, PAD_SM, PAD_STANDARD, PAD_LG, PAD_XL][..],
            &[GAP_XS, GAP_SM, GAP_STANDARD, GAP_LG][..],
            &[FONT_SIZE_XS, FONT_SIZE_SM, FONT_SIZE_MD, FONT_SIZE_LG][..],
            &[RADIUS_XS, RADIUS_SM, RADIUS_MD, RADIUS_LG][..],
        ] {
            assert!(scale.iter().all(|value| value.is_finite() && *value > 0.0));
            assert!(scale.windows(2).all(|pair| pair[0] < pair[1]));
        }
    }
}
