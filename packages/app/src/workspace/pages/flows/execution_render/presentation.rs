//! Labels derived from the durable run projection.
use super::*;
use daruda_flow::{record::AttemptOutcome, run_log::machine::AttemptState};

pub(super) fn node_status(history: &RunHistory, node: &NodeId) -> String {
    if let Some(state) = history.state().and_then(|state| {
        state
            .node_states()
            .find(|(id, _)| *id == node)
            .map(|(_, state)| state)
    }) {
        if state.is_invalidated() {
            return s::flow::run_pending();
        }
        return match state.last() {
            AttemptState::NotRun => s::flow::run_pending(),
            AttemptState::Pinned => s::flow::graph_status_pinned(),
            AttemptState::Started { .. } => s::flow::graph_status_running(),
            AttemptState::Passed { .. } => s::flow::graph_status_passed(),
            AttemptState::Canceled { .. } => s::flow::status_canceled(),
            _ => s::flow::graph_status_failed(),
        };
    }
    history
        .attempts(node)
        .last()
        .and_then(|(_, seq)| history.attempt(node, *seq))
        .map(|a| outcome(&a.outcome))
        .unwrap_or_else(s::flow::run_pending)
}

fn outcome(outcome: &AttemptOutcome) -> String {
    match outcome {
        AttemptOutcome::Passed => s::flow::graph_status_passed(),
        AttemptOutcome::Canceled => s::flow::status_canceled(),
        _ => s::flow::graph_status_failed(),
    }
}

pub(super) fn attempt_label(history: &RunHistory, node: &NodeId, attempt: u32, seq: u32) -> String {
    let record = history.attempt(node, seq);
    let status = record
        .map(|r| outcome(&r.outcome))
        .unwrap_or_else(|| node_status(history, node));
    let duration = record
        .map(|r| format!("{:.1}s", r.took.as_secs_f64()))
        .unwrap_or_else(s::flow::run_not_recorded);
    let label = s::flow::run_attempt_summary(attempt, status, duration, seq);
    if history
        .attempts(node)
        .last()
        .is_some_and(|(_, last)| *last == seq)
    {
        s::flow::run_final_attempt(label)
    } else {
        label
    }
}

#[cfg(test)]
mod tests {}

#[derive(Clone, Copy)]
enum Tone {
    Pending,
    Running,
    Passed,
    Failed,
    Canceled,
    Pinned,
}

impl Tone {
    fn color(self, cx: &gpui::App) -> gpui::Hsla {
        let t = theme::current(cx);
        match self {
            Self::Passed => t.banner_success_text,
            Self::Failed => t.banner_error_text,
            Self::Running | Self::Pinned => t.toast_action_text,
            Self::Canceled => t.banner_warning_text,
            Self::Pending => t.text_muted,
        }
    }
    fn icon(self) -> &'static str {
        match self {
            Self::Passed => icons::CHECK_CIRCLE,
            Self::Failed => icons::WARNING,
            Self::Running => icons::REFRESH,
            Self::Canceled => icons::CLOSE,
            Self::Pinned => icons::PIN,
            Self::Pending => icons::RADIO_OFF,
        }
    }
}

fn status(label: String, tone: Tone, dot: bool, cx: &gpui::App) -> AnyElement {
    let color = tone.color(cx);
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(theme::GAP_SM))
        .text_color(color)
        .child(if dot {
            div()
                .flex_none()
                .size(px(palette::TAB_STATUS_DOT_SIZE))
                .rounded_full()
                .bg(color)
                .into_any_element()
        } else {
            icons::icon(tone.icon())
                .text_color(color)
                .into_any_element()
        })
        .child(label)
        .into_any_element()
}

pub(super) fn run_badge(value: daruda_flow::marker::RunStatus, cx: &gpui::App) -> AnyElement {
    use daruda_flow::marker::RunStatus;
    let tone = match value {
        RunStatus::Done => Tone::Passed,
        RunStatus::Failed | RunStatus::Crashed => Tone::Failed,
        RunStatus::Running => Tone::Running,
        RunStatus::Canceled | RunStatus::Stalled => Tone::Canceled,
        RunStatus::Unknown => Tone::Pending,
    };
    status(s::flow::run_status(value), tone, true, cx)
}

fn attempt_tone(outcome: &AttemptOutcome) -> Tone {
    match outcome {
        AttemptOutcome::Passed => Tone::Passed,
        AttemptOutcome::Canceled => Tone::Canceled,
        _ => Tone::Failed,
    }
}

pub(super) fn attempt_badge(
    attempt: &daruda_flow::record::AttemptRecord,
    cx: &gpui::App,
) -> AnyElement {
    status(
        outcome(&attempt.outcome),
        attempt_tone(&attempt.outcome),
        false,
        cx,
    )
}

pub(super) fn node_badge(history: &RunHistory, node: &NodeId, cx: &gpui::App) -> AnyElement {
    let tone = history
        .state()
        .and_then(|state| {
            state
                .node_states()
                .find(|(id, _)| *id == node)
                .map(|(_, state)| {
                    if state.is_invalidated() {
                        return Tone::Pending;
                    }
                    match state.last() {
                        AttemptState::NotRun => Tone::Pending,
                        AttemptState::Pinned => Tone::Pinned,
                        AttemptState::Started { .. } => Tone::Running,
                        AttemptState::Passed { .. } => Tone::Passed,
                        AttemptState::Canceled { .. } => Tone::Canceled,
                        _ => Tone::Failed,
                    }
                })
        })
        .or_else(|| {
            history
                .attempts(node)
                .last()
                .and_then(|(_, seq)| history.attempt(node, *seq))
                .map(|a| attempt_tone(&a.outcome))
        })
        .unwrap_or(Tone::Pending);
    status(node_status(history, node), tone, false, cx)
}

pub(in crate::workspace::pages::flows) fn started(at: Option<std::time::SystemTime>) -> String {
    at.map(|at| {
        let at: chrono::DateTime<chrono::Local> = at.into();
        s::flow::run_started_label(at.format("%Y-%m-%d %H:%M:%S"))
    })
    .unwrap_or_else(s::flow::run_not_recorded)
}

pub(super) fn duration(value: Option<std::time::Duration>) -> String {
    value
        .map(|d| {
            if d.as_secs() == 0 {
                format!("{:.1}s", d.as_secs_f64())
            } else {
                humantime::format_duration(std::time::Duration::from_secs(d.as_secs())).to_string()
            }
        })
        .unwrap_or_else(s::flow::run_not_recorded)
}
