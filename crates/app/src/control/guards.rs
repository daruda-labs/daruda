//! Three refusals that keep an autonomous agent from running away.
//!
//! The approval gate does most of the work — a creation loop cannot spin
//! without a human tapping each time — so these cover what a gate cannot: a
//! prompt loop with no creation in it, an unbounded queue, and a budget for
//! the case where the gate is approved as fast as it appears.
//!
//! App-level, not per-window, because that is where a tool call enters and
//! because two of the three are app-level facts: which pane the orchestrator
//! is (one per process) and how many worktrees the agent has made (its
//! budget, not a window's — the tools name a project, so a per-window counter
//! would grant the budget once per window).
//!
//! The limits are constants rather than settings on purpose. They bound
//! automation the user is not watching, and a lane agent with file tools
//! could raise a number in `config.toml` — so there is nothing to raise.
//!
//! **Two of the three are applied by the caller, not inherited.**
//! [`guard_agent_budget`] sits inside `exec::run_gated`, so every adapter gets
//! it; [`guard_self_target`] and [`guard_queue_depth`] are applied by
//! `orchestrator::control::guard_immediate`, i.e. on the MCP path only. That
//! is deliberate — Telegram's `/say` is a person, and refusing them for
//! "addressing the orchestrator" would refuse the whole point of the app — but
//! a new adapter driving an *agent* has to apply those two itself.

use gpui::{App, Global};

use crate::control::result::ControlError;
use crate::telegram::bridge::PaneRef;
use crate::window_registry::WindowRegistry;

/// Pending prompts one pane may hold. The queue drains one per turn, so an
/// agent pushing faster than turns complete would grow it without bound.
pub(crate) const QUEUE_DEPTH_MAX: usize = 10;

/// Worktrees the agent may create in one run of the app. Counts *only*
/// agent-created ones — a user with twenty lanes open must not find the
/// orchestrator unable to work.
pub(crate) const AGENT_LANE_BUDGET: u32 = 16;

/// How much of its budget the agent has spent.
///
/// Runtime only. A restart resetting it is the intent: the budget exists to
/// bound one unattended stretch, and the user relaunching the app is the
/// attention it was guarding against the absence of.
#[derive(Default)]
struct AgentBudget {
    created_lanes: u32,
}

impl Global for AgentBudget {}

/// A prompt from the orchestrator to itself makes a turn that makes a turn.
/// With lane agents holding no tools, this is the only pure software loop
/// left.
pub(crate) fn guard_self_target(target: PaneRef, cx: &App) -> Result<(), ControlError> {
    if crate::orchestrator::pane(cx) == Some(target) {
        return Err(ControlError::SelfTargetRefused);
    }
    Ok(())
}

/// Refuse a prompt for a pane that is already holding its limit.
///
/// A pre-check, not the enforcement: `AgentChatView::enqueue_prompt` caps the
/// queue itself, so a phone `/say` and in-app typing get the same bound. This
/// exists so a tool call is told *why* instead of watching its prompt vanish.
pub(crate) fn guard_queue_depth(target: PaneRef, cx: &mut App) -> Result<(), ControlError> {
    let mut depth = None;
    WindowRegistry::for_each_workspace(cx, |ws, _window, cx| {
        if ws.uuid() == target.workspace {
            depth = ws.agent_chat_queue_depth(target.pane, cx);
        }
    });
    match depth {
        // A pane that is not there is the caller's other error, not this one.
        None => Ok(()),
        Some(depth) if depth >= QUEUE_DEPTH_MAX => Err(ControlError::QueueFull),
        Some(_) => Ok(()),
    }
}

/// Refuse a creation the agent has no budget left for. Checked before the
/// approval card, since there is nothing to ask the user about.
pub(crate) fn guard_agent_budget(cx: &mut App) -> Result<(), ControlError> {
    if cx.default_global::<AgentBudget>().created_lanes >= AGENT_LANE_BUDGET {
        return Err(ControlError::AgentLimitReached);
    }
    Ok(())
}

/// Spend one worktree of the budget. Called after a creation actually
/// succeeded — a refused or failed one costs nothing.
pub(crate) fn note_agent_created_lane(cx: &mut App) {
    let budget = cx.default_global::<AgentBudget>();
    budget.created_lanes = budget.created_lanes.saturating_add(1);
}

#[cfg(test)]
pub(crate) fn spent_budget_for_test(cx: &App) -> u32 {
    cx.try_global::<AgentBudget>()
        .map_or(0, |b| b.created_lanes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext as _, TestAppContext};

    fn pane(n: u64) -> PaneRef {
        PaneRef {
            workspace: daruda_store::project::WorkspaceUuid::new(),
            pane: n,
        }
    }

    #[gpui::test]
    fn the_orchestrator_cannot_target_itself(cx: &mut TestAppContext) {
        let own = crate::test_support::register_test_orchestrator(cx);
        cx.update(|cx| {
            assert_eq!(
                guard_self_target(own, cx),
                Err(ControlError::SelfTargetRefused)
            );
        });
    }

    #[gpui::test]
    fn a_non_self_target_passes(cx: &mut TestAppContext) {
        let own = crate::test_support::register_test_orchestrator(cx);
        cx.update(|cx| {
            let other = PaneRef {
                workspace: own.workspace,
                pane: own.pane + 1,
            };
            assert_eq!(guard_self_target(other, cx), Ok(()));
            assert_eq!(guard_self_target(pane(own.pane), cx), Ok(()));
        });
    }

    /// With no orchestrator up, nothing is self — the guard must not refuse
    /// every pane just because the comparison has no left-hand side.
    #[gpui::test]
    fn nothing_is_self_when_no_orchestrator_is_running(cx: &mut TestAppContext) {
        cx.update(|cx| assert_eq!(guard_self_target(pane(0), cx), Ok(())));
    }

    #[gpui::test]
    fn a_full_queue_refuses_another_prompt(cx: &mut TestAppContext) {
        let fixture = crate::test_support::workspace_with_agent_chat(cx);
        let target = fixture
            .workspace
            .read_with(cx, |ws, cx| ws.control_snapshot(cx)[0].1.target);

        cx.update(|cx| assert_eq!(guard_queue_depth(target, cx), Ok(())));

        fixture.workspace.update(cx, |ws, cx| {
            ws.fill_prompt_queue_for_test(target.pane, QUEUE_DEPTH_MAX, cx)
        });
        cx.update(|cx| {
            assert_eq!(guard_queue_depth(target, cx), Err(ControlError::QueueFull));
        });
    }

    /// A pane the caller named wrongly is `TargetGone`'s business; this guard
    /// has no opinion, or a bad handle would report the wrong reason.
    #[gpui::test]
    fn an_unknown_pane_is_not_this_guards_refusal(cx: &mut TestAppContext) {
        let _fixture = crate::test_support::workspace_with_agent_chat(cx);
        cx.update(|cx| assert_eq!(guard_queue_depth(pane(9_999), cx), Ok(())));
    }

    /// Lanes the user opened must not consume the agent's budget.
    #[gpui::test]
    fn the_budget_counts_only_agent_created_lanes(cx: &mut TestAppContext) {
        let fixture = crate::test_support::workspace_with_agent_chat(cx);
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture.workspace.update(cx, |ws, cx| {
                ws.open_agent_chat_panes_in_fresh_lanes_for_test(3, window, cx)
            });
        })
        .expect("window is live");

        cx.update(|cx| {
            assert_eq!(guard_agent_budget(cx), Ok(()));
            for _ in 0..AGENT_LANE_BUDGET {
                note_agent_created_lane(cx);
            }
            assert_eq!(spent_budget_for_test(cx), AGENT_LANE_BUDGET);
            assert_eq!(guard_agent_budget(cx), Err(ControlError::AgentLimitReached));
        });
    }

    /// The counter must not wrap back into an allowance.
    #[gpui::test]
    fn a_spent_budget_stays_spent(cx: &mut TestAppContext) {
        cx.update(|cx| {
            for _ in 0..AGENT_LANE_BUDGET + 4 {
                note_agent_created_lane(cx);
            }
            assert_eq!(guard_agent_budget(cx), Err(ControlError::AgentLimitReached));
        });
    }
}
