//! Size recipes shared by UI surfaces that opt into a presentation preference.

use super::metrics::{CONTROL_TARGET_SIZE, FONT_SIZE_XS};
use daruda_config::UiSize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UiMetrics(pub UiSize);

impl UiMetrics {
    pub fn text(self, normal: f32) -> f32 {
        (normal + self.step()).max(FONT_SIZE_XS)
    }

    pub fn icon(self, normal: f32) -> f32 {
        normal + self.step() * 2.0
    }

    pub fn space(self, normal: f32) -> f32 {
        normal * (1.0 + self.step() / 8.0)
    }

    pub fn row(self, normal: f32) -> f32 {
        (normal + self.step() * 4.0).max(normal.min(CONTROL_TARGET_SIZE))
    }

    pub fn target(self, normal: f32) -> f32 {
        self.row(normal).max(CONTROL_TARGET_SIZE)
    }

    fn step(self) -> f32 {
        match self.0 {
            UiSize::Small => -1.0,
            UiSize::Normal => 0.0,
            UiSize::Large => 1.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_preserves_metrics_and_small_remains_readable_and_clickable() {
        assert_eq!(UiMetrics(UiSize::Normal).icon(16.0), 16.0);
        assert_eq!(UiMetrics(UiSize::Normal).space(36.0), 36.0);
        assert_eq!(UiMetrics(UiSize::Small).text(10.0), 10.0);
        assert_eq!(UiMetrics(UiSize::Small).target(24.0), 24.0);
        assert_eq!(UiMetrics(UiSize::Large).icon(16.0), 18.0);
    }
}
