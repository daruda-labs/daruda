//! Claude Code session-status indicator — four visual modes per
//! [`SessionStatus`].
//!
//! All shapes are procedurally drawn with GPUI primitives — no SVG
//! files, no external assets. Colour, size, and timing values come
//! from `crate::ui::theme::STATUS_INDICATOR_*` (G4 — no
//! inline literals).
//!
//! All states render as a 3×3 dot grid at the same footprint.
//! State is expressed through colour and a low-rate stepped pattern:
//!
//! - **Idle** — all 9 dots fully lit, no animation.
//! - **Connecting** — plus (+) ↔ cross (×), 2-frame blink. Centre stays lit.
//! - **NeedsAttention** — all 9 dots blink `1.0 ↔ 0.4` (2-frame).
//! - **Working** — serpentine "comet" head sweeps the grid in 6 stepped frames;
//!   head at full alpha, trail fades to `STATUS_INDICATOR_DOT_GRID_TAIL_ALPHA_MIN`.
//! - **ExecutingTool** — a quadrant (corner + two edges) rotates clockwise in
//!   4 frames; centre stays dim at `STATUS_INDICATOR_RING_CENTER_ALPHA`.
//!
//! ## Why a shared clock instead of `with_animation`
//!
//! GPUI has no partial redraw: `with_animation` requests a frame every
//! display refresh (~60 fps), and each frame marks the view dirty →
//! the whole window tree re-lays-out and re-paints. A small status
//! badge therefore cost ~40% CPU while it pulsed.
//!
//! Instead, a single [`StatusPulseClock`] global advances one `tick`
//! every `STATUS_INDICATOR_TICK_MS` (~6 fps), driven by a gated pump
//! (`watchers_lifecycle::spawn_status_pulse`) that only notifies
//! windows which are active *and* have an animating session. Each badge
//! derives its frame from the tick and renders a static frame — no
//! per-frame `request_animation_frame`. Result: ~6 redraws/s instead of
//! ~60 while a badge animates, and zero while idle. The visible
//! resolution of the comet (6/4 discrete frames) is unchanged by the
//! lower rate; the smooth fades become 2-frame blinks (Pitfall #10).

use crate::ui::theme;
use daruda_claude::SessionStatus;
use gpui::{
    App, Global, Hsla, IntoElement, ParentElement, Pixels, RenderOnce, Styled, Window, div, px,
};

/// Monotonic animation clock shared by every status badge. Advanced by
/// the gated status-pulse pump (~`STATUS_INDICATOR_TICK_MS` per tick);
/// badges read it during render to pick their current frame. Set as a
/// global at app startup (`main.rs`) so `try_global` never misses after
/// init; badges fall back to tick 0 (static frame) if read before.
#[derive(Default)]
pub struct StatusPulseClock {
    pub tick: u64,
}
impl Global for StatusPulseClock {}

/// Ticks each 2-frame blink holds per state before toggling. At
/// ~6 fps (`TICK_MS ≈ 167`), 3 ticks ≈ 500 ms → ~1 Hz blink.
const BLINK_HOLD_TICKS: u64 = 3;

/// `true` on the "lit" half of a 2-frame blink cycle.
fn blink_on(tick: u64) -> bool {
    (tick / BLINK_HOLD_TICKS).is_multiple_of(2)
}

/// Serpentine head position (0..9) for the Working comet's 6-frame
/// cycle. `round(frame * 9 / 6)` spreads 6 heads across the 9-dot path:
/// `[0, 2, 3, 5, 6, 8]`.
fn snake_head(tick: u64) -> usize {
    let f = tick % 6;
    ((3 * f).div_ceil(2) % 9) as usize
}

/// Size variant of the indicator.
#[derive(Clone, Copy)]
pub enum IndicatorSize {
    /// Left-dock leading indicator on a lane row.
    Leading,
    /// Phase D sub-row per-session badge.
    Badge,
}

impl IndicatorSize {
    fn dim(self) -> Pixels {
        px(match self {
            Self::Leading => theme::STATUS_INDICATOR_SIZE,
            Self::Badge => theme::STATUS_INDICATOR_BADGE_SIZE,
        })
    }
}

/// Pick the theme color for the given status. Reads from the live
/// `DarudaTheme` Global so the indicator picks up light-mode tones on
/// theme switch.
pub fn color_for_status(status: SessionStatus, cx: &App) -> Hsla {
    let t = theme::current(cx);
    // `*_dark` is the production slot — the `*_light` siblings exist
    // only as a future-mode reservation that no code path reads.
    // Light-mode override happens by replacing the `_dark` slot value
    // in `daruda_light.json`.
    match status {
        SessionStatus::Working => t.status_working_dark,
        SessionStatus::ExecutingTool => t.status_executing_tool_dark,
        SessionStatus::NeedsAttention => t.status_needs_attention_dark,
        SessionStatus::Idle => t.status_idle_dark,
        SessionStatus::Connecting => t.status_connecting_dark,
    }
}

/// Stateless GPUI element rendering one of four animation modes.
#[derive(IntoElement)]
pub struct AgentStatusBadge {
    status: SessionStatus,
    size: IndicatorSize,
    color: Hsla,
    /// Phase E — when true, the indicator is wrapped in a 1 px outline
    /// ring marking it as the session attached to the focused tab.
    active: bool,
    /// When false, the indicator renders a single static frame instead
    /// of stepping with the shared clock. The dock gates this on
    /// `window.is_window_active()` so a backgrounded window shows a
    /// frozen frame (the pump also skips inactive windows). The decision
    /// lives in one place — `Workspace::render` →
    /// `LeftDockSnapshot::claude_animate` — and is threaded down here.
    animate: bool,
}

impl AgentStatusBadge {
    pub fn new(status: SessionStatus, size: IndicatorSize, color: Hsla) -> Self {
        Self {
            status,
            size,
            color,
            active: false,
            animate: true,
        }
    }

    /// Construct with the default theme color for the state. Reads
    /// the live `DarudaTheme` Global so the indicator picks up
    /// light-mode tones on theme switch.
    pub fn for_status(status: SessionStatus, size: IndicatorSize, cx: &App) -> Self {
        Self::new(status, size, color_for_status(status, cx))
    }

    /// Wrap the indicator with an outline ring (sub-row "active in
    /// the focused tab" affordance, Phase E).
    pub fn active(mut self) -> Self {
        self.active = true;
        self
    }

    /// Gate the animation. When `false`, the indicator draws a single
    /// static frame. See the `animate` field doc.
    pub fn animate(mut self, animate: bool) -> Self {
        self.animate = animate;
        self
    }
}

impl RenderOnce for AgentStatusBadge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let dim = self.size.dim();
        let tick = cx
            .try_global::<StatusPulseClock>()
            .map(|c| c.tick)
            .unwrap_or(0);
        let anim = self.animate;
        let inner = match self.status {
            SessionStatus::Idle => idle_grid(dim, self.color),
            SessionStatus::Connecting => connecting_grid(dim, self.color, anim, tick),
            SessionStatus::NeedsAttention => needs_attention_grid(dim, self.color, anim, tick),
            SessionStatus::Working => dot_grid(dim, self.color, anim, tick),
            SessionStatus::ExecutingTool => quadrant_grid(dim, self.color, anim, tick),
        };
        if self.active {
            wrap_active(inner, dim, cx).into_any_element()
        } else {
            inner
        }
    }
}

/// Wrap the rendered indicator in an outline ring. The outer frame
/// is `outer = inner + STATUS_BADGE_ACTIVE_OUTER_PAD * 2` so the
/// `border_1` sits a hair outside the indicator without occluding it.
fn wrap_active(inner: gpui::AnyElement, inner_size: Pixels, cx: &App) -> impl IntoElement {
    let outer_px: f32 = inner_size.into();
    let outer = px(outer_px + theme::STATUS_BADGE_ACTIVE_OUTER_PAD * 2.0);
    div()
        .flex_none()
        .w(outer)
        .h(outer)
        .rounded_full()
        .border_1()
        .border_color(theme::current(cx).status_badge_active_outline)
        .flex()
        .items_center()
        .justify_center()
        .child(inner)
}

// ── Idle ─────────────────────────────────────────────────────────────────────

/// 3×3 grid, all dots fully lit, no animation.
fn idle_grid(size: Pixels, color: Hsla) -> gpui::AnyElement {
    FullGrid {
        dim: size,
        color,
        opacity: 1.0,
    }
    .into_any_element()
}

// ── NeedsAttention ───────────────────────────────────────────────────────────

/// 3×3 grid, all dots; opacity blinks `1.0 ↔ PULSE_OPACITY_MIN` (2-frame).
/// Static frame (no animation) is the fully-lit grid.
fn needs_attention_grid(size: Pixels, color: Hsla, animate: bool, tick: u64) -> gpui::AnyElement {
    let opacity = if !animate || blink_on(tick) {
        1.0
    } else {
        theme::STATUS_INDICATOR_PULSE_OPACITY_MIN
    };
    FullGrid {
        dim: size,
        color,
        opacity,
    }
    .into_any_element()
}

// ── Connecting ───────────────────────────────────────────────────────────────

/// 3×3 grid, plus (+) ↔ cross (×) 2-frame blink. Centre stays lit.
/// `ConnectingGrid` derives the pattern from `phase`: `0.0` = full plus,
/// `0.5` = full cross. Static frame is the plus.
fn connecting_grid(size: Pixels, color: Hsla, animate: bool, tick: u64) -> gpui::AnyElement {
    let phase = if !animate || blink_on(tick) { 0.0 } else { 0.5 };
    ConnectingGrid {
        dim: size,
        color,
        phase,
    }
    .into_any_element()
}

// ── Shared helpers ────────────────────────────────────────────────────────────

/// 3×3 grid with all dots at a uniform opacity multiplier.
/// Used for Idle (opacity = 1.0) and NeedsAttention (opacity blinked).
#[derive(IntoElement)]
struct FullGrid {
    dim: Pixels,
    color: Hsla,
    /// Multiplied into `color.a` for every dot; driven by the blink.
    opacity: f32,
}

impl RenderOnce for FullGrid {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let dim_f: f32 = self.dim.into();
        let cell = dim_f / 3.0;
        let dot = cell * theme::STATUS_INDICATOR_DOT_GRID_RATIO;
        let pad = (cell - dot) * 0.5;
        let mut c = self.color;
        c.a *= self.opacity;
        let size = self.dim;

        div().relative().flex_none().w(size).h(size).children(
            (0..3usize)
                .flat_map(|col| (0..3usize).map(move |row| (col, row)))
                .map(move |(col, row)| {
                    div()
                        .absolute()
                        .left(px(cell * col as f32 + pad))
                        .top(px(cell * row as f32 + pad))
                        .w(px(dot))
                        .h(px(dot))
                        .rounded_full()
                        .bg(c)
                }),
        )
    }
}

/// Dot positions forming the plus (+) pattern (centre + 4 edge midpoints).
const PLUS_DOTS: [(usize, usize); 5] = [(1, 0), (0, 1), (1, 1), (2, 1), (1, 2)];
/// Dot positions forming the cross (×) pattern (centre + 4 corners).
const CROSS_DOTS: [(usize, usize); 5] = [(0, 0), (2, 0), (1, 1), (0, 2), (2, 2)];

/// Connecting-state 3×3 grid that shows plus (+) or cross (×).
/// The centre dot (shared by both patterns) stays fully lit throughout.
#[derive(IntoElement)]
struct ConnectingGrid {
    dim: Pixels,
    color: Hsla,
    phase: f32,
}

impl RenderOnce for ConnectingGrid {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let dim_f: f32 = self.dim.into();
        let cell = dim_f / 3.0;
        let dot = cell * theme::STATUS_INDICATOR_DOT_GRID_RATIO;
        let pad = (cell - dot) * 0.5;
        let color = self.color;

        // + peaks at phase = 0; × peaks at phase = 0.5.
        let plus_alpha = (1.0 + (self.phase * 2.0 * std::f32::consts::PI).cos()) * 0.5;
        let cross_alpha = 1.0 - plus_alpha;

        div()
            .relative()
            .flex_none()
            .w(self.dim)
            .h(self.dim)
            .children(
                (0..3usize)
                    .flat_map(|col| (0..3usize).map(move |row| (col, row)))
                    .map(move |(col, row)| {
                        let in_plus = PLUS_DOTS.contains(&(col, row));
                        let in_cross = CROSS_DOTS.contains(&(col, row));
                        let alpha = match (in_plus, in_cross) {
                            (true, true) => 1.0, // centre: always fully lit
                            (true, false) => plus_alpha,
                            (false, true) => cross_alpha,
                            (false, false) => {
                                unreachable!("({col},{row}) not in either pattern")
                            }
                        };
                        let mut c = color;
                        c.a *= alpha;
                        div()
                            .absolute()
                            .left(px(cell * col as f32 + pad))
                            .top(px(cell * row as f32 + pad))
                            .w(px(dot))
                            .h(px(dot))
                            .rounded_full()
                            .bg(c)
                    }),
            )
    }
}

// ── Working ───────────────────────────────────────────────────────────────────

/// Lighting order for the Working-state 3×3 grid: left column up → middle
/// column down → right column up. Each entry is `(col, row)` with row 0
/// at the top. Index 0 is the first dot the head visits.
const DOT_ORDER: [(usize, usize); 9] = [
    (0, 0),
    (1, 0),
    (2, 0),
    (2, 1),
    (1, 1),
    (0, 1),
    (0, 2),
    (1, 2),
    (2, 2),
];

/// Working-state comet. `tick` drives a 6-frame serpentine sweep.
/// Static frame (no animation) is head on the first dot.
fn dot_grid(size: Pixels, color: Hsla, animate: bool, tick: u64) -> gpui::AnyElement {
    let head = if animate { snake_head(tick) } else { 0 };
    // `DotGrid` recovers the head via `floor(phase * 9)`; offset by 0.5
    // so the floor lands exactly on `head`.
    let phase = (head as f32 + 0.5) / DOT_ORDER.len() as f32;
    DotGrid {
        dim: size,
        color,
        phase,
    }
    .into_any_element()
}

/// Working-state 3×3 grid where one head dot is fully lit and trailing dots
/// fade down the serpentine path.
#[derive(IntoElement)]
struct DotGrid {
    dim: Pixels,
    color: Hsla,
    phase: f32,
}

impl RenderOnce for DotGrid {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let dim_f: f32 = self.dim.into();
        let cell = dim_f / 3.0;
        let dot = cell * theme::STATUS_INDICATOR_DOT_GRID_RATIO;
        let pad = (cell - dot) * 0.5;

        let n = DOT_ORDER.len();
        let head = (self.phase * n as f32).floor() as usize % n;
        let tail_min = theme::STATUS_INDICATOR_DOT_GRID_TAIL_ALPHA_MIN;

        div()
            .relative()
            .flex_none()
            .w(self.dim)
            .h(self.dim)
            .children(DOT_ORDER.iter().enumerate().map(|(i, (col, row))| {
                let lag = (head + n - i) % n;
                let t = lag as f32 / (n - 1) as f32;
                let mut c = self.color;
                c.a *= 1.0 - t * (1.0 - tail_min);
                div()
                    .absolute()
                    .left(px(cell * (*col as f32) + pad))
                    .top(px(cell * (*row as f32) + pad))
                    .w(px(dot))
                    .h(px(dot))
                    .rounded_full()
                    .bg(c)
            }))
    }
}

// ── ExecutingTool ─────────────────────────────────────────────────────────────

/// Four clockwise quadrant frames — each lights a corner plus its two
/// adjacent edge dots. The centre is excluded (kept dim as an anchor).
const QUADRANT_FRAMES: [[(usize, usize); 3]; 4] = [
    [(0, 0), (1, 0), (0, 1)], // top-left
    [(2, 0), (1, 0), (2, 1)], // top-right
    [(2, 2), (2, 1), (1, 2)], // bottom-right
    [(0, 2), (0, 1), (1, 2)], // bottom-left
];

/// ExecutingTool rotating quadrant. `tick` drives the 4-frame clockwise
/// sweep. Static frame (no animation) is the top-left quadrant.
fn quadrant_grid(size: Pixels, color: Hsla, animate: bool, tick: u64) -> gpui::AnyElement {
    let frame = if animate { (tick % 4) as usize } else { 0 };
    QuadrantGrid {
        dim: size,
        color,
        frame,
    }
    .into_any_element()
}

/// ExecutingTool-state grid: a 3-dot quadrant rotates clockwise; the
/// centre dot stays dim at `STATUS_INDICATOR_RING_CENTER_ALPHA`.
#[derive(IntoElement)]
struct QuadrantGrid {
    dim: Pixels,
    color: Hsla,
    frame: usize,
}

impl RenderOnce for QuadrantGrid {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let dim_f: f32 = self.dim.into();
        let cell = dim_f / 3.0;
        let dot = cell * theme::STATUS_INDICATOR_DOT_GRID_RATIO;
        let pad = (cell - dot) * 0.5;
        let lit = QUADRANT_FRAMES[self.frame % QUADRANT_FRAMES.len()];

        div()
            .relative()
            .flex_none()
            .w(self.dim)
            .h(self.dim)
            .children(
                (0..3usize)
                    .flat_map(|col| (0..3usize).map(move |row| (col, row)))
                    .map(move |(col, row)| {
                        let alpha = if lit.contains(&(col, row)) {
                            1.0
                        } else if (col, row) == (1, 1) {
                            theme::STATUS_INDICATOR_RING_CENTER_ALPHA
                        } else {
                            0.0
                        };
                        let mut c = self.color;
                        c.a *= alpha;
                        div()
                            .absolute()
                            .left(px(cell * col as f32 + pad))
                            .top(px(cell * row as f32 + pad))
                            .w(px(dot))
                            .h(px(dot))
                            .rounded_full()
                            .bg(c)
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indicator_size_constants_match_theme() {
        assert_eq!(
            IndicatorSize::Leading.dim(),
            px(theme::STATUS_INDICATOR_SIZE)
        );
        assert_eq!(
            IndicatorSize::Badge.dim(),
            px(theme::STATUS_INDICATOR_BADGE_SIZE)
        );
    }

    #[test]
    fn animate_defaults_on_and_builder_overrides() {
        let badge = AgentStatusBadge::new(
            SessionStatus::Working,
            IndicatorSize::Leading,
            Hsla::default(),
        );
        assert!(badge.animate, "new() must default to animated");
        assert!(
            !badge.animate(false).animate,
            "animate(false) must disable the animation"
        );
    }

    #[test]
    fn tick_ms_is_positive_and_low_rate() {
        const {
            // ~6 fps target: low enough to slash redraws, high enough that
            // the 6-frame comet still reads as motion.
            assert!(theme::STATUS_INDICATOR_TICK_MS >= 80);
            assert!(theme::STATUS_INDICATOR_TICK_MS <= 300);
        }
    }

    #[test]
    fn blink_toggles_every_hold_ticks() {
        // On for the first BLINK_HOLD_TICKS, off for the next.
        assert!(blink_on(0));
        assert!(blink_on(BLINK_HOLD_TICKS - 1));
        assert!(!blink_on(BLINK_HOLD_TICKS));
        assert!(!blink_on(2 * BLINK_HOLD_TICKS - 1));
        assert!(blink_on(2 * BLINK_HOLD_TICKS));
    }

    #[test]
    fn snake_head_visits_six_distinct_spread_positions() {
        let heads: Vec<usize> = (0..6).map(snake_head).collect();
        assert_eq!(heads, vec![0, 2, 3, 5, 6, 8], "6 heads spread over 9-path");
        // Cycle repeats every 6 ticks.
        assert_eq!(snake_head(6), snake_head(0));
        for h in &heads {
            assert!(*h < DOT_ORDER.len());
        }
    }

    #[test]
    fn ring_center_alpha_is_visible_but_dim() {
        const {
            assert!(theme::STATUS_INDICATOR_RING_CENTER_ALPHA > 0.0);
            assert!(theme::STATUS_INDICATOR_RING_CENTER_ALPHA < 0.5);
        }
    }

    #[test]
    fn quadrant_frames_each_have_three_in_range_dots_excluding_centre() {
        for frame in QUADRANT_FRAMES.iter() {
            let mut seen = std::collections::HashSet::new();
            for cell in frame.iter() {
                assert!(seen.insert(*cell), "duplicate cell {:?}", cell);
                assert!(cell.0 < 3 && cell.1 < 3, "out-of-range {:?}", cell);
                assert_ne!(*cell, (1, 1), "centre must stay out of quadrant frames");
            }
            assert_eq!(seen.len(), 3);
        }
    }

    #[test]
    fn dot_grid_ratio_is_within_cell() {
        const {
            assert!(theme::STATUS_INDICATOR_DOT_GRID_RATIO > 0.0);
            assert!(theme::STATUS_INDICATOR_DOT_GRID_RATIO < 1.0);
        }
    }

    #[test]
    fn dot_grid_tail_alpha_leaves_visible_floor() {
        const {
            assert!(theme::STATUS_INDICATOR_DOT_GRID_TAIL_ALPHA_MIN > 0.0);
            assert!(theme::STATUS_INDICATOR_DOT_GRID_TAIL_ALPHA_MIN < 1.0);
        }
    }

    #[test]
    fn dot_order_visits_each_cell_exactly_once() {
        let mut seen = std::collections::HashSet::new();
        for cell in DOT_ORDER.iter() {
            assert!(seen.insert(*cell), "duplicate cell {:?}", cell);
            assert!(cell.0 < 3 && cell.1 < 3, "out-of-range cell {:?}", cell);
        }
        assert_eq!(seen.len(), 9);
    }

    #[test]
    fn connecting_patterns_cover_all_nine_cells() {
        let mut all = std::collections::HashSet::new();
        for &cell in PLUS_DOTS.iter().chain(CROSS_DOTS.iter()) {
            all.insert(cell);
        }
        assert_eq!(
            all.len(),
            9,
            "union of + and × patterns must cover all 9 cells"
        );
    }

    #[test]
    fn connecting_centre_is_shared_by_both_patterns() {
        let centre = (1, 1);
        assert!(PLUS_DOTS.contains(&centre));
        assert!(CROSS_DOTS.contains(&centre));
    }
}
