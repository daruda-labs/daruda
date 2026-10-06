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
