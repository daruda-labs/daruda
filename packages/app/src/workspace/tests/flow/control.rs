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
    outbound: &mut crate::telegram::global::TestOutbox,
    cx: &mut TestAppContext,
) -> Option<crate::telegram::bridge::Outbound> {
    // The real engine is the point here: hand-settling would skip
    // `apply_flow_event`, the only production caller of the relay under test.
    // It wakes this app from its own thread, so the caller allowed parking
    // before starting it; this keeps the waiting below allowed too.
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
    let built = build_workspace_with(cx, &config, Some(project));
    let (wh, ws) = registered(cx, built);
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
    cx.update_window(wh.into(), |_, _, cx| {
        ws.update(cx, |ws, cx| {
            assert_eq!(
                ws.control_flow_run(
                    ws.active,
                    "nope",
                    daruda_flow::request::StartedBy::Phone,
                    cx
                ),
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
    cx.update_window(wh.into(), |_, _, cx| {
        ws.update(cx, |ws, cx| {
            // Both spellings reach the same file, and so hit the same refusal
            // — which is what proves resolution happened at all.
            for name in ["ship", "ship.yaml"] {
                assert_eq!(
                    ws.control_flow_run(
                        ws.active,
                        name,
                        daruda_flow::request::StartedBy::Phone,
                        cx
                    ),
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
    cx.update_window(wh.into(), |_, _, cx| {
        ws.update(cx, |ws, cx| {
            assert_eq!(
                ws.control_flow_run(
                    ws.active,
                    "ship",
                    daruda_flow::request::StartedBy::Phone,
                    cx
                ),
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
    cx.update_window(wh.into(), |_, _, cx| {
        ws.update(cx, |ws, cx| {
            assert_eq!(
                ws.control_flow_run(
                    ws.active,
                    "ship",
                    daruda_flow::request::StartedBy::Phone,
                    cx
                ),
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
    cx.update_window(wh.into(), |_, _, cx| {
        ws.update(cx, |ws, cx| {
            assert_eq!(
                ws.control_flow_run(
                    ws.active,
                    "ship",
                    daruda_flow::request::StartedBy::Phone,
                    cx
                ),
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
    // Really starts a run, so the engine wakes the app from its own thread —
    // the sanctioned opt-out, as in `outcome_reaches_the_phone`.
    cx.executor().allow_parking();
    let (_lane, ws, _path, wh) = workspace_with_a_flow(cx, COMMAND_ONLY);
    cx.update_window(wh.into(), |_, _, cx| {
        ws.update(cx, |ws, cx| {
            let entry = ws
                .control_flow_run(
                    ws.active,
                    "ship",
                    daruda_flow::request::StartedBy::Phone,
                    cx,
                )
                .expect("a command-only flow needs neither an agent nor a person");
            assert_eq!(
                entry.entry.name, "ship.yaml",
                "named by the file, not the input"
            );
            assert_eq!(entry.entry.origin, FlowOriginKind::Repo);
            assert!(
                !ws.overlays.flow_picker.is_open(),
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
    cx.update_window(wh.into(), |_, _, cx| {
        ws.update(cx, |ws, cx| {
            let lane_ref = ws.active_ref();
            ws.seed_flow_run_for_test(lane_ref, std::path::PathBuf::from("/tmp/seeded-run"), cx);
            assert_eq!(
                ws.control_flow_run(
                    ws.active,
                    "ship",
                    daruda_flow::request::StartedBy::Phone,
                    cx
                ),
                Err(ControlError::FlowLocked {
                    name: "ship.yaml".into(),
                    held_by: Some("flow:seeded-run".parse().unwrap()),
                })
            );
            assert!(
                !ws.overlays.flow_picker.is_open(),
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
    // This one really starts a run, and the engine wakes the app from its own
    // thread — the sanctioned opt-out, as in `outcome_reaches_the_phone`.
    cx.executor().allow_parking();
    let (_lane, ws, _path, wh) = workspace_with_a_flow(cx, COMMAND_ONLY);
    let (_other_dir, other) = add_lane_with_a_flow(&ws, wh, cx, "deploy.yaml", COMMAND_ONLY);

    cx.update_window(wh.into(), |_, _, cx| {
        ws.update(cx, |ws, cx| {
            let active = ws.active;
            assert_ne!(other, active, "the fixture must give two worktrees");
            // The worktree on screen does not hold it...
            assert_eq!(
                ws.control_flow_run(active, "deploy", daruda_flow::request::StartedBy::Phone, cx),
                Err(ControlError::FlowNotFound {
                    name: "deploy".into()
                }),
                "resolution is worktree-scoped, not window-wide"
            );
            // ...and the one that does runs it, and says so.
            let entry = ws
                .control_flow_run(other, "deploy", daruda_flow::request::StartedBy::Phone, cx)
                .expect("the named worktree holds it");
            assert_eq!(entry.entry.name, "deploy.yaml");
            assert_eq!(entry.entry.lane.lane_ref(), other, "ran where it was told");
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
    cx.update_window(wh.into(), |_, _, cx| {
        ws.update(cx, |ws, cx| {
            let gone = daruda_store::project::LaneRef {
                project: ws.active.project,
                lane: ws.active.lane + 9,
            };
            assert_eq!(
                ws.control_flow_run(gone, "ship", daruda_flow::request::StartedBy::Phone, cx),
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

    cx.update_window(wh.into(), |_, _, cx| {
        ws.update(cx, |ws, cx| {
            ws.seed_flow_run_for_test(other, std::path::PathBuf::from("/tmp/seeded-elsewhere"), cx);
            assert_eq!(
                ws.control_flow_run(other, "deploy", daruda_flow::request::StartedBy::Phone, cx),
                Err(ControlError::FlowLocked {
                    name: "deploy.yaml".into(),
                    held_by: Some("flow:seeded-elsewhere".parse().unwrap()),
                })
            );
            assert!(
                !ws.overlays.flow_picker.is_open(),
                "the refusal is the answer, not a dialog"
            );
        });
    })
    .expect("window is live");
}

/// A run that ends opens nothing over what the person is looking at: its
/// report waits in the Flows page's past runs. A worktree off screen is marked
/// unread instead; the one on screen is not, since it is already in view.
///
/// The run directory has to hold a real `RUN_REPORT_FILE`: `settle_flow_run`
/// answers `None` without one, and then neither pole is reached.
#[gpui::test]
async fn a_run_ending_opens_no_tab_and_marks_its_worktree_unread(cx: &mut TestAppContext) {
    let (here_dir, ws, _path, wh) = workspace_with_a_flow(cx, COMMAND_ONLY);
    let (other_dir, other) = add_lane_with_a_flow(&ws, wh, cx, "deploy.yaml", COMMAND_ONLY);
    let ended = daruda_flow::event::FlowEvent::RunEnded {
        end: daruda_flow::event::RunEnd::Done,
    };
    let run_in = |root: &std::path::Path, name: &str| {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).expect("run dir");
        std::fs::write(dir.join(daruda_flow::record::RUN_REPORT_FILE), "# Run").expect("report");
        dir
    };
    let parked_run = run_in(other_dir.path(), "run-elsewhere");
    let here_run = run_in(here_dir.path(), "run-here");

    let (active, before) = ws.update(cx, |ws, _| (ws.active, ws.active_runtime().panes.len()));
    assert_ne!(other, active);
    let end_in = |lane, dir: &std::path::PathBuf, cx: &mut TestAppContext| {
        ws.update(cx, |ws, cx| {
            ws.seed_flow_run_for_test(lane, dir.clone(), cx)
        });
        ws.update(cx, |ws, cx| ws.apply_flow_event_for_test(lane, &ended, cx));
    };
    let toasted = |ws: &Workspace| {
        ws.error_history()
            .iter()
            .filter(|r| r.title == crate::surface::strings::flow::run_finished_toast())
            .count()
    };

    end_in(other, &parked_run, cx);
    ws.read_with(cx, |ws, cx| {
        assert!(!ws.is_flow_running_in(other, cx), "the run settled");
        assert!(
            ws.lane_for(other).is_some_and(|lane| lane.is_unread),
            "the worktree off screen is marked unread"
        );
    });

    end_in(active, &here_run, cx);
    ws.read_with(cx, |ws, _| {
        assert!(
            ws.lane_for(active).is_some_and(|lane| !lane.is_unread),
            "the one on screen is not"
        );
        assert_eq!(toasted(ws), 1, "it is told by a toast instead");
    });

    // With the Flows page on that worktree, its list already says so.
    ws.update(cx, |ws, cx| {
        ws.show_page(crate::workspace::pages::Page::Flows, cx)
    });
    end_in(active, &here_run, cx);
    ws.read_with(cx, |ws, _| {
        assert_eq!(toasted(ws), 1, "no toast over the page showing it");
        assert_eq!(ws.active_runtime().panes.len(), before, "no tab opened");
        assert!(
            ws.main_area
                .runtimes
                .get(&other)
                .is_none_or(|rt| rt.tabs.is_empty()),
            "not in the worktree it ran in either"
        );
        assert_eq!(ws.active, active, "and the screen did not move");
    });
}

/// A run the overage switch stopped is toasted even over the Flows page,
/// which shows it only as canceled; a run a person stopped is not.
#[gpui::test]
async fn a_run_stopped_for_extra_usage_is_toasted(cx: &mut TestAppContext) {
    let (here_dir, ws, _path, _wh) = workspace_with_a_flow(cx, COMMAND_ONLY);
    let canceled = daruda_flow::event::FlowEvent::RunEnded {
        end: daruda_flow::event::RunEnd::Canceled { node: None },
    };
    let run_dir = here_dir.path().join("run-billed");
    std::fs::create_dir_all(&run_dir).expect("run dir");
    std::fs::write(run_dir.join(daruda_flow::record::RUN_REPORT_FILE), "# Run").expect("report");
    let limit = billed_limit();

    let active = ws.update(cx, |ws, cx| {
        ws.show_page(crate::workspace::pages::Page::Flows, cx);
        ws.active
    });

    ws.update(cx, |ws, cx| {
        ws.seed_flow_run_for_test(active, run_dir.clone(), cx)
    });
    ws.update(cx, |ws, cx| {
        ws.apply_flow_event_for_test(active, &canceled, cx)
    });
    ws.read_with(cx, |ws, _| {
        assert_eq!(overage_toasts(ws), 0, "a Stop by hand needs no telling");
    });

    ws.update(cx, |ws, cx| {
        ws.seed_flow_run_for_test(active, run_dir.clone(), cx);
        ws.trip_flow_overage_for_test(active, &limit, cx);
    });
    ws.update(cx, |ws, cx| {
        ws.apply_flow_event_for_test(active, &canceled, cx)
    });
    ws.read_with(cx, |ws, _| {
        assert_eq!(
            overage_toasts(ws),
            1,
            "the switch's stop is said, page or not"
        );
    });
}

/// Runs off screen that the overage switch stopped are toasted too — their
/// Flows list would read like a manual Stop — and still marked unread, which
/// is what says where. A limit trips runs together, so the toasts merge into
/// one with a repeat count.
#[gpui::test]
async fn runs_stopped_for_extra_usage_off_screen_are_toasted_once(cx: &mut TestAppContext) {
    let (_here_dir, ws, _path, wh) = workspace_with_a_flow(cx, COMMAND_ONLY);
    let (one_dir, one) = add_lane_with_a_flow(&ws, wh, cx, "one.yaml", COMMAND_ONLY);
    let (two_dir, two) = add_lane_with_a_flow(&ws, wh, cx, "two.yaml", COMMAND_ONLY);
    let canceled = daruda_flow::event::FlowEvent::RunEnded {
        end: daruda_flow::event::RunEnd::Canceled { node: None },
    };
    let run_in = |root: &std::path::Path| {
        let dir = root.join("run-billed");
        std::fs::create_dir_all(&dir).expect("run dir");
        std::fs::write(dir.join(daruda_flow::record::RUN_REPORT_FILE), "# Run").expect("report");
        dir
    };
    let limit = billed_limit();

    ws.read_with(cx, |ws, _| assert!(one != ws.active && two != ws.active));
    for (lane, root) in [(one, one_dir.path()), (two, two_dir.path())] {
        ws.update(cx, |ws, cx| {
            ws.seed_flow_run_for_test(lane, run_in(root), cx);
            ws.trip_flow_overage_for_test(lane, &limit, cx);
        });
        ws.update(cx, |ws, cx| {
            ws.apply_flow_event_for_test(lane, &canceled, cx)
        });
        ws.read_with(cx, |ws, _| {
            assert!(
                ws.lane_for(lane).is_some_and(|lane| lane.is_unread),
                "the stopped worktree is marked"
            );
        });
    }
    ws.update(cx, |ws, cx| {
        assert_eq!(overage_toasts(ws), 2, "each stop is said");
        let merged: Vec<u32> = ws
            .error_toasts(cx)
            .iter()
            .filter(|t| t.report.title == crate::surface::strings::flow::overage_stopped())
            .map(|t| t.repeat_count)
            .collect();
        assert_eq!(merged, [2], "in one toast with a count");
    });
}

fn billed_limit() -> daruda_acp::RateLimitView {
    daruda_acp::RateLimitView {
        window: Some(daruda_acp::RateLimitWindow::FiveHour),
        resets_at: None,
        is_using_overage: true,
    }
}

fn overage_toasts(ws: &Workspace) -> usize {
    ws.error_history()
        .iter()
        .filter(|r| r.title == crate::surface::strings::flow::overage_stopped())
        .count()
}

/// The worktree has to survive the picker's *second* question. A flow that
/// declares profiles is dispatched only after the profile is answered, and
/// that answer arrives long after `run_flow_at` was told where to run — so
/// re-reading the active worktree there would run it in the wrong place.
///
/// Not reachable from the control surface today (`needs_desktop_answer`
/// refuses a profile-declaring flow before it gets here), which is exactly
/// why it is worth pinning: the lane's correctness would otherwise rest on
/// that unrelated guard staying in agreement.
#[gpui::test]
async fn the_profile_question_dispatches_to_the_worktree_it_was_opened_for(
    cx: &mut TestAppContext,
) {
    let (_lane, ws, _path, wh) = workspace_with_a_flow(cx, COMMAND_ONLY);
    let (other_dir, other) = add_lane_with_a_flow(&ws, wh, cx, "deploy.yaml", WITH_PROFILES);
    let deploy = flow_paths::flows_dir(other_dir.path()).join("deploy.yaml");

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let active = ws.active;
            assert_ne!(other, active);

            // Asking about `other` puts the profile question up...
            assert!(ws.run_flow_at(
                other,
                &deploy,
                crate::workspace::command::flow_picker::FlowPurpose::Run,
                crate::workspace::flow_request::FlowSelection::default(),
                window,
                cx,
            ));
            let pick = ws
                .overlays
                .flow_picker
                .focused_pick()
                .expect("the profile question");
            assert!(
                matches!(
                    pick,
                    crate::workspace::command::flow_picker::FlowPick::Profile { lane, .. }
                        if lane == other
                ),
                "the second question forgot which worktree it was asked for: {pick:?}"
            );
        });
    })
    .expect("window is live");
}

/// The stop prompt answers for the run it offered to stop, not for whichever
/// worktree is on screen when Enter lands.
#[gpui::test]
async fn the_stop_prompt_stops_the_run_it_was_raised_for(cx: &mut TestAppContext) {
    let (lane, ws, _path, wh) = workspace_with_a_flow(cx, COMMAND_ONLY);
    let (_other_dir, other) = add_lane_with_a_flow(&ws, wh, cx, "deploy.yaml", COMMAND_ONLY);
    let runs = flow_paths::runs_dir(lane.path());
    std::fs::create_dir_all(&runs).expect("runs dir");

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let active = ws.active;
            ws.seed_flow_run_for_test(active, runs.join("run-here"), cx);
            ws.seed_flow_run_for_test(other, runs.join("run-elsewhere"), cx);

            // Raised for `other`, answered with Enter.
            ws.overlays.flow_picker =
                crate::workspace::command::flow_picker::FlowPicker::Stopping { lane: other };
            ws.execute_flow_picker_selection(window, cx);

            let canceled = stopping(ws, [active, other], cx);
            assert!(
                canceled.contains(&(other, true)),
                "the prompt stopped a different run than it named: {canceled:?}"
            );
            assert!(
                canceled.contains(&(active, false)),
                "and the worktree on screen must be left alone: {canceled:?}"
            );
        });
    })
    .expect("window is live");
}

/// The other half of starting one. Without this the orchestrator could set a
/// flow going and had no way to end it.
#[gpui::test]
async fn a_stop_reaches_the_named_worktree_not_the_active_one(cx: &mut TestAppContext) {
    let (lane, ws, _path, wh) = workspace_with_a_flow(cx, COMMAND_ONLY);
    let (_other_dir, other) = add_lane_with_a_flow(&ws, wh, cx, "deploy.yaml", COMMAND_ONLY);

    ws.update(cx, |ws, cx| {
        let active = ws.active;
        ws.seed_flow_run_for_test(active, lane.path().join("run-here"), cx);
        ws.seed_flow_run_for_test(other, lane.path().join("run-elsewhere"), cx);

        assert_eq!(
            ws.control_flow_stop(other, None, cx),
            Ok(crate::control::result::StopDisposition::Stopped)
        );
        // The token is the stop; the entry stays until the event pump retires
        // it, so cancellation is what to read — not whether a run is there.
        let canceled = stopping(ws, [active, other], cx);
        assert!(
            canceled.contains(&(other, true)),
            "the named worktree's run was not stopped: {canceled:?}"
        );
        assert!(
            canceled.contains(&(active, false)),
            "the worktree on screen must be left alone: {canceled:?}"
        );
    });
}

/// Nothing running is the state a stop asked for, so it is an `Ok` with a
/// disposition — the same shape `daruda_chat_stop` answers an idle pane with,
/// and no new error code.
#[gpui::test]
async fn stopping_a_worktree_with_no_run_is_an_ok_disposition(cx: &mut TestAppContext) {
    let (_lane, ws, _path, _wh) = workspace_with_a_flow(cx, COMMAND_ONLY);
    ws.update(cx, |ws, cx| {
        assert_eq!(
            ws.control_flow_stop(ws.active, None, cx),
            Ok(crate::control::result::StopDisposition::AlreadyIdle)
        );
        // And a worktree that is not there at all is a different answer.
        let gone = daruda_store::project::LaneRef {
            project: ws.active.project,
            lane: ws.active.lane + 9,
        };
        assert_eq!(
            ws.control_flow_stop(gone, None, cx),
            Err(ControlError::TargetGone)
        );
    });
}

/// A text adapter names no worktree, so the executor picks one — and has to
/// keep the two refusals apart. "Nowhere to run" and "no such flow" send a
/// person to look at different things.
#[gpui::test]
async fn the_default_target_separates_no_flow_from_no_worktree(cx: &mut TestAppContext) {
    let (_lane, _ws, _path, _wh) = workspace_with_a_flow(cx, COMMAND_ONLY);
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
    started_run_reports_its_outcome(crate::control::exec::CommandOrigin::Phone, cx).await;
}

#[gpui::test]
async fn an_orchestrator_started_run_records_its_origin_and_keeps_phone_notice(
    cx: &mut TestAppContext,
) {
    started_run_reports_its_outcome(crate::control::exec::CommandOrigin::Orchestrator, cx).await;
}

async fn started_run_reports_its_outcome(
    origin: crate::control::exec::CommandOrigin,
    cx: &mut TestAppContext,
) {
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
    let built = build_workspace_with(cx, &config, Some(project));
    let (wh, ws) = registered(cx, built);
    // A real run: its log thread wakes the app from outside the scheduler,
    // so the opt-out has to come before the run starts.
    cx.executor().allow_parking();

    let run_id = cx
        .update_window(wh.into(), |_, _, cx| {
            ws.update(cx, |ws, cx| {
                ws.control_flow_run(ws.active, "ship", origin.flow_started_by(), cx)
                    .expect("dispatched")
                    .run_id
            })
        })
        .expect("window is live");

    // The *real* engine and the *real* event pump produce the outcome — the
    // flow is `run: "true"`, so it finishes on its own.
    let sent = outcome_reaches_the_phone(&mut outbound, cx).expect("the outcome reaches the phone");
    let run_dir = crate::workspace::flow_paths::runs_dir(lane.path()).join(run_id);
    assert_eq!(
        daruda_flow::run_log::reader::read(&run_dir).started_by,
        Some(origin.flow_started_by())
    );
    match sent {
        crate::telegram::bridge::Outbound::Notice(text) => {
            assert_eq!(
                text,
                crate::surface::strings::control::flow_finished_notice()
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

    let lane_ref = ws.update(cx, |ws, cx| {
        let lane_ref = ws.active_ref();
        // Seeded, i.e. not marked as owing anyone an answer.
        ws.seed_flow_run_for_test(lane_ref, lane.path().join("run"), cx);
        lane_ref
    });
    ws.update(cx, |ws, cx| {
        ws.apply_flow_event_for_test(
            lane_ref,
            &daruda_flow::event::FlowEvent::RunEnded {
                end: daruda_flow::event::RunEnd::Done,
            },
            cx,
        )
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
    // A real run: its log thread wakes the app from outside the scheduler,
    // so the opt-out has to come before the run starts.
    cx.executor().allow_parking();

    cx.update_window(wh.into(), |_, _, cx| {
        ws.update(cx, |ws, cx| {
            ws.control_flow_run(
                ws.active,
                "ship",
                daruda_flow::request::StartedBy::Phone,
                cx,
            )
            .expect("dispatched");
            // A stop trips the token and leaves the run in place, so the
            // engine returns and the run still gets to say how it ended.
            let lane_ref = ws.active_ref();
            ws.stop_flow_run_in(lane_ref, cx);
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

/// The app keeps running with every window closed (macOS quits only on an
/// explicit Quit), so a phone-started run can end with no window to speak
/// for it. The phone is still answered, once.
#[gpui::test]
async fn a_phone_started_run_answers_with_no_window_open(cx: &mut TestAppContext) {
    let mut outbound =
        cx.update(|cx| crate::telegram::global::install_for_test(true, Some(42), cx));
    let lane = tempfile::tempdir().expect("tempdir");
    let run_dir = lane.path().join("run");
    cx.update(|cx| {
        crate::runs::RunsHost::ensure_installed(cx);
        Workspace::install_task_effects(cx);
        let source = crate::runs::FlowSource::Resumed {
            run_dir: run_dir.clone(),
        };
        crate::runs::RunsHost::seed_flow_run_submitted_by(
            lane.path().to_path_buf(),
            crate::runs::RunView::new(run_dir.clone(), source, Vec::new()),
            daruda_flow::request::StartedBy::Phone,
            cx,
        );
        crate::runs::RunsHost::apply_flow_event_for_test(
            lane.path(),
            &daruda_flow::event::FlowEvent::RunEnded {
                end: daruda_flow::event::RunEnd::Done,
            },
            cx,
        );
    });
    cx.run_until_parked();

    match outbound.next().now_or_never().flatten() {
        Some(crate::telegram::bridge::Outbound::Notice(text)) => {
            assert_eq!(
                text,
                crate::surface::strings::control::flow_finished_notice()
            );
        }
        other => panic!("the phone was not told how the run ended: {other:?}"),
    }
    assert!(
        outbound.next().now_or_never().flatten().is_none(),
        "the phone was answered more than once"
    );
}

/// Whether each of `lanes`' runs has been told to stop.
fn stopping(
    ws: &Workspace,
    lanes: [daruda_store::project::LaneRef; 2],
    cx: &gpui::App,
) -> Vec<(daruda_store::project::LaneRef, bool)> {
    lanes
        .into_iter()
        .filter_map(|lane| Some((lane, ws.flow_run_in(lane, cx)?.is_stopping())))
        .collect()
}

/// Closing the window that started a run leaves it a stop switch: the
/// process holds the run, so another window on the same worktree sees it
/// and stops it.
#[gpui::test]
async fn closing_the_window_that_started_a_run_keeps_it_stoppable(cx: &mut TestAppContext) {
    let (lane, first, flow_path, first_wh) = workspace_with_a_flow(cx, LONG_COMMAND);
    let config = daruda_config::Config::default();
    let built = build_workspace_with(
        cx,
        &config,
        Some(daruda_store::project::Project::from_path(lane.path())),
    );
    let (_second_wh, second) = registered(cx, built);
    // A real run: its log thread wakes the app from outside the scheduler,
    // so the opt-out has to come before the run starts.
    cx.executor().allow_parking();

    cx.update_window(first_wh.into(), |_, window, cx| {
        first.update(cx, |ws, cx| {
            assert!(ws.run_flow_at(
                ws.active,
                &flow_path,
                crate::workspace::command::flow_picker::FlowPurpose::Run,
                crate::workspace::flow_request::FlowSelection::default(),
                window,
                cx,
            ));
        });
    })
    .expect("window is live");
    drop(first);
    cx.update_window(first_wh.into(), |_, window, _| window.remove_window())
        .expect("window is live");
    cx.run_until_parked();
    assert!(
        run_is_held(cx, lane.path()),
        "closing the window ended the run"
    );

    let here = second.read_with(cx, |ws, _| ws.active);
    let rows = second.read_with(cx, |ws, cx| ws.flow_rows_for_active_lane(cx));
    assert_eq!(rows.len(), 1, "the other window does not see the run");
    second.update(cx, |ws, cx| ws.stop_flow_run_in(here, cx));
    assert!(
        wait_for_run(cx, |cx| !run_is_held(cx, lane.path())).await,
        "the run did not stop from the window that is left"
    );
}

/// Quitting is not a Stop: the command the run started is ended, but the
/// run records no ending, so it is left to resume like a crashed one. Driven
/// through the app's own quit hook, which is the only production caller.
#[cfg(unix)]
#[gpui::test]
async fn quitting_ends_a_runs_command_and_leaves_it_resumable(cx: &mut TestAppContext) {
    const RECORDS_ITS_PID: &str = "\
version: 1
nodes:
  - id: wait
    kind: command
    run: \"echo $$ > quit.pid; sleep 30\"
";
    let (lane, ws, flow_path, wh) = workspace_with_a_flow(cx, RECORDS_ITS_PID);
    cx.update(crate::runs::RunsHost::install_quit_flush);
    // A real run: its log thread wakes the app from outside the scheduler,
    // so the opt-out has to come before the run starts.
    cx.executor().allow_parking();
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            assert!(ws.run_flow_at(
                ws.active,
                &flow_path,
                crate::workspace::command::flow_picker::FlowPurpose::Run,
                crate::workspace::flow_request::FlowSelection::default(),
                window,
                cx,
            ));
        });
    })
    .expect("window is live");
    let pid_file = lane.path().join("quit.pid");
    let mut pid = None;
    assert!(
        wait_for_run(cx, |_| {
            pid = std::fs::read_to_string(&pid_file)
                .ok()
                .and_then(|text| text.trim().parse::<u32>().ok());
            pid.is_some()
        })
        .await,
        "the command never started"
    );
    let run_dir = cx.update(|cx| {
        crate::runs::RunsHost::flow_run(lane.path(), cx)
            .expect("a run")
            .view()
            .run_dir()
            .to_path_buf()
    });

    let quit_at = std::time::Instant::now();
    cx.quit();
    assert!(
        quit_at.elapsed() < std::time::Duration::from_secs(3),
        "quitting waited past its deadline"
    );
    assert!(
        cx.update(|cx| crate::runs::RunsHost::wait_flows_ended(std::time::Instant::now(), cx)),
        "quit returned while the run's thread was still going"
    );

    let pid = pid.expect("a pid");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while daruda_core::process::is_alive(pid) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        !daruda_core::process::is_alive(pid),
        "the command outlived the quit"
    );
    let state = daruda_flow::run_log::reader::state(&run_dir).expect("a log");
    assert_eq!(
        state.phase(),
        daruda_flow::run_log::machine::RunPhase::Running,
        "a quit recorded how the run ended"
    );
}

/// Long enough that only a stop ends it within a test's patience.
const LONG_COMMAND: &str = "\
version: 1
nodes:
  - id: wait
    kind: command
    run: \"sleep 30\"
";

#[gpui::test]
async fn a_stale_flow_handle_does_not_stop_a_new_run_in_the_same_worktree(cx: &mut TestAppContext) {
    let (lane, ws, _path, _wh) = workspace_with_a_flow(cx, COMMAND_ONLY);
    ws.update(cx, |ws, cx| {
        let active = ws.active;
        ws.seed_flow_run_for_test(active, lane.path().join("new-run"), cx);
        let stale = "flow:old-run".parse().unwrap();
        assert_eq!(
            ws.control_flow_stop(active, Some(&stale), cx),
            Err(ControlError::RunMismatch)
        );
        assert!(!ws.flow_run_in(active, cx).unwrap().is_stopping());
        let current = "flow:new-run".parse().unwrap();
        assert_eq!(
            ws.control_flow_stop(active, Some(&current), cx),
            Ok(crate::control::result::StopDisposition::Stopped)
        );
    });
}

/// A worktree another process holds is refused with that process's run, read
/// from the lock, so the caller can say which run is in the way.
#[gpui::test]
async fn a_worktree_held_by_another_process_names_its_run(cx: &mut TestAppContext) {
    let (lane, ws, _path, _wh) = workspace_with_a_flow(cx, COMMAND_ONLY);
    let lock_dir = ws.read_with(cx, |ws, _| {
        flow_paths::lane_lock_dir(&ws.lock_root, lane.path()).expect("the lane resolves")
    });
    std::fs::create_dir_all(&lock_dir).expect("create the lock dir");
    let holder_process = test_process::sleeping();
    std::fs::write(
        lock_dir.join(".lock"),
        format!(
            "pid: {}\nrun_id: someone-elses\nstarted_unix_secs: 1\n",
            holder_process.id()
        ),
    )
    .expect("plant a lock");
    ws.update(cx, |ws, cx| {
        assert_eq!(
            ws.control_flow_run(
                ws.active,
                "ship",
                daruda_flow::request::StartedBy::Phone,
                cx
            ),
            Err(ControlError::FlowLocked {
                name: "ship.yaml".into(),
                held_by: Some("flow:someone-elses".parse().unwrap()),
            })
        );
    });
}

/// S6 with real runs: run A ends, run B starts in the same worktree, and a
/// stop naming A is refused without touching B, which goes on to end on its
/// own. The seeded test above proves the refusal; this proves B survives it.
#[gpui::test]
async fn a_stop_with_an_ended_runs_handle_leaves_the_next_run_to_finish(cx: &mut TestAppContext) {
    use crate::runs::result::{EndedAs, RunKey, RunState};
    // Real runs wake the app from the engine's thread from their first record.
    cx.executor().allow_parking();
    let (lane, ws, _path, _wh) = workspace_with_a_flow(cx, COMMAND_ONLY);
    let gate = lane.path().join("release-b");
    let held_open = format!(
        "version: 1\nnodes:\n  - id: hold\n    kind: command\n    run: \"while [ ! -e '{}' ]; do sleep 0.05; done\"\n",
        gate.display()
    );
    std::fs::write(
        flow_paths::flows_dir(lane.path()).join("hold.yaml"),
        held_open,
    )
    .expect("write the second flow");
    let start = |name: &'static str, ws: &gpui::Entity<Workspace>, cx: &mut TestAppContext| {
        ws.update(cx, |ws, cx| {
            ws.control_flow_run(ws.active, name, daruda_flow::request::StartedBy::Phone, cx)
                .expect("dispatched")
                .run_id
        })
    };

    let run_a = start("ship", &ws, cx);
    assert!(
        wait_for_run(cx, |cx| !run_is_held(cx, lane.path())).await,
        "run A did not end"
    );
    let run_b = start("hold", &ws, cx);
    assert_ne!(run_a, run_b);

    let stale: daruda_control_types::runs::RunHandle =
        format!("flow:{run_a}").parse().expect("a handle");
    ws.update(cx, |ws, cx| {
        let active = ws.active;
        assert_eq!(
            ws.control_flow_stop(active, Some(&stale), cx),
            Err(ControlError::RunMismatch)
        );
        let held = ws.flow_run_in(active, cx).expect("run B is still held");
        assert!(!held.is_stopping(), "the refused stop reached run B");
    });

    std::fs::write(&gate, "").expect("release run B");
    assert!(
        wait_for_run(cx, |cx| !run_is_held(cx, lane.path())).await,
        "run B did not end"
    );
    let ended = cx.update(|cx| crate::runs::RunsHost::run_query(RunKey::Flow(run_b), cx).read());
    assert!(
        matches!(
            ended.state,
            RunState::Ended {
                ended_as: EndedAs::Completed,
                ..
            }
        ),
        "run B did not end on its own: {:?}",
        ended.state
    );
}

#[gpui::test]
async fn a_flow_start_response_names_the_execution_dispatched(cx: &mut TestAppContext) {
    use crate::control::{
        exec,
        result::{ControlResult, LaneHandle},
        spec::{ResolvedCommand, ResolvedFlowCommand},
    };
    cx.executor().allow_parking();
    let (_lane, ws, _path, _wh) = workspace_with_a_flow(cx, COMMAND_ONLY);
    let target = ws.read_with(cx, |ws, _| LaneHandle::new(ws.uuid(), ws.active));
    let outcome = cx.update(|cx| {
        exec::run(
            ResolvedCommand::Flow(ResolvedFlowCommand::Run {
                name: "ship".into(),
                lane: target,
            }),
            exec::CommandOrigin::Orchestrator,
            cx,
        )
    });
    let ControlResult::FlowStarting { handle, .. } = outcome.expect("dispatch") else {
        panic!("flow start");
    };
    cx.update(|cx| {
        assert_eq!(
            crate::runs::RunsHost::run_scope(cx),
            vec![crate::runs::result::RunKey::Flow(match &handle {
                daruda_control_types::runs::RunHandle::Flow { run_id } => run_id.clone(),
                _ => panic!("flow handle"),
            })]
        );
    });
    ws.read_with(cx, |ws, cx| {
        let id = ws
            .flow_run_in(ws.active, cx)
            .unwrap()
            .view()
            .run_dir()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(handle.to_string(), format!("flow:{id}"));
    });
}

/// A start that loses the race to a run this process already holds comes
/// back typed to whoever submitted it. The desktop's "stop it?" offer is
/// the desktop caller's answer, never raised for a control caller.
#[gpui::test]
async fn a_start_that_finds_a_run_held_is_refused_typed_without_a_dialog(cx: &mut TestAppContext) {
    let (lane, ws, flow_path, _wh) = workspace_with_a_flow(cx, COMMAND_ONLY);
    ws.update(cx, |ws, cx| {
        let lane_ref = ws.active_ref();
        ws.seed_flow_run_for_test(lane_ref, lane.path().join("seeded"), cx);
        let started = ws.submit_flow_run(
            lane_ref,
            &flow_path,
            None,
            &crate::workspace::flow_request::FlowSelection::default(),
            daruda_flow::request::StartedBy::Phone,
            cx,
        );
        assert!(
            matches!(
                started,
                Err(crate::workspace::flow_ops::FlowStartRefused::AlreadyRunning)
            ),
            "{started:?}"
        );
        assert!(
            !ws.overlays.flow_picker.is_open(),
            "a submission answers its caller; it opens no dialog"
        );
        assert!(ws.error_history().is_empty(), "nor a toast");
    });
}

/// A start refused for a reason outside the file reaches the control
/// caller with that reason, instead of a toast only the desktop shows.
#[gpui::test]
async fn a_refused_start_tells_the_control_caller_why(cx: &mut TestAppContext) {
    let (_lane, ws, _flow_path, _wh) = workspace_with_a_flow(cx, ONE_AGENT);
    ws.update(cx, |ws, cx| {
        let target = ws.active;
        ws.set_lane_session_host(
            target,
            daruda_store::project::LaneSessionHost::Ssh {
                target: "vm -o ProxyCommand=touch /tmp/pwned".into(),
                session_path: "/srv/app".into(),
                registry_id: None,
            },
            cx,
        );
        let refused = ws.control_flow_run(
            ws.active,
            "ship",
            daruda_flow::request::StartedBy::Phone,
            cx,
        );
        match refused {
            Err(ControlError::FlowNotStarted { name, reason }) => {
                assert_eq!(name, "ship.yaml");
                assert_eq!(
                    reason,
                    crate::control::result::FlowNotStartedReason::SessionHostUnusable {
                        agent: "claude".into()
                    },
                    "the reason is missing"
                );
            }
            other => panic!("not refused with its reason: {other:?}"),
        }
        assert!(
            ws.error_history().is_empty(),
            "the answer replaces the toast"
        );
        assert!(!ws.overlays.flow_picker.is_open());
    });
}

/// The answer is owed to whoever submitted the episode. A run the phone
/// started and the desktop resumed answers the desktop, which reads its
/// own toast; the log still says the phone started it.
#[gpui::test]
async fn a_desktop_resume_of_a_phone_started_run_stays_off_the_phone(cx: &mut TestAppContext) {
    let mut outbound =
        cx.update(|cx| crate::telegram::global::install_for_test(true, Some(42), cx));
    let (lane, ws, _path, _wh) = telegram_workspace_with_a_flow(cx, COMMAND_ONLY);
    // A real run: its log thread wakes the app from outside the scheduler.
    cx.executor().allow_parking();
    let run_id = ws.update(cx, |ws, cx| {
        ws.control_flow_run(
            ws.active,
            "ship",
            daruda_flow::request::StartedBy::Phone,
            cx,
        )
        .expect("dispatched")
        .run_id
    });
    assert!(
        outcome_reaches_the_phone(&mut outbound, cx).is_some(),
        "the phone's own start answers the phone"
    );
    assert!(
        wait_for_run(cx, |cx| !run_is_held(cx, lane.path())).await,
        "the first run did not finish"
    );

    // Make it a run cut off right after it started, by a crash a minute ago.
    let run_dir = flow_paths::runs_dir(lane.path()).join(&run_id);
    let log = daruda_flow::run_log::log_file(&run_dir);
    let text = std::fs::read_to_string(&log).expect("the run log");
    let cut = text
        .lines()
        .position(|line| line.contains("\"run_started\""))
        .expect("the run started");
    let kept: String = text
        .lines()
        .take(cut + 1)
        .map(|line| {
            let mut record: serde_json::Value = serde_json::from_str(line).expect("a record");
            let at_ms = record["at_ms"].as_u64().expect("a record's time");
            record["at_ms"] = at_ms.saturating_sub(60_000).into();
            format!("{record}\n")
        })
        .collect();
    std::fs::write(&log, kept).expect("cut the log");
    std::fs::remove_file(run_dir.join("DONE")).expect("the end's marker");
    let lock_dir = ws.read_with(cx, |ws, _| {
        flow_paths::lane_lock_dir(&ws.lock_root, lane.path()).expect("the lane resolves")
    });
    std::fs::create_dir_all(&lock_dir).expect("lock dir");
    std::fs::write(
        lock_dir.join(".lock"),
        format!("pid: 0\nrun_id: {run_id}\nstarted_unix_secs: 1\n"),
    )
    .expect("lock");

    let here = ws.read_with(cx, |ws, _| ws.active);
    let before = ws.read_with(cx, |ws, _| ws.error_history().len());
    ws.update(cx, |ws, cx| ws.resume_flow_run(here, &run_dir, cx));
    let errors = ws.read_with(cx, |ws, _| {
        ws.error_history()[before..]
            .iter()
            .map(|r| format!("{} {:?}", r.title, r.context))
            .collect::<Vec<_>>()
    });
    assert!(errors.is_empty(), "the resume was refused: {errors:?}");
    assert!(
        wait_for_run(cx, |cx| {
            run_dir.join("DONE").exists() && !run_is_held(cx, lane.path())
        })
        .await,
        "the resumed run did not finish"
    );
    cx.run_until_parked();
    assert!(
        outbound.next().now_or_never().flatten().is_none(),
        "a desktop resume must not answer the phone"
    );
    assert_eq!(
        daruda_flow::run_log::reader::read(&run_dir).started_by,
        Some(daruda_flow::request::StartedBy::Phone),
        "who started the run is kept"
    );
}
