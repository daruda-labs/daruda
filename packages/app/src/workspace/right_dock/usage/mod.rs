//! Usage tab body — a widget-style dashboard modelled on the Übersicht
//! `claude-usage` widget: a refresh header, then one block per signed-in
//! auth domain (name, plan badge, service pill, gauges, then that domain's
//! own 7-day turns chart and 7-day token-usage chart).
//!
//! Plan limits come from `RightDockSnapshot::usage` (limits pump);
//! activity from `RightDockSnapshot::activity` (local session-log
//! aggregation pump, one entry per domain). The ↻ badge dispatches
//! `Workspace::refresh_usage_now`. Static text comes from
//! `surface::strings::usage::*`, pixels/colors from `crate::ui::theme`.

use std::time::{Duration, SystemTime};

use crate::ui::theme;
use daruda_agent::activity::ActivityStats;
use daruda_agent::service_status::{ServiceStatus, StatusIndicator};
use daruda_agent::{LimitSeverity, PlanInfo};
use daruda_core::age::{self, AgeBucket};
use daruda_store::accounts::AccountRecipeId;
use gpui::{AnyElement, Context, Hsla, IntoElement, SharedString, WeakEntity, div, prelude::*, px};

use super::super::layout::Dock;
use super::super::layout::RightDockSnapshot;
use super::super::layout::snap::{RestorableSession, UsageSectionSnapshot};
use super::section::DockSection;
use super::section_view::{ScopeSection, SectionFold, hover_actions, library_row, panel_footer};
use crate::surface::strings;
use crate::ui::{Disableable as _, SectionHeader, tab, tab_bar};
use crate::workspace::Workspace;
use crate::workspace::main_area::pane::AccountDomain;
use crate::workspace::usage_settings_ops::offers_overage_stop;

mod gauges;
use gauges::{chart_block, gauges_block};

/// Render the Usage tab body: the refresh header, then the one section
/// relevant to the focused pane — a single agent-chat pane is always scoped
/// to exactly one domain, so stacking every signed-in domain regardless of
/// focus was clutter. `resolve_displayed_domain` picks which one; a domain
/// switcher only appears when focus doesn't already say which (a terminal
/// pane, or an agent daruda can't resolve a domain for) and there is more
/// than one to choose from.
pub(super) fn render(snap: &RightDockSnapshot, cx: &mut Context<Dock>) -> AnyElement {
    let Some(displayed) = displayed_domain(snap) else {
        return no_provider_body(cx);
    };
    // `Exactly(recipe)` names a domain unconditionally — only reachable here
    // when that domain isn't actually signed in (this pane's own domain has
    // no section). Another domain might still be signed in, so this names
    // the specific missing one instead of the generic "nobody is signed in
    // anywhere" notice.
    let Some(section) = displayed_section(snap, displayed) else {
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
            section.outcome.is_stale(),
            snap.usage_refresh_in_flight,
            &snap.workspace,
            cx,
        ));
    let body = if show_domain_switcher(snap.focused_agent_domain, snap.usage.len()) {
        body.child(domain_switcher(&snap.usage, displayed, &snap.workspace))
    } else {
        body
    };
    let body = body.child(provider_section(section, activity, snap, cx));
    let recent_sessions = snap
        .recent_sessions
        .iter()
        .find(|(recipe, _)| *recipe == displayed)
        .map(|(_, sessions)| sessions.as_slice());
    let body = match recent_sessions {
        Some(sessions) if !sessions.is_empty() => body.child(recent_sessions_block(
            sessions,
            snap.sections.is_open(DockSection::UsageRecentSessions),
            &snap.workspace,
            cx,
        )),
        _ => body,
    };
    body.into_any_element()
}

fn displayed_domain(snap: &RightDockSnapshot) -> Option<AccountRecipeId> {
    if snap.usage.is_empty() {
        return None;
    }
    resolve_displayed_domain(
        snap.focused_agent_domain,
        snap.usage_domain_override,
        &snap.usage,
    )
}

fn displayed_section(
    snap: &RightDockSnapshot,
    displayed: AccountRecipeId,
) -> Option<&UsageSectionSnapshot> {
    snap.usage.iter().find(|s| s.recipe == displayed)
}

/// Freshness of the one section on screen, pinned under the body.
pub(super) fn footer(snap: &RightDockSnapshot, cx: &gpui::App) -> Option<AnyElement> {
    let section = displayed_section(snap, displayed_domain(snap)?)?;
    let fetched_at = section.outcome.snapshot().and_then(|u| u.fetched_at);
    // Before the first fetch there is no freshness to report.
    if fetched_at.is_none() && !snap.usage_refresh_in_flight {
        return None;
    }
    let label = refresh_badge_label(
        fetched_at,
        section.outcome.is_stale(),
        snap.usage_refresh_in_flight,
    );
    Some(panel_footer(crate::ui::icons::REFRESH, label, cx))
}

/// Body when no provider is signed in: a single notice rather than gauges stuck
/// on a permanent placeholder.
fn no_provider_body(cx: &gpui::App) -> AnyElement {
    crate::workspace::right_dock::right_panel_body()
        .child(SectionHeader::new(strings::dock::right_tab_usage()).prominent())
        .child(
            crate::ui::placeholder_text(strings::usage::no_provider())
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
        .child(SectionHeader::new(strings::dock::right_tab_usage()).prominent())
        .child(
            crate::ui::placeholder_text(strings::usage::no_domain_provider(recipe))
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
                .map(|recipe| tab(daruda_settings::labels::account_recipe_label(*recipe))),
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
    is_open: bool,
    workspace: &WeakEntity<Workspace>,
    cx: &gpui::App,
) -> AnyElement {
    let rows = is_open.then(|| {
        sessions
            .iter()
            .fold(div().flex().flex_col(), |block, session| {
                block.child(recent_session_row(session, workspace, cx))
            })
            .into_any_element()
    });
    ScopeSection {
        section: DockSection::UsageRecentSessions,
        label: strings::usage::recent_sessions_section().into(),
        count: Some(sessions.len().to_string().into()),
        fold: SectionFold::toggleable(is_open),
        divided: true,
    }
    .render(rows, workspace, cx)
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
    let row_id = SharedString::from(format!("usage-session-row-{}", session.session_id));
    let restore_id = SharedString::from(format!("usage-restore-session-{}", session.session_id));
    let workspace = workspace.clone();
    let session = session.clone();

    library_row(
        crate::ui::icons::SESSION,
        label,
        Some(meta_label.into_any_element()),
        cx,
    )
    .id(row_id)
    .group("usage-session-row")
    .relative()
    .overflow_hidden()
    .px(px(theme::LIST_ROW_PAD_X))
    .rounded(px(theme::LIST_ROW_RADIUS))
    .hover(move |d| d.bg(row_hover_bg))
    .child(
        hover_actions("usage-session-row", theme::LIST_ROW_PAD_X, cx).child(
            crate::ui::button_icon(restore_id, crate::ui::icons::HISTORY, cx)
                .tooltip(strings::usage::session_restore())
                .debug_selector(|| "usage-session-restore".into())
                .on_click(move |_, window, cx| {
                    if let Some(ws) = workspace.upgrade() {
                        ws.update(cx, |ws, cx| ws.restore_session(session.clone(), window, cx));
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
    snap: &RightDockSnapshot,
    cx: &gpui::App,
) -> impl IntoElement {
    let stale = section.outcome.is_stale();
    let block = div()
        .flex()
        .flex_col()
        .w_full()
        .gap(px(theme::USAGE_BLOCK_GAP))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(theme::RIGHT_PANEL_ROW_GAP))
                .child(header(
                    section.recipe,
                    section.outcome.snapshot().and_then(|u| u.plan.as_ref()),
                    section.account_label.clone(),
                    cx,
                ))
                .child(status_pill(section.service_status.as_ref(), cx)),
        )
        .child(gauges_block(section.outcome.snapshot(), stale, cx));
    let block = if offers_overage_stop(section.recipe) {
        block.child(stop_on_overage_row(
            snap.workspace.clone(),
            snap.stop_on_overage,
            cx,
        ))
    } else {
        block
    };

    let Some(activity) = activity else {
        return block;
    };
    block.child(
        div()
            .flex()
            .flex_col()
            .gap(px(theme::RIGHT_PANEL_SECTION_GAP))
            .child(chart_block(
                DockSection::UsageTurns,
                strings::usage::section_7day(),
                activity,
                |d| d.turns,
                snap,
                cx,
            ))
            .child(chart_block(
                DockSection::UsageTokens,
                strings::usage::section_tokens(),
                activity,
                |d| d.tokens,
                snap,
                cx,
            )),
    )
}

/// Switch + caption for `[usage] stop_on_overage`. The label column takes
/// the slack and wraps so the switch never leaves the dock.
fn stop_on_overage_row(
    workspace: WeakEntity<Workspace>,
    stop_on_overage: bool,
    cx: &gpui::App,
) -> impl IntoElement {
    let t = theme::current(cx);
    div()
        .flex()
        .flex_row()
        .items_start()
        .w_full()
        .min_w_0()
        .gap(px(theme::RIGHT_PANEL_ROW_GAP))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
                        .text_color(t.text_body)
                        .child(daruda_settings::labels::stop_on_overage_label()),
                )
                .child(
                    div()
                        .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
                        .text_color(t.text_muted)
                        .child(strings::usage::stop_on_overage_caption()),
                ),
        )
        .child(div().flex_none().child(
            crate::ui::switch_compact("usage-stop-on-overage", stop_on_overage, cx).on_click(
                move |_, _window, cx| {
                    if let Some(ws) = workspace.upgrade() {
                        ws.update(cx, |ws, cx| ws.set_stop_on_overage(!stop_on_overage, cx));
                    }
                },
            ),
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
            Some(daruda_config::agent::icon::icon_for_recipe(recipe)),
            px(theme::USAGE_SECTION_ICON_SIZE),
            t.text_body,
        ))
        .child(
            div()
                .flex_grow()
                .text_size(px(theme::USAGE_TITLE_FONT_SIZE))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(t.text_primary)
                .child(SharedString::from(
                    daruda_settings::labels::account_recipe_label(recipe),
                )),
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
/// dropdown.
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

/// Panel title with an icon-only refresh; the cache age is its tooltip.
/// Clicking dispatches `Workspace::refresh_usage_now`.
fn usage_section_header(
    fetched_at: Option<SystemTime>,
    stale: bool,
    in_flight: bool,
    workspace: &gpui::WeakEntity<crate::workspace::Workspace>,
    cx: &gpui::App,
) -> impl IntoElement {
    let label = refresh_badge_label(fetched_at, stale, in_flight);
    let workspace = workspace.clone();

    SectionHeader::new(strings::dock::right_tab_usage())
        .prominent()
        .truncate_label(true)
        .actions(
            // Ghost button: subtle text affordance with hover/press
            // feedback. Disabled while a refresh is in flight so a
            // double-click can't fan out a second fetch.
            crate::ui::button_icon("usage-refresh-badge", crate::ui::icons::REFRESH, cx)
                .tooltip(label)
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
fn refresh_badge_label(fetched_at: Option<SystemTime>, stale: bool, in_flight: bool) -> String {
    if in_flight {
        return strings::usage::refreshing();
    }
    let age = fetched_at.and_then(|t| SystemTime::now().duration_since(t).ok());
    let label = match cache_age_bucket(age) {
        None => strings::usage::refresh(),
        Some(AgeBucket::JustNow) => strings::usage::cache_just_now(),
        Some(AgeBucket::Minutes(n)) => strings::usage::cache_minutes(n),
        Some(AgeBucket::Hours(n)) => strings::usage::cache_hours(n),
        Some(AgeBucket::Days(n)) => strings::usage::cache_days(n),
    };
    if stale {
        format!("{label} {}", strings::usage::stale_marker())
    } else {
        label
    }
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
    let label = strings::status::service_status_label(status);
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
        None | Some(AgeBucket::JustNow) => strings::usage::session_just_now(),
        Some(AgeBucket::Minutes(n)) => strings::usage::session_minutes(n),
        Some(AgeBucket::Hours(n)) => strings::usage::session_hours(n),
        Some(AgeBucket::Days(n)) => strings::usage::session_days(n),
    }
}

/// Bucket a "time since" into its display unit; `None` is never fetched.
fn cache_age_bucket(age: Option<Duration>) -> Option<AgeBucket> {
    age.map(|age| age::bucket(age.as_secs()))
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
mod tests;
