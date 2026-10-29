//! Semantic status artwork; frames share the dock's low-frequency clock.

use super::BadgeContent;
use crate::ui::{Sizable as _, theme};
use daruda_agent::{AgentOutcome, SessionStatus};
use gpui::{App, Hsla, IntoElement, Pixels, Styled};

const WORKING: [&str; 3] = [
    "icons/status/working-0.svg",
    "icons/status/working-1.svg",
    "icons/status/working-2.svg",
];
const TOOL: [&str; 2] = ["icons/status/tool-0.svg", "icons/status/tool-1.svg"];
const CONNECTING: [&str; 2] = [
    "icons/status/connecting-0.svg",
    "icons/status/connecting-1.svg",
];
const IDLE: &str = "icons/status/idle.svg";
const ATTENTION: &str = "icons/status/attention.svg";
const DONE: &str = "icons/status/done.svg";
const ERRORED: &str = "icons/status/errored.svg";
const FAILED: &str = "icons/status/failed.svg";

fn frame(content: BadgeContent, tick: u64) -> &'static str {
    match content {
        BadgeContent::Disconnected => CONNECTING[1],
        BadgeContent::Outcome(AgentOutcome::Completed) => DONE,
        BadgeContent::Outcome(AgentOutcome::Errored) => ERRORED,
        BadgeContent::Status(status) => match status {
            SessionStatus::Working => WORKING[(tick % WORKING.len() as u64) as usize],
            SessionStatus::ExecutingTool => TOOL[usize::from(!super::blink_on(tick))],
            SessionStatus::Connecting => CONNECTING[usize::from(!super::blink_on(tick))],
            SessionStatus::Idle => IDLE,
            SessionStatus::NeedsAttention => ATTENTION,
            SessionStatus::Failed => FAILED,
        },
    }
}

pub(super) fn color(content: BadgeContent, fallback: Hsla, cx: &App) -> Hsla {
    let dark = theme::current(cx).is_dark();
    match content {
        BadgeContent::Status(SessionStatus::ExecutingTool) => {
            if dark {
                theme::STATUS_ICON_TOOL_DARK
            } else {
                theme::STATUS_ICON_TOOL_LIGHT
            }
        }
        BadgeContent::Status(SessionStatus::NeedsAttention) => {
            if dark {
                theme::WARNING
            } else {
                theme::STATUS_ICON_ATTENTION_LIGHT
            }
        }
        _ => fallback,
    }
}

pub(super) fn render(
    content: BadgeContent,
    size: Pixels,
    color: Hsla,
    tick: u64,
    _cx: &App,
) -> gpui::AnyElement {
    crate::ui::icons::icon(frame(content, tick))
        .with_size(size)
        .text_color(color)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcomes_are_distinct_from_idle_and_session_failure() {
        assert_ne!(
            frame(BadgeContent::Outcome(AgentOutcome::Completed), 0),
            frame(BadgeContent::Status(SessionStatus::Idle), 0)
        );
        assert_ne!(
            frame(BadgeContent::Outcome(AgentOutcome::Errored), 0),
            frame(BadgeContent::Status(SessionStatus::Failed), 0)
        );
    }

    #[test]
    fn only_running_states_change_frames() {
        for state in [
            SessionStatus::Working,
            SessionStatus::ExecutingTool,
            SessionStatus::Connecting,
        ] {
            let content = BadgeContent::Status(state);
            assert_ne!(frame(content, 0), frame(content, 2));
        }
        for state in [
            SessionStatus::Idle,
            SessionStatus::NeedsAttention,
            SessionStatus::Failed,
        ] {
            assert_eq!(
                frame(BadgeContent::Status(state), 0),
                frame(BadgeContent::Status(state), 2)
            );
        }
    }
}
