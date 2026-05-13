//! Bottom status bar — displays shell state, cwd, and git branch.
//! Always visible at the bottom of the workspace.

use crate::ui::theme;
use gpui::{App, IntoElement, RenderOnce, SharedString, Window, div, prelude::*, px};

/// Fixed height of the status bar in pixels.
pub(super) const STATUS_BAR_HEIGHT: f32 = theme::STATUS_BAR_HEIGHT;

/// Collected status bar data — snapshot taken before rendering to
/// avoid entity reads during element construction (GPUI re-entrant
/// panic prevention).
pub(super) struct StatusBarData {
    pub cwd: Option<SharedString>,
    pub title: SharedString,
    pub git_branch: Option<SharedString>,
    /// Transient error string (pane spawn failures, etc.). When set,
    /// shows in the right section so the user actually notices the
    /// failure.
    pub error: Option<SharedString>,
    /// True when the workspace's project layer has a config.toml on
    /// disk. Drives a small dot in the right section so the user sees
    /// at a glance that some user-global keys are being shadowed.
    pub has_project_config: bool,
}

impl StatusBarData {
    /// Build from a focused pane. Extracts cwd basename and title.
    #[allow(dead_code)]
    pub fn from_pane(title: &str, cwd: Option<&std::path::Path>) -> Self {
        Self {
            title: SharedString::from(title.to_string()),
            cwd: cwd.and_then(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|s| SharedString::from(s.to_string()))
            }),
            git_branch: None,
            error: None,
            has_project_config: false,
        }
    }
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
        let bg = t.status_bar_bg;
        let border = t.status_bar_border;

        let left = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::STATUS_BAR_GAP))
            .child(
                div()
                    .text_size(px(theme::STATUS_BAR_FONT_SIZE))
                    .text_color(muted)
                    .child(data.title.clone()),
            )
            .when_some(data.cwd.clone(), |el, cwd| {
                el.child(
                    div()
                        .text_size(px(theme::STATUS_BAR_FONT_SIZE))
                        .text_color(faint)
                        .child(cwd),
                )
            });

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
                            crate::surface::strings::STATUS_BAR_PROJECT_CONFIG_TOOLTIP,
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
            })
            .when_some(data.git_branch.clone(), |el, branch| {
                el.child(
                    div()
                        .flex()
                        .flex_row()
                        .items_center()
                        .gap(px(theme::STATUS_BAR_GIT_GAP))
                        .text_size(px(theme::STATUS_BAR_FONT_SIZE))
                        .text_color(muted)
                        .child(
                            div()
                                .text_size(px(theme::STATUS_BAR_GIT_ICON_SIZE))
                                .child("⎇"),
                        )
                        .child(branch),
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
    fn from_pane_extracts_cwd_basename() {
        let data = StatusBarData::from_pane(
            "zsh",
            Some(std::path::Path::new("/Users/test/projects/daruda")),
        );
        assert_eq!(data.title.as_ref(), "zsh");
        assert_eq!(data.cwd.as_ref().map(|s| s.as_ref()), Some("daruda"));
    }

    #[test]
    fn from_pane_handles_no_cwd() {
        let data = StatusBarData::from_pane("bash", None);
        assert_eq!(data.title.as_ref(), "bash");
        assert!(data.cwd.is_none());
    }

    #[test]
    fn from_pane_handles_root_path() {
        let data = StatusBarData::from_pane("zsh", Some(std::path::Path::new("/")));
        assert!(data.cwd.is_none());
    }

    #[test]
    fn git_branch_defaults_to_none() {
        let data = StatusBarData::from_pane("zsh", None);
        assert!(data.git_branch.is_none());
    }

    #[test]
    fn error_defaults_to_none() {
        let data = StatusBarData::from_pane("zsh", None);
        assert!(data.error.is_none());
    }
}
