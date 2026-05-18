//! Claude Code session-status indicator — four visual modes per
//! [`SessionStatus`].
//!
//! All shapes are procedurally drawn with GPUI primitives — no SVG
//! files, no external assets. Colour, size, and timing values come
//! from `crate::ui::theme::STATUS_INDICATOR_*` (G4 — no
//! inline literals).
//!
//! All states render as a 3×3 dot grid at the same footprint.
//! State is expressed through colour, animation pattern, and timing:
//!
//! - **Idle** — all 9 dots fully lit, no animation.
//! - **Connecting** — plus (+) pattern cross-fades to cross (×) and back.
//!   The centre dot stays fully lit throughout.
//! - **NeedsAttention** — all 9 dots, opacity pulses `0.4 ↔ 1.0` over
//!   `1000 ms` ease-in-out.
//! - **Working** — serpentine "comet" head sweeps all 9 dots; head at full
//!   alpha, trailing dots fade to `STATUS_INDICATOR_DOT_GRID_TAIL_ALPHA_MIN`.
//! - **ExecutingTool** — amber comet sweeps the outer 8-dot ring clockwise;
//!   centre dot stays at `STATUS_INDICATOR_RING_CENTER_ALPHA`.

use std::time::Duration;

use crate::ui::theme;
use daruda_claude::SessionStatus;
use gpui::{
    Animation, AnimationExt, App, Hsla, IntoElement, ParentElement, Pixels, RenderOnce, Styled,
    Window, div, pulsating_between, px,
};

/// Size variant of the indicator.
#[derive(Clone, Copy)]
pub enum IndicatorSize {
    /// Left-dock leading indicator on a worktree row.
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
}

impl AgentStatusBadge {
    pub fn new(status: SessionStatus, size: IndicatorSize, color: Hsla) -> Self {
        Self {
            status,
            size,
            color,
            active: false,
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
}

impl RenderOnce for AgentStatusBadge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let dim = self.size.dim();
        let inner = match self.status {
            SessionStatus::Idle => idle_grid(dim, self.color),
            SessionStatus::Connecting => connecting_grid(dim, self.color),
            SessionStatus::NeedsAttention => needs_attention_grid(dim, self.color),
            SessionStatus::Working => dot_grid(dim, self.color),
            SessionStatus::ExecutingTool => ring_grid(dim, self.color),
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

/// 3×3 grid, all dots, opacity pulses 0.4 ↔ 1.0.
fn needs_attention_grid(size: Pixels, color: Hsla) -> gpui::AnyElement {
    FullGrid {
        dim: size,
        color,
        opacity: 1.0,
    }
    .with_animation(
        "claude-status-pulse",
        Animation::new(Duration::from_millis(
            theme::STATUS_INDICATOR_PULSE_DURATION_MS,
        ))
        .repeat()
        .with_easing(pulsating_between(
            theme::STATUS_INDICATOR_PULSE_OPACITY_MIN,
            1.0,
        )),
        |grid, alpha| grid.with_opacity(alpha),
    )
    .into_any_element()
}

// ── Connecting ───────────────────────────────────────────────────────────────

/// 3×3 grid, plus (+) ↔ cross (×) cross-fade.
fn connecting_grid(size: Pixels, color: Hsla) -> gpui::AnyElement {
    ConnectingGrid {
        dim: size,
        color,
        phase: 0.0,
    }
    .with_animation(
        "claude-status-connecting",
        Animation::new(Duration::from_millis(
            theme::STATUS_INDICATOR_CONNECTING_PERIOD_MS,
        ))
        .repeat(),
        |grid, phase| grid.with_phase(phase),
    )
    .into_any_element()
}

// ── Shared helpers ────────────────────────────────────────────────────────────

/// 3×3 grid with all dots at a uniform opacity multiplier.
/// Used for Idle (opacity = 1.0) and NeedsAttention (opacity animated).
#[derive(IntoElement)]
struct FullGrid {
    dim: Pixels,
    color: Hsla,
    /// Multiplied into `color.a` for every dot; driven by animation.
    opacity: f32,
}

impl FullGrid {
    fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }
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

/// Connecting-state 3×3 grid that cross-fades between plus (+) and cross (×).
/// The centre dot (shared by both patterns) stays fully lit throughout.
#[derive(IntoElement)]
struct ConnectingGrid {
    dim: Pixels,
    color: Hsla,
    phase: f32,
}

impl ConnectingGrid {
    fn with_phase(mut self, phase: f32) -> Self {
        self.phase = phase;
        self
    }
}

impl RenderOnce for ConnectingGrid {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let dim_f: f32 = self.dim.into();
        let cell = dim_f / 3.0;
        let dot = cell * theme::STATUS_INDICATOR_DOT_GRID_RATIO;
        let pad = (cell - dot) * 0.5;
        let color = self.color;

        // + peaks at phase = 0 / 1; × peaks at phase = 0.5.
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

fn dot_grid(size: Pixels, color: Hsla) -> gpui::AnyElement {
    DotGrid {
        dim: size,
        color,
        phase: 0.0,
    }
    .with_animation(
        "claude-status-dot-grid",
        Animation::new(Duration::from_millis(
            theme::STATUS_INDICATOR_SPINNER_PERIOD_MS,
        ))
        .repeat(),
        |grid, phase| grid.with_phase(phase),
    )
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

impl DotGrid {
    fn with_phase(mut self, phase: f32) -> Self {
        self.phase = phase;
        self
    }
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

/// Clockwise ring order — 8 outer dots, centre excluded.
const RING_ORDER: [(usize, usize); 8] = [
    (1, 0), // top
    (2, 0), // top-right
    (2, 1), // right
    (2, 2), // bottom-right
    (1, 2), // bottom
    (0, 2), // bottom-left
    (0, 1), // left
    (0, 0), // top-left
];

fn ring_grid(size: Pixels, color: Hsla) -> gpui::AnyElement {
    RingGrid {
        dim: size,
        color,
        phase: 0.0,
    }
    .with_animation(
        "claude-status-ring",
        Animation::new(Duration::from_millis(
            theme::STATUS_INDICATOR_RING_PERIOD_MS,
        ))
        .repeat(),
        |grid, phase| grid.with_phase(phase),
    )
    .into_any_element()
}

/// ExecutingTool-state grid: amber comet sweeps the outer ring clockwise;
/// the centre dot stays dim at `STATUS_INDICATOR_RING_CENTER_ALPHA`.
#[derive(IntoElement)]
struct RingGrid {
    dim: Pixels,
    color: Hsla,
    phase: f32,
}

impl RingGrid {
    fn with_phase(mut self, phase: f32) -> Self {
        self.phase = phase;
        self
    }
}

impl RenderOnce for RingGrid {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let dim_f: f32 = self.dim.into();
        let cell = dim_f / 3.0;
        let dot = cell * theme::STATUS_INDICATOR_DOT_GRID_RATIO;
        let pad = (cell - dot) * 0.5;

        let n = RING_ORDER.len();
        let head = (self.phase * n as f32).floor() as usize % n;
        let tail_min = theme::STATUS_INDICATOR_DOT_GRID_TAIL_ALPHA_MIN;

        div()
            .relative()
            .flex_none()
            .w(self.dim)
            .h(self.dim)
            .children(
                (0..3usize)
                    .flat_map(|col| (0..3usize).map(move |row| (col, row)))
                    .map(move |(col, row)| {
                        let mut c = self.color;
                        c.a *= match RING_ORDER.iter().position(|&pos| pos == (col, row)) {
                            Some(i) => {
                                let lag = (head + n - i) % n;
                                let t = lag as f32 / (n - 1) as f32;
                                1.0 - t * (1.0 - tail_min)
                            }
                            None => theme::STATUS_INDICATOR_RING_CENTER_ALPHA,
                        };
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
    fn animation_periods_are_positive() {
        const {
            assert!(theme::STATUS_INDICATOR_SPINNER_PERIOD_MS > 0);
            assert!(theme::STATUS_INDICATOR_PULSE_DURATION_MS > 0);
            assert!(theme::STATUS_INDICATOR_CONNECTING_PERIOD_MS > 0);
            assert!(theme::STATUS_INDICATOR_RING_PERIOD_MS > 0);
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
    fn ring_order_covers_all_outer_cells_exactly_once() {
        let mut seen = std::collections::HashSet::new();
        for cell in RING_ORDER.iter() {
            assert!(seen.insert(*cell), "duplicate cell {:?}", cell);
            assert!(cell.0 < 3 && cell.1 < 3, "out-of-range {:?}", cell);
        }
        assert_eq!(seen.len(), 8, "ring must have exactly 8 outer dots");
        assert!(!seen.contains(&(1, 1)), "centre must not be in ring");
    }

    #[test]
    fn dot_grid_ratio_is_within_cell() {
        // 0 ⇒ invisible; ≥1 ⇒ dots overlap into neighbouring cells.
        const {
            assert!(theme::STATUS_INDICATOR_DOT_GRID_RATIO > 0.0);
            assert!(theme::STATUS_INDICATOR_DOT_GRID_RATIO < 1.0);
        }
    }

    #[test]
    fn dot_grid_tail_alpha_leaves_visible_floor() {
        // 0 ⇒ tail fully invisible (motion gap); 1 ⇒ no fade at all.
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
