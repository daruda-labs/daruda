//! The one animation clock every status indicator reads.
//!
//! A per-widget 60 fps `with_animation` repaints the whole window tree (GPUI
//! has no partial redraw), so indicators instead read this shared, gated
//! ~4 fps tick. The app's pulse pump advances it while something animates.

use gpui::Global;

/// Monotonic animation clock shared by every status indicator.
#[derive(Default)]
pub struct StatusPulseClock {
    pub tick: u64,
}
impl Global for StatusPulseClock {}
