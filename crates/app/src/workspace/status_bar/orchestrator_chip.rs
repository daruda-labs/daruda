//! Snapshot-only status and toggle for the workspace's orchestrator.

use daruda_terminal::ux::theme as metrics;
use gpui::{AnyElement, App, IntoElement, SharedString, WeakEntity, div, prelude::*, px};

use crate::surface::strings as s;
use crate::ui::{button_status_pill_bare, theme};
use crate::workspace::Workspace;
use crate::workspace::main_area::agent_chat_pane::view::{ActivityState, AgentSessionStatus};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum OrchestratorChipState {
    /// Configured, but no session yet — what the chip shows from launch.
    ///
    /// The session stays lazy (an agent process is not spent on a day nobody
    /// asks for one); only the chip is eager, and clicking it is the
    /// desktop's way to start one.
    NotStarted,
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

    /// Glyph, colour and tooltip. Colours come from the live theme, so the
    /// chip reads as the same signal set every peer chip uses.
    fn appearance(self, cx: &App) -> (&'static str, gpui::Hsla, String) {
        let t = theme::current(cx);
        match self {
            Self::NotStarted => (
                metrics::ORCHESTRATOR_NOT_STARTED_GLYPH,
                t.orchestrator_not_started,
                s::orchestrator_not_started(),
            ),
            Self::Idle => (
                metrics::ORCHESTRATOR_IDLE_GLYPH,
                t.text_muted,
                s::orchestrator_idle(),
            ),
            Self::Working => (
                metrics::ORCHESTRATOR_WORKING_GLYPH,
                t.orchestrator_working,
                s::orchestrator_working(),
            ),
            Self::AwaitingPermission => (
                metrics::ORCHESTRATOR_PERMISSION_GLYPH,
                t.orchestrator_permission,
                s::orchestrator_awaiting_permission(),
            ),
            Self::Failed => (
                metrics::ORCHESTRATOR_FAILED_GLYPH,
                t.orchestrator_failed,
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
    let (glyph, color, tooltip) = state.appearance(cx);
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
        // One dispatch; the body cannot be a `Workspace` method — see
        // `orchestrator::start_or_toggle_from_chip`.
        .on_click(move |_, window, cx| {
            crate::orchestrator::start_or_toggle_from_chip(&workspace, window, cx);
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A person distinguishes these at a glance, so no two may share a glyph
    /// or a colour — and an error outranks whatever the activity says.
    #[gpui::test]
    fn chip_states_have_distinct_signals_and_errors_take_precedence(cx: &mut gpui::TestAppContext) {
        crate::test_support::init_gpui_component(cx);
        let connected = AgentSessionStatus::Connected;
        let cases = [
            (ActivityState::Idle, OrchestratorChipState::Idle),
            (ActivityState::Working, OrchestratorChipState::Working),
            (
                ActivityState::AwaitingPermission,
                OrchestratorChipState::AwaitingPermission,
            ),
        ];
        let error = AgentSessionStatus::Error {
            message: "test failure".into(),
            remedy: daruda_acp::Remedy::Retry,
        };
        cx.update(|cx| {
            let mut glyphs = std::collections::HashSet::new();
            let mut colors = Vec::new();
            for (activity, expected) in cases {
                assert_eq!(
                    OrchestratorChipState::from_activity(activity, &connected),
                    expected
                );
                let (glyph, color, tooltip) = expected.appearance(cx);
                assert!(glyphs.insert(glyph), "{expected:?} reuses a glyph");
                assert!(!tooltip.is_empty(), "{expected:?} has no tooltip");
                colors.push((expected, color));
            }
            assert_eq!(
                OrchestratorChipState::from_activity(ActivityState::Working, &error),
                OrchestratorChipState::Failed,
                "an error outranks the activity"
            );
            for extra in [
                OrchestratorChipState::Failed,
                OrchestratorChipState::NotStarted,
            ] {
                let (glyph, color, tooltip) = extra.appearance(cx);
                assert!(glyphs.insert(glyph), "{extra:?} reuses a glyph");
                assert!(!tooltip.is_empty(), "{extra:?} has no tooltip");
                colors.push((extra, color));
            }
            for (i, (state, color)) in colors.iter().enumerate() {
                for (other, other_color) in &colors[i + 1..] {
                    assert_ne!(
                        color, other_color,
                        "{state:?} and {other:?} are the same colour"
                    );
                }
            }
        });
    }
}
