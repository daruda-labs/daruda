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
    AcpEvent, AdapterCommand, DiffView, PermissionDecision, PermissionKindView, apply_update,
    connect_session, finalize_streaming, permission_item,
};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use futures::StreamExt as _;
use gpui::{AppContext as _, Context, Window};

use crate::path_ext::PathExt as _;
use crate::surface::strings as s;
use crate::workspace::Workspace;
use crate::workspace::main_area::file_view_pane::diff_editor::{
    DiffColors, DiffEditorModel, build_diff_editor_model,
};
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
                diff_editors: std::collections::HashMap::new(),
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
        self.reconcile_diff_editors(pane_id, &syntax_theme, is_light, cx);
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
        self.reconcile_diff_editors(pane_id, &syntax_theme, is_light, cx);
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

    /// Build the read-only diff editor entity for every tool-call file
    /// modification that does not yet have one. Mirrors `reconcile_markdown`:
    /// called from the event pump (and the local prompt echo) after `items`
    /// is mutated, so the (cached) pane subtree shows the diff through the
    /// same editor the File viewer uses rather than the inline fallback.
    ///
    /// Keyed by `"{tool_call_id}#{diff_index}"` — one editor per file. A diff
    /// is converted to a `DiffEditorModel` purely (no GPUI), then the editor
    /// entity is created + configured inside a single window re-entry
    /// (`InputState::new` / `set_value` need a live `&mut Window`). Entities
    /// are never created in `render`; the renderer only embeds them.
    ///
    /// Build-once: keys are only filled when absent. A `ToolCallUpdate` that
    /// replaces a diff's text keeps the original editor — re-streaming a diff
    /// in place is not observed today; if it becomes necessary, compare the
    /// stored model and rebuild on change.
    fn reconcile_diff_editors(
        &mut self,
        pane_id: PaneId,
        syntax_theme: &str,
        is_light: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(colors) = cx
            .try_global::<crate::ui::theme::DarudaTheme>()
            .map(DiffColors::from_theme)
        else {
            // Theme global not yet installed (transient cold-start) — skip
            // editor creation; every diff renders via the inline fallback.
            // Logged so the blanket fallback isn't a silent no-op.
            daruda_store::observability::log_writer::LogWriter::log(
                ErrorReport::new("Skipping agent-chat diff editors: theme global absent")
                    .severity(ErrorSeverity::Warning)
                    .at(file!(), line!())
                    .dedup("agent_chat.diff_editor.theme_missing")
                    .build(),
            );
            return;
        };

        // Collect the pure work first; entity creation re-enters the window,
        // which can't happen while the immutable `items` borrow is live.
        let Some(ac) = self.agent_chat_content_for_pane(pane_id) else {
            return;
        };
        let mut pending: Vec<(String, String, DiffEditorModel)> = Vec::new();
        for item in &ac.items {
            let daruda_acp::ChatItem::ToolCall(tc) = item else {
                continue;
            };
            for (di, diff) in tc.diffs.iter().enumerate() {
                let key = diff_editor_key(&tc.id, di);
                if ac.diff_editors.contains_key(&key) {
                    continue;
                }
                let Some(model) = build_diff_view_model(diff, syntax_theme, is_light, &colors)
                else {
                    continue;
                };
                let language = diff_editor_language(diff).to_owned();
                pending.push((key, language, model));
            }
        }
        if pending.is_empty() {
            return;
        }

        for (key, language, model) in pending {
            if let Some(editor) = create_diff_editor(cx, &language, model)
                && let Some(ac) = self.agent_chat_content_mut_for_pane(pane_id)
            {
                ac.diff_editors.insert(key, editor);
            }
        }
    }

    /// Immutable lookup of an AgentChat pane's content by id.
    fn agent_chat_content_for_pane(&self, pane_id: PaneId) -> Option<&AgentChatContent> {
        self.main_area
            .panes
            .iter()
            .find(|p| p.id == pane_id)?
            .agent_chat_content()
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

/// Cache key for a tool call's `di`-th diff editor: one editor per file.
/// Shared with the renderer so the embed lookup matches the insert key.
pub(in crate::workspace) fn diff_editor_key(tool_call_id: &str, di: usize) -> String {
    format!("{tool_call_id}#{di}")
}

/// Language id for an editor's syntax tree, from the diff's file extension.
/// Empty when unknown (the editor falls back to `"text"`).
fn diff_editor_language(diff: &DiffView) -> &'static str {
    match diff.path.extension_str() {
        "rs" => "rust",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" | "mts" | "cts" => "typescript",
        "jsx" => "jsx",
        "tsx" => "tsx",
        "py" => "python",
        "go" => "go",
        "toml" => "toml",
        "json" | "jsonc" => "json",
        "yaml" | "yml" => "yaml",
        "md" | "markdown" => "markdown",
        "html" | "htm" => "html",
        "css" => "css",
        "sh" | "bash" | "zsh" => "bash",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" => "cpp",
        _ => "",
    }
}

/// Convert a tool-call [`DiffView`] into the editor inputs the shared
/// diff-through-editor renderer consumes. Pure / GPUI-free: builds the unified
/// diff from `old_text`/`new_text` (a created file has no `old_text` → empty
/// old side), syntax-highlights and word-diffs the hunks exactly as the File
/// viewer's `load_diff` does, then folds them into a [`DiffEditorModel`].
///
/// Returns `None` when the two sides are identical (no hunks → nothing to
/// render), so the caller leaves the inline fallback in place.
fn build_diff_view_model(
    diff: &DiffView,
    syntax_theme: &str,
    is_light: bool,
    colors: &DiffColors,
) -> Option<DiffEditorModel> {
    use crate::workspace::main_area::file_view_pane::highlighter::highlight_hunks;
    use crate::workspace::main_area::file_view_pane::line_diff::unified_diff_text;
    use crate::workspace::main_area::file_view_pane::word_diff::apply_word_diff;
    use crate::workspace::main_area::file_view_pane::{build_diff_rows, parse_diff_hunks};

    let old = diff.old_text.as_deref().unwrap_or("");
    let text = unified_diff_text(old, &diff.new_text);
    let mut hunks = parse_diff_hunks(&text);
    if hunks.is_empty() {
        return None;
    }
    let ext = diff.path.extension_str();
    highlight_hunks(&mut hunks, ext, syntax_theme, is_light);
    apply_word_diff(&mut hunks);
    let rows = build_diff_rows(&hunks, false);
    Some(build_diff_editor_model(&rows, colors))
}

/// Create + configure a read-only diff editor entity inside a single window
/// re-entry. Mirrors the File viewer's editor construction
/// (`multi_line` + `soft_wrap(false)` + `code_editor`) and the diff-config
/// it applies (`set_disabled(true)` for read-only + decorations + injected
/// highlight spans). Returns `None` if the owning window is gone.
fn create_diff_editor(
    cx: &mut Context<Workspace>,
    language: &str,
    model: DiffEditorModel,
) -> Option<gpui::Entity<gpui_component::input::InputState>> {
    let entity_id = cx.entity_id();
    let wh = crate::window_registry::WindowRegistry::handle_for_workspace(entity_id, cx)?;
    let language = language.to_owned();
    match cx.update_window(wh, move |_, window, cx_w| {
        cx_w.new(|cx_state| {
            let mut state = gpui_component::input::InputState::new(window, cx_state)
                .multi_line(true)
                .soft_wrap(false);
            state = if language.is_empty() {
                state.code_editor("text")
            } else {
                state.code_editor(&language)
            };
            state.set_value(model.text, window, cx_state);
            state.set_disabled(true, cx_state);
            state.set_line_decorations(model.decorations, cx_state);
            state.set_highlight_override(Some(model.highlights), cx_state);
            state
        })
    }) {
        Ok(editor) => Some(editor),
        Err(e) => {
            // Window gone mid-stream — drop this editor; the inline
            // fallback renders. Logged so it isn't a silent no-op.
            daruda_store::observability::log_writer::LogWriter::log(
                ErrorReport::new("Failed to build agent-chat diff editor")
                    .severity(ErrorSeverity::Warning)
                    .at(file!(), line!())
                    .with_context("error", format!("{e}"))
                    .dedup("agent_chat.diff_editor.window_gone")
                    .build(),
            );
            None
        }
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
                diff_editors: std::collections::HashMap::new(),
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

    /// A flat `DiffColors` fixture so the pure model build is testable
    /// without a live theme.
    fn diff_colors() -> DiffColors {
        let c = |l: f32| gpui::Hsla {
            h: 0.,
            s: 0.,
            l,
            a: 1.,
        };
        DiffColors {
            add_bg: c(0.1),
            del_bg: c(0.11),
            hunk_bg: c(0.12),
            add_text: c(0.2),
            del_text: c(0.21),
            ctx_text: c(0.22),
            hunk_text: c(0.23),
            hunk_ctx_text: c(0.24),
            word_add_bg: c(0.3),
            word_del_bg: c(0.31),
        }
    }

    fn diff(old: Option<&str>, new: &str, path: &str) -> DiffView {
        DiffView {
            path: std::path::PathBuf::from(path),
            old_text: old.map(str::to_owned),
            new_text: new.to_owned(),
        }
    }

    /// `build_diff_view_model` turns a single-line modification into a
    /// `DiffEditorModel` whose synthetic buffer carries the hunk header plus
    /// both sides (no `+`/`-` markers — the kind is in the decorations) and
    /// whose per-row decorations include add/del backgrounds.
    #[test]
    fn diff_view_model_builds_rows_and_decorations() {
        let d = diff(
            Some("fn a() {}\nlet x = 1;\nfn b() {}\n"),
            "fn a() {}\nlet y = 2;\nfn b() {}\n",
            "src/lib.rs",
        );
        let m = build_diff_view_model(&d, "base16-ocean.dark", false, &diff_colors())
            .expect("a modified file produces hunks");
        // Hunk header row + content rows, no marker prefix on content.
        assert!(m.text.starts_with("@@"), "buffer leads with a hunk header");
        assert!(m.text.contains("let x = 1;"), "removed line present");
        assert!(m.text.contains("let y = 2;"), "added line present");
        // Some rows carry an add/del background (the changed pair).
        let with_bg = m
            .decorations
            .iter()
            .filter(|d| d.background.is_some())
            .count();
        assert!(with_bg >= 2, "at least the changed pair is tinted");
    }

    /// A newly created file (`old_text == None`) diffs against an empty old
    /// side — every line is an addition, so the model is built (non-empty).
    #[test]
    fn diff_view_model_handles_created_file() {
        let d = diff(None, "line one\nline two\n", "new.txt");
        let m = build_diff_view_model(&d, "base16-ocean.dark", false, &diff_colors())
            .expect("a created file produces an all-added hunk");
        assert!(m.text.contains("line one"));
        assert!(m.text.contains("line two"));
    }

    /// Identical sides yield no hunks, so the adapter returns `None` and the
    /// caller keeps the inline fallback.
    #[test]
    fn diff_view_model_none_when_unchanged() {
        let d = diff(Some("same\n"), "same\n", "same.txt");
        assert!(build_diff_view_model(&d, "base16-ocean.dark", false, &diff_colors()).is_none());
    }

    /// The cache key is per-(tool-call, diff index) so two files in one tool
    /// call get distinct editors.
    #[test]
    fn diff_editor_keys_are_per_file() {
        assert_eq!(diff_editor_key("call-1", 0), "call-1#0");
        assert_ne!(diff_editor_key("call-1", 0), diff_editor_key("call-1", 1));
        assert_ne!(diff_editor_key("call-1", 0), diff_editor_key("call-2", 0));
    }
}
