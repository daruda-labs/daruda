//! App-level dispatch for control commands.
//!
//! Two shapes. Enumeration walks every open window, because `/list` and
//! `/brief` answer about the app, not about one window. Targeted commands
//! resolve their `PaneRef`'s workspace and run there.
//!
//! The enumeration walk collects plain data inside the per-window callback and
//! assembles the result afterwards — building it inside would read entities
//! while their workspace is mid-`update` (CLAUDE.md pitfall 5).

use gpui::App;

use crate::control::result::{
    Activity, BriefSummary, ChatSummary, ControlError, ControlOutcome, ControlResult, FlowEntry,
    Health, LaneGroup, Listing, ProjectGroup, WindowGroup,
};
use crate::control::spec::{FlowCommand, ResolvedCommand};
use crate::telegram::bridge::PaneRef;
use crate::window_registry::WindowRegistry;
use crate::workspace::Workspace;
use daruda_store::project::ProjectId;

/// One row of the flat walk, before grouping.
struct Row {
    window: u32,
    /// Identity, not the name: two projects opened from different directories
    /// can share a basename, and grouping by name would merge them into one
    /// listing section that misreports the structure.
    project_id: ProjectId,
    project: String,
    lane: String,
    lane_order: u32,
    summary: ChatSummary,
}

pub(crate) fn listing(cx: &mut App) -> Listing {
    Listing {
        windows: group(collect_rows(cx)),
        omitted: 0,
    }
}

pub(crate) fn brief(cx: &mut App) -> BriefSummary {
    let rows = collect_rows(cx);
    BriefSummary {
        working: count(&rows, |s| s.activity == Activity::Working),
        awaiting_permission: count(&rows, |s| s.activity == Activity::AwaitingPermission),
        error: count(&rows, |s| s.health == Health::Error),
        total: rows.len() as u32,
    }
}

pub(crate) fn run(cmd: ResolvedCommand, cx: &mut App) -> ControlOutcome {
    match cmd {
        ResolvedCommand::List => Ok(ControlResult::Listing(listing(cx))),
        ResolvedCommand::Brief => Ok(ControlResult::Brief(brief(cx))),
        ResolvedCommand::Say { target, text } => in_workspace(target, cx, |ws, _window, cx| {
            ws.control_say(target.pane, text.clone(), cx)
        })
        .map(|disposition| ControlResult::Sent {
            target,
            disposition,
        }),
        ResolvedCommand::Stop { target } => in_workspace(target, cx, |ws, _window, cx| {
            ws.control_stop(target.pane, cx)
        })
        .map(|disposition| ControlResult::Stopped {
            target,
            disposition,
        }),
        // A flow is lane-scoped, and which flows a lane can run depends on its
        // repository — so two windows genuinely see different sets and neither
        // arm may stop at the first window.
        ResolvedCommand::Flow(FlowCommand::List) => {
            let mut flows: Vec<FlowEntry> = Vec::new();
            WindowRegistry::for_each_workspace(cx, |ws, _window, _cx| {
                flows.extend(ws.control_flow_list());
            });
            // A name can legitimately appear in two windows' repositories.
            // De-duplicate on (name, origin) so the list reads as a set of
            // runnable names rather than repeating one per window.
            flows.sort_by(|a, b| a.name.cmp(&b.name).then(a.origin.cmp(&b.origin)));
            flows.dedup();
            Ok(ControlResult::FlowList { flows })
        }
        ResolvedCommand::Flow(FlowCommand::Run { name }) => {
            let mut out = Err(ControlError::NoActiveLane);
            WindowRegistry::for_each_workspace(cx, |ws, window, cx| {
                // Retry on the two refusals another window can answer
                // differently: it may have an active lane where this one has
                // none, and its repository may hold a flow this one lacks.
                // Neither started anything, so retrying cannot double-start.
                // Every other refusal is about the file and would repeat.
                if matches!(
                    out,
                    Err(ControlError::NoActiveLane) | Err(ControlError::FlowNotFound { .. })
                ) {
                    out = ws.control_flow_run(&name, window, cx);
                }
            });
            // Named by the file that resolved, not by what the user typed —
            // `/flow ship` may pick `ship.yaml` or `ship.yml`, and the answer
            // has to say which.
            out.map(|entry| ControlResult::FlowStarting {
                name: entry.name,
                origin: entry.origin,
            })
        }
    }
}

fn count(rows: &[Row], pred: impl Fn(&ChatSummary) -> bool) -> u32 {
    rows.iter().filter(|r| pred(&r.summary)).count() as u32
}

fn collect_rows(cx: &mut App) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut window = 0u32;
    WindowRegistry::for_each_workspace(cx, |ws, _win, cx| {
        for (lane_ref, summary) in ws.control_snapshot(cx) {
            rows.push(Row {
                window,
                project_id: lane_ref.project,
                project: ws.control_project_name(lane_ref.project),
                lane: ws.control_lane_name(lane_ref),
                lane_order: ws.control_lane_tab_order(lane_ref),
                summary,
            });
        }
        window += 1;
    });
    // `runtimes` is a HashMap, and window registration order is not a
    // guarantee we want ordinals to depend on. Sort once, here.
    rows.sort_by(|a, b| {
        a.window
            .cmp(&b.window)
            // Name first so the listing reads alphabetically, then id so two
            // same-named projects stay apart instead of interleaving.
            .then_with(|| a.project.cmp(&b.project))
            .then_with(|| a.project_id.cmp(&b.project_id))
            .then_with(|| a.lane_order.cmp(&b.lane_order))
            .then_with(|| a.summary.target.pane.cmp(&b.summary.target.pane))
    });
    rows
}

fn group(rows: Vec<Row>) -> Vec<WindowGroup> {
    let mut windows: Vec<WindowGroup> = Vec::new();
    // Tracks which project the open group belongs to, since `ProjectGroup`
    // carries only the name and two projects can share one.
    let mut open_project: Option<ProjectId> = None;
    for row in rows {
        let win = match windows.last_mut() {
            Some(w) if w.index == row.window => w,
            _ => {
                windows.push(WindowGroup {
                    index: row.window,
                    projects: Vec::new(),
                });
                windows.last_mut().expect("just pushed")
            }
        };
        let proj = match win.projects.last_mut() {
            Some(p) if open_project == Some(row.project_id) => p,
            _ => {
                open_project = Some(row.project_id);
                win.projects.push(ProjectGroup {
                    name: row.project,
                    lanes: Vec::new(),
                });
                win.projects.last_mut().expect("just pushed")
            }
        };
        match proj.lanes.last_mut() {
            Some(l) if l.name == row.lane => l.chats.push(row.summary),
            _ => proj.lanes.push(LaneGroup {
                name: row.lane,
                chats: vec![row.summary],
            }),
        }
    }
    windows
}

/// Run `f` against the workspace `target` names. `WindowRegistry` has no
/// uuid lookup, so this is the same scan `telegram::global::dispatch_to_workspace`
/// does — kept separate because that one returns nothing.
fn in_workspace<T>(
    target: PaneRef,
    cx: &mut App,
    mut f: impl FnMut(
        &mut Workspace,
        &mut gpui::Window,
        &mut gpui::Context<Workspace>,
    ) -> Result<T, ControlError>,
) -> Result<T, ControlError> {
    let mut out = Err(ControlError::TargetGone);
    WindowRegistry::for_each_workspace(cx, |ws, window, cx| {
        if ws.uuid() == target.workspace {
            out = f(ws, window, cx);
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{workspace_for_control, workspace_with_agent_chat};
    use gpui::AppContext as _;

    #[gpui::test]
    async fn listing_spans_every_window(cx: &mut gpui::TestAppContext) {
        let _a = workspace_with_agent_chat(cx);
        let _b = workspace_with_agent_chat(cx);
        cx.update(|cx| {
            let listing = listing(cx);
            assert_eq!(listing.windows.len(), 2, "both windows enumerated");
            assert_eq!(listing.windows[0].index, 0);
            assert_eq!(listing.windows[1].index, 1);
        });
    }

    /// Calling `listing` twice against an unmutated `HashMap` yields the same
    /// order whether or not `collect_rows` sorts — `RandomState` varies per
    /// map, not per iteration — so equality alone proves nothing. This asserts
    /// the *expected* sequence over a fixture with several lanes, which is
    /// what actually fails if the sort is dropped.
    #[gpui::test]
    async fn listing_order_follows_the_sort_key_not_hashmap_order(cx: &mut gpui::TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        // Extra lanes, each with a chat pane, so `runtimes` holds enough
        // entries for hash order to diverge from tab order.
        let extra = cx
            .update_window(fixture.window.into(), |_, window, cx| {
                fixture.workspace.update(cx, |ws, cx| {
                    ws.open_agent_chat_panes_in_fresh_lanes_for_test(3, window, cx)
                })
            })
            .expect("window is live");

        let panes: Vec<u64> = cx.update(|cx| {
            listing(cx)
                .windows
                .into_iter()
                .flat_map(|w| w.projects)
                .flat_map(|p| p.lanes)
                .flat_map(|l| l.chats)
                .map(|c| c.target.pane)
                .collect()
        });

        let mut expected = vec![fixture.pane()];
        expected.extend(extra);
        expected.sort_unstable();
        assert_eq!(
            panes, expected,
            "rows must follow (window, project, lane order, pane), not hash order"
        );
    }

    /// The counts `/brief` answers with, each non-zero and each distinct — the
    /// earlier test only checked `total`, so three of the four fields of the
    /// reply were unverified.
    #[gpui::test]
    async fn brief_counts_each_state_separately(cx: &mut gpui::TestAppContext) {
        let fixture = workspace_with_agent_chat(cx);
        let panes = cx
            .update_window(fixture.window.into(), |_, window, cx| {
                fixture.workspace.update(cx, |ws, cx| {
                    ws.open_agent_chat_panes_in_fresh_lanes_for_test(2, window, cx)
                })
            })
            .expect("window is live");

        fixture.workspace.update(cx, |ws, cx| {
            ws.set_pane_working_for_test(fixture.pane(), cx);
            ws.set_pane_awaiting_permission_for_test(panes[0], cx);
            ws.set_pane_errored_for_test(panes[1], cx);
        });

        cx.update(|cx| {
            let brief = brief(cx);
            assert_eq!(brief.total, 3);
            assert_eq!(brief.working, 1, "{brief:?}");
            assert_eq!(brief.awaiting_permission, 1, "{brief:?}");
            assert_eq!(brief.error, 1, "{brief:?}");
        });
    }

    #[gpui::test]
    async fn brief_counts_across_windows(cx: &mut gpui::TestAppContext) {
        let _a = workspace_with_agent_chat(cx);
        let _b = workspace_with_agent_chat(cx);
        cx.update(|cx| {
            assert_eq!(brief(cx).total, 2);
        });
    }

    #[gpui::test]
    async fn a_window_with_no_chat_pane_contributes_no_rows(cx: &mut gpui::TestAppContext) {
        let _empty = workspace_for_control(cx);
        cx.update(|cx| {
            assert!(listing(cx).windows.is_empty());
            assert_eq!(brief(cx).total, 0);
        });
    }

    #[gpui::test]
    async fn say_to_an_unknown_workspace_is_target_gone(cx: &mut gpui::TestAppContext) {
        let _a = workspace_with_agent_chat(cx);
        cx.update(|cx| {
            let target = PaneRef {
                workspace: daruda_store::project::WorkspaceUuid::new(),
                pane: 1,
            };
            assert_eq!(
                run(
                    ResolvedCommand::Say {
                        target,
                        text: "hi".into()
                    },
                    cx
                ),
                Err(ControlError::TargetGone)
            );
        });
    }
}
