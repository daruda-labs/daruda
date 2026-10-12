//! The Workspace hosting AgentChat panes, by host concern. Tests build the view
//! via `create_agent_chat_pane`, which opens no connection, so `handle` stays
//! `None` while host-side state transitions still run. The helpers here are
//! shared with the other workspace tests that open a chat pane.

mod composer;
mod config;
mod panes;
mod restore;

use daruda_store::project::PaneCwd;
use gpui::{AppContext as _, Entity, TestAppContext};

use super::build_workspace;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane::PaneContent;
use crate::workspace::main_area::pane_tree::PaneId;
use daruda_chat::pane_choice::PaneChoice;
use daruda_chat::rows::tail::{TailLevel, TailWindow};
use daruda_chat::view::{AgentChatView, AgentSessionStatus, ChatContentWidth, ChatPaneChoices};
use daruda_transcript::display_filter::{DisplayFilter, FilterFacet};
use daruda_transcript::fold_mode::{FoldMode, FoldPreset};

/// Fetch the `AgentChatView` entity for `pane_id` (panics if the pane is gone
/// or is not an AgentChat pane).
pub(super) fn agent_view(ws: &Workspace, pane_id: PaneId) -> Entity<AgentChatView> {
    ws.active_runtime()
        .panes
        .iter()
        .find(|p| p.id == pane_id)
        .and_then(|p| p.agent_chat_view())
        .cloned()
        .expect("agent chat pane present")
}

/// The queued-prompt texts in FIFO order — the queue holds `QueuedPrompt`
/// (id + text), so tests compare on the text projection.
fn queue_texts(v: &AgentChatView) -> Vec<String> {
    v.queue()
        .in_strip_order()
        .filter(|(_, parked)| !parked)
        .map(|(q, _)| q.text().to_owned())
        .collect()
}

/// A non-default `codex` id. Used by the restore-wiring tests below to prove the
/// persisted `agent_id` is threaded through `restore_from_disk` (not just the
/// pure `resolve_restored_agent` seam covered by unit tests).
fn codex_agent() -> daruda_config::AgentDefinition {
    daruda_config::AgentDefinition::new(
        "codex".to_string(),
        "Codex".to_string(),
        daruda_config::AgentLaunch::Raw("codex-acp".to_string()),
    )
}

/// The restore-visible slice of one agent-chat pane. A struct rather than a
/// tuple so each persisted preference is asserted by name — the mis-wire this
/// covers (one preference's tokens read into another's field) is invisible
/// when the assertions read by position.
struct RestoredChat {
    agent_id: String,
    session_id: Option<String>,
    title: Option<String>,
    replaying: bool,
    dormant: bool,
    content_width: Option<ChatContentWidth>,
    tail_steps: PaneChoice<TailWindow>,
    tail_calls: PaneChoice<TailWindow>,
    display_filter: PaneChoice<DisplayFilter>,
    fold_mode: Option<FoldMode>,
}

fn restored_chats(ws: &Workspace, cx: &gpui::App) -> Vec<RestoredChat> {
    ws.active_runtime()
        .panes
        .iter()
        .filter_map(|p| p.agent_chat_view())
        .map(|view| {
            let view = view.read(cx);
            RestoredChat {
                agent_id: view.agent_id().to_owned(),
                session_id: view.session_id().map(str::to_owned),
                title: view.session_title().map(str::to_owned),
                replaying: view.is_replaying(),
                dormant: view.any_handle().is_none(),
                content_width: view.pane_choices().content_width,
                tail_steps: view.tail_steps_for_test(),
                tail_calls: view.tail_calls_for_test(),
                display_filter: view.display_filter_for_test(),
                fold_mode: view.pane_choices().fold_mode,
            }
        })
        .collect()
}

fn push_agent_chat_pane(
    cx: &mut TestAppContext,
    window_handle: gpui::WindowHandle<crate::ui::Root>,
    workspace: &Entity<Workspace>,
    cwd: PaneCwd,
) -> PaneId {
    cx.update_window(window_handle.into(), |_, window, cx| {
        workspace.update(cx, |ws, cx| {
            let pane = ws.create_agent_chat_pane(
                Some(cwd),
                None,
                daruda_config::AgentDefinition::claude_default().id,
                None,
                window,
                cx,
            );
            let pane_id = pane.id;
            ws.active_runtime_mut().panes.push(pane);
            pane_id
        })
    })
    .unwrap()
}

/// The built-in Claude entry with transcript axes stated on it. The reload
/// tests open their panes on that id, so this is how a changed config reaches
/// them now that the entry is the only layer above the built-in values.
fn claude_entry_with(tail_window: u8, fold_mode: &str) -> daruda_config::AgentEntry {
    daruda_config::AgentEntry::custom(daruda_config::AgentDefinition {
        tail_window: Some(tail_window),
        fold_mode: Some(vec![fold_mode.to_string()]),
        ..daruda_config::AgentDefinition::claude_default()
    })
}

/// The two agent ids the per-agent transcript tests below run their panes on.
const TRANSCRIPT_AGENT_A: &str = "transcript-agent-a";
const TRANSCRIPT_AGENT_B: &str = "transcript-agent-b";

/// One `[[agents]]` row stating all three transcript axes. Callers give the two
/// agents disagreeing values on every axis, so a pane on one can never be
/// mistaken for a pane on the other. The id doubles as the name: these entries
/// exist only to be told apart.
fn transcript_agent_entry(
    id: &str,
    tail_window: u8,
    fold_mode: &str,
    filter: FilterFacet,
) -> daruda_config::AgentEntry {
    daruda_config::AgentEntry::custom(daruda_config::AgentDefinition {
        id: id.to_string(),
        name: id.to_string(),
        fold_mode: Some(vec![fold_mode.to_string()]),
        tail_window: Some(tail_window),
        display_filter: Some(vec![filter.token().to_string()]),
        ..daruda_config::AgentDefinition::claude_default()
    })
}

/// Open a chat pane on `agent_id` in the active runtime, in its own tab, and
/// return its id.
fn open_agent_chat_pane_on(
    ws: &mut Workspace,
    agent_id: &str,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<Workspace>,
) -> PaneId {
    use crate::workspace::main_area::pane::TabEntry;
    use crate::workspace::main_area::pane_tree::PaneLayout;

    let cwd = ws.active_lane().map(|w| PaneCwd::Local(w.path.clone()));
    let pane = ws.create_agent_chat_pane(cwd, None, agent_id.to_string(), None, window, cx);
    let pane_id = pane.id;
    ws.active_runtime_mut().panes.push(pane);
    let tab_id = ws.alloc_id();
    ws.active_runtime_mut().tabs.push(TabEntry {
        id: tab_id,
        layout: PaneLayout::Pane(pane_id),
        last_focused_pane: pane_id,
        user_label: None,
    });
    pane_id
}

/// The three transcript axes a pane currently sits on, as one tuple: the tests
/// below compare two panes against each other as well as against config. The
/// tail axis contributes its step level — the one `claude_entry_with`'s
/// `tail_window` states.
fn transcript_settings(
    view: &Entity<AgentChatView>,
    cx: &mut TestAppContext,
) -> (PaneChoice<TailWindow>, FoldMode, PaneChoice<DisplayFilter>) {
    view.read_with(cx, |v, _| {
        (
            v.tail_steps_for_test(),
            v.fold_mode_for_test().value(),
            v.display_filter_for_test(),
        )
    })
}

/// A `Model`-category select advertising `choices`, plus a `ThoughtLevel`
/// select that must never be mistaken for the model axis.
fn model_options(choices: &[(&str, &str)]) -> Vec<daruda_acp::ConfigOptionView> {
    use daruda_acp::{
        ConfigChoiceView, ConfigOptionCategoryView, ConfigOptionKindView, ConfigOptionView,
    };
    vec![
        ConfigOptionView {
            id: "model".into(),
            name: "Model".into(),
            description: None,
            category: ConfigOptionCategoryView::Model,
            kind: ConfigOptionKindView::Select {
                current_value: choices[0].0.to_string(),
                options: choices
                    .iter()
                    .map(|(value, name)| ConfigChoiceView {
                        value: (*value).to_string(),
                        name: (*name).to_string(),
                        description: None,
                    })
                    .collect(),
            },
        },
        ConfigOptionView {
            id: "effort".into(),
            name: "Effort".into(),
            description: None,
            category: ConfigOptionCategoryView::ThoughtLevel,
            kind: ConfigOptionKindView::Select {
                current_value: "high".into(),
                options: vec![ConfigChoiceView {
                    value: "high".into(),
                    name: "High".into(),
                    description: None,
                }],
            },
        },
    ]
}
