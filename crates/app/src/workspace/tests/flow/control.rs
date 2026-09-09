//! `/flow` from the external control surface: which flows it lists, and the
//! three refusals that precede a run.
//!
//! The refusals are what these are really about. A run started from the
//! desktop reports a problem in a toast; a run started from a phone has no
//! toast to read, so anything the run would refuse *after* dispatch has to
//! become an answer before it.

use super::*;
use futures::{FutureExt as _, StreamExt as _};

use crate::control::result::{ControlError, FlowOriginKind};
use crate::workspace::flow_paths;

/// The next thing the phone is told, once the run that produces it finishes.
///
/// `now_or_never` in a loop rather than `await`: the sender lives in a global,
/// so an empty channel is `Pending` forever and awaiting it would turn a
/// missing relay into a hang. The retry is for the *engine*, which runs a real
/// subprocess — one `run_until_parked` only proves gpui has nothing left to
/// do, not that `true` has exited, and under a loaded test machine it has not.
fn outcome_reaches_the_phone(
    outbound: &mut futures::channel::mpsc::UnboundedReceiver<crate::telegram::bridge::Outbound>,
    cx: &mut TestAppContext,
) -> Option<crate::telegram::bridge::Outbound> {
    // The engine runs on a thread of its own and wakes this app from it,
    // which gpui's test scheduler otherwise reports as non-determinism. This
    // is the sanctioned opt-out for a test that awaits real work — and the
    // real engine is the point here: hand-settling would skip
    // `apply_flow_event`, the only production caller of the relay under test.
    cx.executor().allow_parking();
    for _ in 0..40 {
        cx.run_until_parked();
        if let Some(sent) = outbound.next().now_or_never().flatten() {
            return Some(sent);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    None
}

/// A second worktree in the same project, holding its own flows directory,
/// and *not* activated. What a targeted run has to be able to reach.
fn add_lane_with_a_flow(
    ws: &gpui::Entity<Workspace>,
    wh: gpui::WindowHandle<gpui_component::Root>,
    cx: &mut TestAppContext,
    file: &str,
    flow: &str,
) -> (tempfile::TempDir, daruda_store::project::LaneRef) {
    let dir = tempfile::tempdir().expect("tempdir");
    let flows = flow_paths::flows_dir(dir.path());
    std::fs::create_dir_all(&flows).expect("create flows dir");
    std::fs::write(flows.join(file), flow).expect("write flow");
    let target = ws.update(cx, |ws, _cx| {
        let project = ws.active.project;
        let lane_id = ws.alloc_id();
        let mut lane = crate::lane::Lane::default_for_project(lane_id, dir.path().to_path_buf());
        lane.tab_order = 1;
        ws.project_for_mut(project)
            .expect("the fixture's project")
            .lanes
            .push(lane);
        daruda_store::project::LaneRef {
            project,
            lane: lane_id,
        }
    });
    // Visit it and come back. Pushing the entry alone leaves the worktree
    // without the runtime a lane is expected to own, and the round trip is
    // what builds one — while leaving the original worktree active, which is
    // the whole point of the fixture.
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let back = ws.active;
            ws.activate_lane(target, window, cx);
            ws.activate_lane(back, window, cx);
        });
    })
    .expect("window is live");
    (dir, target)
}

/// A flow that stops to ask a person — the exact shape a phone cannot answer.
const ASKS_A_PERSON: &str = "\
version: 1
defaults:
  agent:
    id: claude
    mode: acceptEdits
    permission: ask
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write a line
";

/// A flow that needs no agent and no person: the one shape whose *successful*
/// start a test can assert without a live ACP session.
const COMMAND_ONLY: &str = "\
version: 1
nodes:
  - id: check
    kind: command
    run: \"true\"
";

/// [`workspace_with_a_flow`] with the bridge on and a chat paired — what the
/// relay tests need and the refusal tests above do not.
fn telegram_workspace_with_a_flow(
    cx: &mut TestAppContext,
    flow: &str,
) -> (
    tempfile::TempDir,
    gpui::Entity<Workspace>,
    std::path::PathBuf,
    gpui::WindowHandle<gpui_component::Root>,
) {
    let lane = tempfile::tempdir().expect("tempdir");
    let flows = flow_paths::flows_dir(lane.path());
    std::fs::create_dir_all(&flows).expect("create flows dir");
    let flow_path = flows.join("ship.yaml");
    std::fs::write(&flow_path, flow).expect("write flow");

    let config = daruda_config::Config {
        telegram: daruda_config::TelegramConfig {
            enabled: true,
            authorized_chat_id: Some(42),
            ..daruda_config::TelegramConfig::default()
        },
        ..daruda_config::Config::default()
    };
    let project = daruda_store::project::Project::from_path(lane.path());
    let (wh, ws) = build_workspace_with(cx, &config, Some(project));
    (lane, ws, flow_path, wh)
}

/// Names a dependency no node declares — a graph the engine refuses to build.
const DANGLING_DEP: &str = "\
version: 1
defaults:
  agent:
    id: claude
    mode: bypassPermissions
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write a line
    deps:
      - nowhere
";

fn write_global_flow(ws: &Workspace, name: &str, body: &str) {
    let dir = flow_paths::global_flows_dir(&ws.data_dir);
    std::fs::create_dir_all(&dir).expect("create global flows dir");
    std::fs::write(dir.join(name), body).expect("write global flow");
}

#[gpui::test]
async fn the_listing_names_every_flow_with_the_scope_it_came_from(cx: &mut TestAppContext) {
    let (_lane, ws, _path, _wh) = workspace_with_a_flow(cx, ONE_AGENT);
    ws.update(cx, |ws, _| write_global_flow(ws, "mine.yaml", ONE_AGENT));

    ws.read_with(cx, |ws, _| {
        let entries = ws.control_flow_list();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"ship.yaml"), "the repo's own: {names:?}");
        assert!(names.contains(&"mine.yaml"), "the person's own: {names:?}");
        let ship = entries
            .iter()
            .find(|e| e.name == "ship.yaml")
            .expect("ship");
        assert_eq!(ship.origin, FlowOriginKind::Repo);
        let mine = entries
            .iter()
            .find(|e| e.name == "mine.yaml")
            .expect("mine");
        assert_eq!(mine.origin, FlowOriginKind::Global);
    });
}

/// The repo's copy shadows the person's, so a name resolves to exactly one
/// file and there is no ambiguity for the caller to disambiguate.
#[gpui::test]
async fn a_name_held_by_two_scopes_resolves_to_the_narrower_one(cx: &mut TestAppContext) {
    let (_lane, ws, _path, _wh) = workspace_with_a_flow(cx, ONE_AGENT);
    ws.update(cx, |ws, _| write_global_flow(ws, "ship.yaml", ONE_AGENT));

    ws.read_with(cx, |ws, _| {
        let ship: Vec<_> = ws
            .control_flow_list()
            .into_iter()
            .filter(|e| e.name == "ship.yaml")
            .collect();
        assert_eq!(ship.len(), 1, "one entry per name: {ship:?}");
        assert_eq!(ship[0].origin, FlowOriginKind::Repo);
    });
}

#[gpui::test]
async fn an_unknown_flow_name_is_refused(cx: &mut TestAppContext) {
    let (_lane, ws, _path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            assert_eq!(
                ws.control_flow_run(ws.active, "nope", window, cx),
                Err(ControlError::FlowNotFound {
                    name: "nope".into()
                })
            );
        });
    })
    .expect("window is live");
}

/// A phone should be able to type `deploy`, not `deploy.yaml`.
#[gpui::test]
async fn a_name_resolves_with_or_without_its_extension(cx: &mut TestAppContext) {
    let (_lane, ws, _path, wh) = workspace_with_a_flow(cx, ASKS_A_PERSON);
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            // Both spellings reach the same file, and so hit the same refusal
            // — which is what proves resolution happened at all.
            for name in ["ship", "ship.yaml"] {
                assert_eq!(
                    ws.control_flow_run(ws.active, name, window, cx),
                    Err(ControlError::FlowNeedsInteraction {
                        name: "ship.yaml".into()
                    }),
                    "{name} must resolve"
                );
            }
        });
    })
    .expect("window is live");
}

#[gpui::test]
async fn a_flow_that_asks_a_person_is_refused_before_it_can_hang(cx: &mut TestAppContext) {
    let (_lane, ws, _path, wh) = workspace_with_a_flow(cx, ASKS_A_PERSON);
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            assert_eq!(
                ws.control_flow_run(ws.active, "ship", window, cx),
                Err(ControlError::FlowNeedsInteraction {
                    name: "ship.yaml".into()
                })
            );
        });
    })
    .expect("window is live");
}

/// A file declaring profiles opens the picker's second question, which is a
/// desktop dialog like any other.
#[gpui::test]
async fn a_flow_declaring_profiles_is_refused_for_the_same_reason(cx: &mut TestAppContext) {
    let (_lane, ws, _path, wh) = workspace_with_a_flow(cx, WITH_PROFILES);
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            assert_eq!(
                ws.control_flow_run(ws.active, "ship", window, cx),
                Err(ControlError::FlowNeedsInteraction {
                    name: "ship.yaml".into()
                })
            );
        });
    })
    .expect("window is live");
}

/// The regression this guards: reporting a run as started that the desktop
/// then refuses in a toast the phone never sees.
#[gpui::test]
async fn a_flow_that_will_not_run_is_answered_not_reported_as_started(cx: &mut TestAppContext) {
    let (_lane, ws, _path, wh) = workspace_with_a_flow(cx, DANGLING_DEP);
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            assert_eq!(
                ws.control_flow_run(ws.active, "ship", window, cx),
                Err(ControlError::FlowRefused {
                    name: "ship.yaml".into()
                })
            );
        });
    })
    .expect("window is live");
}

/// With no worktree open anywhere there is nowhere to run, and that is the
/// answer. The refusal now belongs to the step that *picks* a worktree:
/// `control_flow_run` is told one, so a missing one is `TargetGone` (see
/// `a_worktree_that_is_gone_is_not_reported_as_a_missing_flow`).
#[gpui::test]
async fn running_a_flow_without_an_active_lane_is_refused(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let (wh, ws) = build_workspace_with(cx, &config, None);
    cx.update(|cx| {
        crate::window_registry::WindowRegistry::register(wh.into(), ws.downgrade(), cx);
    });
    cx.update(|cx| {
        assert_eq!(
            crate::control::exec::first_lane_offering("ship", cx),
            Err(ControlError::NoActiveLane)
        );
    });
}

/// The headline path, and the one every refusal test above is the negative of:
/// a runnable flow is dispatched, is reported under the file that resolved,
/// and — the invariant the whole `needs_desktop_answer` / profile machinery
/// exists for — raises no desktop dialog on the way.
#[gpui::test]
async fn a_runnable_flow_starts_and_names_the_file_that_ran(cx: &mut TestAppContext) {
    let (_lane, ws, _path, wh) = workspace_with_a_flow(cx, COMMAND_ONLY);
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let entry = ws
                .control_flow_run(ws.active, "ship", window, cx)
                .expect("a command-only flow needs neither an agent nor a person");
            assert_eq!(entry.name, "ship.yaml", "named by the file, not the input");
            assert_eq!(entry.origin, FlowOriginKind::Repo);
            assert!(
                !ws.flow_picker.is_open(),
                "a phone command must never raise a desktop dialog"
            );
        });
    })
    .expect("window is live");
}

/// A lane already running a flow refuses the next one — and, critically,
/// refuses it *as an answer* rather than by opening the desktop "stop it?"
/// picker the guard would otherwise show.
#[gpui::test]
async fn a_lane_already_running_a_flow_refuses_the_next(cx: &mut TestAppContext) {
    let (_lane, ws, _path, wh) = workspace_with_a_flow(cx, COMMAND_ONLY);
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let lane_ref = ws.active_ref();
            ws.seed_flow_run_for_test(lane_ref, std::path::PathBuf::from("/tmp/seeded-run"));
            assert_eq!(
                ws.control_flow_run(ws.active, "ship", window, cx),
                Err(ControlError::FlowLocked {
                    name: "ship.yaml".into()
                })
            );
            assert!(
                !ws.flow_picker.is_open(),
                "the refusal is the answer, not a dialog"
            );
        });
    })
    .expect("window is live");
}

/// The reason a run names its worktree: it lands where the caller said, and a
/// flow only that worktree holds is reachable only through it. Before this,
/// the answer was the first place a caller learned where its run had gone.
#[gpui::test]
async fn a_flow_runs_in_the_named_worktree_not_the_active_one(cx: &mut TestAppContext) {
    let (_lane, ws, _path, wh) = workspace_with_a_flow(cx, COMMAND_ONLY);
    let (_other_dir, other) = add_lane_with_a_flow(&ws, wh, cx, "deploy.yaml", COMMAND_ONLY);

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let active = ws.active;
            assert_ne!(other, active, "the fixture must give two worktrees");
            // The worktree on screen does not hold it...
            assert_eq!(
                ws.control_flow_run(active, "deploy", window, cx),
                Err(ControlError::FlowNotFound {
                    name: "deploy".into()
                }),
                "resolution is worktree-scoped, not window-wide"
            );
            // ...and the one that does runs it, and says so.
            let entry = ws
                .control_flow_run(other, "deploy", window, cx)
                .expect("the named worktree holds it");
            assert_eq!(entry.name, "deploy.yaml");
            assert_eq!(entry.lane.lane_ref(), other, "ran where it was told");
            assert_eq!(
                ws.active, active,
                "a targeted run must not move what the person is looking at"
            );
        });
    })
    .expect("window is live");
}

/// A named worktree that is gone gets its own answer. `FlowNotFound` would
/// send the caller looking for a file when what is missing is the place it
/// would have run.
#[gpui::test]
async fn a_worktree_that_is_gone_is_not_reported_as_a_missing_flow(cx: &mut TestAppContext) {
    let (_lane, ws, _path, wh) = workspace_with_a_flow(cx, COMMAND_ONLY);
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let gone = daruda_store::project::LaneRef {
                project: ws.active.project,
                lane: ws.active.lane + 9,
            };
            assert_eq!(
                ws.control_flow_run(gone, "ship", window, cx),
                Err(ControlError::TargetGone)
            );
        });
    })
    .expect("window is live");
}

/// A worktree that is not on screen still refuses *as an answer*. The guard
/// it goes through opens the desktop "stop it?" picker for the active lane,
/// and a targeted run must not be able to raise one.
#[gpui::test]
async fn a_named_worktree_already_running_refuses_without_a_dialog(cx: &mut TestAppContext) {
    let (_lane, ws, _path, wh) = workspace_with_a_flow(cx, COMMAND_ONLY);
    let (_other_dir, other) = add_lane_with_a_flow(&ws, wh, cx, "deploy.yaml", COMMAND_ONLY);

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.seed_flow_run_for_test(other, std::path::PathBuf::from("/tmp/seeded-elsewhere"));
            assert_eq!(
                ws.control_flow_run(other, "deploy", window, cx),
                Err(ControlError::FlowLocked {
                    name: "deploy.yaml".into()
                })
            );
            assert!(
                !ws.flow_picker.is_open(),
                "the refusal is the answer, not a dialog"
            );
        });
    })
    .expect("window is live");
}

/// A run that ends in a worktree nobody is looking at settles without opening
/// its report there.
///
/// `open_pane_file_view` pushes into the *active* runtime while stamping the
/// pane with the owner it is handed, so a parked lane's report built a pane
/// whose owner named one runtime and whose home was another.
/// `load_pane_file_content` then resolves it by owner, misses, and drops the
/// content — the pane sits on "Loading" for good. Reachable from the desktop
/// alone (start a run, switch worktrees, wait), and the invariant is guarded
/// by `git_ops::file_view::debug_assert_owner_is_active`, which is what makes
/// simply driving the event here a sufficient assertion.
#[gpui::test]
async fn a_run_ending_in_a_parked_worktree_opens_no_report_there(cx: &mut TestAppContext) {
    let (lane, ws, _path, wh) = workspace_with_a_flow(cx, COMMAND_ONLY);
    let (_other_dir, other) = add_lane_with_a_flow(&ws, wh, cx, "deploy.yaml", COMMAND_ONLY);

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let active = ws.active;
            assert_ne!(other, active);
            ws.seed_flow_run_for_test(other, lane.path().join("run-elsewhere"));
            ws.apply_flow_event_with_window_for_test(
                other,
                &daruda_flow::event::FlowEvent::RunEnded {
                    end: daruda_flow::event::RunEnd::Done,
                },
                window,
                cx,
            );
            assert!(
                !ws.runs.is_running(other),
                "settling still has to happen — only the pane is withheld"
            );
            assert_eq!(ws.active, active, "and the screen must not move");
        });
    })
    .expect("window is live");
}

/// A text adapter names no worktree, so the executor picks one — and has to
/// keep the two refusals apart. "Nowhere to run" and "no such flow" send a
/// person to look at different things.
#[gpui::test]
async fn the_default_target_separates_no_flow_from_no_worktree(cx: &mut TestAppContext) {
    let (_lane, ws, _path, wh) = workspace_with_a_flow(cx, COMMAND_ONLY);
    cx.update(|cx| {
        crate::window_registry::WindowRegistry::register(wh.into(), ws.downgrade(), cx);
    });
    cx.update(|cx| {
        assert!(
            crate::control::exec::first_lane_offering("ship", cx).is_ok(),
            "the open worktree holds it"
        );
        assert_eq!(
            crate::control::exec::first_lane_offering("nope", cx),
            Err(ControlError::FlowNotFound {
                name: "nope".into()
            }),
            "a worktree is open, it just does not hold that flow"
        );
    });
}

/// A run asked for from a phone has to be told how it ended. Without this the
/// `/flow` answer is the last thing the caller ever hears: the toast and the
/// run report both land on a desktop they are not at.
#[gpui::test]
async fn a_phone_started_run_reports_its_outcome_back(cx: &mut TestAppContext) {
    let mut outbound =
        cx.update(|cx| crate::telegram::global::install_for_test(true, Some(42), cx));
    let lane = tempfile::tempdir().expect("tempdir");
    let flows = crate::workspace::flow_paths::flows_dir(lane.path());
    std::fs::create_dir_all(&flows).expect("create flows dir");
    std::fs::write(flows.join("ship.yaml"), COMMAND_ONLY).expect("write flow");

    let mut config = daruda_config::Config::default();
    config.telegram.enabled = true;
    config.telegram.authorized_chat_id = Some(42);
    let project = daruda_store::project::Project::from_path(lane.path());
    let (wh, ws) = build_workspace_with(cx, &config, Some(project));

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.control_flow_run(ws.active, "ship", window, cx)
                .expect("dispatched");
        });
    })
    .expect("window is live");

    // The *real* engine and the *real* event pump produce the outcome — the
    // flow is `run: "true"`, so it finishes on its own.
    let sent = outcome_reaches_the_phone(&mut outbound, cx).expect("the outcome reaches the phone");
    match sent {
        crate::telegram::bridge::Outbound::Notice(text) => {
            assert_eq!(
                text,
                crate::surface::strings::control_flow_finished_notice()
            );
        }
        // A ping would register a reply-to, so answering "flow finished" would
        // prompt whichever agent spoke last.
        crate::telegram::bridge::Outbound::Approval(prompt) => {
            panic!("a flow outcome asks nothing: {prompt:?}")
        }
        crate::telegram::bridge::Outbound::Ping(ping) => {
            panic!("a flow outcome must not be attributed to a pane: {ping:?}")
        }
    }
}

/// A run nobody asked for remotely stays off the phone entirely — the person
/// who started it is at the desktop, reading the toast.
#[gpui::test]
async fn a_desktop_started_run_stays_off_the_phone(cx: &mut TestAppContext) {
    let mut outbound =
        cx.update(|cx| crate::telegram::global::install_for_test(true, Some(42), cx));
    let (lane, ws, _path, _wh) = telegram_workspace_with_a_flow(cx, COMMAND_ONLY);

    ws.update(cx, |ws, cx| {
        let lane_ref = ws.active_ref();
        // Seeded, i.e. not marked as owing anyone an answer.
        ws.seed_flow_run_for_test(lane_ref, lane.path().join("run"));
        let _ = ws.settle_flow_run(lane_ref, &daruda_flow::event::RunEnd::Done, cx);
    });

    assert!(
        outbound.next().now_or_never().is_none(),
        "a desktop run must not ping the phone"
    );
}

/// Stop leaves the phone hanging is precisely the class of bug this relay
/// exists to kill, so the cancelled outcome gets its own test.
#[gpui::test]
async fn a_cancelled_phone_started_run_still_reports_back(cx: &mut TestAppContext) {
    let mut outbound =
        cx.update(|cx| crate::telegram::global::install_for_test(true, Some(42), cx));
    let (_lane, ws, _path, wh) = telegram_workspace_with_a_flow(cx, COMMAND_ONLY);

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.control_flow_run(ws.active, "ship", window, cx)
                .expect("dispatched");
            // `cancel` trips the token and leaves the handle in place, so the
            // engine returns and the run still gets to say how it ended.
            let lane_ref = ws.active_ref();
            ws.runs.cancel(lane_ref);
        });
    })
    .expect("window is live");
    let sent = outcome_reaches_the_phone(&mut outbound, cx).expect("a stopped run still answers");
    let crate::telegram::bridge::Outbound::Notice(text) = sent else {
        panic!("a flow outcome must not be attributed to a pane");
    };
    assert!(
        !text.is_empty(),
        "the phone is told something rather than left waiting"
    );
}
