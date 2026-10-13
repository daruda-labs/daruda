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
