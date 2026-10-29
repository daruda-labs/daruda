//! Claude Code session-status indicator.
//!
//! Draws configurable dot grids or semantic icons using
//! `theme::STATUS_INDICATOR_*` constants (G4: no inline literals).
//!
//! Animation uses one gated [`StatusPulseClock`] instead of `with_animation`:
//! GPUI has no partial redraw, so a per-badge 60 fps animation repaints the
//! window tree. The shared ~4 fps clock keeps animating windows alive, leaves
//! idle windows at zero redraws, and satisfies AGENTS.md Pitfall #10.

use crate::ui::{StatusPulseClock, theme};
use daruda_agent::{AgentOutcome, SessionStatus};
use gpui::{App, Hsla, IntoElement, ParentElement, Pixels, RenderOnce, Styled, Window, div, px};

mod icons;

#[derive(Clone, Copy)]
enum BadgeContent {
    Disconnected,
    Status(SessionStatus),
    Outcome(AgentOutcome),
}

impl BadgeContent {
    fn state(self) -> crate::status_icon_themes::State {
        use crate::status_icon_themes::State;
        match self {
            Self::Disconnected => State::Disconnected,
            Self::Outcome(AgentOutcome::Completed) => State::Completed,
            Self::Outcome(AgentOutcome::Errored) => State::Errored,
            Self::Status(status) => state_for_status(status),
        }
    }

    fn status(self) -> SessionStatus {
        match self {
            Self::Disconnected => SessionStatus::Idle,
            Self::Status(status) => status,
            Self::Outcome(AgentOutcome::Completed) => SessionStatus::Idle,
            Self::Outcome(AgentOutcome::Errored) => SessionStatus::Failed,
        }
    }
}

pub(crate) fn icon_theme(cx: &App) -> daruda_config::StatusIconTheme {
    cx.try_global::<crate::settings_store::SettingsStore>()
        .map(|store| store.user().left_dock.status_icon_theme.clone())
        .unwrap_or_default()
}

pub(crate) fn state_for_status(status: SessionStatus) -> crate::status_icon_themes::State {
    use crate::status_icon_themes::State;
    match status {
        SessionStatus::Connecting => State::Connecting,
        SessionStatus::Working => State::Working,
        SessionStatus::ExecutingTool => State::ExecutingTool,
        SessionStatus::NeedsAttention => State::NeedsAttention,
        SessionStatus::Idle => State::Idle,
        SessionStatus::Failed => State::Failed,
    }
}

/// Ticks each 2-frame blink holds before toggling (~1 Hz at 4 fps).
const BLINK_HOLD_TICKS: u64 = 2;

/// `true` on the "lit" half of a 2-frame blink cycle.
fn blink_on(tick: u64) -> bool {
    (tick / BLINK_HOLD_TICKS).is_multiple_of(2)
}

/// Size variant of the indicator.
#[derive(Clone, Copy)]
pub enum IndicatorSize {
    /// Left-dock leading indicator on a lane row.
    Leading,
    /// Sub-row per-session badge.
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
        SessionStatus::Idle => t.text_muted,
        SessionStatus::Connecting => t.status_connecting_dark,
        SessionStatus::Failed => t.status_failed_dark,
    }
}

/// Stateless dock artwork, animated by the shared status clock.
#[derive(IntoElement)]
pub struct AgentStatusBadge {
    content: BadgeContent,
    size: IndicatorSize,
    color: Hsla,
    /// When true, the indicator is wrapped in a 1 px outline
    /// ring marking it as the session attached to the focused tab.
    active: bool,
    metrics: theme::UiMetrics,
}

impl AgentStatusBadge {
    pub fn disconnected(size: IndicatorSize, cx: &App) -> Self {
        Self {
            content: BadgeContent::Disconnected,
            ..Self::new(SessionStatus::Idle, size, theme::current(cx).text_muted)
        }
    }

    pub fn new(status: SessionStatus, size: IndicatorSize, color: Hsla) -> Self {
        Self {
            content: BadgeContent::Status(status),
            size,
            color,
            active: false,
            metrics: theme::UiMetrics::default(),
        }
    }

    /// Construct with the default theme color for the state. Reads
    /// the live `DarudaTheme` Global so the indicator picks up
    /// light-mode tones on theme switch.
    pub fn for_status(status: SessionStatus, size: IndicatorSize, cx: &App) -> Self {
        Self::new(status, size, color_for_status(status, cx))
    }

    pub fn for_outcome(outcome: AgentOutcome, size: IndicatorSize, cx: &App) -> Self {
        let content = BadgeContent::Outcome(outcome);
        let color = match outcome {
            AgentOutcome::Completed => theme::current(cx).status_idle_dark,
            AgentOutcome::Errored => theme::current(cx).status_failed_dark,
        };
        Self {
            content,
            ..Self::new(content.status(), size, color)
        }
    }

    /// Use the host surface's artwork size without changing its status shape.
    pub fn with_metrics(mut self, metrics: theme::UiMetrics) -> Self {
        self.metrics = metrics;
        self
    }

    /// Wrap the indicator with an outline ring for the focused session.
    pub fn active(mut self) -> Self {
        self.active = true;
        self
    }
}

impl RenderOnce for AgentStatusBadge {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let dim = px(self.metrics.icon(self.size.dim().into()));
        let tick = cx
            .try_global::<StatusPulseClock>()
            .map(|c| c.tick)
            .unwrap_or(0);
        let selected = icon_theme(cx);
        let catalog = crate::status_icon_themes::current(cx);
        let base = catalog.base(&selected);
        let state = self.content.state();
        let artwork = catalog
            .get(&selected)
            .and_then(|theme| theme.states.get(&state));
        let default_color = if base == crate::status_icon_themes::Base::Icons {
            icons::color(self.content, self.color, cx)
        } else {
            self.color
        };
        let color = artwork
            .and_then(|art| art.colors)
            .map(|(dark, light)| {
                gpui::rgb(if theme::current(cx).is_dark() {
                    dark
                } else {
                    light
                })
                .into()
            })
            .unwrap_or(default_color);
        let inner = if let Some(frame) = artwork.and_then(|art| art.frame(tick)) {
            crate::status_icon_themes::render::frame(frame.clone(), dim, color)
        } else if base == crate::status_icon_themes::Base::Icons
            || matches!(self.content, BadgeContent::Disconnected)
        {
            icons::render(self.content, dim, color, tick, cx)
        } else {
            match self.content.status() {
                SessionStatus::Idle => idle_grid(dim, color),
                SessionStatus::Connecting => connecting_grid(dim, color, tick),
                SessionStatus::NeedsAttention => needs_attention_grid(dim, color, tick),
                SessionStatus::Working => working_grid(dim, color, tick),
                SessionStatus::ExecutingTool => quadrant_grid(dim, color, tick),
                SessionStatus::Failed => failed_grid(dim, color),
            }
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
fn needs_attention_grid(size: Pixels, color: Hsla, tick: u64) -> gpui::AnyElement {
    let opacity = if blink_on(tick) {
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
/// `0.5` = full cross.
fn connecting_grid(size: Pixels, color: Hsla, tick: u64) -> gpui::AnyElement {
    let phase = if blink_on(tick) { 0.0 } else { 0.5 };
    ConnectingGrid {
        dim: size,
        color,
        phase,
    }
    .into_any_element()
}

// ── Failed ───────────────────────────────────────────────────────────────────

/// A still cross (×): Connecting's plus↔cross blink stopped on the cross, so
/// the shape, not only the red, tells it from NeedsAttention's full grid.
fn failed_grid(size: Pixels, color: Hsla) -> gpui::AnyElement {
    ConnectingGrid {
        dim: size,
        color,
        phase: 0.5,
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

/// Clockwise perimeter of the 3×3 grid, starting at the top-left corner.
const WORKING_PATH: [(usize, usize); 8] = [
    (0, 0),
    (1, 0),
    (2, 0),
    (2, 1),
    (2, 2),
    (1, 2),
    (0, 2),
    (0, 1),
];
const WORKING_DOT_COUNT: usize = 3;

fn working_dots(tick: u64) -> [(usize, usize); WORKING_DOT_COUNT] {
    let head = (tick % WORKING_PATH.len() as u64) as usize;
    std::array::from_fn(|offset| {
        WORKING_PATH[(head + WORKING_PATH.len() - offset) % WORKING_PATH.len()]
    })
}

/// Three equally bright dots advance one perimeter cell per shared tick.
fn working_grid(size: Pixels, color: Hsla, tick: u64) -> gpui::AnyElement {
    WorkingGrid {
        dim: size,
        color,
        tick,
    }
    .into_any_element()
}

#[derive(IntoElement)]
struct WorkingGrid {
    dim: Pixels,
    color: Hsla,
    tick: u64,
}

impl RenderOnce for WorkingGrid {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let dim_f: f32 = self.dim.into();
        let cell = dim_f / 3.0;
        let dot = cell * theme::STATUS_INDICATOR_DOT_GRID_RATIO;
        let pad = (cell - dot) * 0.5;

        div()
            .relative()
            .flex_none()
            .w(self.dim)
            .h(self.dim)
            .children(working_dots(self.tick).into_iter().map(|(col, row)| {
                div()
                    .absolute()
                    .left(px(cell * col as f32 + pad))
                    .top(px(cell * row as f32 + pad))
                    .w(px(dot))
                    .h(px(dot))
                    .rounded_full()
                    .bg(self.color)
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
/// sweep.
fn quadrant_grid(size: Pixels, color: Hsla, tick: u64) -> gpui::AnyElement {
    let frame = (tick % 4) as usize;
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
    fn tick_ms_is_positive_and_low_rate() {
        const {
            // ~4 fps target: low enough to bound off-focus redraws, high
            // enough that the perimeter sweep still reads as motion.
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
    fn working_dots_move_clockwise_and_wrap_without_lighting_the_centre() {
        let expected = [
            [(0, 0), (0, 1), (0, 2)],
            [(1, 0), (0, 0), (0, 1)],
            [(2, 0), (1, 0), (0, 0)],
            [(2, 1), (2, 0), (1, 0)],
            [(2, 2), (2, 1), (2, 0)],
            [(1, 2), (2, 2), (2, 1)],
            [(0, 2), (1, 2), (2, 2)],
            [(0, 1), (0, 2), (1, 2)],
        ];
        for (tick, dots) in expected.into_iter().enumerate() {
            assert_eq!(working_dots(tick as u64), dots);
            assert_eq!(working_dots(tick as u64 + 8), dots);
        }
        assert_eq!(working_dots(u64::MAX), expected[7]);
        assert_eq!(working_dots(u64::MAX.wrapping_add(1)), expected[0]);
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
