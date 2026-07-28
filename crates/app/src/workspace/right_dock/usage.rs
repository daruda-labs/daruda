//! Usage tab body — a widget-style dashboard (header, status pill, plan
//! gauges, today's stats, 7-day chart, totals) modelled on the Übersicht
//! `claude-usage` widget.
//!
//! Plan limits come from `RightDockSnapshot::usage` (limits pump);
//! activity from `RightDockSnapshot::activity` (local JSONL aggregation
//! pump). The ↻ badge dispatches `Workspace::refresh_usage_now`. Static
//! text comes from `surface::strings::usage_*`, pixels/colors from
//! `crate::ui::theme`.

use std::time::{Duration, SystemTime};

use crate::ui::theme;
use daruda_claude::activity::ActivityStats;
use daruda_claude::service_status::{ServiceStatus, StatusIndicator};
use daruda_claude::{LimitSeverity, PlanInfo, ProviderUsage, UsageWindow};
use gpui::{AnyElement, Context, Hsla, IntoElement, SharedString, div, prelude::*, px};

use super::super::layout::Dock;
use super::super::layout::RightDockSnapshot;
use crate::surface::strings;
use crate::ui::{
    ButtonVariants as _, Disableable as _, GroupBoxVariants as _, SectionHeader, Sizable as _,
    button, group_box,
};

/// Render the Usage tab body.
pub(in crate::workspace) fn render(snap: &RightDockSnapshot, cx: &mut Context<Dock>) -> AnyElement {
    if snap.usage.is_signed_out() {
        return signed_out_body(snap.account_label.clone(), cx);
    }
    crate::workspace::right_dock::right_panel_body()
        .child(header(
            snap.usage.snapshot().and_then(|u| u.plan.as_ref()),
            snap.account_label.clone(),
            cx,
        ))
        .child(status_pill(snap.service_status.as_ref(), cx))
        .child(usage_section_header(
            snap.usage.snapshot().and_then(|u| u.fetched_at),
            snap.usage_refresh_in_flight,
            &snap.workspace,
        ))
        .child(gauges_block(snap.usage.snapshot(), cx))
        .child(today_block(&snap.activity, cx))
        .child(chart_block(&snap.activity, cx))
        .child(totals_block(&snap.activity, cx))
        .into_any_element()
}

/// Body for a domain nobody is signed into: whose account slot this is, then a
/// short notice — no brand title and no gauges stuck on a permanent
/// placeholder.
fn signed_out_body(account_label: SharedString, cx: &gpui::App) -> AnyElement {
    crate::workspace::right_dock::right_panel_body()
        .child(account_label_text(
            account_label,
            theme::current(cx).text_muted,
        ))
        .child(
            crate::ui::placeholder_text(strings::usage_domain_unavailable())
                .text_size(px(theme::DOCK_PLACEHOLDER_FONT_SIZE))
                .text_color(theme::current(cx).text_subtle),
        )
        .into_any_element()
}

// ----------------------------------------------------------------
// Header (logo + title + plan badge)
// ----------------------------------------------------------------

/// Account label sits on its own full-width row below the title row so a
/// long email can't push the plan badge off-screen — `w_full` lets it wrap
/// instead of being clipped by the fixed-width title row.
fn header(
    plan: Option<&PlanInfo>,
    account_label: SharedString,
    cx: &gpui::App,
) -> impl IntoElement {
    let t = theme::current(cx);
    let mut title_row = div()
        .flex()
        .flex_row()
        .items_center()
        .w_full()
        .gap(px(theme::USAGE_HEADER_GAP))
        .child(
            div()
                .flex_grow()
                .text_size(px(theme::USAGE_TITLE_FONT_SIZE))
                .text_color(t.text_muted)
                .child(SharedString::from(strings::usage_brand_title())),
        );

    if let Some(label) = plan_badge_label(plan) {
        title_row = title_row.child(plan_badge(label));
    }

    div()
        .flex()
        .flex_col()
        .w_full()
        .gap(px(theme::GAP_XS))
        .child(title_row)
        .child(account_label_text(account_label, t.text_muted))
}

/// Display-only identity of the account the tab's usage/activity data
/// belongs to — the focused pane's managed-account email, or the
/// "System" fallback. Purely informational: no click target, no
/// dropdown. An account selector is deferred (plan §6.8 option B).
fn account_label_text(label: SharedString, muted: Hsla) -> impl IntoElement {
    div()
        .w_full()
        .text_size(px(theme::USAGE_PLAN_BADGE_FONT_SIZE))
        .text_color(muted)
        .child(label)
}

/// Trailing plan badge ("TEAM 5x").
fn plan_badge(label: String) -> impl IntoElement {
    div()
        .flex_none()
        .px(px(theme::USAGE_PLAN_BADGE_PAD_X))
        .py(px(theme::USAGE_PLAN_BADGE_PAD_Y))
        .rounded(px(theme::USAGE_PLAN_BADGE_RADIUS))
        .bg(theme::USAGE_ACCENT_CHIP_BG)
        .text_size(px(theme::USAGE_PLAN_BADGE_FONT_SIZE))
        .text_color(theme::USAGE_ACCENT_CHIP_FG)
        .child(SharedString::from(label))
}

// ----------------------------------------------------------------
// Section header + refresh badge
// ----------------------------------------------------------------

/// "PLAN USAGE" heading with a trailing clickable cache-age / refresh
/// badge. Clicking dispatches `Workspace::refresh_usage_now`.
fn usage_section_header(
    fetched_at: Option<SystemTime>,
    in_flight: bool,
    workspace: &gpui::WeakEntity<crate::workspace::Workspace>,
) -> impl IntoElement {
    let label = refresh_badge_label(fetched_at, in_flight);
    let workspace = workspace.clone();

    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .w_full()
        .child(SectionHeader::new(strings::usage_limits_section_label()))
        .child(
            // Ghost button: subtle text affordance with hover/press
            // feedback. Disabled while a refresh is in flight so a
            // double-click can't fan out a second fetch.
            button("usage-refresh-badge", label)
                .ghost()
                .xsmall()
                .disabled(in_flight)
                .on_click(move |_, _window, cx| {
                    if let Some(ws) = workspace.upgrade() {
                        ws.update(cx, |ws, cx| ws.refresh_usage_now(cx));
                    }
                }),
        )
}

/// Resolve the refresh-badge label: the in-flight spinner wins; then a
/// never-fetched state shows the plain "Refresh"; otherwise the cache
/// age bucket.
fn refresh_badge_label(fetched_at: Option<SystemTime>, in_flight: bool) -> String {
    if in_flight {
        return strings::usage_refreshing();
    }
    let age = fetched_at.and_then(|t| SystemTime::now().duration_since(t).ok());
    match cache_age_bucket(age) {
        CacheAge::Never => strings::usage_refresh(),
        CacheAge::JustNow => strings::usage_cache_just_now(),
        CacheAge::Minutes(n) => strings::usage_cache_minutes(n),
        CacheAge::Hours(n) => strings::usage_cache_hours(n),
        CacheAge::Days(n) => strings::usage_cache_days(n),
    }
}

// ----------------------------------------------------------------
// Plan-limit gauge cards
// ----------------------------------------------------------------

/// One gauge card per window the provider reported, shortest first
/// (`ProviderUsage` sorts them). Before the first fetch lands — or when the
/// provider metered nothing — a single placeholder card holds the space, since
/// which windows exist is now the provider's answer rather than a fixed set.
fn gauges_block(usage: Option<&ProviderUsage>, cx: &gpui::App) -> impl IntoElement {
    let windows = usage.map(|u| u.windows.as_slice()).unwrap_or_default();
    let col = div().flex().flex_col().gap(px(theme::USAGE_CARD_GAP));
    if windows.is_empty() {
        return col.child(gauge_card(strings::usage_limit_unavailable(), None, cx));
    }
    windows.iter().fold(col, |col, window| {
        col.child(gauge_card(
            crate::workspace::usage_labels::window_label(window),
            Some(window),
            cx,
        ))
    })
}

/// One gauge card: header row (label + big %), bar, optional reset text.
/// `None` window → placeholder (dim bar + "Unavailable", no %).
fn gauge_card(
    label: impl Into<SharedString>,
    window: Option<&UsageWindow>,
    cx: &gpui::App,
) -> AnyElement {
    let t = theme::current(cx);
    let label: SharedString = label.into();

    // Border-only card (no fill), `theme.radius` corners.
    let card = group_box().outline();

    let Some(win) = window else {
        // Placeholder: label + dim bar + "Unavailable".
        return card
            .child(gauge_header_row(label, None, t.text_muted, t.text_subtle))
            .child(gauge_bar(0.0, t.gauge_track_bg))
            .into_any_element();
    };

    let pct = win.utilization;
    let color = severity_color(LimitSeverity::from_utilization(pct));
    let reset_text = win
        .resets_at
        .and_then(|at| at.duration_since(SystemTime::now()).ok())
        .map(strings::format_reset_countdown);

    let mut card = card
        .child(gauge_header_row(
            label,
            Some(pct),
            t.text_subtle,
            t.text_muted,
        ))
        .child(gauge_bar(pct, color));
    if let Some(reset) = reset_text {
        card = card.child(
            div()
                .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
                .text_color(t.text_subtle)
                .child(SharedString::from(reset)),
        );
    }
    card.into_any_element()
}

/// The label + percentage row at the top of a gauge card. `pct == None`
/// renders the "Unavailable" placeholder in place of the big number.
fn gauge_header_row(
    label: SharedString,
    pct: Option<f32>,
    label_color: Hsla,
    pct_color: Hsla,
) -> impl IntoElement {
    let value: SharedString = match pct {
        Some(p) => format!("{:.0}%", p.clamp(0.0, 100.0)).into(),
        None => strings::usage_limit_unavailable().into(),
    };
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .w_full()
        .child(
            div()
                .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
                .text_color(label_color)
                .child(label),
        )
        .child(
            div()
                .text_size(px(theme::USAGE_GAUGE_PERCENT_FONT_SIZE))
                .text_color(pct_color)
                .child(value),
        )
}

/// Filled bar via `gpui_component::Progress`. `color` is the fill; the
/// widget renders the track as that color at 20% opacity. The widget
/// clamps `pct` to `0..=100`.
fn gauge_bar(pct: f32, color: Hsla) -> impl IntoElement {
    crate::ui::progress(pct)
        .bg(color)
        .h(px(theme::GAUGE_BAR_HEIGHT))
        .rounded(px(theme::GAUGE_BAR_RADIUS))
}

// ----------------------------------------------------------------
// Today's activity (3 stat cards)
// ----------------------------------------------------------------

/// "TODAY" heading + a 3-up grid of stat cards. Counts come from the
/// `DayActivity` whose date matches the local calendar day; if none is
/// present (no activity yet today) the cards show zeros.
fn today_block(activity: &ActivityStats, cx: &gpui::App) -> impl IntoElement {
    let today_str = local_today();
    let today = activity.daily.iter().find(|d| d.date == today_str);
    let (messages, sessions, tool_calls) = today
        .map(|d| (d.messages, d.sessions, d.tool_calls))
        .unwrap_or((0, 0, 0));

    div()
        .flex()
        .flex_col()
        .gap(px(theme::RIGHT_PANEL_ROW_GAP))
        .child(SectionHeader::new(strings::usage_section_today()))
        .child(
            div()
                .flex()
                .flex_row()
                .w_full()
                .gap(px(theme::USAGE_STAT_GRID_GAP))
                .child(stat_card(
                    fmt_count(messages),
                    strings::usage_stat_messages(),
                    cx,
                ))
                .child(stat_card(
                    fmt_count(sessions),
                    strings::usage_stat_sessions(),
                    cx,
                ))
                .child(stat_card(
                    fmt_count(tool_calls),
                    strings::usage_stat_tool_calls(),
                    cx,
                )),
        )
}

/// One stat card: a big value over a muted label, centered.
fn stat_card(value: String, label: String, cx: &gpui::App) -> impl IntoElement {
    let t = theme::current(cx);
    // `flex_1` on a wrapper div, not the GroupBox (GroupBox forces
    // `w_full`, so the wrapper owns the 1/3 grid width). A full-width
    // `items_center` child re-centers over the left-aligned GroupBox.
    div().flex_1().min_w_0().child(
        group_box().outline().child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .w_full()
                .gap(px(theme::USAGE_STAT_CARD_GAP))
                .child(
                    div()
                        .text_size(px(theme::USAGE_STAT_VALUE_FONT_SIZE))
                        .text_color(t.text_muted)
                        .child(SharedString::from(value)),
                )
                .child(
                    div()
                        .text_size(px(theme::USAGE_STAT_LABEL_FONT_SIZE))
                        .text_color(t.text_subtle)
                        .child(SharedString::from(label)),
                ),
        ),
    )
}

// ----------------------------------------------------------------
// 7-day activity chart
// ----------------------------------------------------------------

/// "LAST 7 DAYS" heading + a bar chart over the most recent ≤7 days with
/// activity, weekday-labeled, today highlighted. The aggregator stores
/// only active days (ascending by date), so zero days are dropped, not
/// padded; today is matched by date (not position). Heights normalize to
/// the busiest day in the window.
fn chart_block(activity: &ActivityStats, cx: &gpui::App) -> AnyElement {
    let today = chrono::Local::now().date_naive();
    let n = activity.daily.len();
    let recent = &activity.daily[n.saturating_sub(7)..];

    let block = div()
        .flex()
        .flex_col()
        .gap(px(theme::RIGHT_PANEL_ROW_GAP))
        .child(SectionHeader::new(strings::usage_section_7day()));

    // Nothing aggregated yet — just the heading, no empty chart frame.
    if recent.is_empty() {
        return block.into_any_element();
    }

    let messages: Vec<u64> = recent.iter().map(|d| d.messages).collect();
    let heights = chart_heights(
        &messages,
        theme::USAGE_CHART_BAR_MAX_HEIGHT,
        theme::USAGE_CHART_BAR_MIN_HEIGHT,
    );

    let mut row = div()
        .flex()
        .flex_row()
        .items_end()
        .w_full()
        .gap(px(theme::USAGE_CHART_BAR_GAP));
    for (i, day) in recent.iter().enumerate() {
        row = row.child(chart_bar(&day.date, today, heights[i], cx));
    }

    block.child(row).into_any_element()
}

/// One chart column: the bar (bottom-aligned) over its weekday label.
/// `date` is the aggregator's `%Y-%m-%d` key; an unparseable date falls
/// back to a blank label and "not today".
fn chart_bar(
    date: &str,
    today: chrono::NaiveDate,
    height: f32,
    cx: &gpui::App,
) -> impl IntoElement {
    use chrono::Datelike as _;
    let parsed = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok();
    let is_today = parsed == Some(today);
    let fill = if is_today {
        theme::USAGE_CHART_BAR_TODAY
    } else {
        theme::USAGE_CHART_BAR_OTHER
    };
    let label = parsed
        .map(|d| strings::usage_weekday_label(d.weekday().num_days_from_sunday() as u8))
        .unwrap_or_default();
    let label_color = theme::current(cx).text_subtle;

    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_end()
        .gap(px(theme::USAGE_CHART_LABEL_GAP))
        .child(
            div()
                .w_full()
                .h(px(height))
                .rounded(px(theme::USAGE_CHART_BAR_RADIUS))
                .bg(fill),
        )
        .child(
            div()
                .text_size(px(theme::USAGE_CHART_LABEL_FONT_SIZE))
                .text_color(label_color)
                .child(SharedString::from(label)),
        )
}

// ----------------------------------------------------------------
// Totals row
// ----------------------------------------------------------------

/// All-time totals: a "TOTAL" section header over a 3-up row of
/// value-over-label cells. Borderless GroupBox (`.normal()`) so the
/// footer reads as a summary, not a card.
fn totals_block(activity: &ActivityStats, cx: &gpui::App) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(theme::RIGHT_PANEL_ROW_GAP))
        .child(SectionHeader::new(strings::usage_section_total()))
        .child(totals_row(activity, cx))
}

/// The 3-up totals cells row.
fn totals_row(activity: &ActivityStats, cx: &gpui::App) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .w_full()
        .gap(px(theme::USAGE_STAT_GRID_GAP))
        .child(total_item(
            fmt_count(activity.total_messages),
            strings::usage_total_messages(),
            cx,
        ))
        .child(total_item(
            fmt_count(activity.total_sessions),
            strings::usage_total_sessions(),
            cx,
        ))
        .child(total_item(
            fmt_count(activity.active_days),
            strings::usage_total_active_days(),
            cx,
        ))
}

/// One totals cell: a borderless GroupBox holding a centered value over
/// its label. `flex_1` on the wrapper, not the GroupBox (which forces
/// `w_full`).
fn total_item(value: String, label: String, cx: &gpui::App) -> impl IntoElement {
    let t = theme::current(cx);
    div().flex_1().min_w_0().child(
        group_box().normal().child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .w_full()
                .gap(px(theme::USAGE_STAT_CARD_GAP))
                .child(
                    div()
                        .text_size(px(theme::USAGE_TOTAL_VALUE_FONT_SIZE))
                        .text_color(t.text_muted)
                        .child(SharedString::from(value)),
                )
                .child(
                    div()
                        .text_size(px(theme::USAGE_TOTAL_LABEL_FONT_SIZE))
                        .text_color(t.text_subtle)
                        .child(SharedString::from(label)),
                ),
        ),
    )
}

// ----------------------------------------------------------------
// Service-status pill (tinted)
// ----------------------------------------------------------------

/// Status row: a colored dot plus the upstream status label on a faint
/// tint of the indicator color.
fn status_pill(status: Option<&ServiceStatus>, cx: &gpui::App) -> impl IntoElement {
    // Absent until this domain's first status fetch lands — the same dimmed
    // "unknown" pill the API's own `Unknown` indicator produces.
    let unknown = ServiceStatus::default();
    let status = status.unwrap_or(&unknown);
    let color = indicator_color(status.indicator, cx);
    let label = strings::service_status_label(status);
    let muted_text = theme::current(cx).text_muted;
    let mut tint = color;
    tint.a = theme::RIGHT_PANEL_STATUS_PILL_BG_ALPHA;

    div()
        .flex()
        .flex_row()
        .w_full()
        .items_center()
        .gap(px(theme::STATUS_PILL_GAP))
        .px(px(theme::RIGHT_PANEL_STATUS_PILL_PADDING_X_PX))
        .py(px(theme::STATUS_PILL_PAD_Y))
        .rounded(px(theme::RIGHT_PANEL_STATUS_PILL_RADIUS_PX))
        .bg(tint)
        .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
        .text_color(muted_text)
        .child(status_dot(color))
        .child(SharedString::from(label))
}

/// Colored dot inside the status pill.
fn status_dot(color: Hsla) -> impl IntoElement {
    div()
        .flex_none()
        .w(px(theme::STATUS_PILL_DOT_SIZE))
        .h(px(theme::STATUS_PILL_DOT_SIZE))
        .rounded_full()
        .bg(color)
}

// ----------------------------------------------------------------
// Shared helpers
// ----------------------------------------------------------------

/// Map a severity bucket to its bar color. Free function (not a method
/// on `LimitSeverity`) because the data layer is GPUI-free — `Hsla`
/// lives in the theme module. Shared with the status bar's usage chip
/// (`status_bar::usage_chip`) so both surfaces bucket the same number
/// into the same colour.
pub(in crate::workspace) fn severity_color(severity: LimitSeverity) -> Hsla {
    match severity {
        LimitSeverity::Low => theme::SIGNAL_GREEN,
        LimitSeverity::Medium => theme::SIGNAL_YELLOW,
        LimitSeverity::High => theme::SIGNAL_RED,
    }
}

/// Map a service-status indicator to its pill color.
fn indicator_color(indicator: StatusIndicator, cx: &gpui::App) -> Hsla {
    match indicator {
        StatusIndicator::None => theme::SIGNAL_GREEN,
        StatusIndicator::Minor => theme::SIGNAL_YELLOW,
        StatusIndicator::Major => theme::SIGNAL_ORANGE,
        StatusIndicator::Critical => theme::SIGNAL_RED,
        StatusIndicator::Unknown => theme::current(cx).text_muted,
    }
}

/// Local calendar day as `"YYYY-MM-DD"`, matching the date keys the
/// activity aggregator writes.
fn local_today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

// ---- Pure display logic (GPUI-free, unit-tested) ----

/// Cache-age bucket for the refresh badge. Pure so the bucket
/// boundaries are tested without touching i18n or the clock.
#[derive(Debug, PartialEq, Eq)]
enum CacheAge {
    /// No successful fetch yet.
    Never,
    /// Under a minute since the last refresh.
    JustNow,
    Minutes(u64),
    Hours(u64),
    Days(u64),
}

/// Bucket a "time since last fetch" into the badge's display unit.
/// `None` (never fetched) maps to [`CacheAge::Never`].
fn cache_age_bucket(age: Option<Duration>) -> CacheAge {
    let Some(age) = age else {
        return CacheAge::Never;
    };
    let secs = age.as_secs();
    if secs < 60 {
        CacheAge::JustNow
    } else if secs < 3_600 {
        CacheAge::Minutes(secs / 60)
    } else if secs < 86_400 {
        CacheAge::Hours(secs / 3_600)
    } else {
        CacheAge::Days(secs / 86_400)
    }
}

/// Compact count format matching the widget's `formatNumber`: ≥ 1M →
/// `"<x>.<y>M"`, ≥ 1000 → `"<x>.<y>K"`, otherwise the plain integer.
fn fmt_count(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Normalize message counts to pixel bar heights. The busiest day in
/// the window maps to `max_px`; every bar is floored at `min_px` so a
/// zero/low day still reads as a bar. An all-zero window uses a divisor
/// of 1 (every bar at `min_px`), mirroring the widget's `Math.max(…, 1)`.
fn chart_heights(messages: &[u64], max_px: f32, min_px: f32) -> Vec<f32> {
    let peak = messages.iter().copied().max().unwrap_or(0).max(1) as f32;
    messages
        .iter()
        .map(|&m| ((m as f32 / peak) * max_px).max(min_px))
        .collect()
}

/// Build the plan badge label from the Keychain plan metadata, mirroring
/// the widget's `getPlanBadge`: a 5x/20x multiplier suffix is shown only
/// for `max`/`team` tiers; known subscription types map to a fixed
/// uppercase name, others fall back to the uppercased raw value.
/// Returns `None` when there is no plan metadata at all (no badge).
fn plan_badge_label(plan: Option<&PlanInfo>) -> Option<String> {
    let plan = plan?;
    let sub = plan.tier.as_deref().unwrap_or("").to_lowercase();
    let tier = plan.qualifier.as_deref().unwrap_or("").to_lowercase();

    let base = match sub.as_str() {
        "team" => "TEAM",
        "enterprise" => "ENTERPRISE",
        "max" => "MAX",
        "pro" => "PRO",
        "free" => "FREE",
        "" => "CLAUDE",
        other => return Some(plan_badge_with_mult(&other.to_uppercase(), &sub, &tier)),
    };
    Some(plan_badge_with_mult(base, &sub, &tier))
}

/// Append the ` 5x` / ` 20x` multiplier to `base`, but only for
/// `max`/`team` subscriptions (where the rate-limit tier is meaningful).
fn plan_badge_with_mult(base: &str, sub: &str, tier: &str) -> String {
    let mult = if tier.contains("20x") {
        " 20x"
    } else if tier.contains("5x") {
        " 5x"
    } else {
        ""
    };
    if sub == "max" || sub == "team" {
        format!("{base}{mult}")
    } else {
        base.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_count_matches_widget() {
        assert_eq!(fmt_count(0), "0");
        assert_eq!(fmt_count(21), "21");
        assert_eq!(fmt_count(999), "999");
        assert_eq!(fmt_count(1_000), "1.0K");
        assert_eq!(fmt_count(5_012), "5.0K");
        assert_eq!(fmt_count(1_300), "1.3K");
        assert_eq!(fmt_count(248_200), "248.2K");
        assert_eq!(fmt_count(1_500_000), "1.5M");
    }

    #[test]
    fn chart_heights_normalize_to_max() {
        let h = chart_heights(&[10, 20, 0, 5], 44.0, 3.0);
        assert_eq!(h, vec![22.0, 44.0, 3.0, 11.0]);
    }

    #[test]
    fn chart_heights_all_zero_floor_to_min() {
        let h = chart_heights(&[0, 0, 0], 44.0, 3.0);
        assert_eq!(h, vec![3.0, 3.0, 3.0]);
    }

    #[test]
    fn chart_heights_empty_is_empty() {
        assert!(chart_heights(&[], 44.0, 3.0).is_empty());
    }

    fn plan(sub: Option<&str>, tier: Option<&str>) -> PlanInfo {
        PlanInfo {
            tier: sub.map(str::to_string),
            qualifier: tier.map(str::to_string),
        }
    }

    #[test]
    fn plan_badge_label_maps_tier() {
        assert_eq!(
            plan_badge_label(Some(&plan(Some("team"), Some("default_claude_ai_5x")))),
            Some("TEAM 5x".to_string())
        );
        assert_eq!(
            plan_badge_label(Some(&plan(Some("max"), Some("claude_20x")))),
            Some("MAX 20x".to_string())
        );
        // Multiplier is suppressed for non-max/team tiers.
        assert_eq!(
            plan_badge_label(Some(&plan(Some("pro"), Some("claude_5x")))),
            Some("PRO".to_string())
        );
        // team with no tier → no multiplier.
        assert_eq!(
            plan_badge_label(Some(&plan(Some("team"), None))),
            Some("TEAM".to_string())
        );
        // Unknown subscription falls back to uppercase.
        assert_eq!(
            plan_badge_label(Some(&plan(Some("scale"), None))),
            Some("SCALE".to_string())
        );
        // Empty subscription → generic brand.
        assert_eq!(
            plan_badge_label(Some(&plan(None, None))),
            Some("CLAUDE".to_string())
        );
        // No plan metadata → no badge.
        assert_eq!(plan_badge_label(None), None);
    }

    #[test]
    fn cache_age_bucket_boundaries() {
        assert_eq!(cache_age_bucket(None), CacheAge::Never);
        assert_eq!(
            cache_age_bucket(Some(Duration::from_secs(0))),
            CacheAge::JustNow
        );
        assert_eq!(
            cache_age_bucket(Some(Duration::from_secs(59))),
            CacheAge::JustNow
        );
        assert_eq!(
            cache_age_bucket(Some(Duration::from_secs(60))),
            CacheAge::Minutes(1)
        );
        assert_eq!(
            cache_age_bucket(Some(Duration::from_secs(59 * 60))),
            CacheAge::Minutes(59)
        );
        assert_eq!(
            cache_age_bucket(Some(Duration::from_secs(3_600))),
            CacheAge::Hours(1)
        );
        assert_eq!(
            cache_age_bucket(Some(Duration::from_secs(23 * 3_600))),
            CacheAge::Hours(23)
        );
        assert_eq!(
            cache_age_bucket(Some(Duration::from_secs(86_400))),
            CacheAge::Days(1)
        );
        assert_eq!(
            cache_age_bucket(Some(Duration::from_secs(3 * 86_400))),
            CacheAge::Days(3)
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
        cx.update(|cx| {
            assert_eq!(
                indicator_color(StatusIndicator::None, cx),
                theme::SIGNAL_GREEN
            );
            assert_eq!(
                indicator_color(StatusIndicator::Minor, cx),
                theme::SIGNAL_YELLOW
            );
            assert_eq!(
                indicator_color(StatusIndicator::Major, cx),
                theme::SIGNAL_ORANGE
            );
            assert_eq!(
                indicator_color(StatusIndicator::Critical, cx),
                theme::SIGNAL_RED
            );
            assert_eq!(
                indicator_color(StatusIndicator::Unknown, cx),
                theme::TEXT_TERTIARY
            );
        });
    }
}
