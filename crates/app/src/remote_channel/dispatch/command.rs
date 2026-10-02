use crate::control::{
    result::{ControlOutcome, ControlResult},
    spec::ControlCommand,
};
use crate::remote_channel::{bridge::PaneRef, command};
use crate::telegram::trace;
use gpui::App;

/// Resolve, run, and render one command.
///
/// Three steps, each holding the bridge global for as long as it needs and no
/// longer: resolution needs the ordinal table, execution needs the whole `App`
/// to walk every window, and rendering needs the table again — now updated by
/// whatever the execution produced.
pub(crate) fn run_command(
    command: ControlCommand,
    target: &super::Target,
    cx: &mut gpui::AsyncApp,
) -> Option<command::RenderedReply> {
    trace::delivery("command", || format!("{command:?}"));
    let step = cx.update(|cx| {
        let state = target.state(cx)?;
        Some(command::resolve_command(command, state))
    })?;
    let (outcome, addressed) = match step {
        command::Resolution::Answer(outcome) => (outcome, None),
        // Fill in the one target no listing can name, then run like any other.
        command::Resolution::Ask(text) => (
            cx.update(|cx| {
                let (destination, connecting) = crate::orchestrator::destination(cx)?;
                crate::control::exec::run(
                    crate::control::spec::ResolvedCommand::AskOrchestrator {
                        text,
                        destination,
                        connecting,
                    },
                    cx,
                )
            }),
            None,
        ),
        // Same shape as `Ask`: pick the target the text did not name, then
        // run like any other command.
        command::Resolution::RunFlow(name) => (
            cx.update(|cx| {
                let lane = crate::control::exec::first_lane_offering(&name, cx)?;
                crate::control::exec::run(
                    crate::control::spec::ResolvedCommand::Flow(
                        crate::control::spec::ResolvedFlowCommand::Run { name, lane },
                    ),
                    cx,
                )
            }),
            None,
        ),
        command::Resolution::StartTask(task) => (start_task(task, cx), None),
        command::Resolution::OpenTask(task) => (
            cx.update(|cx| crate::control::exec::open_task(task, cx)),
            None,
        ),
        command::Resolution::Run(resolved, addressed) => (
            cx.update(|cx| crate::control::exec::run(resolved, cx)),
            addressed,
        ),
    };
    cx.update(|cx| {
        let state = target.state(cx)?;
        let absorbed = command::absorb(&outcome, addressed, state);
        Some(command::render(&outcome, absorbed, state))
    })
}

/// Start a task for the phone. The reply says whether it was accepted; once
/// git and the agent are up, the outcome follows as its own message — a
/// chat announced as the new target, or a notice.
fn start_task(task: String, cx: &mut gpui::AsyncApp) -> ControlOutcome {
    let pending = cx.update(|cx| crate::control::exec::begin_task_start(task, cx))?;
    if let Some(outcome) = cx.update(|cx| pending.try_finish(cx)) {
        return outcome;
    }
    let accepted = ControlResult::TaskStarting {
        task: pending.task.clone(),
        title: pending.title.clone(),
    };
    cx.spawn(async move |cx| {
        let title = pending.title.clone();
        let outcome = pending.finish(cx).await;
        cx.update(|cx| match outcome {
            // A chat was announced as the phone's new target on the way.
            Ok(ControlResult::TaskStarted { chat: Some(_), .. }) => {}
            Ok(_) => crate::remote_channel::send_notice_everywhere(
                crate::surface::strings::control::task_started_terminal(&title),
                cx,
            ),
            Err(e) => crate::remote_channel::send_notice_everywhere(
                crate::surface::strings::control::task_start_failed(
                    &title,
                    command::render_error(&e),
                ),
                cx,
            ),
        });
    })
    .detach();
    Ok(accepted)
}

/// Answer a message that named a pane which is no longer there, and stop
/// remembering that pane. Without the second half, the *next* plain message
/// resolves to the same dead target and is lost just as silently.
pub(crate) fn report_target_gone(
    pane: PaneRef,
    target: &super::Target,
    cx: &mut App,
) -> Option<command::RenderedReply> {
    trace::state("target.forgotten", || format!("pane={}", trace::pane(pane)));
    let state = target.state(cx)?;
    state.forget(pane);
    Some(command::render(
        &Err(crate::control::result::ControlError::TargetGone),
        command::Absorbed::SelectionDropped,
        state,
    ))
}

/// Render an outcome the executor never saw, against the live ordinal table.
pub(crate) fn render_outcome(
    outcome: &ControlOutcome,
    target: &super::Target,
    cx: &mut App,
) -> Option<command::RenderedReply> {
    let state = target.state(cx)?;
    Some(command::render(outcome, command::Absorbed::Nothing, state))
}

/// Point the target at `pane` and produce the toast naming it. A tap is the
/// same act as `/use <n>`, so it goes through the same render funnel.
pub(crate) fn select_target(pane: PaneRef, target: &super::Target, cx: &mut App) -> Option<String> {
    trace::state("target.selected", || format!("pane={}", trace::pane(pane)));
    let state = target.state(cx)?;
    state.select(Some(pane));
    let summary = state.summary_for(pane);
    Some(
        command::render(
            &Ok(ControlResult::Selected { target: summary }),
            command::Absorbed::Nothing,
            state,
        )
        .text,
    )
}
