//! Snapshot-only status and toggle for the workspace's orchestrator.

use daruda_terminal::ux::theme as metrics;
use gpui::{AnyElement, App, IntoElement, SharedString, WeakEntity, div, prelude::*, px};

use crate::surface::strings as s;
use crate::ui::{button_status_pill_bare, theme};
use crate::workspace::Workspace;
use crate::workspace::main_area::agent_chat_pane::view::{ActivityState, AgentSessionStatus};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum OrchestratorChipState {
    Idle,
    Working,
    AwaitingPermission,
    Failed,
}

impl OrchestratorChipState {
    pub(in crate::workspace) fn from_activity(
        activity: ActivityState,
        status: &AgentSessionStatus,
    ) -> Self {
        if matches!(status, AgentSessionStatus::Error { .. }) {
            return Self::Failed;
        }
        match activity {
            ActivityState::Idle => Self::Idle,
            ActivityState::Working => Self::Working,
            ActivityState::AwaitingPermission => Self::AwaitingPermission,
        }
    }

    fn appearance(self) -> (&'static str, gpui::Hsla, String) {
        match self {
            Self::Idle => (
                metrics::ORCHESTRATOR_IDLE_GLYPH,
                metrics::ORCHESTRATOR_IDLE_COLOR,
                s::orchestrator_idle(),
            ),
            Self::Working => (
                metrics::ORCHESTRATOR_WORKING_GLYPH,
                metrics::ORCHESTRATOR_WORKING_COLOR,
                s::orchestrator_working(),
            ),
            Self::AwaitingPermission => (
                metrics::ORCHESTRATOR_PERMISSION_GLYPH,
                metrics::ORCHESTRATOR_PERMISSION_COLOR,
                s::orchestrator_awaiting_permission(),
            ),
            Self::Failed => (
                metrics::ORCHESTRATOR_FAILED_GLYPH,
                metrics::ORCHESTRATOR_FAILED_COLOR,
                s::orchestrator_failed(),
            ),
        }
    }
}

pub(super) fn render(
    state: OrchestratorChipState,
    workspace: WeakEntity<Workspace>,
    cx: &App,
) -> AnyElement {
    let (glyph, color, tooltip) = state.appearance();
    button_status_pill_bare("status-orchestrator", cx)
        .flex_none()
        .text_size(px(theme::STATUS_BAR_FONT_SIZE))
        .tooltip(SharedString::from(tooltip))
        .child(
            div()
                .w(px(metrics::ORCHESTRATOR_GLYPH_WIDTH))
                .text_color(color)
                .child(glyph),
        )
        .child(SharedString::from(s::orchestrator_label()))
        .on_click(move |_, window, cx| {
            if let Err(error) =
                workspace.update(cx, |ws, cx| ws.toggle_orchestrator_tab(window, cx))
            {
                daruda_store::observability::log_writer::LogWriter::log(
                    daruda_store::observability::error_report::ErrorReport::new(
                        "Orchestrator host closed before toggle",
                    )
                    .severity(daruda_store::observability::error_report::ErrorSeverity::Warning)
                    .with_context("error", error.to_string())
                    .at(file!(), line!())
                    .dedup("orchestrator.toggle")
                    .build(),
                );
            }
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chip_states_have_distinct_signals_and_errors_take_precedence() {
        let connected = AgentSessionStatus::Connected;
        let cases = [
            (ActivityState::Idle, OrchestratorChipState::Idle),
            (ActivityState::Working, OrchestratorChipState::Working),
            (
                ActivityState::AwaitingPermission,
                OrchestratorChipState::AwaitingPermission,
            ),
        ];
        let mut glyphs = std::collections::HashSet::new();
        for (activity, expected) in cases {
            assert_eq!(
                OrchestratorChipState::from_activity(activity, &connected),
                expected
            );
            assert!(glyphs.insert(expected.appearance().0));
        }
        let error = AgentSessionStatus::Error {
            message: "test failure".into(),
            remedy: daruda_acp::Remedy::Retry,
        };
        assert_eq!(
            OrchestratorChipState::from_activity(ActivityState::Working, &error),
            OrchestratorChipState::Failed
        );
        assert!(glyphs.insert(OrchestratorChipState::Failed.appearance().0));
    }
}
