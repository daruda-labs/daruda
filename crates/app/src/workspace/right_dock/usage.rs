//! Usage tab body — a widget-style dashboard modelled on the Übersicht
//! `claude-usage` widget: a refresh header, then one block per signed-in
//! auth domain (name, plan badge, service pill, gauges, then that domain's
//! own 7-day turns chart and 7-day token-usage chart).
//!
//! Plan limits come from `RightDockSnapshot::usage` (limits pump);
//! activity from `RightDockSnapshot::activity` (local session-log
//! aggregation pump, one entry per domain). The ↻ badge dispatches
//! `Workspace::refresh_usage_now`. Static text comes from
//! `surface::strings::usage_*`, pixels/colors from `crate::ui::theme`.

use std::time::{Duration, SystemTime};

use crate::ui::theme;
use daruda_agent::activity::{ActivityStats, DayActivity};
use daruda_agent::service_status::{ServiceStatus, StatusIndicator};
use daruda_agent::{LimitSeverity, PlanInfo, ProviderUsage, UsageWindow};
use daruda_store::accounts::AccountRecipeId;
use gpui::{AnyElement, Context, Hsla, IntoElement, SharedString, WeakEntity, div, prelude::*, px};

use super::super::layout::Dock;
use super::super::layout::RightDockSnapshot;
use super::super::layout::snap::{RestorableSession, UsageSectionSnapshot};
use crate::surface::strings;
use crate::ui::{
    ButtonVariants as _, Disableable as _, GroupBoxVariants as _, SectionHeader, Sizable as _,
    button, group_box, tab, tab_bar,
};
use crate::workspace::Workspace;
use crate::workspace::main_area::pane::AccountDomain;

/// Render the Usage tab body: the refresh header, then the one section
/// relevant to the focused pane — a single agent-chat pane is always scoped
/// to exactly one domain, so stacking every signed-in domain regardless of
/// focus was clutter. `resolve_displayed_domain` picks which one; a domain
/// switcher only appears when focus doesn't already say which (a terminal
/// pane, or an agent daruda can't resolve a domain for) and there is more
/// than one to choose from.
pub(in crate::workspace) fn render(snap: &RightDockSnapshot, cx: &mut Context<Dock>) -> AnyElement {
    if snap.usage.is_empty() {
        return no_provider_body(cx);
    }
    let Some(displayed) = resolve_displayed_domain(
        snap.focused_agent_domain,
        snap.usage_domain_override,
        &snap.usage,
    ) else {
        return no_provider_body(cx);
    };
    // `Exactly(recipe)` names a domain unconditionally — only reachable here
    // when that domain isn't actually signed in (this pane's own domain has
    // no section). Another domain might still be signed in, so this names
    // the specific missing one instead of the generic "nobody is signed in
    // anywhere" notice.
    let Some(section) = snap.usage.iter().find(|s| s.recipe == displayed) else {
        return no_provider_body_for_domain(displayed, cx);
    };
    let activity = snap
        .activity
        .iter()
        .find(|(recipe, _)| *recipe == section.recipe)
        .map(|(_, stats)| stats);

    let body = crate::workspace::right_dock::right_panel_body()
        // One badge for the whole tab: the button refreshes every domain, so
        // clicking it isn't scoped to the visible section alone. But the age
        // it displays must be — only one section is on screen, so the badge
        // reports *its* freshness, not some other, hidden domain's.
        .child(usage_section_header(
            section.outcome.snapshot().and_then(|u| u.fetched_at),
            snap.usage_refresh_in_flight,
            &snap.workspace,
        ));
    let body = if show_domain_switcher(snap.focused_agent_domain, snap.usage.len()) {
        body.child(domain_switcher(&snap.usage, displayed, &snap.workspace))
    } else {
        body
    };
    let body = body.child(provider_section(section, activity, cx));
    let recent_sessions = snap
        .recent_sessions
        .iter()
        .find(|(recipe, _)| *recipe == displayed)
        .map(|(_, sessions)| sessions.as_slice());
    let body = match recent_sessions {
        Some(sessions) if !sessions.is_empty() => {
            body.child(recent_sessions_block(sessions, &snap.workspace, cx))
        }
        _ => body,
    };
    body.into_any_element()
}

/// Body when no provider is signed in: a single notice rather than gauges stuck
/// on a permanent placeholder.
fn no_provider_body(cx: &gpui::App) -> AnyElement {
    crate::workspace::right_dock::right_panel_body()
        .child(
            crate::ui::placeholder_text(strings::usage_no_provider())
                .text_size(px(theme::DOCK_PLACEHOLDER_FONT_SIZE))
                .text_color(theme::current(cx).text_subtle),
        )
        .into_any_element()
}

/// Body when the focused pane's own domain isn't signed in, even though
/// another domain might be — names `recipe` specifically rather than
/// [`no_provider_body`]'s generic notice, which would misleadingly claim
/// nobody is signed into anything.
fn no_provider_body_for_domain(recipe: AccountRecipeId, cx: &gpui::App) -> AnyElement {
    crate::workspace::right_dock::right_panel_body()
        .child(
            crate::ui::placeholder_text(strings::usage_no_domain_provider(recipe))
                .text_size(px(theme::DOCK_PLACEHOLDER_FONT_SIZE))
                .text_color(theme::current(cx).text_subtle),
        )
        .into_any_element()
}

// ----------------------------------------------------------------
// Domain switcher (ambiguous focus only)
// ----------------------------------------------------------------

/// Whether the domain switcher tab row should render: only while the
/// focused pane doesn't already name a domain exactly (a terminal pane, or
/// an agent daruda can't resolve a domain for), and only when there is more
/// than one signed-in domain to switch between — a lone section needs no
/// picker even in that state.
fn show_domain_switcher(pane_domain: AccountDomain, section_count: usize) -> bool {
    !matches!(pane_domain, AccountDomain::Exactly(_)) && section_count > 1
}

/// The switcher tab row: one tab per signed-in domain (never a hardcoded
/// pair), driving `Workspace::set_usage_domain_override` on click. Reused
/// `tab_bar`/`tab` widget, same as the right dock's own view switcher.
fn domain_switcher(
    sections: &[UsageSectionSnapshot],
    selected: AccountRecipeId,
    workspace: &WeakEntity<Workspace>,
) -> impl IntoElement {
    let recipes: Vec<AccountRecipeId> = sections.iter().map(|s| s.recipe).collect();
    let active_ix = recipes.iter().position(|r| *r == selected).unwrap_or(0);
    let workspace = workspace.clone();

    tab_bar("usage-domain-switcher")
        .w_full()
        .selected_index(active_ix)
        .children(
            recipes
                .iter()
                .map(|recipe| tab(strings::account_recipe_label(*recipe))),
        )
        .on_click(move |ix, _window, cx| {
            let Some(recipe) = recipes.get(*ix).copied() else {
                return;
            };
            if let Some(ws) = workspace.upgrade() {
                ws.update(cx, |ws, cx| ws.set_usage_domain_override(recipe, cx));
            }
        })
}

// ----------------------------------------------------------------
// Recent sessions (restore into a new pane)
// ----------------------------------------------------------------

/// Heading + one row per past session, restricted (by the snapshot layer)
/// to sessions matching the active Lane. Caller omits this block entirely
/// when the list is empty — no empty-frame placeholder, matching
/// `chart_block`'s "nothing yet" precedent.
fn recent_sessions_block(
    sessions: &[RestorableSession],
    workspace: &WeakEntity<Workspace>,
    cx: &gpui::App,
) -> AnyElement {
    sessions
        .iter()
        .fold(
            div()
                .flex()
                .flex_col()
                .gap(px(theme::RIGHT_PANEL_ROW_GAP))
                .child(SectionHeader::new(strings::usage_recent_sessions_section())),
            |block, session| block.child(recent_session_row(session, workspace, cx)),
        )
        .into_any_element()
}

/// One row: title/prompt preview/cwd fallback + compact session metadata +
/// a hover-revealed Restore button. No row-level click handler exists to
/// protect against, unlike `tasks.rs::task_row` — only the button triggers
/// anything, so no `stop_propagation` is needed.
fn recent_session_row(
    session: &RestorableSession,
    workspace: &WeakEntity<Workspace>,
    cx: &gpui::App,
) -> impl IntoElement {
    let t = theme::current(cx);
    let label = session_row_label(session);
    let meta_label = session_row_meta_label(session);
    let row_hover_bg = t.skill_row_hover_bg;
    let actions_bg = t.skill_row_hover_bg;
    let row_id = SharedString::from(format!("usage-session-row-{}", session.session_id));
    let restore_id = SharedString::from(format!("usage-restore-session-{}", session.session_id));
    let workspace = workspace.clone();
    let session = session.clone();

    div()
        .id(row_id)
        .group("usage-session-row")
        .relative()
        .flex()
        .flex_row()
        .items_center()
        .w_full()
        .min_w_0()
        .overflow_hidden()
        .gap(px(theme::RIGHT_PANEL_ROW_GAP))
        .px(px(theme::SKILL_ROW_PAD_X))
        .py(px(theme::SKILL_ROW_PAD_Y))
        .rounded(px(theme::SKILL_ROW_RADIUS))
        .hover(move |d| d.bg(row_hover_bg))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
                .text_color(t.text_body)
                .child(label),
        )
        .child(
            div()
                .flex_shrink()
                .max_w(gpui::relative(0.45))
                .min_w_0()
                .truncate()
                .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
                .text_color(t.text_muted)
                .child(meta_label),
        )
        .child(
            div()
                .absolute()
                .right(px(theme::SKILL_ROW_PAD_X))
                .top_0()
                .bottom_0()
                .flex()
                .items_center()
                .bg(actions_bg)
                .pl(px(theme::SKILL_ROW_PAD_X))
                .invisible()
                .group_hover("usage-session-row", |s| s.visible())
                .child(
                    button(restore_id, strings::usage_session_restore())
                        .ghost()
                        .xsmall()
                        .on_click(move |_, window, cx| {
                            if let Some(ws) = workspace.upgrade() {
                                ws.update(cx, |ws, cx| {
                                    ws.restore_session(session.clone(), window, cx)
                                });
                            }
                        }),
                ),
        )
}

/// One auth domain's block: header (icon + name + plan badge), account label,
/// service pill, gauge card per window, then (when this domain has any) its
/// own 7-day turns chart and 7-day token-usage chart.
fn provider_section(
    section: &crate::workspace::layout::snap::UsageSectionSnapshot,
    activity: Option<&ActivityStats>,
    cx: &gpui::App,
) -> impl IntoElement {
    let block = div()
        .flex()
        .flex_col()
        .w_full()
        .gap(px(theme::RIGHT_PANEL_ROW_GAP))
        .child(header(
            section.recipe,
            section.outcome.snapshot().and_then(|u| u.plan.as_ref()),
            section.account_label.clone(),
            cx,
        ))
        .child(status_pill(section.service_status.as_ref(), cx))
        .child(gauges_block(section.outcome.snapshot(), cx));

    let Some(activity) = activity else {
        return block;
    };
    block
        .child(chart_block(
            strings::usage_section_7day(),
            activity,
            |d| d.turns,
            cx,
        ))
        .child(chart_block(
            strings::usage_section_tokens(),
            activity,
            |d| d.tokens,
            cx,
        ))
}

// ----------------------------------------------------------------
// Header (logo + title + plan badge)
// ----------------------------------------------------------------

/// Account label sits on its own full-width row below the title row so a
/// long email can't push the plan badge off-screen — `w_full` lets it wrap
/// instead of being clipped by the fixed-width title row.
fn header(
    recipe: daruda_store::accounts::AccountRecipeId,
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
        .child(crate::ui::agent_icon(
            Some(crate::agent::icons::icon_for_recipe(recipe)),
            px(theme::USAGE_SECTION_ICON_SIZE),
            t.text_muted,
        ))
        .child(
            div()
                .flex_grow()
                .text_size(px(theme::USAGE_TITLE_FONT_SIZE))
                .text_color(t.text_muted)
                .child(SharedString::from(strings::account_recipe_label(recipe))),
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
// 7-day activity charts (turns, tokens)
// ----------------------------------------------------------------

/// `heading` + a bar chart over the most recent ≤7 days with activity,
/// weekday-labeled, today highlighted, sized by `value_of` (turns or
/// tokens — the two quantities the Usage tab charts, each its own instance
/// of this block). The aggregator stores only active days (ascending by
/// date), so zero days are dropped, not padded; today is matched by date
/// (not position). Heights normalize to the busiest day in the window.
fn chart_block(
    heading: String,
    activity: &ActivityStats,
    value_of: impl Fn(&DayActivity) -> u64,
    cx: &gpui::App,
) -> AnyElement {
    let today = chrono::Local::now().date_naive();
    let n = activity.daily.len();
    let recent = &activity.daily[n.saturating_sub(7)..];

    let block = div()
        .flex()
        .flex_col()
        .gap(px(theme::RIGHT_PANEL_ROW_GAP))
        .child(SectionHeader::new(heading));

    // Nothing aggregated yet — just the heading, no empty chart frame.
    if recent.is_empty() {
        return block.into_any_element();
    }

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

// ---- Pure display logic (GPUI-free, unit-tested) ----

/// Which domain's section the Usage tab shows this frame. `Exactly(recipe)`
/// always wins — a single agent-chat pane never has a second domain worth
/// showing. Otherwise (a terminal pane, or an agent daruda can't resolve a
/// domain for) the switcher's last manual pick wins if it still names a
/// signed-in domain; a pick for a domain that has since signed out (or was
/// never signed in) is silently ignored rather than tracked/cleared
/// separately, falling through to the first signed-in section.
fn resolve_displayed_domain(
    pane_domain: AccountDomain,
    override_: Option<AccountRecipeId>,
    sections: &[UsageSectionSnapshot],
) -> Option<AccountRecipeId> {
    if let AccountDomain::Exactly(recipe) = pane_domain {
        return Some(recipe);
    }
    override_
        .filter(|r| sections.iter().any(|s| s.recipe == *r))
        .or_else(|| sections.first().map(|s| s.recipe))
}

/// A recent-session row's label: the captured title, the latest prompt
/// preview, then the cwd's last path component, or the full cwd as a last
/// resort (an empty/root path has no file name).
fn session_row_label(session: &RestorableSession) -> SharedString {
    session
        .display_title()
        .map(SharedString::from)
        .unwrap_or_else(|| {
            session
                .cwd
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| session.cwd.to_string_lossy().into_owned())
                .into()
        })
}

fn session_row_meta_label(session: &RestorableSession) -> SharedString {
    let mut parts = Vec::new();
    if let Some(branch) = session
        .git_branch
        .as_ref()
        .map(|b| b.as_ref().trim())
        .filter(|b| !b.is_empty())
    {
        parts.push(branch.to_string());
    }
    parts.push(relative_time_label(session.last_active));
    SharedString::from(parts.join(" · "))
}

/// A recent-session row's "last active" label — same bucket boundaries as
/// the refresh badge's cache age, but with row-specific strings so the
/// refresh glyph stays off this passive timestamp.
fn relative_time_label(last_active: SystemTime) -> String {
    let age = SystemTime::now().duration_since(last_active).ok();
    match cache_age_bucket(age) {
        CacheAge::Never | CacheAge::JustNow => strings::usage_session_just_now(),
        CacheAge::Minutes(n) => strings::usage_session_minutes(n),
        CacheAge::Hours(n) => strings::usage_session_hours(n),
        CacheAge::Days(n) => strings::usage_session_days(n),
    }
}

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
    use daruda_agent::UsageOutcome;

    fn section(recipe: AccountRecipeId) -> UsageSectionSnapshot {
        UsageSectionSnapshot {
            recipe,
            account_label: "System".into(),
            outcome: UsageOutcome::Pending,
            service_status: None,
        }
    }

    #[test]
    fn resolve_displayed_domain_cases() {
        let sections = [section(AccountRecipeId::Codex)];
        assert_eq!(
            resolve_displayed_domain(
                AccountDomain::Exactly(AccountRecipeId::Claude),
                Some(AccountRecipeId::Codex),
                &sections,
            ),
            Some(AccountRecipeId::Claude),
            "auto mode always wins, even when it names a domain absent from `sections`"
        );

        let sections = [
            section(AccountRecipeId::Claude),
            section(AccountRecipeId::Codex),
        ];
        assert_eq!(
            resolve_displayed_domain(AccountDomain::Any, Some(AccountRecipeId::Codex), &sections),
            Some(AccountRecipeId::Codex)
        );

        // The override names a domain that has since signed out (absent from
        // `sections`) — it must not be tracked/cleared separately, just
        // ignored in favor of the default.
        let sections = [section(AccountRecipeId::Claude)];
        assert_eq!(
            resolve_displayed_domain(AccountDomain::Any, Some(AccountRecipeId::Codex), &sections),
            Some(AccountRecipeId::Claude)
        );

        let sections = [section(AccountRecipeId::Codex)];
        assert_eq!(
            resolve_displayed_domain(AccountDomain::Unsupported, None, &sections),
            Some(AccountRecipeId::Codex)
        );

        assert_eq!(
            resolve_displayed_domain(AccountDomain::Any, None, &[]),
            None
        );
    }

    fn restorable_session(
        title: Option<&str>,
        cwd: &str,
        last_active: SystemTime,
    ) -> RestorableSession {
        RestorableSession {
            session_id: "s1".to_string(),
            agent_id: "claude".to_string(),
            account: daruda_store::accounts::AccountSelection::SystemDefault,
            lane_ref: daruda_store::project::LaneRef::default(),
            title: title.map(SharedString::from),
            prompt_preview: None,
            git_branch: None,
            cwd: std::path::PathBuf::from(cwd),
            last_active,
        }
    }

    #[test]
    fn session_row_label_fallback_cases() {
        let mut session =
            restorable_session(Some("Fix the bug"), "/Users/x/proj", SystemTime::now());
        session.prompt_preview = Some(SharedString::from("Raw prompt"));
        assert_eq!(session_row_label(&session).as_ref(), "Fix the bug");

        let mut session = restorable_session(None, "/Users/x/proj", SystemTime::now());
        session.prompt_preview = Some(SharedString::from("Improve recent sessions"));
        assert_eq!(
            session_row_label(&session).as_ref(),
            "Improve recent sessions"
        );

        let session = restorable_session(None, "/Users/x/proj", SystemTime::now());
        assert_eq!(session_row_label(&session).as_ref(), "proj");

        let session = restorable_session(None, "/", SystemTime::now());
        assert_eq!(session_row_label(&session).as_ref(), "/");
    }

    #[test]
    fn session_row_meta_label_includes_branch_and_one_relative_time() {
        let last_active = std::time::UNIX_EPOCH + Duration::from_secs(3_600);
        let mut session = restorable_session(None, "/Users/x/proj", last_active);
        session.git_branch = Some(SharedString::from("main"));
        assert_eq!(
            session_row_meta_label(&session).as_ref(),
            format!("main · {}", relative_time_label(last_active))
        );
    }

    #[test]
    fn relative_time_label_buckets_like_the_refresh_badge_without_the_refresh_glyph() {
        let now = SystemTime::now();
        assert_eq!(relative_time_label(now), strings::usage_session_just_now());
        assert!(
            !relative_time_label(now).starts_with('\u{21bb}'),
            "recent-session timestamps must not show the refresh glyph"
        );
        assert_eq!(
            relative_time_label(now - Duration::from_secs(5 * 60)),
            strings::usage_session_minutes(5)
        );
        assert_eq!(
            relative_time_label(now - Duration::from_secs(3 * 3_600)),
            strings::usage_session_hours(3)
        );
        assert_eq!(
            relative_time_label(now - Duration::from_secs(2 * 86_400)),
            strings::usage_session_days(2)
        );
    }

    #[test]
    fn show_domain_switcher_cases() {
        assert!(!show_domain_switcher(
            AccountDomain::Exactly(AccountRecipeId::Claude),
            2
        ));
        assert!(!show_domain_switcher(AccountDomain::Any, 1));

        assert!(show_domain_switcher(AccountDomain::Any, 2));
        assert!(show_domain_switcher(AccountDomain::Unsupported, 2));
    }

    #[test]
    fn chart_heights_cases() {
        let h = chart_heights(&[10, 20, 0, 5], 44.0, 3.0);
        assert_eq!(h, vec![22.0, 44.0, 3.0, 11.0]);

        let h = chart_heights(&[0, 0, 0], 44.0, 3.0);
        assert_eq!(h, vec![3.0, 3.0, 3.0]);

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
