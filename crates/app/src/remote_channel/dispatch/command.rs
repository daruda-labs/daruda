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
