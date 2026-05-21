//! Bottom status bar — displays project/branch context and focused
//! pane title. Always visible at the bottom of the workspace.

use crate::ui::theme;
use gpui::{App, IntoElement, RenderOnce, SharedString, Window, div, prelude::*, px};

/// Fixed height of the status bar in pixels.
pub(super) const STATUS_BAR_HEIGHT: f32 = theme::STATUS_BAR_HEIGHT;

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
}

/// GPUI render-once wrapper for the status bar element.
#[derive(IntoElement)]
pub(super) struct StatusBar(pub(super) StatusBarData);

impl RenderOnce for StatusBar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let data = self.0;
        // Snapshot every theme slot once so the chain below reads each
        // colour through the live `DarudaTheme` Global; the four mid-render
        // `.text_color(t.muted_text)` etc. lookups stay consistent even if
        // a theme swap fires between expressions.
        let t = theme::current(cx);
        let muted = t.muted_text;
        let faint = t.faint_text;
        let project_dot = t.status_bar_project_dot;
        let error_color = t.status_bar_error;
        let detached_bg = t.status_bar_detached_bg;
        let detached_text = t.status_bar_detached_text;
        let bg = t.status_bar_bg;
        let border = t.status_bar_border;

        // Detached chip is meaningful only when there's a
        // project/branch slot to anchor next to; in Welcome state
        // (`project_branch` is `None`) suppress the chip too.
        let show_detached = data.is_detached && data.project_branch.is_some();

        let left = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::STATUS_BAR_GAP))
            .when_some(data.project_branch.clone(), |el, pb| {
                el.child(
                    div()
                        .text_size(px(theme::STATUS_BAR_FONT_SIZE))
                        .text_color(muted)
                        .child(pb),
                )
            })
            .when(show_detached, |el| {
                el.child(
                    div()
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
                        .text_size(px(theme::STATUS_BAR_FONT_SIZE))
                        .text_color(faint)
                        .child(SharedString::from("—")),
                )
            })
            .child(
                div()
                    .text_size(px(theme::STATUS_BAR_FONT_SIZE))
                    .text_color(muted)
                    .child(data.title.clone()),
            );

        let right = div()
            .flex()
            .flex_row()
            .items_center()
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
            .when_some(data.error.clone(), |el, err| {
                el.child(
                    div()
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
            .w_full()
            .h(px(STATUS_BAR_HEIGHT))
            .px(px(theme::STATUS_BAR_PAD_X))
            .bg(bg)
            .border_t_1()
            .border_color(border)
            .child(left)
            .child(right)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::assertions_on_constants)]
    fn height_is_reasonable() {
        assert!(STATUS_BAR_HEIGHT >= 16.0);
        assert!(STATUS_BAR_HEIGHT <= 40.0);
    }

    #[test]
    fn data_defaults_to_no_project_branch() {
        let data = StatusBarData {
            project_branch: None,
            is_detached: false,
            title: "zsh".into(),
            error: None,
            has_project_config: false,
        };
        assert!(data.project_branch.is_none());
        assert!(!data.is_detached);
        assert!(data.error.is_none());
        assert_eq!(data.title.as_ref(), "zsh");
    }

    #[test]
    fn data_carries_project_and_branch() {
        let data = StatusBarData {
            project_branch: Some("daruda/main".into()),
            is_detached: false,
            title: "zsh".into(),
            error: None,
            has_project_config: false,
        };
        assert_eq!(
            data.project_branch.as_ref().map(|s| s.as_ref()),
            Some("daruda/main")
        );
    }

    #[test]
    fn data_marks_detached_when_branch_missing() {
        let data = StatusBarData {
            project_branch: Some("daruda".into()),
            is_detached: true,
            title: "zsh".into(),
            error: None,
            has_project_config: false,
        };
        assert!(data.is_detached);
        assert_eq!(
            data.project_branch.as_ref().map(|s| s.as_ref()),
            Some("daruda")
        );
    }
}
