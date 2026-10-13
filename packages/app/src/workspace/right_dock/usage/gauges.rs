//! Plan-limit gauge cards and the 7-day activity charts of the Usage tab.

use std::time::SystemTime;

use crate::ui::theme;
use daruda_agent::activity::{ActivityStats, DayActivity};
use daruda_agent::{LimitSeverity, ProviderUsage, UsageWindow};
use gpui::{AnyElement, Hsla, IntoElement, SharedString, div, prelude::*, px};

use super::super::super::layout::RightDockSnapshot;
use super::super::section::DockSection;
use super::super::section_view::{ScopeSection, SectionFold};
use super::severity_color;
use crate::surface::strings;
use crate::ui::{GroupBoxVariants as _, group_box};

// ----------------------------------------------------------------
// Plan-limit gauge cards
// ----------------------------------------------------------------

/// One gauge card per window the provider reported, shortest first
/// (`ProviderUsage` sorts them). Before the first fetch lands — or when the
/// provider metered nothing — a single placeholder card holds the space, since
/// which windows exist is now the provider's answer rather than a fixed set.
pub(super) fn gauges_block(
    usage: Option<&ProviderUsage>,
    stale: bool,
    cx: &gpui::App,
) -> impl IntoElement {
    let windows = usage.map(|u| u.windows.as_slice()).unwrap_or_default();
    let col = div().flex().flex_col().gap(px(theme::USAGE_CARD_GAP));
    if windows.is_empty() {
        return col.child(gauge_card(
            strings::usage::limit_unavailable(),
            None,
            stale,
            cx,
        ));
    }
    windows.iter().fold(col, |col, window| {
        col.child(gauge_card(
            crate::workspace::usage_labels::window_label(window),
            Some(window),
            stale,
            cx,
        ))
    })
}

/// One gauge card: header row (label + big %), bar, optional reset text.
/// `None` window → placeholder (dim bar + "Unavailable", no %).
fn gauge_card(
    label: impl Into<SharedString>,
    window: Option<&UsageWindow>,
    stale: bool,
    cx: &gpui::App,
) -> AnyElement {
    let t = theme::current(cx);
    let label: SharedString = label.into();

    // Border-only card (no fill), `theme.radius` corners.
    let card = group_box().outline();

    let Some(win) = window else {
        // Placeholder: label + dim bar + "Unavailable".
        return card
            .child(gauge_header_row(
                label,
                None,
                stale,
                t.text_muted,
                t.text_subtle,
            ))
            .child(gauge_bar(0.0, t.gauge_track_bg))
            .into_any_element();
    };

    let pct = win.utilization;
    let color = severity_color(LimitSeverity::from_utilization(pct));
    let reset_text = win
        .resets_at
        .and_then(|at| at.duration_since(SystemTime::now()).ok())
        .map(strings::usage::format_reset_countdown);

    let mut card = card
        .child(gauge_header_row(
            label,
            Some(pct),
            stale,
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
    stale: bool,
    label_color: Hsla,
    pct_color: Hsla,
) -> impl IntoElement {
    let value = SharedString::from(gauge_value_text(pct, stale));
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

pub(super) fn gauge_value_text(pct: Option<f32>, stale: bool) -> String {
    let Some(pct) = pct else {
        return strings::usage::limit_unavailable();
    };
    let value = format!("{:.0}%", pct.clamp(0.0, 100.0));
    if stale {
        format!("{value} {}", strings::usage::stale_marker())
    } else {
        value
    }
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
// 7-day activity charts (turns, tokens)
// ----------------------------------------------------------------

/// `heading` + a bar chart over the most recent ≤7 days with activity,
/// weekday-labeled, today highlighted, sized by `value_of` (turns or
/// tokens — the two quantities the Usage tab charts, each its own instance
/// of this block). The aggregator stores only active days (ascending by
/// date), so zero days are dropped, not padded; today is matched by date
/// (not position). Heights normalize to the busiest day in the window.
pub(super) fn chart_block(
    section: DockSection,
    heading: String,
    activity: &ActivityStats,
    value_of: impl Fn(&DayActivity) -> u64,
    snap: &RightDockSnapshot,
    cx: &gpui::App,
) -> AnyElement {
    let n = activity.daily.len();
    let recent = &activity.daily[n.saturating_sub(7)..];
    // Nothing aggregated yet — just the heading, no empty chart frame, and
    // nothing to fold.
    let fold = if recent.is_empty() {
        SectionFold::Fixed
    } else {
        SectionFold::toggleable(snap.sections.is_open(section))
    };
    let chart = (fold.is_open() && !recent.is_empty()).then(|| chart_row(recent, value_of, cx));
    ScopeSection {
        section,
        label: heading.into(),
        count: None,
        fold,
        divided: true,
    }
    .render(chart, &snap.workspace, cx)
    .into_any_element()
}

fn chart_row(
    recent: &[DayActivity],
    value_of: impl Fn(&DayActivity) -> u64,
    cx: &gpui::App,
) -> AnyElement {
    let today = chrono::Local::now().date_naive();

    let values: Vec<u64> = recent.iter().map(&value_of).collect();
    let heights = chart_heights(
        &values,
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

    row.into_any_element()
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
        .map(|d| strings::usage::weekday_label(d.weekday().num_days_from_sunday() as u8))
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

/// Normalize message counts to pixel bar heights. The busiest day in
/// the window maps to `max_px`; every bar is floored at `min_px` so a
/// zero/low day still reads as a bar. An all-zero window uses a divisor
/// of 1 (every bar at `min_px`), mirroring the widget's `Math.max(…, 1)`.
pub(super) fn chart_heights(messages: &[u64], max_px: f32, min_px: f32) -> Vec<f32> {
    let peak = messages.iter().copied().max().unwrap_or(0).max(1) as f32;
    messages
        .iter()
        .map(|&m| ((m as f32 / peak) * max_px).max(min_px))
        .collect()
}
