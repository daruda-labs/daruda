//! Workspace ops for the Agent chat pane — pane construction, the
//! open-in-new-tab affordance, the live ACP connection + event pump, and
//! the prompt / cancel / permission handlers.
//!
//! Kept GPUI-side (creates focus handles + input entities, mutates the
//! pane/tab tree, drives `cx.spawn`) and separate from the renderer so the
//! render module stays a pure view of `AgentChatContent`.
//!
//! ## Connection + pump shape
//!
//! ```text
//!   create_agent_chat_pane
//!         │  builds Pane (status = Connecting, handle = None)
//!         ▼
//!   connect_agent_chat (cx.spawn)
//!         │  connect_session on bg executor → (handle, rx)
//!         │  store handle on the pane, status → Connected on first event
//!         ▼
//!   event pump (cx.spawn, weak workspace)
//!         while rx.next().await:
//!           Connected            → status = Connected
//!           Update(u)            → apply_update(items, u)
//!           PermissionRequested  → items.push(permission_item); pending = Some(id)
//!           TurnEnded            → finalize_streaming(items)
//!           Error(e)             → status = Error; report_error
//!         each arm: cx.notify(workspace) so the cached pane subtree repaints
//! ```
//!
//! Both the handle and the pump task live on `AgentChatContent`, so closing
//! the pane drops them: the handle drop closes the command channel (the
//! connection task exits) and the pump-task drop ends the loop. No explicit
//! teardown code is needed.

use daruda_acp::{
    AcpEvent, AdapterCommand, PermissionDecision, PermissionKindView, apply_update,
    connect_session, finalize_streaming, permission_item,
};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use futures::StreamExt as _;
use gpui::{Context, Window};

use crate::surface::strings as s;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane::{
    AgentChatContent, AgentSessionStatus, Pane, PaneContent, TabEntry,
};
use crate::workspace::main_area::pane_tree::{PaneId, PaneLayout};

impl Workspace {
    /// Construct an Agent chat `Pane` (no tab side-effects). Allocates
    /// the pane id and a focus handle, seeds the conversation as empty,
    /// parks the session in `Connecting`, and kicks off
    /// [`Self::connect_agent_chat`] so the live ACP session attaches in
    /// the background. The prompt input is the shared bottom-dock input,
    /// not a per-pane field. Used by [`Self::open_agent_chat_pane`] and
    /// by session restore.
    pub(in crate::workspace) fn create_agent_chat_pane(
        &mut self,
        cwd: Option<std::path::PathBuf>,
        cx: &mut Context<Self>,
    ) -> Pane {
        let pane_id = self.alloc_id();
        let focus_handle = cx.focus_handle();
        // The connection roots at the lane cwd; without one there is no
        // working directory to attach the agent to. Park such a pane in
        // an error state rather than a perpetual "Connecting…". The cwd
        // case stays `Connecting` until the caller pushes the pane and
        // calls `connect_agent_chat` (which stores the live handle + the
        // event-pump task on the now-resolvable pane).
        let status = match &cwd {
            Some(_) => AgentSessionStatus::Connecting,
            None => AgentSessionStatus::Error(s::agent_chat_error_prefix()),
        };
        Pane {
            id: pane_id,
            content: PaneContent::AgentChat(AgentChatContent {
                focus_handle,
                cached_title: s::agent_chat_tab_title().into(),
                cwd,
                status,
                items: Vec::new(),
                handle: None,
                _event_pump: None,
                pending_permission: None,
                turn_in_flight: false,
                md_blocks: std::collections::HashMap::new(),
            }),
        }
    }

    /// Open a fresh Agent chat pane in a new tab, anchored at the
    /// active lane's working directory. Mirrors `open_task_edit_pane`'s
    /// tab-append + focus flow.
    pub(in crate::workspace) fn open_agent_chat_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // An inaccessible active lane renders the empty-state; opening a
        // pane there would escape that state (mirrors `add_tab`).
        if self.active_lane_is_inaccessible() {
            return;
        }
        let cwd = self.active_lane().map(|w| w.path.clone());
        let pane = self.create_agent_chat_pane(cwd.clone(), cx);
        let pane_id = pane.id;
        let tab_id = self.alloc_id();
        self.main_area.panes.push(pane);
        self.main_area.tabs.push(TabEntry {
            id: tab_id,
            layout: PaneLayout::Pane(pane_id),
            last_focused_pane: pane_id,
            user_label: None,
        });
        self.main_area
            .tab_history
            .push(self.main_area.active_tab_index);
        self.main_area.active_tab_index = self.main_area.tabs.len() - 1;
        // Pane is now in `self.panes` — start the live session so the
        // event pump can find it by id.
        if let Some(cwd) = cwd {
            self.connect_agent_chat(pane_id, cwd, cx);
        }
        // The prompt input lives in the bottom dock, not the pane. Open the
        // dock first so the input is visible before `focus_pane` (the shared
        // focus path) activates the input panel, syncs the placeholder, and
        // moves keyboard focus to it for AgentChat panes. The focused *pane*
        // stays this one, so `send_terminal_input` routes to its ACP session.
        if !self.bottom_dock.read(cx).is_open {
            self.bottom_dock.update(cx, |d, cx| {
                d.toggle();
                cx.notify();
            });
            self.main_area.pending_resize = true;
        }
        self.set_focused_pane(pane_id, cx);
        self.bump_activity(pane_id);
        self.focus_pane(pane_id, window, cx);
        self.resize_all_tabs(window, cx);
        cx.notify();
    }

    /// Open the live ACP session for an already-pushed Agent chat pane and
    /// store the event-pump task on it. Runs the (synchronous-to-parse,
    /// then async) connect on the background executor — `connect_session`
    /// spawns the protocol task on the smol executor gpui shares — then
    /// re-enters the workspace to store the handle and fold events.
    ///
    /// The spawned task is stored in the pane's `_event_pump`, so closing
    /// the pane drops it (ending the loop) in addition to dropping the
    /// session handle (which closes the connection).
    pub(in crate::workspace) fn connect_agent_chat(
        &mut self,
        pane_id: PaneId,
        cwd: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        let pump = cx.spawn(async move |this, cx| {
            // `connect_session` itself is synchronous (it parses the
            // command and spawns the connection task); run it on the
            // background executor so the smol `spawn` inside binds to a
            // worker thread rather than the main loop.
            let connected = cx
                .background_executor()
                .spawn(async move { connect_session(AdapterCommand::default(), cwd) })
                .await;

            match connected {
                Ok((handle, mut events)) => {
                    // Store the handle on the pane. If the pane/window is
                    // already gone, drop the handle (closing the session).
                    if this
                        .update(cx, |ws, _cx| {
                            if let Some(ac) = ws.agent_chat_content_mut_for_pane(pane_id) {
                                ac.handle = Some(handle);
                            }
                        })
                        .is_err()
                    {
                        return;
                    }

                    // Pump the event stream until end-of-stream (handle
                    // dropped on pane close, or terminal protocol error).
                    while let Some(event) = events.next().await {
                        if this
                            .update(cx, |ws, cx| ws.apply_agent_event(pane_id, event, cx))
                            .is_err()
                        {
                            // Workspace/window gone — stop pumping.
                            break;
                        }
                    }
                }
                Err(err) => {
                    let message = format!("{err}");
                    // workspace gone before the connect resolved — nothing left
                    // to surface the failure on.
                    // SILENT-OK: workspace/window dropped before connect resolved
                    let _ = this.update(cx, |ws, cx| {
                        if let Some(ac) = ws.agent_chat_content_mut_for_pane(pane_id) {
                            ac.status = AgentSessionStatus::Error(message.clone());
                        }
                        let report = ErrorReport::new("ACP session connect failed")
                            .severity(ErrorSeverity::Error)
                            .with_context("detail", message)
                            .at(file!(), line!())
                            .dedup("agent_chat.connect")
                            .build();
                        ws.report_error(report, cx);
                    });
                }
            }
        });
        // Store the pump on the pane so a pane close drops it (ending the
        // loop) on top of dropping the session handle.
        if let Some(ac) = self.agent_chat_content_mut_for_pane(pane_id) {
            ac._event_pump = Some(pump);
        }
    }

    /// Fold a single [`AcpEvent`] into the pane's chat model + status,
    /// then notify so the (cached) pane subtree repaints. The pump calls
    /// this on the foreground for every event.
    fn apply_agent_event(&mut self, pane_id: PaneId, event: AcpEvent, cx: &mut Context<Self>) {
        // Connection-fatal errors are reported through the pipeline in
        // addition to being shown inline, so split that out first.
        if let AcpEvent::Error(message) = &event {
            let report = ErrorReport::new("ACP session error")
                .severity(ErrorSeverity::Error)
                .with_context("detail", message.clone())
                .at(file!(), line!())
                .dedup("agent_chat.session_error")
                .build();
            self.report_error(report, cx);
        }

        // Parse params for the Markdown reconcile, read before the mutable
        // borrow of the pane content. Mirrors the file-viewer loader:
        // `is_light = !diagram_dark`, where `diagram_dark` comes from the
        // active theme (default dark when the global is absent).
        let syntax_theme = self.syntax_theme.clone();
        let is_light = cx
            .try_global::<crate::ui::theme::DarudaTheme>()
            .map(crate::ui::theme::DarudaTheme::is_dark)
            .map(|dark| !dark)
            .unwrap_or(false);

        let Some(ac) = self.agent_chat_content_mut_for_pane(pane_id) else {
            return;
        };
        match event {
            AcpEvent::Connected => ac.status = AgentSessionStatus::Connected,
            AcpEvent::Update(update) => apply_update(&mut ac.items, &update),
            AcpEvent::PermissionRequested { id, request } => {
                ac.items.push(permission_item(&request));
                ac.pending_permission = Some(id);
            }
            AcpEvent::TurnEnded { .. } => {
                finalize_streaming(&mut ac.items);
                ac.turn_in_flight = false;
            }
            AcpEvent::Error(message) => {
                ac.status = AgentSessionStatus::Error(message);
                ac.turn_in_flight = false;
            }
        }
        reconcile_markdown(ac, &syntax_theme, is_light);
        cx.notify();
    }

    /// Send `text` as a prompt to an Agent chat pane: echo it locally,
    /// forward it over the session, and mark a turn in flight. Shared by
    /// the pane's own input and the bottom-dock input (the caller owns
    /// clearing whichever input field the text came from).
    pub(in crate::workspace) fn send_agent_prompt_text(
        &mut self,
        pane_id: PaneId,
        text: String,
        cx: &mut Context<Self>,
    ) {
        let Some(ac) = self.agent_chat_content_mut_for_pane(pane_id) else {
            return;
        };
        // Echo locally so the prompt shows immediately even before the
        // agent streams it back as a user-message chunk.
        ac.items.push(daruda_acp::ChatItem::UserText(text.clone()));
        if let Some(handle) = &ac.handle {
            handle.send_prompt(text);
            ac.turn_in_flight = true;
        }
        let syntax_theme = self.syntax_theme.clone();
        let is_light = cx
            .try_global::<crate::ui::theme::DarudaTheme>()
            .map(crate::ui::theme::DarudaTheme::is_dark)
            .map(|dark| !dark)
            .unwrap_or(false);
        if let Some(ac) = self.agent_chat_content_mut_for_pane(pane_id) {
            reconcile_markdown(ac, &syntax_theme, is_light);
        }
        cx.notify();
    }

    /// True when `pane_id` is an Agent chat pane — lets the bottom-dock
    /// input route prompts to the session instead of a PTY.
    pub(in crate::workspace) fn is_agent_chat_pane(&self, pane_id: PaneId) -> bool {
        self.main_area
            .panes
            .iter()
            .any(|p| p.id == pane_id && p.agent_chat_content().is_some())
    }

    /// Request cancellation of the active turn. View dispatch only.
    pub(in crate::workspace) fn cancel_agent_turn(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        if let Some(ac) = self.agent_chat_content_mut_for_pane(pane_id)
            && let Some(handle) = &ac.handle
        {
            handle.cancel();
        }
        cx.notify();
    }

    /// Resolve the pane's pending permission request with the chosen
    /// option. Marks the matching card resolved, sends the decision over
    /// the session, and clears the pending id. View dispatch only —
    /// `kind` selects Allow vs. Reject semantics.
    pub(in crate::workspace) fn respond_agent_permission(
        &mut self,
        pane_id: PaneId,
        option_id: String,
        kind: PermissionKindView,
        cx: &mut Context<Self>,
    ) {
        let Some(ac) = self.agent_chat_content_mut_for_pane(pane_id) else {
            return;
        };
        let Some(id) = ac.pending_permission.take() else {
            return;
        };

        // Mark the trailing unresolved permission card resolved so the
        // buttons disable and the choice shows.
        for item in ac.items.iter_mut().rev() {
            if let daruda_acp::ChatItem::Permission(card) = item
                && card.resolved.is_none()
            {
                card.resolved = Some(option_id.clone());
                break;
            }
        }

        let decision = match kind {
            PermissionKindView::AllowOnce | PermissionKindView::AllowAlways => {
                PermissionDecision::Allow { option_id }
            }
            PermissionKindView::RejectOnce | PermissionKindView::RejectAlways => {
                PermissionDecision::Reject { option_id }
            }
        };
        if let Some(handle) = &ac.handle {
            handle.respond_permission(id, decision);
        }
        cx.notify();
    }

    /// Mutable lookup of an AgentChat pane's content by id. Returns `None`
    /// when the pane is gone or is not an AgentChat pane.
    fn agent_chat_content_mut_for_pane(
        &mut self,
        pane_id: PaneId,
    ) -> Option<&mut AgentChatContent> {
        self.main_area
            .panes
            .iter_mut()
            .find(|p| p.id == pane_id)?
            .agent_chat_content_mut()
    }
}

/// Markdown-parse text of `item` if it carries any; tool-call / permission /
/// error items have no Markdown body and return `None`.
fn item_text(item: &daruda_acp::ChatItem) -> Option<&str> {
    use daruda_acp::ChatItem;
    match item {
        ChatItem::UserText(text)
        | ChatItem::AssistantText { text, .. }
        | ChatItem::Thinking { text, .. } => Some(text.as_str()),
        ChatItem::ToolCall(_) | ChatItem::Permission(_) | ChatItem::Error(_) => None,
    }
}

/// Whether the text item at `idx` has settled — its text will no longer
/// change, so it is safe to parse once and cache.
///
/// Rule: every text item *except the last* is settled (the agent never
/// re-streams an earlier message). The last item is settled only when it is a
/// finished text item — `UserText` (always complete) or a non-streaming
/// `AssistantText` / `Thinking`. A streaming tail is left unsettled so it
/// renders as plain wrapped text until `TurnEnded` flips `streaming` off.
fn item_settled(items: &[daruda_acp::ChatItem], idx: usize) -> bool {
    use daruda_acp::ChatItem;
    if idx + 1 < items.len() {
        return true;
    }
    matches!(
        items.get(idx),
        Some(
            ChatItem::UserText(_)
                | ChatItem::AssistantText {
                    streaming: false,
                    ..
                }
                | ChatItem::Thinking {
                    streaming: false,
                    ..
                }
        )
    )
}

/// Parse-and-cache Markdown for every settled text item that is not already
/// cached. Called from the event pump (and the local prompt echo) after
/// `items` is mutated. `items` is append-only with only the tail mutating, so
/// index keys stay stable and no cache invalidation is needed.
fn reconcile_markdown(ac: &mut AgentChatContent, syntax_theme: &str, is_light: bool) {
    use crate::workspace::main_area::file_view_pane::markdown_viewer::parse_markdown;
    for idx in 0..ac.items.len() {
        if ac.md_blocks.contains_key(&idx) {
            continue;
        }
        if !item_settled(&ac.items, idx) {
            continue;
        }
        if let Some(text) = item_text(&ac.items[idx]) {
            ac.md_blocks
                .insert(idx, parse_markdown(text, syntax_theme, is_light));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_acp::ChatItem;

    /// `reconcile_markdown` parses settled text items and leaves the streaming
    /// tail untouched until it settles. Only the GPUI-bound `focus_handle`
    /// needs a live context; the rest is plain data.
    #[gpui::test]
    fn reconcile_caches_settled_text_only(cx: &mut gpui::TestAppContext) {
        let ac = cx.update(|cx| {
            let focus_handle = cx.focus_handle();
            AgentChatContent {
                focus_handle,
                cached_title: "t".into(),
                cwd: None,
                status: AgentSessionStatus::Connected,
                items: vec![
                    ChatItem::UserText("# hello".to_owned()),
                    ChatItem::AssistantText {
                        text: "world".to_owned(),
                        streaming: true,
                    },
                ],
                handle: None,
                _event_pump: None,
                pending_permission: None,
                turn_in_flight: false,
                md_blocks: std::collections::HashMap::new(),
            }
        });
        let mut ac = ac;

        reconcile_markdown(&mut ac, "base16-ocean.dark", false);
        // index 0 (UserText) is settled → cached; index 1 (streaming tail)
        // is not.
        assert!(ac.md_blocks.contains_key(&0));
        assert!(!ac.md_blocks.contains_key(&1));

        // Flip the tail to settled → it now parses on the next reconcile.
        if let ChatItem::AssistantText { streaming, .. } = &mut ac.items[1] {
            *streaming = false;
        }
        reconcile_markdown(&mut ac, "base16-ocean.dark", false);
        assert!(ac.md_blocks.contains_key(&1));
    }

    #[test]
    fn item_settled_rules() {
        // A non-tail item is always settled.
        let items = vec![
            ChatItem::AssistantText {
                text: "a".to_owned(),
                streaming: true,
            },
            ChatItem::UserText("b".to_owned()),
        ];
        assert!(item_settled(&items, 0)); // not the tail → settled even if streaming
        assert!(item_settled(&items, 1)); // UserText tail → settled

        // Streaming tail → unsettled.
        let items = vec![ChatItem::Thinking {
            text: "x".to_owned(),
            streaming: true,
        }];
        assert!(!item_settled(&items, 0));

        // Non-streaming tail → settled.
        let items = vec![ChatItem::AssistantText {
            text: "x".to_owned(),
            streaming: false,
        }];
        assert!(item_settled(&items, 0));
    }

    #[test]
    fn item_text_skips_non_text_items() {
        assert!(item_text(&ChatItem::Error("e".to_owned())).is_none());
        assert_eq!(item_text(&ChatItem::UserText("u".to_owned())), Some("u"));
    }
}
