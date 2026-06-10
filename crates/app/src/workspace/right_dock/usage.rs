//! Usage tab body — renders aggregated token + cost telemetry pulled
//! from `daruda_claude::usage::UsageState`, plus the 5h/7d plan-rate
//! gauges and the public service-status pill (R-4 / R-5).
//!
//! Layout (top to bottom):
//! ```text
//! ┌─ Usage ──────────────────────────────────────────────────────┐
//! │  ● Operational                          [Last 7d ▼]          │ ← header
//! │  5h  ████████░░░░  68%   Resets in 2h 14m                    │ ← gauge
//! │  7d  ███░░░░░░░░░  31%   Resets in 3d 7h                     │ ← gauge
//! │  ──────────────────────────────────────────────────────────  │ ← Divider
//! │  Total  in: 142k  out: 28k  cache: 98k  ~$0.32              │ ← summary
//! │  ──────────────────────────────────────────────────────────  │ ← Divider
//! │  [abc12345]  daruda          in: 87k  out: 18k  cache: 60k   │
//! └──────────────────────────────────────────────────────────────┘
//! ```
//!
//! Per-session rows are sorted with the most recently updated session
//! at the top; cost is computed off the workspace-wide pricing
//! resolved from `[usage.pricing]` and snapshotted into
//! `RightDockSnapshot::usage_pricing` each frame. Gauges and the status
//! pill draw their data from `RightDockSnapshot::plan_limits` /
//! `service_status`, refreshed by `limits_pump` every
//! `[usage.poll]` interval.
//!
//! All static text comes from `surface::strings::USAGE_*`; all pixel
//! and color values from `daruda_terminal::ux::theme`. Direct
//! `hsla(...)` / `px(N)` literals are caught by
//! `scripts/lint-inline-literals.sh`.

use std::time::SystemTime;

use crate::ui::theme;
use daruda_claude::limits::{LimitSeverity, LimitWindow, PlanLimits};
use daruda_claude::service_status::{ServiceStatus, StatusIndicator};
use daruda_claude::usage::{SessionUsage, UsagePricing};
use gpui::{AnyElement, Context, Hsla, IntoElement, SharedString, div, prelude::*, px};

use super::super::layout::Dock;
use super::super::layout::RightDockSnapshot;
use crate::surface::strings;
use crate::ui::Divider;
use crate::ui::SectionHeader;

use crate::ui::Badge;

/// Render the Usage tab body.
///
/// Layout (top to bottom):
/// 1. Header row with the time-window dropdown (right-aligned).
/// 2. Summary row aggregating sessions inside the window.
/// 3. Divider.
/// 4. Per-session rows, most-recently-updated first.
///
/// When the window filter excludes every session the body falls
/// back to an empty-state message but still shows the dropdown so
/// the user can switch to `Lifetime` and see their data.
pub(in crate::workspace) fn render(snap: &RightDockSnapshot, cx: &mut Context<Dock>) -> AnyElement {
    let usage = &snap.usage;
    let cutoff = snap.usage_window.cutoff(SystemTime::now());
    let pricing = &snap.usage_pricing;
    let total = usage.filtered_total(cutoff);
    let mut sessions: Vec<&SessionUsage> = usage.filtered_sessions(cutoff).collect();
    // Most recently updated first — `Reverse` flips the natural
    // ascending order without resorting to a manual `cmp` closure.
    sessions.sort_by_key(|s| std::cmp::Reverse(s.last_updated));

    let mut body = crate::workspace::right_dock::right_panel_body()
        .child(header_row(snap, cx))
        .child(gauges_block(&snap.plan_limits, cx))
        .child(Divider::horizontal());

    if sessions.is_empty() {
        body = body.child(empty_state_inline(cx));
    } else {
        body = body
            .child(summary_row(snap.usage_window, &total, pricing, cx))
            .child(Divider::horizontal())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(theme::RIGHT_PANEL_ROW_GAP))
                    .children(sessions.into_iter().map(|s| session_row(s, pricing, cx))),
            );
    }

    body.into_any_element()
}

/// Top of the panel: full-width service-status row above the
/// time-window dropdown. Stacking vertically lets the status row
/// span the dock width while keeping the dropdown reachable on a
/// dedicated line.
fn header_row(snap: &RightDockSnapshot, cx: &gpui::App) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(theme::RIGHT_PANEL_ROW_GAP))
        .child(status_pill(&snap.service_status, cx))
        // Single dropdown — slot it at tab_index 0 so keyboard
        // users can change the time window without reaching for
        // the mouse.
        .child(crate::ui::select::select(&snap.usage_select, cx, 0))
}

/// Body shown when no session passes the time-window filter.
/// Stacked under the dropdown so the user can still switch to a
/// wider window without leaving the tab.
fn empty_state_inline(cx: &gpui::App) -> AnyElement {
    crate::ui::placeholder_text(strings::usage_empty_state())
        .py(px(theme::RIGHT_PANEL_PAD_Y))
        .text_size(px(theme::DOCK_PLACEHOLDER_FONT_SIZE))
        .text_color(theme::current(cx).dock_placeholder_text)
        .into_any_element()
}

/// One-line aggregate of sessions inside the active window.
///
/// Layout: a leading window-aware label ("Last 7d", "Lifetime", …)
/// plus a metrics group ([in][out][cache][~$]). Both groups are
/// `flex_none` and the parent uses `flex_wrap`, so a narrow dock
/// pushes the metrics group onto a second line instead of
/// truncating cell contents.
fn summary_row(
    window: daruda_store::project::UsageWindow,
    total: &SessionUsage,
    pricing: &UsagePricing,
    cx: &gpui::App,
) -> AnyElement {
    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_baseline()
        .gap(px(theme::RIGHT_PANEL_ROW_GAP))
        .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
        .text_color(theme::current(cx).muted_text)
        .child(label_pill(strings::usage_window_label(window), cx))
        .child(metrics_group(
            total.input_tokens,
            total.output_tokens,
            total.cache_read_tokens + total.cache_creation_tokens,
            total.estimated_cost(pricing),
            cx,
        ))
        .into_any_element()
}

/// One row per Claude session.
///
/// Layout mirrors [`summary_row`]: identity group ([badge][lane])
/// on the left, metrics group on the right; `flex_wrap` on the
/// parent puts metrics on a second line when the row is narrower
/// than ~identity_group_width + metrics_group_width.
fn session_row(s: &SessionUsage, pricing: &UsagePricing, cx: &gpui::App) -> AnyElement {
    let id_prefix = short_session_id(&s.session_id);
    let wt_label = worktree_label(s);

    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_baseline()
        .gap(px(theme::RIGHT_PANEL_ROW_GAP))
        .py(px(theme::RIGHT_PANEL_ROW_PAD_Y))
        .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
        .text_color(theme::current(cx).muted_text)
        .child(identity_group(id_prefix, wt_label, cx))
        .child(metrics_group(
            s.input_tokens,
            s.output_tokens,
            s.cache_read_tokens + s.cache_creation_tokens,
            s.estimated_cost(pricing),
            cx,
        ))
        .into_any_element()
}

/// Left group of a session row: short id badge + lane label.
/// `flex_none` so the entire group wraps as a unit; the inner
/// lane cell caps at [`theme::RIGHT_PANEL_WT_MAX_W`] and
/// truncates on overflow.
fn identity_group(
    id_prefix: SharedString,
    wt_label: SharedString,
    cx: &gpui::App,
) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .flex_none()
        .items_baseline()
        .gap(px(theme::RIGHT_PANEL_ROW_GAP))
        .child(Badge::new(id_prefix).monospace())
        .child(worktree_label_el(wt_label, cx))
}

/// Right group of summary + session rows: in / out / cache / cost.
/// `flex_none` so wrap behaviour treats this as one block —
/// individual metrics never split across lines.
fn metrics_group(
    input_tokens: u64,
    output_tokens: u64,
    cache_tokens: u64,
    cost: f64,
    cx: &gpui::App,
) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .flex_none()
        .items_baseline()
        .gap(px(theme::RIGHT_PANEL_ROW_GAP))
        .child(token_count(strings::usage_in_label(), input_tokens, cx))
        .child(token_count(strings::usage_out_label(), output_tokens, cx))
        .child(token_count(strings::usage_cache_label(), cache_tokens, cx))
        .child(cost_label(cost))
}

/// First 8 characters of the session id (or the whole id if it's
/// shorter). UUIDs from Claude Code's project directory naming are
/// always longer than 8, but defensive truncation keeps this
/// resistant to corrupted JSONL data.
fn short_session_id(session_id: &str) -> SharedString {
    let len = session_id.len().min(8);
    SharedString::from(session_id[..len].to_string())
}

/// Display name for a session's lane — basename of `worktree_path`,
/// or the configured fallback when the path has no name component.
fn worktree_label(s: &SessionUsage) -> SharedString {
    s.worktree_path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|n| SharedString::from(n.to_string()))
        .unwrap_or_else(|| SharedString::from(strings::USAGE_UNKNOWN_WORKTREE))
}

/// Compact summary "label" pill that doesn't carry data — used to
/// open the summary row with the literal "Total" so that row reads
/// the same as session rows from the eye's perspective.
fn label_pill(text: impl Into<gpui::SharedString>, cx: &gpui::App) -> impl IntoElement {
    div()
        .flex_none()
        .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
        .text_color(theme::current(cx).faint_text)
        .child(text.into())
}

/// "label: 142k" pair. Label is rendered dim, count in body color so
/// the eye scans the numbers first.
fn token_count(label: impl Into<gpui::SharedString>, n: u64, cx: &gpui::App) -> impl IntoElement {
    let t = theme::current(cx);
    let label_color = t.faint_text;
    let value_color = t.muted_text;
    div()
        .flex()
        .flex_row()
        .flex_none()
        .items_baseline()
        .gap(px(theme::RIGHT_PANEL_ROW_PAD_Y))
        .child(
            div()
                .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
                .text_color(label_color)
                .child(label.into()),
        )
        .child(
            div()
                .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
                .text_color(value_color)
                .child(SharedString::from(fmt_tokens(n))),
        )
}

/// "$0.32"-style price with two decimal places. The `~` prefix hints
/// to the user that this is an estimate based on default pricing —
/// real billing depends on the user's Anthropic plan.
fn cost_label(cost: f64) -> impl IntoElement {
    div()
        .flex_none()
        .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
        .text_color(theme::WARNING)
        .child(SharedString::from(format!("~${cost:.2}")))
}

/// Renderable lane label inside [`identity_group`]. Capped at
/// [`theme::RIGHT_PANEL_WT_MAX_W`] so a long branch name never
/// pushes the metrics group off-screen — instead the metrics group
/// wraps onto a second line via the row's `flex_wrap`.
fn worktree_label_el(label: SharedString, cx: &gpui::App) -> impl IntoElement {
    div()
        .flex_none()
        .max_w(px(theme::RIGHT_PANEL_WT_MAX_W))
        .overflow_hidden()
        .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
        .text_color(theme::current(cx).muted_text)
        .child(label)
}

// ----------------------------------------------------------------
// Plan-limit gauges (R-4)
// ----------------------------------------------------------------

/// Stack of two gauges (5h above 7d). Always renders both rows so
/// the layout is stable even before the first `limits_pump` tick —
/// missing windows fall back to the placeholder treatment.
fn gauges_block(limits: &PlanLimits, cx: &gpui::App) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(theme::RIGHT_PANEL_ROW_GAP))
        .child(SectionHeader::new(strings::usage_limits_section_label()))
        .child(gauge_row(
            strings::usage_limit_5h_label(),
            limits.five_hour.as_ref(),
            cx,
        ))
        .child(gauge_row(
            strings::usage_limit_7d_label(),
            limits.seven_day.as_ref(),
            cx,
        ))
}

/// One gauge: `[ "5h"  ━━━━━░░░░░  68%   Resets in 2h 14m ]`.
/// `None` window → placeholder (dim track + "Unavailable").
fn gauge_row(
    label: impl Into<gpui::SharedString>,
    window: Option<&LimitWindow>,
    cx: &gpui::App,
) -> AnyElement {
    let label: gpui::SharedString = label.into();
    let Some(win) = window else {
        return placeholder_gauge(label, cx);
    };
    let pct = win.utilization;
    let color = severity_color(LimitSeverity::from_utilization(pct));
    let reset_text = win
        .resets_at
        .and_then(|t| t.duration_since(SystemTime::now()).ok())
        .map(strings::format_reset_countdown);

    let theme_t = theme::current(cx);
    let row_label_text = theme_t.faint_text;
    let percent_text = theme_t.muted_text;
    let reset_text_color = theme_t.faint_text;

    let mut row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::RIGHT_PANEL_ROW_GAP))
        .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
        .text_color(row_label_text)
        .child(gauge_label(label, cx))
        .child(gauge_bar(pct, color, cx))
        .child(gauge_percent(pct, percent_text));

    if let Some(t) = reset_text {
        row = row.child(gauge_reset_text(t, reset_text_color));
    }
    row.into_any_element()
}

/// Dimmed placeholder used when the OAuth token is unavailable, the
/// `/api/oauth/usage` request failed, or Anthropic omitted this
/// window from the response. Renders the same shape as a real
/// gauge so the layout doesn't shift when data arrives.
fn placeholder_gauge(label: impl Into<gpui::SharedString>, cx: &gpui::App) -> AnyElement {
    let track_bg = theme::current(cx).gauge_track_bg;
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::RIGHT_PANEL_ROW_GAP))
        .child(gauge_label(label, cx))
        .child(gauge_bar(0.0, track_bg, cx))
        .child(placeholder_unavailable_text())
        .into_any_element()
}

/// "Unavailable" trailing label rendered next to a placeholder
/// gauge bar. Carries its own `text_size` because GPUI does not
/// cascade `text_size` from parent divs through nested div
/// children — only direct text content on the same div inherits.
/// Without this the text fell back to a near-zero default and
/// looked invisible.
fn placeholder_unavailable_text() -> impl IntoElement {
    div()
        .flex_none()
        .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
        .text_color(theme::TEXT_TERTIARY)
        .child(strings::usage_limit_unavailable())
}

/// "5h" / "7d" leading label. Fixed width so the bars align across
/// the two rows.
fn gauge_label(text: impl Into<gpui::SharedString>, cx: &gpui::App) -> impl IntoElement {
    div()
        .flex_none()
        .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
        .text_color(theme::current(cx).faint_text)
        .child(text.into())
}

/// Filled bar — outer track with a percentage-width fill. `pct` is
/// expected in the 0–100 range (the parser already clamps); we
/// clamp again as a defensive belt-and-braces.
fn gauge_bar(pct: f32, color: Hsla, cx: &gpui::App) -> impl IntoElement {
    let pct = pct.clamp(0.0, 100.0);
    let track_bg = theme::current(cx).gauge_track_bg;
    div()
        .flex_grow()
        .h(px(theme::GAUGE_BAR_HEIGHT))
        .rounded(px(theme::GAUGE_BAR_RADIUS))
        .bg(track_bg)
        .child(
            div()
                .h(px(theme::GAUGE_BAR_HEIGHT))
                .w(gpui::relative(pct / 100.0))
                .rounded(px(theme::GAUGE_BAR_RADIUS))
                .bg(color),
        )
}

/// "68%" label trailing the bar. Integer rounded — fractional
/// percentages are noise at this resolution.
fn gauge_percent(pct: f32, text_color: Hsla) -> impl IntoElement {
    div()
        .flex_none()
        .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
        .text_color(text_color)
        .child(SharedString::from(format!("{:.0}%", pct)))
}

/// "Resets in 2h 14m" sub-label. Hidden when `resets_at` was
/// missing from the API response.
fn gauge_reset_text(text: String, color: Hsla) -> impl IntoElement {
    div()
        .flex_none()
        .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
        .text_color(color)
        .child(SharedString::from(text))
}

/// Map a severity bucket to its bar color. Kept as a free function
/// (rather than a method on `LimitSeverity` in `daruda_claude`)
/// because the data layer is GPUI-free — `Hsla` lives in
/// `daruda_terminal::ux::theme`.
fn severity_color(severity: LimitSeverity) -> Hsla {
    match severity {
        LimitSeverity::Low => theme::SIGNAL_GREEN,
        LimitSeverity::Medium => theme::SIGNAL_YELLOW,
        LimitSeverity::High => theme::SIGNAL_RED,
    }
}

// ----------------------------------------------------------------
// Service-status pill (R-5)
// ----------------------------------------------------------------

/// Top-of-panel status row: a colored dot plus the upstream status
/// label, spanning the full dock width. Color and label both flow
/// from the indicator — the description is only used for
/// non-operational indicators (matching the Übersicht widget's
/// behavior). No pill chrome (bg / border / rounded) — the row
/// sits flush with the surrounding panel padding.
fn status_pill(status: &ServiceStatus, cx: &gpui::App) -> impl IntoElement {
    let color = indicator_color(status.indicator);
    let label = strings::service_status_label(status);
    let muted_text = theme::current(cx).muted_text;

    div()
        .flex()
        .flex_row()
        .w_full()
        .items_center()
        .gap(px(theme::STATUS_PILL_GAP))
        .py(px(theme::STATUS_PILL_PAD_Y))
        .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
        .text_color(muted_text)
        .child(status_dot(color))
        .child(SharedString::from(label))
}

/// Colored dot inside the status pill. Sized via `STATUS_PILL_DOT_SIZE`.
fn status_dot(color: Hsla) -> impl IntoElement {
    div()
        .flex_none()
        .w(px(theme::STATUS_PILL_DOT_SIZE))
        .h(px(theme::STATUS_PILL_DOT_SIZE))
        .rounded_full()
        .bg(color)
}

/// Map an indicator to its pill color. `Unknown` uses the dim
/// placeholder color so a parse miss doesn't look like green.
fn indicator_color(indicator: StatusIndicator) -> Hsla {
    match indicator {
        StatusIndicator::None => theme::SIGNAL_GREEN,
        StatusIndicator::Minor => theme::SIGNAL_YELLOW,
        StatusIndicator::Major => theme::SIGNAL_ORANGE,
        StatusIndicator::Critical => theme::SIGNAL_RED,
        StatusIndicator::Unknown => theme::TEXT_TERTIARY,
    }
}

/// Format a raw token count into a compact human-readable string.
///
/// - `>= 1_000_000`: one decimal place + `M` (e.g. `1.2M`).
/// - `>= 1_000`: integer + `k` (e.g. `142k`).
/// - `< 1_000`: bare integer (e.g. `532`).
///
/// Boundaries are exclusive on the upper side so `1_000_000`
/// displays as `1.0M` and `1_000` as `1k`. The `M` formatter rounds
/// to the nearest tenth via `f64`'s default formatter, which
/// matches `format!("{:.1}", ...)`.
fn fmt_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{}k", n / 1_000)
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn fmt_tokens_under_1k_is_bare_integer() {
        assert_eq!(fmt_tokens(0), "0");
        assert_eq!(fmt_tokens(1), "1");
        assert_eq!(fmt_tokens(532), "532");
        assert_eq!(fmt_tokens(999), "999");
    }

    #[test]
    fn fmt_tokens_thousands_use_k_suffix() {
        assert_eq!(fmt_tokens(1_000), "1k");
        assert_eq!(fmt_tokens(1_500), "1k");
        assert_eq!(fmt_tokens(142_310), "142k");
        assert_eq!(fmt_tokens(999_999), "999k");
    }

    #[test]
    fn fmt_tokens_millions_use_m_suffix_with_one_decimal() {
        assert_eq!(fmt_tokens(1_000_000), "1.0M");
        assert_eq!(fmt_tokens(1_200_000), "1.2M");
        assert_eq!(fmt_tokens(2_500_000), "2.5M");
        assert_eq!(fmt_tokens(15_700_000), "15.7M");
    }

    #[test]
    fn fmt_tokens_handles_u64_max_without_overflow() {
        // Sanity: huge inputs (won't happen in practice) still
        // produce a string rather than panicking on cast.
        let s = fmt_tokens(u64::MAX);
        assert!(s.ends_with('M'), "got {s}");
    }

    #[test]
    fn short_session_id_truncates_to_eight_chars() {
        assert_eq!(short_session_id("abc1234567890def"), "abc12345");
    }

    #[test]
    fn short_session_id_passes_through_when_already_short() {
        assert_eq!(short_session_id("abc"), "abc");
        assert_eq!(short_session_id(""), "");
    }

    #[test]
    fn worktree_label_uses_path_basename() {
        let s = SessionUsage {
            worktree_path: PathBuf::from("/Users/me/git/daruda"),
            ..Default::default()
        };
        assert_eq!(worktree_label(&s), SharedString::from("daruda"));
    }

    #[test]
    fn worktree_label_falls_back_when_basename_missing() {
        let s = SessionUsage {
            // Root path has no file_name component on Unix.
            worktree_path: PathBuf::from("/"),
            ..Default::default()
        };
        assert_eq!(
            worktree_label(&s),
            SharedString::from(strings::USAGE_UNKNOWN_WORKTREE)
        );
    }

    #[gpui::test]
    fn severity_color_matches_uebersicht_palette(cx: &mut gpui::TestAppContext) {
        crate::test_support::init_gpui_component(cx);
        cx.update(|_cx| {
            assert_eq!(severity_color(LimitSeverity::Low), theme::SIGNAL_GREEN);
            assert_eq!(severity_color(LimitSeverity::Medium), theme::SIGNAL_YELLOW);
            assert_eq!(severity_color(LimitSeverity::High), theme::SIGNAL_RED);
        });
    }

    #[gpui::test]
    fn indicator_color_dims_unknown(cx: &mut gpui::TestAppContext) {
        crate::test_support::init_gpui_component(cx);
        cx.update(|_cx| {
            assert_eq!(indicator_color(StatusIndicator::None), theme::SIGNAL_GREEN);
            assert_eq!(
                indicator_color(StatusIndicator::Minor),
                theme::SIGNAL_YELLOW
            );
            assert_eq!(
                indicator_color(StatusIndicator::Major),
                theme::SIGNAL_ORANGE
            );
            assert_eq!(
                indicator_color(StatusIndicator::Critical),
                theme::SIGNAL_RED
            );
            assert_eq!(
                indicator_color(StatusIndicator::Unknown),
                theme::TEXT_TERTIARY
            );
        });
    }
}
