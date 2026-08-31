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

use super::main_area::agent_chat_pane::view::ActivityOptionsTab;
use super::{ToggleCommandPalette, Workspace, dialog_helpers};
use daruda_config::BuiltinSection;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};

/// CLI token for the command-palette scenario.
const NAME_COMMAND_PALETTE: &str = "command-palette";
/// CLI token for the error-report-modal scenario.
const NAME_ERROR_MODAL: &str = "error-modal";
/// CLI token for the error-toast scenario.
const NAME_TOAST: &str = "toast";
/// CLI token for the Settings-window scenario. Bare opens the default section;
/// `settings:<slug>` opens a specific section (e.g. `settings:font`).
const NAME_SETTINGS: &str = "settings";
/// CLI token for the pane context-menu scenario.
const NAME_PANE_CONTEXT_MENU: &str = "pane-context-menu";
/// CLI token for the mermaid-diagram lightbox scenario.
const NAME_MERMAID_LIGHTBOX: &str = "mermaid-lightbox";
/// CLI token for the flow-graph pane scenario.
const NAME_FLOW_GRAPH: &str = "flow-graph";
/// CLI token for the flow-graph pane with a run colouring it.
const NAME_FLOW_GRAPH_RUNNING: &str = "flow-graph-running";
/// CLI token for the flow-graph pane with a node selected and its inspector up.
const NAME_FLOW_GRAPH_FORM: &str = "flow-graph-form";
/// CLI token for the inspector showing why a save was refused.
const NAME_FLOW_GRAPH_FORM_REFUSED: &str = "flow-graph-form-refused";
const NAME_FLOW_GRAPH_PINNED: &str = "flow-graph-pinned";
/// CLI token for the card affordances that only exist while authoring: an
/// issue count, a failure policy, a dropped pin's reason, inherited defaults.
const NAME_FLOW_GRAPH_AUTHORING: &str = "flow-graph-authoring";
const NAME_AGENT_CHAT_FAILURE: &str = "agent-chat-failure";
/// CLI token for an empty agent-chat pane with its view options open.
const NAME_AGENT_CHAT_EMPTY: &str = "agent-chat-empty";
/// CLI token for the settled-transcript scenario.
const NAME_AGENT_CHAT: &str = "agent-chat";
/// CLI token for the mid-turn transcript, before the agent writes its answer.
const NAME_AGENT_CHAT_WORKING: &str = "agent-chat-working";
/// CLI token for the same transcript with the filter and tail chips engaged.
const NAME_AGENT_CHAT_NARROWED: &str = "agent-chat-narrowed";
/// CLI token for the transcript with the custom fold editor open.
const NAME_AGENT_CHAT_FOLD: &str = "agent-chat-fold";
/// CLI token for the tail window's boundary row, closed.
const NAME_AGENT_CHAT_TAIL: &str = "agent-chat-tail";
/// CLI token for the same boundary row, open.
const NAME_AGENT_CHAT_TAIL_OPEN: &str = "agent-chat-tail-open";
/// CLI token prefix for the compact bar's combined options popover, suffixed
/// with an [`ActivityOptionsTab`] token (`agent-chat-options:filter`). Bare
/// `agent-chat-options` opens the Fold tab.
const NAME_AGENT_CHAT_OPTIONS: &str = "agent-chat-options";
/// CLI token for the flow delete confirmation, on the repository's copy.
const NAME_FLOW_DELETE_CONFIRM: &str = "flow-delete-confirm";
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

const NAME_FLOW_PICKER: &str = "flow-picker";
const NAME_FLOW_PROFILE_PICKER: &str = "flow-profile-picker";
const NAME_FLOW_RESUMABLE: &str = "flow-resumable";
const NAME_FLOW_RUNNING: &str = "flow-running";
const NAME_FLOW_ASKING: &str = "flow-asking";

/// Where the pane menu is deployed for the capture, in window coordinates.
/// Near the top-left of the content area so the menu opens downward at its
/// natural length — the shot is meant to show every entry, not the edge flip.
const PANE_MENU_ANCHOR_X: f32 = 320.;
const PANE_MENU_ANCHOR_Y: f32 = 160.;

/// A transient UI state to drive into view before a `--screenshot` capture.
/// One scenario per capture — these overlays are mutually exclusive on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScreenshotScenario {
    /// Open the command palette (`CommandPaletteState::open`).
    CommandPalette,
    /// Open the Layer-2 error-report modal with a synthetic report.
    ErrorModal,
    /// Push a synthetic error toast.
    Toast,
    /// Open the Settings window at the given section.
    Settings(BuiltinSection),
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
    /// An agent-chat pane parked on an expired login: the connect banner with
    /// its remedy buttons, and the failure card the conversation ends on.
    AgentChatFailure,
    /// A fresh agent-chat pane before the first prompt, with view options open.
    AgentChatEmpty,
    /// A settled transcript in the default state. The only way to look at the
    /// Step layer and the three activity-bar chips at once — whether the chips
    /// fit beside the pane title, whether they read as three controls, and
    /// whether a Step header's glyph says anything about what it folded.
    AgentChat,
    /// The transcript mid-turn: the agent has run tools and written the
    /// preamble in front of them, but not yet the answer. The run's last prose
    /// therefore sits *inside* a step, which is the only shape where a step
    /// header and the prose row beneath it can end up saying the same line —
    /// and the shape [`Self::AgentChat`]'s settled seed cannot reach.
    AgentChatWorking,
    /// The same transcript with the display filter engaged and the folds open,
    /// so the filter's reveal chip renders on the response bar — the only way
    /// to check that it reads as a view control beside the run's counts rather
    /// than as one more step in the list below it.
    AgentChatNarrowed,
    /// The same transcript with a custom fold matrix and its editor open.
    AgentChatFold,
    /// The tail window's boundary row with nothing floating over it. The pair
    /// with [`Self::AgentChatTailOpen`] is the only way to judge the one thing
    /// the row exists to say — whether its two states are distinguishable —
    /// since `agent-chat-narrowed` covers it with the filter popover and no
    /// state test can look at a rule, an inset label, or a rail.
    AgentChatTail,
    /// The boundary open: the label anchors left and the steps it revealed carry
    /// the rail.
    AgentChatTailOpen,
    /// The compact Activity Bar's combined options popover, open on one tab
    /// with every axis off its default — the only way to see the gear's own
    /// selected state, whether the tab strip reads as three choices, and
    /// whether a tab's panel matches the chip that opens it on a wide bar.
    ///
    /// Forces the compact layout rather than narrowing the pane, so it captures
    /// that bar's chrome at full width and does *not* exercise the breakpoint
    /// itself.
    AgentChatOptions(ActivityOptionsTab),
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
    /// state test can look at, and the surface Task 1 proved needs eyes.
    FlowAsking,
}

impl ScreenshotScenario {
    /// Map a CLI token to a scenario. Unknown tokens return `None`.
    /// `settings:<slug>` selects a Settings section; bare `settings` uses the
    /// default section.
    pub(crate) fn from_cli_name(name: &str) -> Option<Self> {
        match name {
            NAME_COMMAND_PALETTE => Some(Self::CommandPalette),
            NAME_ERROR_MODAL => Some(Self::ErrorModal),
            NAME_TOAST => Some(Self::Toast),
            NAME_SETTINGS => Some(Self::Settings(BuiltinSection::default())),
            NAME_PANE_CONTEXT_MENU => Some(Self::PaneContextMenu),
            NAME_MERMAID_LIGHTBOX => Some(Self::MermaidLightbox),
            NAME_FLOW_GRAPH => Some(Self::FlowGraph),
            NAME_FLOW_GRAPH_RUNNING => Some(Self::FlowGraphRunning),
            NAME_FLOW_GRAPH_FORM => Some(Self::FlowGraphForm),
            NAME_FLOW_GRAPH_FORM_REFUSED => Some(Self::FlowGraphFormRefused),
            NAME_FLOW_GRAPH_PINNED => Some(Self::FlowGraphPinned),
            NAME_FLOW_GRAPH_AUTHORING => Some(Self::FlowGraphAuthoring),
            NAME_AGENT_CHAT_FAILURE => Some(Self::AgentChatFailure),
            NAME_AGENT_CHAT_EMPTY => Some(Self::AgentChatEmpty),
            NAME_AGENT_CHAT => Some(Self::AgentChat),
            NAME_AGENT_CHAT_WORKING => Some(Self::AgentChatWorking),
            NAME_AGENT_CHAT_NARROWED => Some(Self::AgentChatNarrowed),
            NAME_AGENT_CHAT_FOLD => Some(Self::AgentChatFold),
            NAME_AGENT_CHAT_TAIL => Some(Self::AgentChatTail),
            NAME_AGENT_CHAT_TAIL_OPEN => Some(Self::AgentChatTailOpen),
            NAME_AGENT_CHAT_OPTIONS => Some(Self::AgentChatOptions(ActivityOptionsTab::Fold)),
            NAME_FLOW_PICKER => Some(Self::FlowPicker),
            NAME_FLOW_PROFILE_PICKER => Some(Self::FlowProfilePicker),
            NAME_FLOW_RESUMABLE => Some(Self::FlowResumable),
            NAME_FLOW_RUNNING => Some(Self::FlowRunning),
            NAME_FLOW_ASKING => Some(Self::FlowAsking),
            NAME_FLOW_DELETE_CONFIRM => Some(Self::FlowDeleteConfirm),
            _ => name
                .strip_prefix(NAME_AGENT_CHAT_OPTIONS)
                .and_then(|rest| rest.strip_prefix(':'))
                .and_then(ActivityOptionsTab::from_token)
                .map(Self::AgentChatOptions)
                .or_else(|| Self::settings_section_from_cli_name(name)),
        }
    }

    fn settings_section_from_cli_name(name: &str) -> Option<Self> {
        name.strip_prefix(concat!("settings", ":"))
            .and_then(BuiltinSection::from_slug)
            .map(Self::Settings)
    }
}

/// Drive `scenario` into view on `workspace`'s `window`. Called from the
/// screenshot capture path with the live workspace window in scope.
pub(crate) fn drive(
    scenario: ScreenshotScenario,
    workspace: &Entity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) {
    match scenario {
        ScreenshotScenario::CommandPalette => {
            workspace.update(cx, |ws, cx| {
                ws.on_toggle_command_palette(&ToggleCommandPalette, window, cx);
            });
        }
        ScreenshotScenario::FlowPicker => {
            workspace.update(cx, |ws, cx| {
                ws.open_flow_picker(
                    crate::workspace::command::flow_picker::FlowPurpose::Validate,
                    cx,
                );
            });
        }
        ScreenshotScenario::FlowResumable => {
            workspace.update(cx, |ws, cx| ws.seed_crashed_run_for_shot(cx));
        }
        ScreenshotScenario::FlowProfilePicker => {
            workspace.update(cx, |ws, cx| ws.ask_flow_profile_for_shot(cx));
        }
        ScreenshotScenario::FlowRunning => {
            workspace.update(cx, |ws, cx| ws.seed_flow_run_for_shot(false, window, cx));
        }
        ScreenshotScenario::FlowAsking => {
            workspace.update(cx, |ws, cx| {
                // The panel too: it is where the question is answered, and a
                // capture would otherwise inherit whichever tab and dock width
                // were last persisted.
                ws.reveal_flows_panel(cx);
                ws.seed_flow_run_for_shot(true, window, cx);
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
        ScreenshotScenario::Settings(section) => {
            // The same refresh the `OpenSettings` action takes. Without it the
            // Accounts rows capture blank where the running app shows how each
            // account signed in — a scenario that under-reports the real screen
            // is worse than no scenario.
            workspace.update(cx, |ws, cx| ws.probe_auth_statuses(cx));
            crate::windows::open_settings_window(section, window, cx);
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
        ScreenshotScenario::FlowGraphFormRefused => {
            workspace.update(cx, |ws, cx| {
                ws.open_first_flow_graph_refused_for_shot(window, cx)
            });
        }
        ScreenshotScenario::AgentChatFailure => {
            workspace.update(cx, |ws, cx| ws.open_agent_chat_failure_for_shot(window, cx));
        }
        ScreenshotScenario::AgentChatEmpty => {
            workspace.update(cx, |ws, cx| ws.open_agent_chat_empty_for_shot(window, cx));
        }
        ScreenshotScenario::AgentChat => {
            workspace.update(cx, |ws, cx| {
                ws.open_agent_chat_transcript_for_shot(window, cx)
            });
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
    use super::main_area::agent_chat_pane::render::mermaid_lightbox;
    use super::main_area::file_view_pane::mermaid_theme::MermaidPalette;
    use super::main_area::file_view_pane::render::CachedImage;
    use super::main_area::file_view_pane::visual::render_mermaid_raster;

    let palette = MermaidPalette::default();
    let Some(raster) = render_mermaid_raster(MERMAID_LIGHTBOX_SAMPLE, &palette) else {
        return;
    };
    let Some(image) = CachedImage::from_raster(&raster) else {
        return;
    };
    mermaid_lightbox::open(&image, window, cx);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_palette_name_maps_to_scenario() {
        assert_eq!(
            ScreenshotScenario::from_cli_name("command-palette"),
            Some(ScreenshotScenario::CommandPalette)
        );
    }

    #[test]
    fn error_modal_name_maps_to_scenario() {
        assert_eq!(
            ScreenshotScenario::from_cli_name("error-modal"),
            Some(ScreenshotScenario::ErrorModal)
        );
    }

    #[test]
    fn unknown_name_maps_to_none() {
        assert_eq!(ScreenshotScenario::from_cli_name("nope"), None);
        assert_eq!(ScreenshotScenario::from_cli_name(""), None);
    }

    #[test]
    fn bare_settings_maps_to_default_section() {
        assert_eq!(
            ScreenshotScenario::from_cli_name("settings"),
            Some(ScreenshotScenario::Settings(BuiltinSection::default()))
        );
    }

    #[test]
    fn settings_slug_maps_to_that_section() {
        assert_eq!(
            ScreenshotScenario::from_cli_name("settings:font"),
            Some(ScreenshotScenario::Settings(BuiltinSection::Font))
        );
        assert_eq!(
            ScreenshotScenario::from_cli_name("settings:notifications"),
            Some(ScreenshotScenario::Settings(BuiltinSection::Notifications))
        );
    }

    #[test]
    fn unknown_settings_slug_maps_to_none() {
        assert_eq!(ScreenshotScenario::from_cli_name("settings:bogus"), None);
    }

    #[test]
    fn mermaid_lightbox_name_maps_to_scenario() {
        assert_eq!(
            ScreenshotScenario::from_cli_name("mermaid-lightbox"),
            Some(ScreenshotScenario::MermaidLightbox)
        );
    }

    #[test]
    fn agent_chat_names_map_to_their_scenarios() {
        assert_eq!(
            ScreenshotScenario::from_cli_name("agent-chat-empty"),
            Some(ScreenshotScenario::AgentChatEmpty)
        );
        assert_eq!(
            ScreenshotScenario::from_cli_name("agent-chat"),
            Some(ScreenshotScenario::AgentChat)
        );
        assert_eq!(
            ScreenshotScenario::from_cli_name("agent-chat-working"),
            Some(ScreenshotScenario::AgentChatWorking)
        );
        assert_eq!(
            ScreenshotScenario::from_cli_name("agent-chat-narrowed"),
            Some(ScreenshotScenario::AgentChatNarrowed)
        );
        assert_eq!(
            ScreenshotScenario::from_cli_name("agent-chat-fold"),
            Some(ScreenshotScenario::AgentChatFold)
        );
        assert_eq!(
            ScreenshotScenario::from_cli_name("agent-chat-tail"),
            Some(ScreenshotScenario::AgentChatTail)
        );
        assert_eq!(
            ScreenshotScenario::from_cli_name("agent-chat-tail-open"),
            Some(ScreenshotScenario::AgentChatTailOpen)
        );
        assert_eq!(
            ScreenshotScenario::from_cli_name("agent-chat-failure"),
            Some(ScreenshotScenario::AgentChatFailure)
        );
    }

    /// Every tab the panel offers has to be reachable from the CLI, or a tab
    /// silently loses its permanent capture coverage.
    #[test]
    fn every_options_tab_is_addressable_by_its_own_token() {
        assert_eq!(
            ScreenshotScenario::from_cli_name("agent-chat-options"),
            Some(ScreenshotScenario::AgentChatOptions(
                ActivityOptionsTab::Fold
            ))
        );
        for tab in ActivityOptionsTab::ALL {
            assert_eq!(
                ScreenshotScenario::from_cli_name(&format!("agent-chat-options:{}", tab.token())),
                Some(ScreenshotScenario::AgentChatOptions(tab)),
                "{tab:?}"
            );
        }
        assert_eq!(
            ScreenshotScenario::from_cli_name("agent-chat-options:nope"),
            None
        );
    }

    #[test]
    fn toast_name_maps_to_scenario() {
        assert_eq!(
            ScreenshotScenario::from_cli_name("toast"),
            Some(ScreenshotScenario::Toast)
        );
    }
}
