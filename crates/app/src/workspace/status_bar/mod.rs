//! Bottom status bar — displays project/branch context, focused pane
//! title, the account slot, the Ports segment, and the Claude usage
//! chip. Always visible at the bottom of the workspace. Split into
//! submodules per segment (`account_slot`, `ports_segment`,
//! `usage_chip`) since a single file grew multiple independent
//! responsibilities (G1).

mod account_slot;
mod context_menu;
mod ports_segment;
mod usage_chip;

pub(in crate::workspace) use account_slot::{AccountSlot, account_label};

use crate::ui::ContextMenuExt as _;
use crate::ui::theme;
use crate::workspace::Workspace;
use crate::workspace::sync::ports::{PortEntry, PortScanStatus};
use daruda_config::StatusBarItem;
use gpui::{App, IntoElement, RenderOnce, SharedString, WeakEntity, Window, div, prelude::*, px};

/// Fixed height of the status bar in pixels.
pub(super) const STATUS_BAR_HEIGHT: f32 = theme::STATUS_BAR_HEIGHT;
const PROJECT_BRANCH_MAX_WIDTH: f32 = 220.0;
const PROJECT_BRANCH_REDUCED_MAX_WIDTH: f32 = 150.0;
const STATUS_BAR_ERROR_MAX_WIDTH: f32 = 240.0;
const STATUS_BAR_ERROR_REDUCED_MAX_WIDTH: f32 = 140.0;

/// Responsive tier derived from the window's current width. The status
/// bar has no horizontal scroll, so as the window narrows each tier
/// sheds label text rather than letting content wrap or clip.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum StatusBarDensity {
    /// Full `<project>/<branch>`, `"Ports: N"`, and the account slot's
    /// full label.
    Full,
    /// Branch-only project/branch label; bare port count (no "Ports:"
    /// word); account slot label unchanged.
    Compact,
    /// Narrowest tier. Same abbreviated project/branch and bare port
    /// count as `Compact`; segments keep their labels and shed only
    /// inter-word padding, so the name is historical.
    IconOnly,
}

impl StatusBarDensity {
    fn for_width(width: f32) -> Self {
        if width < theme::STATUS_BAR_ICON_ONLY_WIDTH {
            Self::IconOnly
        } else if width < theme::STATUS_BAR_COMPACT_WIDTH {
            Self::Compact
        } else {
            Self::Full
        }
    }

    /// True for `Compact` and `IconOnly` — the two tiers that
    /// abbreviate the project/branch label and the Ports chip.
    fn is_reduced(self) -> bool {
        self != Self::Full
    }
}

/// `"project/branch"` -> `"branch"` (last `/`-separated segment) for
/// `Compact`/`IconOnly`; unchanged at `Full`. A non-git label (no `/`)
/// passes through as-is at every tier — there's nothing to strip.
fn abbreviate_project_branch(label: &str, density: StatusBarDensity) -> &str {
    if density.is_reduced() {
        label.rsplit_once('/').map_or(label, |(_, branch)| branch)
    } else {
        label
    }
}

/// Collected status bar data — snapshot taken before rendering to
/// avoid entity reads during element construction (GPUI re-entrant
/// panic prevention).
pub(super) struct StatusBarData {
    /// `<project>/<branch>` for git-backed active lanes, just
    /// `<project>` for non-git or detached HEAD, `None` in Welcome
    /// state (no project loaded). The detached marker is rendered
    /// separately via [`Self::is_detached`].
    pub project_branch: Option<SharedString>,
    /// True when the active lane is git-backed but on a detached
    /// HEAD. Drives the inline "detached" chip rendered next to
    /// `project_branch`; harmless when `project_branch` is `None`
    /// (the chip suppresses itself).
    pub is_detached: bool,
    /// Focused pane title (process name / shell prompt).
    pub title: SharedString,
    /// Transient error string (pane spawn failures, etc.). When set,
    /// shows in the right section so the user actually notices the
    /// failure.
    pub error: Option<SharedString>,
    /// True when the workspace's project layer has a config.toml on
    /// disk. Drives a small dot in the right section so the user sees
    /// at a glance that some user-global keys are being shadowed.
    pub has_project_config: bool,
    /// The focused pane's resolved account slot. `None` when the focused
    /// pane doesn't track an account (File / TaskEdit panes) — hides the
    /// slot entirely. `Some` for Terminal / AgentChat panes, even when no
    /// account is configured (shows the "System" fallback label).
    pub account: Option<AccountSlot>,
    /// Latest attributed port scan (`Workspace::attributed_ports`), for
    /// the Ports segment. Empty before the first scan tick lands or
    /// when nothing is currently listening.
    pub ports: Vec<PortEntry>,
    /// Status associated with `ports`, used to keep the Ports chip
    /// visible for pending, unavailable, and successful-empty scans.
    pub ports_status: PortScanStatus,
    /// Plan-rate limits cached for the focused pane's account
    /// (`ClaudeContext::usage_by_account`), for the usage chip. Default
    /// (every window `None`) before the first fetch lands for that
    /// account, or for an account with no Claude usage backend — the
    /// chip hides itself rather than showing an empty gauge.
    pub usage: Option<daruda_claude::ProviderUsage>,
    /// Which segments the user has toggled on, via the status bar's
    /// right-click menu. Mirrors `daruda_config::StatusBarConfig`;
    /// `title` / `error` / the project-config dot are not user-toggleable.
    pub visible: daruda_config::StatusBarConfig,
    /// Dispatch target for the right-click toggle menu's clicks
    /// (`Workspace::toggle_status_bar_item`).
    pub workspace: WeakEntity<Workspace>,
}

/// GPUI render-once wrapper for the status bar element.
#[derive(IntoElement)]
pub(super) struct StatusBar(pub(super) StatusBarData);

impl RenderOnce for StatusBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let data = self.0;
        // Snapshot every theme slot once so the chain below reads each
        // colour through the live `DarudaTheme` Global; the four mid-render
        // `.text_color(t.text_muted)` etc. lookups stay consistent even if
        // a theme swap fires between expressions.
        let t = theme::current(cx);
        let muted = t.text_muted;
        let faint = t.text_subtle;
        let project_dot = t.status_bar_project_dot;
        let error_color = theme::ERROR;
        let detached_bg = t.status_bar_detached_bg;
        let detached_text = t.status_bar_detached_text;
        let bg = t.status_bar_bg;
        let border = t.border;

        let density = StatusBarDensity::for_width(f32::from(window.viewport_size().width));

        let show_project_branch = data.visible.is_visible(StatusBarItem::ProjectBranch);
        let show_account = data.visible.is_visible(StatusBarItem::AccountSlot);
        let toggle_menu = context_menu::build(data.visible.clone(), data.workspace.clone());

        // Detached chip is meaningful only when there's a
        // project/branch slot to anchor next to; in Welcome state
        // (`project_branch` is `None`) or when the segment is hidden,
        // suppress the chip too.
        let show_detached =
            show_project_branch && data.is_detached && data.project_branch.is_some();
        let project_branch_max_width = if density.is_reduced() {
            PROJECT_BRANCH_REDUCED_MAX_WIDTH
        } else {
            PROJECT_BRANCH_MAX_WIDTH
        };
        let error_max_width = if density.is_reduced() {
            STATUS_BAR_ERROR_REDUCED_MAX_WIDTH
        } else {
            STATUS_BAR_ERROR_MAX_WIDTH
        };

        let left = div()
            .flex()
            .flex_row()
            .items_center()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .gap(px(theme::STATUS_BAR_GAP))
            .when(show_project_branch, |el| {
                el.when_some(data.project_branch.clone(), |el, pb| {
                    let text = abbreviate_project_branch(&pb, density).to_string();
                    el.child(
                        div()
                            .flex_none()
                            .max_w(px(project_branch_max_width))
                            .overflow_hidden()
                            .truncate()
                            .text_size(px(theme::STATUS_BAR_FONT_SIZE))
                            .text_color(muted)
                            .child(SharedString::from(text)),
                    )
                })
                .when(show_detached, |el| {
                    el.child(
                        div()
                            .flex_none()
                            .px(px(theme::STATUS_BAR_DETACHED_PAD_X))
                            .py(px(theme::STATUS_BAR_DETACHED_PAD_Y))
                            .rounded(px(theme::STATUS_BAR_DETACHED_RADIUS))
                            .bg(detached_bg)
                            .text_size(px(theme::STATUS_BAR_DETACHED_FONT_SIZE))
                            .text_color(detached_text)
                            .child(SharedString::from(
                                crate::surface::strings::status_bar_detached_chip(),
                            )),
                    )
                })
                .when(data.project_branch.is_some(), |el| {
                    el.child(
                        div()
                            .flex_none()
                            .text_size(px(theme::STATUS_BAR_FONT_SIZE))
                            .text_color(faint)
                            .child(SharedString::from("—")),
                    )
                })
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .truncate()
                    .text_size(px(theme::STATUS_BAR_FONT_SIZE))
                    .text_color(muted)
                    .child(data.title.clone()),
            );

        let right = div()
            .flex()
            .flex_row()
            .items_center()
            .flex_none()
            .min_w_0()
            .gap(px(theme::STATUS_BAR_GAP))
            .when(data.has_project_config, |el| {
                el.child(
                    div()
                        .id("status-project-config")
                        .w(px(theme::STATUS_BAR_PROJECT_DOT_SIZE))
                        .h(px(theme::STATUS_BAR_PROJECT_DOT_SIZE))
                        .rounded_full()
                        .bg(project_dot)
                        .tooltip(crate::ui::tooltip::text(
                            crate::surface::strings::status_bar_project_config_tooltip(),
                        )),
                )
            })
            .when(data.visible.is_visible(StatusBarItem::Ports), |el| {
                el.child(ports_segment::render(
                    &data.ports,
                    data.ports_status,
                    density,
                    cx,
                ))
            })
            .when(data.visible.is_visible(StatusBarItem::ClaudeUsage), |el| {
                el.children(usage_chip::render(
                    data.usage.as_ref(),
                    density,
                    data.workspace.clone(),
                    cx,
                ))
            })
            .when(show_account, |el| {
                el.children(
                    data.account
                        .as_ref()
                        .map(|slot| account_slot::render(slot, density, cx)),
                )
            })
            .when_some(data.error.clone(), |el, err| {
                el.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .max_w(px(error_max_width))
                        .overflow_hidden()
                        .truncate()
                        .text_size(px(theme::STATUS_BAR_FONT_SIZE))
                        .text_color(error_color)
                        .child(err),
                )
            });

        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(theme::STATUS_BAR_GAP))
            .w_full()
            .h(px(STATUS_BAR_HEIGHT))
            .px(px(theme::STATUS_BAR_PAD_X))
            .bg(bg)
            .border_t_1()
            .border_color(border)
            .child(left)
            .child(right)
            .context_menu(toggle_menu)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn density_full_above_compact_width() {
        assert_eq!(
            StatusBarDensity::for_width(theme::STATUS_BAR_COMPACT_WIDTH),
            StatusBarDensity::Full
        );
    }

    #[test]
    fn density_compact_between_breakpoints() {
        let width = (theme::STATUS_BAR_COMPACT_WIDTH + theme::STATUS_BAR_ICON_ONLY_WIDTH) / 2.0;
        assert_eq!(
            StatusBarDensity::for_width(width),
            StatusBarDensity::Compact
        );
    }

    #[test]
    fn density_icon_only_below_icon_only_width() {
        assert_eq!(
            StatusBarDensity::for_width(theme::STATUS_BAR_ICON_ONLY_WIDTH - 1.0),
            StatusBarDensity::IconOnly
        );
    }

    #[test]
    fn abbreviate_keeps_full_label_at_full_density() {
        assert_eq!(
            abbreviate_project_branch("daruda/main", StatusBarDensity::Full),
            "daruda/main"
        );
    }

    #[test]
    fn abbreviate_strips_project_prefix_when_reduced() {
        assert_eq!(
            abbreviate_project_branch("daruda/main", StatusBarDensity::Compact),
            "main"
        );
        assert_eq!(
            abbreviate_project_branch("daruda/main", StatusBarDensity::IconOnly),
            "main"
        );
    }

    #[test]
    fn abbreviate_passes_through_non_git_label_unchanged() {
        // No `/` separator (non-git project, or detached-HEAD label) —
        // nothing to strip at any density.
        assert_eq!(
            abbreviate_project_branch("daruda", StatusBarDensity::Compact),
            "daruda"
        );
    }
}
