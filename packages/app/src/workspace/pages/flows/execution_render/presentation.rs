//! Labels derived from the durable run projection.
use super::*;
use daruda_flow::NodeId;
use daruda_flow::{history::NodeStatus, record::AttemptOutcome};

fn node_label(status: NodeStatus) -> String {
    match status {
        NodeStatus::Pending => s::flow::run_pending(),
        NodeStatus::Pinned => s::flow::graph_status_pinned(),
        NodeStatus::Running { .. } | NodeStatus::Fixing => s::flow::graph_status_running(),
        NodeStatus::Passed => s::flow::graph_status_passed(),
        NodeStatus::Canceled => s::flow::status_canceled(),
        NodeStatus::Failed | NodeStatus::Interrupted => s::flow::graph_status_failed(),
    }
}

fn outcome(outcome: &AttemptOutcome) -> String {
    node_label(NodeStatus::from_outcome(outcome))
}

pub(super) fn attempt_label(history: &RunHistory, node: &NodeId, attempt: u32, seq: u32) -> String {
    let record = history.attempt(node, seq);
    let status = record
        .map(|r| outcome(&r.outcome))
        .unwrap_or_else(|| node_label(history.node_status(node)));
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

fn node_tone(status: NodeStatus) -> Tone {
    match status {
        NodeStatus::Pending => Tone::Pending,
        NodeStatus::Running { .. } | NodeStatus::Fixing => Tone::Running,
        NodeStatus::Passed => Tone::Passed,
        NodeStatus::Failed | NodeStatus::Interrupted => Tone::Failed,
        NodeStatus::Canceled => Tone::Canceled,
        NodeStatus::Pinned => Tone::Pinned,
    }
}

pub(super) fn attempt_badge(
    attempt: &daruda_flow::record::AttemptRecord,
    cx: &gpui::App,
) -> AnyElement {
    status(
        outcome(&attempt.outcome),
        node_tone(NodeStatus::from_outcome(&attempt.outcome)),
        false,
        cx,
    )
}

pub(super) fn node_badge(history: &RunHistory, node: &NodeId, cx: &gpui::App) -> AnyElement {
    let value = history.node_status(node);
    status(node_label(value), node_tone(value), false, cx)
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
