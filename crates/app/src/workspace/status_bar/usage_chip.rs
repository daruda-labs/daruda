//! Status bar's usage chip — a severity-coloured pill over the focused
//! account's plan-rate utilization, with a dropdown listing every rolling
//! window. Reads the cache `sync::limits` fills for the Usage tab and never
//! fetches, so an unfetched account has no chip.

use super::StatusBarDensity;
use crate::surface::strings;
use crate::ui::theme;
use crate::ui::{
    DropdownMenu as _, PopupMenu, PopupMenuItem, button_status_pill_bare, menu_builder, progress,
};
use crate::workspace::Workspace;
use crate::workspace::right_dock::usage::severity_color;
use crate::workspace::usage_labels::{percent, window_label};
use daruda_claude::{LimitSeverity, ProviderUsage, UsageWindow};
use daruda_store::accounts::AccountRecipeId;
use gpui::{AnyElement, App, Hsla, IntoElement, SharedString, WeakEntity, div, prelude::*, px};
use std::time::{Duration, SystemTime};

/// One plan-rate window projected for display — the parts the chip and
/// its dropdown render, resolved against a single `now` so every row in
/// one frame counts down from the same instant.
#[derive(Clone, Debug, PartialEq)]
struct WindowRow {
    /// Short window name (`"5h"` / `"7d"` / `"7d · Opus"` / `"1mo"`), shared
    /// with the Usage tab's gauge labels.
    label: String,
    /// `0.0 ..= 100.0`, clamped by the parser in `daruda_claude`.
    utilization: f32,
    /// Time until this window resets. `None` means the API omitted the
    /// reset time; a reset already in the past collapses to zero.
    resets_in: Option<Duration>,
}

/// Render one auth domain's usage chip. `None` when there are no numbers to
/// show — an empty chip would take status bar width to say nothing, and a
/// domain nobody is signed into should leave no trace.
pub(super) fn render(
    recipe: AccountRecipeId,
    outcome: &daruda_claude::UsageOutcome,
    density: StatusBarDensity,
    workspace: WeakEntity<Workspace>,
    cx: &App,
) -> Option<impl IntoElement> {
    let now = SystemTime::now();
    let usage = outcome.snapshot()?;
    let rows = rows(usage, now);
    let chip = window_row(usage.headline_window()?, now);
    let parts = chip_parts(&chip, density);
    let percent_color = row_color(&chip);
    let menu_rows = rows.clone();
    let t = theme::current(cx);
    // The brand mark is what tells two chips apart once the percentages are
    // all that's left of them at the narrowest density.
    let icon = crate::ui::agent_icon(
        Some(crate::agent::icons::icon_for_recipe(recipe)),
        px(theme::STATUS_BAR_AGENT_ICON_SIZE),
        t.text_muted,
    );
    let label = crate::surface::strings::account_recipe_label(recipe);
    Some(
        button_status_pill_bare(SharedString::from(format!("status-usage-{label}")), cx)
            .text_size(px(theme::STATUS_BAR_FONT_SIZE))
            .tooltip(SharedString::from(label))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(theme::STATUS_BAR_USAGE_CHIP_GAP))
                    // Only the percentage takes the severity colour; the icon,
                    // window name and countdown stay muted like every other
                    // status-bar label, so the colour reads as a gauge on one
                    // number instead of tinting the whole pill.
                    .child(icon)
                    .children(
                        parts
                            .window
                            .map(|window| div().child(SharedString::from(window))),
                    )
                    .child(
                        div()
                            .text_color(percent_color)
                            .child(SharedString::from(parts.percent)),
                    )
                    // No dropdown chevron: this is a reading, not a control,
                    // and the pill's hover state already says it is clickable.
                    .children(
                        parts
                            .reset
                            .map(|reset| div().child(SharedString::from(reset))),
                    )
                    // A refresh that failed keeps the last numbers on screen;
                    // this is what says they are no longer current.
                    .when(outcome.is_stale(), |row| {
                        row.child(
                            div()
                                .text_color(t.text_subtle)
                                .child(SharedString::from(strings::usage_stale_marker())),
                        )
                    }),
            )
            .dropdown_menu(menu_builder(move |menu, _window, _cx| {
                build_menu(recipe, &menu_rows, workspace.clone(), menu)
            })),
    )
}

/// The severity colour for a row's utilization — the same buckets and
/// palette the Usage tab's gauges use (`< 50` green, `< 80` yellow,
/// else red), read through the shared mapping so the two surfaces can't
/// drift apart.
fn row_color(row: &WindowRow) -> Hsla {
    severity_color(LimitSeverity::from_utilization(row.utilization))
}

/// Project the provider's windows into display rows, in the Usage tab's gauge
/// order (shortest first — `ProviderUsage` sorts them).
fn rows(usage: &ProviderUsage, now: SystemTime) -> Vec<WindowRow> {
    usage.windows.iter().map(|w| window_row(w, now)).collect()
}

fn window_row(window: &UsageWindow, now: SystemTime) -> WindowRow {
    WindowRow {
        label: window_label(window),
        utilization: window.utilization,
        resets_in: remaining(window, now),
    }
}

/// Time from `now` until `window` resets. A reset time that has already
/// passed yields `Duration::ZERO` (rendered as "now") rather than `None`
/// — `None` is reserved for the API not reporting one at all.
fn remaining(window: &UsageWindow, now: SystemTime) -> Option<Duration> {
    window
        .resets_at
        .map(|at| at.duration_since(now).unwrap_or(Duration::ZERO))
}

/// The chip's text, split at the boundaries where colour changes: the
/// percentage renders in its severity colour, the rest stays muted.
#[derive(Clone, Debug, PartialEq)]
struct ChipParts {
    /// Window name (`"5h"`) — dropped at reduced density.
    window: Option<String>,
    /// `"42%"` — the only part present at every tier, and the only one
    /// that carries colour.
    percent: String,
    /// `"· 2h 13m"` — dropped at reduced density, and when the API
    /// reported no reset time.
    reset: Option<String>,
}

/// `"5h" + "42%" + "· 2h 13m"` at `Full`; just `"42%"` at
/// `Compact`/`IconOnly` — as with the Ports chip, the number is the one
/// reading that survives every tier.
fn chip_parts(row: &WindowRow, density: StatusBarDensity) -> ChipParts {
    let percent = strings::status_bar_usage_chip_percent(percent(row.utilization));
    if density.is_reduced() {
        return ChipParts {
            window: None,
            percent,
            reset: None,
        };
    }
    ChipParts {
        window: Some(row.label.clone()),
        percent,
        reset: row.resets_in.map(|resets_in| {
            strings::status_bar_usage_chip_reset(&strings::format_reset_short(resets_in))
        }),
    }
}

/// Section header, one gauge row per window, then a jump to the Usage
/// tab. The gauge rows carry no click handler — there is nothing to act
/// on per window, and `PopupMenu` dismisses on any click regardless.
fn build_menu(
    recipe: AccountRecipeId,
    rows: &[WindowRow],
    workspace: WeakEntity<Workspace>,
    menu: PopupMenu,
) -> PopupMenu {
    // Named after the domain, not "PLAN USAGE": with a pill per provider the
    // dropdown has to say whose limits these are.
    let menu = menu.label(SharedString::from(strings::account_recipe_label(recipe)));
    let menu = rows.iter().fold(menu, |menu, row| {
        let row = row.clone();
        menu.item(PopupMenuItem::element(move |_window, cx| {
            gauge_row(&row, cx)
        }))
    });
    // Reveals the panel (opens the dock, then selects the tab) rather
    // than dispatching the tab-switch action, which does nothing when
    // the dock is closed or Usage is already the selected view.
    menu.separator().item(
        PopupMenuItem::new(SharedString::from(strings::status_bar_usage_open_panel())).on_click(
            move |_, _window, app| {
                if let Some(ws) = workspace.upgrade() {
                    ws.update(app, |ws, cx| {
                        ws.reveal_right_dock_view(daruda_store::project::RightDockView::Usage, cx)
                    });
                }
            },
        ),
    )
}

/// One dropdown row — header line, gauge bar, reset countdown: the Usage
/// tab's `gauge_card` shape at status-bar scale. The width is fixed
/// (`PopupMenu` sizes to content) so every bar shares one scale rather than
/// tracking its own label's length. `AnyElement` because
/// `PopupMenuItem::element`'s builder is `'static`.
fn gauge_row(row: &WindowRow, cx: &App) -> AnyElement {
    let t = theme::current(cx);
    let color = row_color(row);
    let caption = row.resets_in.map(strings::format_reset_countdown);
    div()
        .flex()
        .flex_col()
        .w(px(theme::STATUS_BAR_USAGE_ROW_WIDTH))
        .gap(px(theme::STATUS_BAR_USAGE_ROW_GAP))
        .child(
            div()
                .flex()
                .flex_row()
                .items_center()
                .justify_between()
                .child(div().child(SharedString::from(row.label.clone())))
                .child(div().text_color(color).child(SharedString::from(
                    strings::status_bar_usage_chip_percent(percent(row.utilization)),
                ))),
        )
        .child(
            progress(row.utilization)
                .bg(color)
                .h(px(theme::GAUGE_BAR_HEIGHT))
                .rounded(px(theme::GAUGE_BAR_RADIUS)),
        )
        .children(caption.map(|caption| {
            div()
                .text_color(t.text_subtle)
                .child(SharedString::from(caption))
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_claude::WindowScope;
    use daruda_store::accounts::AccountRecipeId;

    const FIVE_HOURS: u64 = 5 * 3600;
    const SEVEN_DAYS: u64 = 7 * 24 * 3600;
    const ONE_MONTH: u64 = 2_628_000;

    fn win(secs: u64, utilization: f32, resets_in_secs: Option<u64>) -> UsageWindow {
        UsageWindow {
            window: Duration::from_secs(secs),
            utilization,
            resets_at: resets_in_secs.map(|s| SystemTime::UNIX_EPOCH + Duration::from_secs(s)),
            scope: WindowScope::Overall,
        }
    }

    fn opus(secs: u64, utilization: f32) -> UsageWindow {
        UsageWindow {
            scope: WindowScope::Opus,
            ..win(secs, utilization, None)
        }
    }

    fn usage(windows: Vec<UsageWindow>) -> ProviderUsage {
        ProviderUsage::new(AccountRecipeId::Claude, windows, None)
    }

    /// `rows`/`remaining` take `now` explicitly so the tests pin it here
    /// instead of racing wall-clock.
    fn now() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_000)
    }

    fn row(label: &str, utilization: f32) -> WindowRow {
        WindowRow {
            label: label.to_string(),
            utilization,
            resets_in: None,
        }
    }

    #[test]
    fn rows_project_every_reported_window_shortest_first() {
        let usage = usage(vec![opus(SEVEN_DAYS, 20.0), win(FIVE_HOURS, 10.0, None)]);
        let rows = rows(&usage, now());
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].label, "5h");
        assert_eq!(rows[0].utilization, 10.0);
        assert_eq!(rows[1].label, "7d · Opus");
    }

    #[test]
    fn rows_are_empty_without_any_window() {
        assert!(rows(&usage(Vec::new()), now()).is_empty());
    }

    #[test]
    fn remaining_counts_down_from_the_given_now() {
        assert_eq!(
            remaining(&win(FIVE_HOURS, 0.0, Some(1_600)), now()),
            Some(Duration::from_secs(600))
        );
    }

    #[test]
    fn remaining_collapses_a_past_reset_to_zero() {
        // Distinct from `None` (the API omitted the field): the window is
        // resetting right now, so the chip says "now" rather than going
        // silent.
        assert_eq!(
            remaining(&win(FIVE_HOURS, 0.0, Some(500)), now()),
            Some(Duration::ZERO)
        );
        assert_eq!(remaining(&win(FIVE_HOURS, 0.0, None), now()), None);
    }

    /// The chip names one window; which one is `ProviderUsage::headline_window`
    /// (covered there). These cases pin the label the chip actually shows for
    /// the shapes each provider reports.
    #[test]
    fn the_chip_names_the_short_window_for_an_anthropic_plan() {
        let usage = usage(vec![
            win(FIVE_HOURS, 12.0, None),
            win(SEVEN_DAYS, 99.9, None),
        ]);
        let chip = window_row(usage.headline_window().unwrap(), now());
        assert_eq!(chip.label, "5h");
        assert_eq!(chip.utilization, 12.0);
    }

    #[test]
    fn the_chip_names_the_only_window_a_monthly_provider_reports() {
        let usage = usage(vec![win(ONE_MONTH, 2.0, None)]);
        let chip = window_row(usage.headline_window().unwrap(), now());
        assert_eq!(chip.label, "1mo");
        assert_eq!(chip.utilization, 2.0);
    }

    #[test]
    fn chip_parts_carry_window_and_countdown_at_full_density() {
        let row = WindowRow {
            label: "5h".to_string(),
            utilization: 42.0,
            resets_in: Some(Duration::from_secs(2 * 3600 + 13 * 60)),
        };
        let parts = chip_parts(&row, StatusBarDensity::Full);
        assert_eq!(parts.window.as_deref(), Some("5h"));
        assert_eq!(parts.percent, "42%");
        assert_eq!(parts.reset.as_deref(), Some("· 2h 13m"));
    }

    #[test]
    fn chip_parts_keep_only_the_percent_when_reduced() {
        let row = WindowRow {
            label: "7d".to_string(),
            utilization: 42.0,
            resets_in: Some(Duration::from_secs(3600)),
        };
        for density in [StatusBarDensity::Compact, StatusBarDensity::IconOnly] {
            let parts = chip_parts(&row, density);
            assert_eq!(parts.percent, "42%");
            assert_eq!(parts.window, None);
            assert_eq!(parts.reset, None);
        }
    }

    #[test]
    fn chip_parts_omit_the_countdown_when_the_api_reported_none() {
        let parts = chip_parts(&row("5h", 42.0), StatusBarDensity::Full);
        assert_eq!(parts.window.as_deref(), Some("5h"));
        assert_eq!(parts.percent, "42%");
        assert_eq!(parts.reset, None);
    }

    /// The percentage is the only span that gets colour, and it buckets
    /// exactly like the Usage tab's gauges: green under 50, yellow from
    /// 50, red from 80.
    #[gpui::test]
    fn percent_color_buckets_match_the_usage_gauges(cx: &mut gpui::TestAppContext) {
        crate::test_support::init_gpui_component(cx);
        cx.update(|_cx| {
            assert_eq!(row_color(&row("5h", 49.9)), theme::SIGNAL_GREEN);
            assert_eq!(row_color(&row("5h", 50.0)), theme::SIGNAL_YELLOW);
            assert_eq!(row_color(&row("5h", 79.9)), theme::SIGNAL_YELLOW);
            assert_eq!(row_color(&row("5h", 80.0)), theme::SIGNAL_RED);
            assert_eq!(row_color(&row("5h", 100.0)), theme::SIGNAL_RED);
        });
    }
}
