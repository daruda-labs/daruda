//! Status bar's Claude usage chip — the focused pane's account plan-rate
//! utilization, shown as a severity-coloured pill with a dropdown listing
//! every rolling window. Reads the cache `sync::limits` already fills for
//! the Usage tab (`ClaudeContext::usage_by_account`); no fetch of its own,
//! so an account whose usage has never been fetched simply has no chip.
//! Claude-only: `daruda_claude::limits` is the sole rate-limit backend, so
//! a Codex-account pane shows nothing here.

use super::StatusBarDensity;
use crate::surface::strings;
use crate::ui::theme;
use crate::ui::{
    DropdownMenu as _, PopupMenu, PopupMenuItem, button_status_pill_bare, menu_builder, progress,
};
use crate::workspace::Workspace;
use crate::workspace::right_dock::usage::severity_color;
use daruda_claude::{LimitSeverity, LimitWindow, PlanLimits};
use gpui::{AnyElement, App, Hsla, IntoElement, SharedString, WeakEntity, div, prelude::*, px};
use std::time::{Duration, SystemTime};

/// One plan-rate window projected for display — the parts the chip and
/// its dropdown render, resolved against a single `now` so every row in
/// one frame counts down from the same instant.
#[derive(Clone, Debug, PartialEq)]
struct WindowRow {
    /// Short window name (`"5h"` / `"7d"` / `"7d · Opus"`), shared with
    /// the Usage tab's gauge labels.
    label: String,
    /// `0.0 ..= 100.0`, clamped by the parser in `daruda_claude`.
    utilization: f32,
    /// Time until this window resets. `None` means the API omitted the
    /// reset time; a reset already in the past collapses to zero.
    resets_in: Option<Duration>,
}

/// Render the usage chip. `None` when the focused account's cache holds
/// no window at all (never fetched, fetch failed, or a Codex account) —
/// an empty chip would take status bar width to say nothing.
pub(super) fn render(
    limits: &PlanLimits,
    density: StatusBarDensity,
    workspace: WeakEntity<Workspace>,
    cx: &App,
) -> Option<impl IntoElement> {
    let rows = rows(limits, SystemTime::now());
    let binding = binding(&rows)?;
    let parts = chip_parts(binding, density);
    let percent_color = row_color(binding);
    let menu_rows = rows.clone();
    Some(
        button_status_pill_bare("status-claude-usage", cx)
            .text_size(px(theme::STATUS_BAR_FONT_SIZE))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(theme::STATUS_BAR_USAGE_CHIP_GAP))
                    // Only the percentage takes the severity colour; the
                    // window name and countdown stay muted like every other
                    // status-bar label, so the colour reads as a gauge on one
                    // number instead of tinting the whole pill.
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
                    .children(
                        parts
                            .reset
                            .map(|reset| div().child(SharedString::from(reset))),
                    )
                    .child(
                        div().child(SharedString::from(strings::TASK_PILL_CHEVRON.trim_start())),
                    ),
            )
            .dropdown_menu(menu_builder(move |menu, _window, _cx| {
                build_menu(&menu_rows, workspace.clone(), menu)
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

/// Project the plan's three optional windows into display rows, in the
/// Usage tab's gauge order, skipping the ones the plan doesn't meter.
fn rows(limits: &PlanLimits, now: SystemTime) -> Vec<WindowRow> {
    [
        (strings::usage_limit_5h_label(), limits.five_hour.as_ref()),
        (strings::usage_limit_7d_label(), limits.seven_day.as_ref()),
        (
            strings::usage_limit_opus_label(),
            limits.seven_day_opus.as_ref(),
        ),
    ]
    .into_iter()
    .filter_map(|(label, window)| {
        window.map(|window| WindowRow {
            label,
            utilization: window.utilization,
            resets_in: remaining(window, now),
        })
    })
    .collect()
}

/// Time from `now` until `window` resets. A reset time that has already
/// passed yields `Duration::ZERO` (rendered as "now") rather than `None`
/// — `None` is reserved for the API not reporting one at all.
fn remaining(window: &LimitWindow, now: SystemTime) -> Option<Duration> {
    window
        .resets_at
        .map(|at| at.duration_since(now).unwrap_or(Duration::ZERO))
}

/// The window that binds first — the highest utilization, not a fixed
/// one. A chip pinned to the 5-hour window would read a calm green while
/// the weekly budget is one prompt away from throttling. Ties keep the
/// earlier (shorter) window, which is the one that recovers first.
fn binding(rows: &[WindowRow]) -> Option<&WindowRow> {
    rows.iter().reduce(|best, row| {
        if row.utilization > best.utilization {
            row
        } else {
            best
        }
    })
}

/// Utilization as the whole percent the chip and rows display.
fn percent(utilization: f32) -> u32 {
    utilization.clamp(0.0, 100.0).round() as u32
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
fn build_menu(rows: &[WindowRow], workspace: WeakEntity<Workspace>, menu: PopupMenu) -> PopupMenu {
    let menu = menu.label(SharedString::from(strings::usage_limits_section_label()));
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

/// One dropdown row: window name + severity-coloured percent on a header
/// line, the filled gauge bar under it, then the reset countdown. Same
/// three-part shape as the Usage tab's `gauge_card`, at status-bar scale.
/// The width is fixed (`PopupMenu` sizes to content) so every bar shares
/// one scale instead of tracking its own label's length. Returns
/// `AnyElement` because `PopupMenuItem::element`'s builder is `'static`
/// and an `impl IntoElement` here would capture `cx`'s lifetime.
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

    fn window(utilization: f32, resets_in_secs: Option<u64>) -> LimitWindow {
        LimitWindow {
            utilization,
            resets_at: resets_in_secs.map(|s| SystemTime::UNIX_EPOCH + Duration::from_secs(s)),
        }
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
    fn rows_skip_windows_the_plan_does_not_meter() {
        let limits = PlanLimits {
            five_hour: Some(window(10.0, None)),
            seven_day: None,
            seven_day_opus: Some(window(20.0, None)),
            ..PlanLimits::default()
        };
        let rows = rows(&limits, now());
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].utilization, 10.0);
        assert_eq!(rows[1].utilization, 20.0);
    }

    #[test]
    fn rows_are_empty_without_any_window() {
        assert!(rows(&PlanLimits::default(), now()).is_empty());
        assert!(binding(&[]).is_none());
    }

    #[test]
    fn remaining_counts_down_from_the_given_now() {
        let win = window(0.0, Some(1_600));
        assert_eq!(remaining(&win, now()), Some(Duration::from_secs(600)));
    }

    #[test]
    fn remaining_collapses_a_past_reset_to_zero() {
        // Distinct from `None` (the API omitted the field): the window is
        // resetting right now, so the chip says "now" rather than going
        // silent.
        let win = window(0.0, Some(500));
        assert_eq!(remaining(&win, now()), Some(Duration::ZERO));
        assert_eq!(remaining(&window(0.0, None), now()), None);
    }

    #[test]
    fn binding_picks_the_highest_utilization_not_the_first() {
        let rows = vec![row("5h", 12.0), row("7d", 91.0), row("7d · Opus", 40.0)];
        assert_eq!(binding(&rows).unwrap().label, "7d");
    }

    #[test]
    fn binding_breaks_a_tie_toward_the_shorter_window() {
        let rows = vec![row("5h", 50.0), row("7d", 50.0)];
        assert_eq!(binding(&rows).unwrap().label, "5h");
    }

    #[test]
    fn percent_rounds_to_a_whole_number() {
        assert_eq!(percent(41.6), 42);
        assert_eq!(percent(0.0), 0);
        assert_eq!(percent(100.0), 100);
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
