//! Public API tests for render.

use daruda_config::render::*;
use std::time::Duration;

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
