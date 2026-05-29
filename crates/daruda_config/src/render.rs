use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Supported terminal repaint caps, in frames per second. The Settings
/// dropdown offers exactly these; hand-edited values snap to the
/// nearest via [`RenderConfig::clamp`].
pub const ALLOWED_MAX_FPS: [u32; 4] = [30, 60, 120, 144];

/// Rendering / repaint tuning.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct RenderConfig {
    /// Cap on how often terminal output triggers a repaint, in fps.
    /// PTY output is batched for `1000 / max_fps` ms before notifying
    /// the view, so this bounds the streaming redraw rate (and thus the
    /// CPU spent re-painting the window while a program streams output).
    /// Default 30 halves the redraw rate versus a 60 fps cap; pick a
    /// higher value for smoother fast-scrolling output at higher CPU.
    pub max_fps: u32,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self { max_fps: 30 }
    }
}

impl RenderConfig {
    /// Snap `max_fps` to the nearest supported value.
    pub fn clamp(&mut self) {
        self.max_fps = ALLOWED_MAX_FPS
            .iter()
            .copied()
            .min_by_key(|&v| v.abs_diff(self.max_fps))
            .unwrap_or(30);
    }

    /// Batch / repaint interval = `1000 / max_fps` ms. `max_fps` is
    /// floored at 1 so a corrupt zero can't divide-by-zero.
    pub fn redraw_interval(&self) -> Duration {
        Duration::from_millis((1000 / self.max_fps.max(1)) as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_thirty_fps() {
        assert_eq!(RenderConfig::default().max_fps, 30);
    }

    #[test]
    fn interval_maps_fps_to_millis() {
        let mk = |fps| RenderConfig { max_fps: fps }.redraw_interval();
        assert_eq!(mk(30), Duration::from_millis(33));
        assert_eq!(mk(60), Duration::from_millis(16));
        assert_eq!(mk(120), Duration::from_millis(8));
        assert_eq!(mk(144), Duration::from_millis(6));
    }

    #[test]
    fn clamp_snaps_to_nearest_allowed() {
        let snap = |fps| {
            let mut c = RenderConfig { max_fps: fps };
            c.clamp();
            c.max_fps
        };
        assert_eq!(snap(45), 30); // 45: |45-30|=15 < |45-60|=15 → first wins (30)
        assert_eq!(snap(50), 60);
        assert_eq!(snap(100), 120);
        assert_eq!(snap(200), 144);
        assert_eq!(snap(0), 30);
        for v in ALLOWED_MAX_FPS {
            assert_eq!(snap(v), v, "allowed value must be a fixed point");
        }
    }
}
