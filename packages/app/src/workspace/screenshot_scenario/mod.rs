//! Scenario hooks for `--screenshot`. The capture path restores only
//! *persisted* workspace state, so transient overlays (command palette,
//! modals) never appear in a vanilla screenshot. A scenario drives one
//! such overlay into view after the settle delay and just before capture,
//! making those states reachable for visual verification.
//!
//! Parsing of the `--screenshot-scenario <name>` CLI flag lives in
//! `crate::screenshot`; this module owns the scenario enum, its CLI-name
//! mapping, and the workspace-driving dispatch ([`drive`]).

use gpui::{App, Entity, Point, Window, px};

use super::main_area::agent_chat_host::agent_chat_ops::ThoughtsShot;
use super::main_area::tab_ops::OpenIntent;
use super::pages::tasks::editor::TaskEditorShot;
use super::{ToggleCommandPalette, Workspace, dialog_helpers};
use daruda_chat::view::ActivityOptionsTab;
use daruda_config::BuiltinSection;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};

mod names;

/// The name the delete dialog is asked about. Nothing is deleted — a capture
/// never presses the button — so it names no real file.
const FLOW_DELETE_SAMPLE_NAME: &str = "deploy.yaml";

/// A wide sample diagram (two side-by-side subgraphs) — the shape that
/// exercises the lightbox's horizontal scroll/clamp path, not just the
/// common single-column case.
const MERMAID_LIGHTBOX_SAMPLE: &str = concat!(
    "flowchart TD\n",
    "    subgraph ASIS[\"AS-IS\"]\n",
    "        A1[registeredUri] --> A2{\"endsWith('/*')?\"}\n",
    "        A2 -->|no| A3[거부]\n",
    "        A2 -->|yes| A4[\"substring 으로 '/*' 절단\"]\n",
    "        A4 --> A5[parse]\n",
    "        A5 --> A6[\"scheme/host/port/query 일치\"]\n",
    "        A6 --> A7[\"AntPathMatcher.match(pattern, path)\"]\n",
    "        A7 --> A8[허용]\n",
    "    end\n",
    "    subgraph TOBE[\"TO-BE\"]\n",
    "        B1[registeredUri] --> B2[parse]\n",
    "        B2 --> B3{\"path.endsWith('/*')?\"}\n",
    "        B3 -->|no| B4[\"path 정확 일치\"]\n",
    "        B3 -->|yes| B5[\"prefix + 단일 세그먼트 직접 비교\"]\n",
    "        B2 --> B6[\"userInfo/fragment 있으면 거부\"]\n",
    "        B2 --> B7[\"port: 등록에 없으면 any허용\"]\n",
    "        B4 --> B8[허용]\n",
    "        B5 --> B8\n",
    "    end\n",
);

/// Where the pane menu is deployed for the capture, in window coordinates.
/// Near the top-left of the content area so the menu opens downward at its
/// natural length — the shot is meant to show every entry, not the edge flip.
const PANE_MENU_ANCHOR_X: f32 = 320.;
const PANE_MENU_ANCHOR_Y: f32 = 160.;

/// A transient UI state to drive into view before a `--screenshot` capture.
/// One scenario per capture — these overlays are mutually exclusive on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScreenshotScenario {
    AgentChatDiff,
    StatusBar(super::status_bar::screenshot::StatusBarShot),
    QuickEntry(super::right_dock::quick_entry_shot::QuickEntryShot),
    FlowBrowser(super::flow_browser::screenshot::FlowBrowserShot),
    Tasks,
    TasksAll,
    TasksNoResults,
    TasksByStatus,
    TasksByProject,
    TaskSaved,
    TaskEditor(TaskEditorShot),
    OrchestratorChip,
    OrchestratorTab,
    OrchestratorApproval,
    /// One tab per status dot — working, waiting, failed, and the two unseen
    /// outcomes — behind a resting tab. Colour is all that tells them apart,
    /// which only a capture can judge.
    TabIndicators,
    LeftDockSessions,
    LeftDockSessionsSix,
    LeftDockRail,
    LeftDockExpanded,
    LeftDockGroups,
    LeftDockGroupStatus,
    LeftDockGroupHover,
    LeftDockPaneHover,
    LeftDockProjectAlias,
    LeftDockWorktrees,
    LeftDockGit,
    /// Open the command palette (`CommandPaletteState::open`).
    CommandPalette,
    /// Open the Lane switcher with a real candidate whose label is
    /// replaced by a synthetic one long enough to overflow the popup width.
    /// The only way to eyeball hard-clipping on a long
    /// `"{project} / {branch}"` label — the real lanes in the test
    /// workspace are always short enough to fit.
    LaneSwitcher,
    /// Draw the title bar as if the platform left no caption — the Windows and
    /// Linux arm. macOS still paints its real traffic lights over the left
    /// inset, so this capture speaks for the right edge and the spacing, not
    /// for the left.
    ClientChrome,
    /// Open the Layer-2 error-report modal with a synthetic report.
    ErrorModal,
    /// Push a synthetic error toast.
    Toast,
    /// Protocol advisories through the chat host's reporting path.
    SessionNotices,
    /// Two file tabs side by side — one committed, one the replaceable
    /// scratch tab. The scratch one renders in italics, which is the whole
    /// affordance and the one thing a unit test cannot see.
    ScratchTab,
    /// Empty the workspace so the center paints the Landing view. A capture
    /// always restores a workspace that has projects, so this is the only
    /// way to reach the state `NewEmptyWindow` and a last-project close land on.
    Landing,
    /// Open the Settings window at the given section.
    Settings(BuiltinSection),
    /// The same window with an action's failure banner up. Every Settings
    /// action that cannot be carried out reports through this one alert, and
    /// no unit test can look at it — only that the field behind it was set.
    SettingsError,
    /// Settings with a query typed into its search: the results page and the
    /// sidebar narrowed to the pages that matched, with their counts.
    SettingsSearch,
    /// The agent catalog with its first card and that card's advanced block
    /// open, and the preset lists narrowed by a query — a restored Settings
    /// opens every card folded, so the fields inside are reachable no other way.
    AgentCatalogExpanded,
    /// The first catalog row with one transcript editor's popover open — the
    /// Settings host of the shared editors, whose panel a closed field cannot
    /// show.
    AgentCatalogEditor(daruda_settings::EditorShot),
    /// Deploy the focused pane's right-click menu. The only way to eyeball
    /// menu length, edge-flip and the keybinding column — none of which any
    /// unit test can see.
    PaneContextMenu,
    /// Open the mermaid diagram lightbox with a wide (two-subgraph) sample —
    /// the only way to eyeball the clamp/scroll behavior a unit test can
    /// only assert numerically.
    MermaidLightbox,
    /// Draw the first flow the active lane has. The only way to eyeball the
    /// card layout, the layered placement and the dashed `rerun` edge —
    /// none of which a unit test can look at.
    FlowGraph,
    /// The same graph, coloured by a run. The only way to see the card states
    /// side by side — a pass, a second attempt, a gate under repair, and the
    /// nodes still waiting — which is a comparison no single assertion makes.
    FlowGraphRunning,
    /// The same graph with its first node selected, so the inspector is up. The
    /// only way to look at the column's width against the graph it takes width
    /// from, and at whether a prompt box that size is worth reading.
    FlowGraphForm,
    /// The real creation entry point with the first node ready to edit.
    FlowCreate,
    /// The same graph with its first node's output pinned. The only way to see
    /// whether a pinned card is distinguishable from a pending one beside it,
    /// and whether the pin glyph in the toolbar reads as a pin at 16px.
    FlowGraphPinned,
    /// The inspector after a save the engine refused. The only way to look at
    /// the banner — where it sits, and whether the engine's sentence reads as
    /// something a person can act on.
    FlowGraphFormRefused,
    /// A flow the engine refuses, with a pin that has just gone — the only way
    /// to see the card header carrying three things at once.
    FlowGraphAuthoring,
    /// A past run's report as the Flows page's detail. Reached only by a row
    /// click on a run that wrote one, which no restored state holds.
    FlowRunDetail,
    /// An agent-chat pane parked on an expired login: the connect banner with
    /// its remedy buttons, and the failure card the conversation ends on.
    AgentChatFailure,
    /// A disconnected adapter with localized recovery copy and a retry button.
    AgentChatTransportClosed,
    AgentChatPreparationFailure(daruda_acp::preparation::PreparationKind),
    /// A fresh agent-chat pane before the first prompt, with view options open.
    AgentChatEmpty,
    /// A settled transcript in the default state. The only way to look at the
    /// prose spine, its group bars and the three activity-bar chips at once —
    /// whether the chips fit beside the pane title, whether they read as three
    /// controls, and whether a group bar's rollup says anything about what it
    /// folded.
    AgentChat,
    /// Stable compaction boundaries, with one retained summary disclosed.
    AgentChatCompaction,
    AgentChatAttachments,
    /// The transcript mid-turn: the agent has run tools and written the
    /// preamble in front of them, but not yet the answer. The run's last prose
    /// is therefore a preamble rather than a conclusion — the one shape
    /// [`Self::AgentChat`]'s settled seed cannot reach.
    AgentChatWorking,
    /// The same transcript with the display filter engaged and the folds open,
    /// so the filter's reveal chip renders on the response bar — the only way
    /// to check that it reads as a view control beside the run's counts rather
    /// than as one more row in the list below it.
    AgentChatNarrowed,
    /// The same transcript with a custom fold matrix and its editor open.
    AgentChatFold,
    /// A seeded transcript whose run a Stop cut. The marker is pushed by the
    /// Stop path, not by anything the agent sends, so no settled seed reaches
    /// it — and it is the one row whose whole job is to read as an edge rather
    /// than a message, which only a capture can confirm.
    AgentChatInterrupted,
    /// The interrupted transcript under the banner an overage stop raises —
    /// reached only by a live billed `usage_update` with the setting on.
    AgentChatOverageStopped,
    /// A Terminal task's conversation mirrored read-only while its CLI runs:
    /// the composer gives way to a status row with only a refresh control.
    AgentChatCliRunning,
    /// The same mirror once the CLI's exit is confirmed, which is the only
    /// state that offers continuing in Agent Chat.
    AgentChatCliEnded,
    /// The queued-prompt strip holding a queue a Stop parked. The strip is
    /// reachable no other way — a restored pane has no queue — and its Resume
    /// button now shares a line with the key hint for the keyboard path.
    AgentChatQueueParked,
    /// The same strip after one empty-composer Enter armed the resume gesture,
    /// where the count gives way to the sentence asking for the second press.
    AgentChatQueueArmed,
    /// Prose-only replies — the `/usage` shape — with the first reply's own
    /// fold shut and the second open. Such a turn renders one block, which the
    /// response bar cannot fold (the conclusion escape keeps it on screen), so
    /// the bare chevron is its only control and whether it reads as one is a
    /// question only a capture answers.
    AgentChatSoleReply,
    /// A tool card whose call is still running, its group open. The badge's
    /// live state is the one a settled seed cannot reach, and it is the only
    /// state that carries a number — which only a capture can judge.
    AgentChatRunningTool,
    /// The plan region mid-run: one step done, one running, one still to come.
    /// Its four states are icons carrying meaning by shape, which a unit test
    /// cannot judge — only a capture shows whether they read apart.
    AgentChatPlan,
    /// The same region after a Stop, where the running step became `Cancelled`.
    /// Paired with [`Self::AgentChatPlan`] because the question is whether the
    /// cut step is distinguishable from both the done one and the untouched one.
    AgentChatPlanStopped,
    /// The tail window's boundary row with nothing floating over it. The pair
    /// with [`Self::AgentChatTailOpen`] is the only way to judge the one thing
    /// the row exists to say — whether its two states are distinguishable —
    /// since `agent-chat-narrowed` covers it with the filter popover and no
    /// state test can look at a rule, an inset label, or an indent.
    AgentChatTail,
    /// The boundary open: the label anchors left and the rows it revealed sit at
    /// the response's indent.
    AgentChatTailOpen,
    /// The same boundary one level in: a tool group holding more calls than the
    /// window keeps, open, so the row the group's own window puts inside it is
    /// on screen. Its indent and rule are what say it belongs to the group
    /// rather than to the response — nothing a state test can check.
    AgentChatGroupTail,
    /// The in-group boundary open, with the calls it revealed indented.
    AgentChatGroupTailOpen,
    /// The same boundary a third level in: a subagent card whose flattened
    /// children outnumber the window, expanded. Those children own no row, so
    /// this boundary renders inside the card — whether it reads as part of the
    /// card rather than as transcript is the whole question, and no state test
    /// can look at it.
    AgentChatSubagentTail,
    /// The in-card boundary open, showing every child it held back.
    AgentChatSubagentTailOpen,
    /// The compact Activity Bar's combined options popover, open on one tab
    /// with every axis off its default — the only way to see the gear's own
    /// selected state, whether the tab strip reads as three choices, and
    /// whether a tab's panel matches the chip that opens it on a wide bar.
    ///
    /// Forces the compact layout rather than narrowing the pane, so it captures
    /// that bar's chrome at full width and does *not* exercise the breakpoint
    /// itself.
    AgentChatOptions(ActivityOptionsTab),
    /// One tool group holding the thoughts between its calls, open under a
    /// call window narrower than it. How a thought row reads among calls, what
    /// the bar says once the filter keeps only thoughts, and whether a thought
    /// still being written stays on screen are only judgeable on screen.
    AgentChatThoughts(ThoughtsShot),
    /// Open the flow picker, listing the active lane's `.daruda/flows/`.
    /// The only way to see the row highlight, the empty state and the
    /// prompt line — none of which the state tests can look at.
    FlowPicker,
    /// The second question, for a flow that declares profiles.
    FlowProfilePicker,
    /// A killed run in the panel, with the way back into it.
    FlowResumable,
    /// A flow mid-run, so the status bar chip and its dropdown can be seen.
    /// Nothing else puts a run on screen without one actually running.
    FlowRunning,
    /// The delete confirmation for a repository-committed flow — the longest
    /// of the three sentences, and the only way to see whether it still reads
    /// as a sentence inside the dialog rather than as a wall.
    FlowDeleteConfirm,
    /// A flow parked on a permission question, with the Flows panel showing.
    /// The buttons a person has to read and hit — the one part of `ask` no
    /// state test can look at, and a surface that needs visual verification.
    FlowAsking,
}

/// Drive `scenario` into view on `workspace`'s `window`. Called from the
/// screenshot capture path with the live workspace window in scope.
pub(crate) fn drive(
    scenario: ScreenshotScenario,
    workspace: &Entity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) {
    workspace.update(cx, |ws, cx| ws.suspend_persistence_for_shot(cx));
    match scenario {
        ScreenshotScenario::AgentChatDiff => {
            workspace.update(cx, |ws, cx| ws.open_agent_chat_diff_for_shot(window, cx));
        }
        ScreenshotScenario::StatusBar(shot) => cx.set_global(shot),
        ScreenshotScenario::QuickEntry(shot) => {
            super::right_dock::quick_entry_shot::drive(shot, workspace, window, cx)
        }
        ScreenshotScenario::Tasks
        | ScreenshotScenario::TasksAll
        | ScreenshotScenario::TasksNoResults
        | ScreenshotScenario::TasksByStatus
        | ScreenshotScenario::TasksByProject => {
            let scope = if matches!(
                scenario,
                ScreenshotScenario::TasksAll | ScreenshotScenario::TasksByProject
            ) {
                daruda_store::tasks::TaskScope::AllProjects
            } else {
                daruda_store::tasks::TaskScope::ActiveProject
            };
            let query = if scenario == ScreenshotScenario::TasksNoResults {
                "No matching task"
            } else {
                ""
            };
            workspace.update(cx, |ws, cx| {
                use super::right_dock::tasks::{TaskGroupKey, TaskGrouping};
                ws.seed_task_list_for_shot(scope, query, window, cx);
                if scenario == ScreenshotScenario::TasksByStatus {
                    ws.set_task_grouping(TaskGrouping::Status, cx);
                    ws.toggle_task_group(
                        TaskGroupKey::Status(daruda_store::tasks::TaskFilter::Cancelled),
                        cx,
                    );
                } else if scenario == ScreenshotScenario::TasksByProject {
                    ws.set_task_grouping(TaskGrouping::Project, cx);
                }
            });
        }
        ScreenshotScenario::TaskSaved => {
            workspace.update(cx, |ws, cx| ws.seed_saved_task_for_shot(window, cx));
        }
        ScreenshotScenario::TaskEditor(shot) => {
            workspace.update(cx, |ws, cx| ws.seed_task_editor_for_shot(shot, window, cx));
        }
        ScreenshotScenario::ClientChrome => {
            crate::title_bar::force_client_chrome_for_shot();
            workspace.update(cx, |_, cx| cx.notify());
        }
        ScreenshotScenario::CommandPalette => {
            workspace.update(cx, |ws, cx| {
                ws.on_toggle_command_palette(&ToggleCommandPalette, window, cx);
            });
        }
        ScreenshotScenario::LaneSwitcher => {
            workspace.update(cx, |ws, cx| ws.open_lane_switcher_long_label_for_shot(cx));
        }
        ScreenshotScenario::FlowPicker => {
            workspace.update(cx, |ws, cx| {
                ws.open_flow_picker(
                    ws.active,
                    crate::workspace::command::flow_picker::FlowPurpose::Validate,
                    cx,
                );
            });
        }
        ScreenshotScenario::FlowBrowser(shot) => {
            workspace.update(cx, |ws, cx| ws.seed_flow_browser_for_shot(shot, window, cx));
        }
        ScreenshotScenario::FlowResumable => {
            workspace.update(cx, |ws, cx| ws.seed_crashed_run_for_shot(cx));
        }
        ScreenshotScenario::FlowProfilePicker => {
            workspace.update(cx, |ws, cx| ws.ask_flow_profile_for_shot(cx));
        }
        ScreenshotScenario::FlowRunning => {
            workspace.update(cx, |ws, cx| ws.seed_flow_run_for_shot(false, cx));
        }
        ScreenshotScenario::FlowAsking => {
            workspace.update(cx, |ws, cx| {
                // The panel too: it is where the question is answered, and a
                // capture would otherwise inherit whichever tab and dock width
                // were last persisted.
                ws.reveal_flows_panel(cx);
                ws.seed_flow_run_for_shot(true, cx);
            });
        }
        ScreenshotScenario::FlowDeleteConfirm => {
            // The list the dialog was raised from is the context that says
            // which flow is about to go; the dialog itself is an overlay
            // above it either way.
            workspace.update(cx, |ws, cx| ws.reveal_flows_panel(cx));
            crate::workspace::flow_file_ops::ask_before_deleting(
                std::path::PathBuf::from(FLOW_DELETE_SAMPLE_NAME),
                FLOW_DELETE_SAMPLE_NAME,
                crate::workspace::flow_paths::FlowOrigin::Repo,
                workspace.downgrade(),
                window,
                cx,
            );
        }
        ScreenshotScenario::ErrorModal => {
            dialog_helpers::open_error_report_dialog(sample_report(), window, cx);
        }
        ScreenshotScenario::Toast => {
            workspace.update(cx, |ws, cx| ws.report_error(sample_report(), cx));
        }
        ScreenshotScenario::SessionNotices => {
            workspace.update(cx, |ws, cx| {
                ws.open_agent_chat_transcript_for_shot(window, cx);
                let pane_id = ws.active_runtime().focused_pane_id;
                for notice in sample_session_notices() {
                    ws.report_agent_notice(
                        pane_id,
                        &daruda_acp::AcpEvent::SessionNotice {
                            session_id: "screenshot-session".into(),
                            notice,
                        },
                        cx,
                    );
                }
            });
        }
        ScreenshotScenario::Landing => {
            workspace.update(cx, |ws, cx| ws.empty_workspace_for_shot(window, cx));
        }
        ScreenshotScenario::ScratchTab => {
            workspace.update(cx, |ws, cx| {
                let lane = ws.active_ref();
                let Some(root) = ws.lane_for(lane).map(|w| w.path.clone()) else {
                    return;
                };
                // Two real files from the lane root, so the tab labels are the
                // names the running app would show.
                let mut names: Vec<std::path::PathBuf> = std::fs::read_dir(&root)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter(|e| e.path().is_file())
                    .map(|e| e.path())
                    .collect();
                names.sort();
                let (Some(kept), Some(scratch)) = (names.first(), names.get(1)) else {
                    return;
                };
                ws.open_files_entry(lane, kept.clone(), OpenIntent::Commit, window, cx);
                ws.open_files_entry(lane, scratch.clone(), OpenIntent::Preview, window, cx);
            });
        }
        ScreenshotScenario::Settings(section) => {
            // The same refresh the `OpenSettings` action takes. Without it the
            // Accounts rows capture blank where the running app shows how each
            // account signed in — a scenario that under-reports the real screen
            // is worse than no scenario.
            workspace.update(cx, |ws, cx| {
                ws.probe_auth_statuses(cx);
                ws.open_settings(section, window, cx);
            });
        }
        ScreenshotScenario::SettingsError => {
            workspace.update(cx, |ws, cx| {
                ws.open_settings(BuiltinSection::Notifications, window, cx);
                if let Some(view) = ws.settings_view().cloned() {
                    view.update(cx, |this, cx| this.seed_error_for_shot(cx));
                }
            });
        }
        ScreenshotScenario::SettingsSearch => {
            workspace.update(cx, |ws, cx| {
                ws.open_settings(BuiltinSection::default(), window, cx);
                if let Some(view) = ws.settings_view().cloned() {
                    view.update(cx, |this, cx| this.seed_search_for_shot(window, cx));
                }
            });
        }
        ScreenshotScenario::AgentCatalogExpanded => {
            workspace.update(cx, |ws, cx| {
                ws.open_settings(BuiltinSection::Agent, window, cx);
                if let Some(view) = ws.settings_view().cloned() {
                    view.update(cx, |this, cx| this.seed_agent_catalog_for_shot(window, cx));
                }
            });
        }
        ScreenshotScenario::AgentCatalogEditor(axis) => {
            workspace.update(cx, |ws, cx| {
                ws.open_settings(BuiltinSection::Agent, window, cx);
                if let Some(view) = ws.settings_view().cloned() {
                    view.update(cx, |this, cx| {
                        this.seed_agent_catalog_editor_for_shot(axis, window, cx)
                    });
                }
            });
        }
        ScreenshotScenario::PaneContextMenu => {
            workspace.update(cx, |ws, cx| {
                let pane_id = ws.active_runtime().focused_pane_id;
                let anchor = Point::new(px(PANE_MENU_ANCHOR_X), px(PANE_MENU_ANCHOR_Y));
                ws.open_pane_context_menu_at(pane_id, anchor, window, cx);
            });
        }
        ScreenshotScenario::MermaidLightbox => open_mermaid_lightbox_sample(window, cx),
        ScreenshotScenario::FlowGraph => {
            workspace.update(cx, |ws, cx| {
                // The panel too: the list is where a flow is opened from, and a
                // capture that inherited a collapsed dock would show the graph
                // with no way to have reached it.
                ws.reveal_flows_panel(cx);
                ws.open_first_flow_graph_for_shot(window, cx);
            });
        }
        ScreenshotScenario::FlowCreate => {
            workspace.update(cx, |ws, cx| ws.prompt_new_flow(ws.active, window, cx));
        }
        ScreenshotScenario::FlowGraphRunning => {
            workspace.update(cx, |ws, cx| {
                ws.open_first_flow_graph_running_for_shot(window, cx)
            });
        }
        ScreenshotScenario::FlowGraphForm => {
            workspace.update(cx, |ws, cx| {
                ws.open_first_flow_graph_selected_for_shot(window, cx)
            });
        }
        ScreenshotScenario::FlowGraphPinned => {
            workspace.update(cx, |ws, cx| {
                ws.open_first_flow_graph_pinned_for_shot(window, cx)
            });
        }
        ScreenshotScenario::FlowGraphAuthoring => {
            workspace.update(cx, |ws, cx| {
                ws.open_authoring_flow_graph_for_shot(window, cx)
            });
        }
        ScreenshotScenario::FlowRunDetail => {
            workspace.update(cx, |ws, cx| ws.open_run_detail_for_shot(window, cx));
        }
        ScreenshotScenario::FlowGraphFormRefused => {
            workspace.update(cx, |ws, cx| {
                ws.open_first_flow_graph_refused_for_shot(window, cx)
            });
        }
        ScreenshotScenario::AgentChatFailure => {
            workspace.update(cx, |ws, cx| {
                ws.open_agent_chat_failure_for_shot(
                    daruda_acp::AcpFailure::AuthRequired {
                        message: "Authentication required".into(),
                    },
                    window,
                    cx,
                )
            });
        }
        ScreenshotScenario::AgentChatTransportClosed => {
            workspace.update(cx, |ws, cx| {
                ws.open_agent_chat_failure_for_shot(
                    daruda_acp::AcpFailure::TransportClosed {
                        message: "Incoming transport closed".into(),
                    },
                    window,
                    cx,
                )
            });
        }
        ScreenshotScenario::AgentChatPreparationFailure(kind) => {
            workspace.update(cx, |ws, cx| {
                ws.open_agent_chat_failure_for_shot(
                    daruda_acp::AcpFailure::AdapterInstall {
                        kind,
                        message: "npm setup failed".into(),
                    },
                    window,
                    cx,
                )
            });
        }
        ScreenshotScenario::AgentChatEmpty => {
            workspace.update(cx, |ws, cx| ws.open_agent_chat_empty_for_shot(window, cx));
        }
        ScreenshotScenario::AgentChat => {
            workspace.update(cx, |ws, cx| {
                ws.open_agent_chat_transcript_for_shot(window, cx)
            });
        }
        ScreenshotScenario::AgentChatCompaction => {
            workspace.update(cx, |ws, cx| {
                ws.open_agent_chat_pane_seeded(
                    None,
                    |view, window, cx| {
                        let items = daruda_chat::shot_transcript::compaction_transcript();
                        view.seed_transcript(items, window, cx);
                        view.set_session_id_for_shot("main");
                        view.set_fold_for_shot(
                            daruda_chat::fold::FoldKey::Compaction(1),
                            true,
                            window,
                            cx,
                        );
                    },
                    window,
                    cx,
                );
            });
        }
        ScreenshotScenario::AgentChatAttachments => {
            workspace.update(cx, |ws, cx| ws.open_attachment_draft_for_shot(window, cx));
        }
        ScreenshotScenario::OrchestratorChip => {
            workspace.update(cx, |ws, cx| {
                ws.seed_orchestrator_for_shot(false, window, cx)
            });
        }
        ScreenshotScenario::OrchestratorTab => {
            workspace.update(cx, |ws, cx| ws.seed_orchestrator_for_shot(true, window, cx));
        }
        ScreenshotScenario::OrchestratorApproval => {
            use crate::control::approval;
            use crate::remote_channel::global::Delivery;
            workspace.update(cx, |ws, cx| {
                ws.seed_orchestrator_for_shot(true, window, cx);
                let pane = ws.orchestrator_chat_pane().unwrap();
                ws.agent_chat_view(pane.pane)
                    .unwrap()
                    .update(cx, |view, cx| {
                        view.seed_transcript(Vec::new(), window, cx);
                    });
            });
            crate::window_registry::WindowRegistry::register_orchestrator(
                window.window_handle(),
                workspace.downgrade(),
                cx,
            );
            let (settled, _) = approval::request(
                crate::surface::strings::control::approval_task_start("Review changes"),
                Delivery::Presence { away: false },
                cx,
            );
            let _ = approval::request(
                crate::surface::strings::control::approval_task_create(
                    "Add desktop approvals",
                    "Daruda",
                ),
                Delivery::Presence { away: false },
                cx,
            );
            cx.defer(move |cx| {
                approval::resolve_desktop(settled.0, "approve", cx);
            });
        }
        ScreenshotScenario::LeftDockGit => {
            workspace.update(cx, |ws, cx| ws.seed_left_dock_git_for_shot(window, cx));
        }
        ScreenshotScenario::LeftDockGroups
        | ScreenshotScenario::LeftDockWorktrees
        | ScreenshotScenario::LeftDockGroupStatus
        | ScreenshotScenario::LeftDockGroupHover
        | ScreenshotScenario::LeftDockPaneHover
        | ScreenshotScenario::LeftDockProjectAlias => {
            workspace.update(cx, |ws, cx| {
                ws.seed_left_dock_groups_for_shot(
                    scenario == ScreenshotScenario::LeftDockWorktrees,
                    window,
                    cx,
                );
                if matches!(
                    scenario,
                    ScreenshotScenario::LeftDockGroupStatus
                        | ScreenshotScenario::LeftDockGroupHover
                ) {
                    ws.seed_collapsed_group_status_for_shot(window, cx);
                }
                if scenario == ScreenshotScenario::LeftDockProjectAlias {
                    ws.open_project_alias_editor(ws.active.project, window, cx);
                }
            });
            if matches!(
                scenario,
                ScreenshotScenario::LeftDockGroupHover | ScreenshotScenario::LeftDockPaneHover
            ) {
                use crate::ui::theme;
                let hover_offset = if scenario == ScreenshotScenario::LeftDockPaneHover {
                    theme::CONTROL_TARGET_SIZE
                        + theme::DOCK_RAIL_ROW_H
                        + theme::DOCK_RAIL_PANE_ROW_H / 2.0
                } else {
                    theme::CONTROL_TARGET_SIZE / 2.0
                };
                window.draw(cx).clear();
                window.dispatch_event(
                    gpui::PlatformInput::MouseMove(gpui::MouseMoveEvent {
                        position: gpui::point(
                            px(theme::DOCK_LEFT_RAIL_W / 2.0),
                            px(theme::TITLE_BAR_HEIGHT
                                + theme::PAD_XS * 2.0
                                + theme::CONTROL_TARGET_SIZE * 5.0
                                + theme::GAP_LG
                                + hover_offset),
                        ),
                        pressed_button: None,
                        modifiers: Default::default(),
                    }),
                    cx,
                );
            }
        }
        ScreenshotScenario::LeftDockSessions
        | ScreenshotScenario::LeftDockSessionsSix
        | ScreenshotScenario::LeftDockRail
        | ScreenshotScenario::LeftDockExpanded => {
            workspace.update(cx, |ws, cx| {
                ws.seed_left_dock_for_shot(
                    matches!(
                        scenario,
                        ScreenshotScenario::LeftDockSessions
                            | ScreenshotScenario::LeftDockSessionsSix
                    ),
                    scenario == ScreenshotScenario::LeftDockExpanded,
                    scenario == ScreenshotScenario::LeftDockSessionsSix,
                    window,
                    cx,
                )
            });
        }
        ScreenshotScenario::TabIndicators => {
            workspace.update(cx, |ws, cx| ws.seed_tab_indicators_for_shot(window, cx));
        }
        ScreenshotScenario::AgentChatWorking => {
            workspace.update(cx, |ws, cx| {
                ws.open_agent_chat_working_transcript_for_shot(window, cx)
            });
        }
        ScreenshotScenario::AgentChatNarrowed => {
            workspace.update(cx, |ws, cx| {
                ws.open_agent_chat_narrowed_transcript_for_shot(window, cx)
            });
        }
        ScreenshotScenario::AgentChatInterrupted => {
            workspace.update(cx, |ws, cx| {
                ws.open_agent_chat_interrupted_transcript_for_shot(window, cx)
            });
        }
        ScreenshotScenario::AgentChatOverageStopped => {
            workspace.update(cx, |ws, cx| {
                ws.open_agent_chat_overage_stopped_for_shot(window, cx)
            });
        }
        ScreenshotScenario::AgentChatCliRunning => {
            workspace.update(cx, |ws, cx| {
                ws.open_cli_snapshot_for_shot(false, window, cx)
            });
        }
        ScreenshotScenario::AgentChatCliEnded => {
            workspace.update(cx, |ws, cx| ws.open_cli_snapshot_for_shot(true, window, cx));
        }
        ScreenshotScenario::AgentChatQueueParked => {
            workspace.update(cx, |ws, cx| {
                ws.open_agent_chat_parked_queue_for_shot(false, window, cx)
            });
        }
        ScreenshotScenario::AgentChatQueueArmed => {
            workspace.update(cx, |ws, cx| {
                ws.open_agent_chat_parked_queue_for_shot(true, window, cx)
            });
        }
        ScreenshotScenario::AgentChatSoleReply => {
            workspace.update(cx, |ws, cx| {
                ws.open_agent_chat_sole_reply_for_shot(window, cx)
            });
        }
        ScreenshotScenario::AgentChatRunningTool => {
            workspace.update(cx, |ws, cx| {
                ws.open_agent_chat_running_tool_for_shot(window, cx)
            });
        }
        ScreenshotScenario::AgentChatPlan => {
            workspace.update(cx, |ws, cx| {
                ws.open_agent_chat_plan_for_shot(false, window, cx)
            });
        }
        ScreenshotScenario::AgentChatPlanStopped => {
            workspace.update(cx, |ws, cx| {
                ws.open_agent_chat_plan_for_shot(true, window, cx)
            });
        }
        ScreenshotScenario::AgentChatFold => {
            workspace.update(cx, |ws, cx| {
                ws.open_agent_chat_fold_editor_for_shot(window, cx)
            });
        }
        ScreenshotScenario::AgentChatTail => {
            workspace.update(cx, |ws, cx| {
                ws.open_agent_chat_tail_boundary_for_shot(false, window, cx)
            });
        }
        ScreenshotScenario::AgentChatTailOpen => {
            workspace.update(cx, |ws, cx| {
                ws.open_agent_chat_tail_boundary_for_shot(true, window, cx)
            });
        }
        ScreenshotScenario::AgentChatGroupTail => {
            workspace.update(cx, |ws, cx| {
                ws.open_agent_chat_group_tail_boundary_for_shot(false, window, cx)
            });
        }
        ScreenshotScenario::AgentChatGroupTailOpen => {
            workspace.update(cx, |ws, cx| {
                ws.open_agent_chat_group_tail_boundary_for_shot(true, window, cx)
            });
        }
        ScreenshotScenario::AgentChatSubagentTail => {
            workspace.update(cx, |ws, cx| {
                ws.open_agent_chat_subagent_tail_boundary_for_shot(false, window, cx)
            });
        }
        ScreenshotScenario::AgentChatSubagentTailOpen => {
            workspace.update(cx, |ws, cx| {
                ws.open_agent_chat_subagent_tail_boundary_for_shot(true, window, cx)
            });
        }
        ScreenshotScenario::AgentChatThoughts(shot) => {
            workspace.update(cx, |ws, cx| {
                ws.open_agent_chat_thoughts_for_shot(shot, window, cx)
            });
        }
        ScreenshotScenario::AgentChatOptions(tab) => {
            workspace.update(cx, |ws, cx| {
                ws.open_agent_chat_options_for_shot(tab, window, cx)
            });
        }
    }
}

/// Render [`MERMAID_LIGHTBOX_SAMPLE`] and open it in the lightbox — no chat
/// history needed, the same render path the diagram card's zoom button uses.
fn open_mermaid_lightbox_sample(window: &mut Window, cx: &mut App) {
    use super::main_area::file_view_pane::visual::render_mermaid_raster;
    use crate::ui::CachedImage;
    use crate::ui::mermaid_palette;
    use daruda_chat::render::image_lightbox;

    let palette = mermaid_palette::default_palette();
    let Some(raster) = render_mermaid_raster(MERMAID_LIGHTBOX_SAMPLE, &palette) else {
        return;
    };
    let Some(image) = CachedImage::from_raster(raster) else {
        return;
    };
    image_lightbox::open(&image, window, cx);
}

/// Synthetic report for the error-modal scenario — representative of a real
/// Layer-2 details view (title, message, context table, source location).
fn sample_report() -> ErrorReport {
    ErrorReport::new("Screenshot scenario")
        .severity(ErrorSeverity::Error)
        .message("Synthetic error report for visual verification")
        .at(file!(), line!())
        .with_context("scenario", "error-modal")
        .dedup("screenshot.scenario.error_modal")
        .build()
}

fn sample_session_notices() -> [daruda_acp::SessionNoticeView; 3] {
    use daruda_acp::{NoticeSeverityView, SessionNoticeView};
    [
        SessionNoticeView {
            severity: NoticeSeverityView::Warning,
            title: "Optional integration unavailable".into(),
            description: Some(
                "Continuing with the tools already available in this session.".into(),
            ),
        },
        SessionNoticeView {
            severity: NoticeSeverityView::Error,
            title: "An auxiliary service could not be reached".into(),
            description: Some(
                "The agent is continuing its work.\nThis notice does not end the turn.".into(),
            ),
        },
        SessionNoticeView {
            severity: NoticeSeverityView::Other("_provider/update".into()),
            title: "**Plain-text** provider notice".into(),
            description: None,
        },
    ]
}

#[cfg(test)]
mod tests;
