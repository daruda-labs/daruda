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
    cx: &mut App,
) -> super::Effect {
    trace::delivery("command", || format!("{command:?}"));
    let step = (|| {
        let state = target.state(cx)?;
        Some(command::resolve_command(command, state))
    })();
    let Some(step) = step else {
        return super::Effect::None;
    };
    let (outcome, addressed) = match step {
        command::Resolution::Answer(outcome) => (outcome, None),
        // Fill in the one target no listing can name, then run like any other.
        command::Resolution::Ask(text) => (
            (|| {
                let (destination, connecting) = crate::orchestrator::destination(cx)?;
                let outcome = crate::control::exec::run(
                    crate::control::spec::ResolvedCommand::AskOrchestrator {
                        text,
                        destination,
                        connecting,
                    },
                    crate::control::exec::CommandOrigin::Phone,
                    cx,
                );
                // Its answer arrives as a bystander ping, so asking is what
                // makes the orchestrator the conversation; `/stop` keeps the pick.
                if outcome.is_ok()
                    && let Some(state) = target.state(cx)
                {
                    state.note_ping(destination);
                }
                outcome
            })(),
            None,
        ),
        // Same shape as `Ask`: pick the target the text did not name, then
        // run like any other command.
        command::Resolution::RunFlow(name) => (
            (|| {
                let lane = crate::control::exec::first_lane_offering(&name, cx)?;
                crate::control::exec::run(
                    crate::control::spec::ResolvedCommand::Flow(
                        crate::control::spec::ResolvedFlowCommand::Run { name, lane },
                    ),
                    crate::control::exec::CommandOrigin::Phone,
                    cx,
                )
            })(),
            None,
        ),
        command::Resolution::StartTask(task) => (start_task(task, cx), None),
        // Answered once the cancel is in the task log, so a listing sent
        // right after the reply no longer shows the task running.
        command::Resolution::StopTask(task) => {
            let started = crate::control::exec::run_waiting(
                crate::control::spec::ResolvedCommand::TaskStop { task },
                crate::control::exec::CommandOrigin::Phone,
                cx,
            );
            let deferred = match started {
                crate::control::exec::Dispatch::Ready(outcome) => {
                    return render_reply(&outcome, None, target, cx)
                        .map_or(super::Effect::None, super::Effect::Reply);
                }
                crate::control::exec::Dispatch::Deferred { outcome, .. } => outcome,
            };
            let target = target.clone();
            return super::Effect::Later(cx.spawn(async move |cx| {
                let outcome = deferred.recv().await.ok().flatten()?;
                cx.update(|cx| render_reply(&outcome, None, &target, cx))
            }));
        }
        command::Resolution::OpenTask(task) => {
            (crate::control::exec::open_task(task, true, cx), None)
        }
        command::Resolution::Run(resolved, addressed) => (
            crate::control::exec::run(resolved, crate::control::exec::CommandOrigin::Phone, cx),
            addressed,
        ),
    };
    render_reply(&outcome, addressed, target, cx).map_or(super::Effect::None, super::Effect::Reply)
}

fn render_reply(
    outcome: &ControlOutcome,
    addressed: Option<PaneRef>,
    target: &super::Target,
    cx: &mut App,
) -> Option<command::RenderedReply> {
    let state = target.state(cx)?;
    let absorbed = command::absorb(outcome, addressed, state);
    Some(command::render(outcome, absorbed, state))
}

/// Start a task for the phone. The reply says whether it was accepted; once
/// git and the agent are up, the outcome follows as its own message — a
/// chat announced as the new target, or a notice.
fn start_task(task: String, cx: &mut App) -> ControlOutcome {
    let pending = crate::control::exec::begin_task_start(
        task,
        crate::control::exec::CommandOrigin::Phone,
        true,
        cx,
    )?;
    if let Some(outcome) = pending.try_finish(cx) {
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
    let (absorbed, state) = target.forget(pane, cx)?;
    Some(command::render(
        &Err(crate::control::result::ControlError::TargetGone),
        absorbed,
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
