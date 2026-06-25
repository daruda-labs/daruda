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

use super::fold::{FoldKey, FoldState};
use crate::path_ext::PathExt as _;
use crate::surface::strings as s;
use crate::workspace::Workspace;
use crate::workspace::main_area::file_view_pane::diff_editor::{
    DiffColors, DiffEditorModel, build_diff_editor_model,
};
use crate::workspace::main_area::file_view_pane::markdown_viewer::mermaid_with_theme;
use crate::workspace::main_area::file_view_pane::visual;
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
            // The status banner re-adds the error prefix, so carry the bare
            // reason here — not the prefix (which would render doubled).
            None => AgentSessionStatus::Error(s::agent_chat_no_lane_cwd()),
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
                diff_editors: std::collections::HashMap::new(),
                diff_stats: std::collections::HashMap::new(),
                mermaid_rasters: std::collections::HashMap::new(),
                mermaid_inflight: std::collections::HashSet::new(),
                fold: FoldState::default(),
                scroll_handle: gpui::ScrollHandle::new(),
                stick_to_bottom: true,
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

        // Theme params for the diff reconcile, read before the mutable
        // borrow of the pane content.
        let (syntax_theme, is_light) = self.agent_chat_theme_params(cx);

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
                // A turn only ends with a permission still pending when it was
                // cancelled / refused mid-request — drain it so no card is
                // left with live buttons (no-op on a normal turn).
                cancel_pending_permission(ac);
            }
            AcpEvent::Error(message) => {
                ac.status = AgentSessionStatus::Error(message);
                ac.turn_in_flight = false;
                cancel_pending_permission(ac);
            }
        }
        self.reconcile_diff_editors(pane_id, &syntax_theme, is_light, cx);
        self.reconcile_mermaid(pane_id, !is_light, cx);
        // Auto-follow: while pinned to the bottom, keep the view there as new
        // content folds in. `scroll_to_bottom` sets a flag resolved at the next
        // prepaint, so it accounts for the content appended above.
        if let Some(ac) = self.agent_chat_content_mut_for_pane(pane_id)
            && ac.stick_to_bottom
        {
            ac.scroll_handle.scroll_to_bottom();
        }
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
        // There is no `ToolCall` at a prompt-echo, so `reconcile_diff_editors`
        // would always be a no-op here; diff editors are reconciled solely on
        // the event-pump path. The echoed `UserText` renders its markdown
        // directly in the view via `crate::ui::markdown`; a prompt may carry a
        // ` ```mermaid ` fence, so rasterize those (no-op when there are none).
        let (_, is_light) = self.agent_chat_theme_params(cx);
        self.reconcile_mermaid(pane_id, !is_light, cx);
        if let Some(ac) = self.agent_chat_content_mut_for_pane(pane_id) {
            // Submitting a prompt jumps the view to the bottom so the user sees
            // their message and the streaming response.
            ac.stick_to_bottom = true;
            ac.scroll_handle.scroll_to_bottom();
        }
        cx.notify();
    }

    /// The (syntax theme, is-light) pair the Markdown / diff reconcilers read
    /// from the active theme. `is_light = !is_dark`, mirroring the file-viewer
    /// loader; defaults to dark (`is_light = false`) when the theme global is
    /// not yet installed.
    fn agent_chat_theme_params(&self, cx: &Context<Self>) -> (String, bool) {
        let is_light = cx
            .try_global::<crate::ui::theme::DarudaTheme>()
            .map(crate::ui::theme::DarudaTheme::is_dark)
            .map(|dark| !dark)
            .unwrap_or(false);
        (self.syntax_theme.clone(), is_light)
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
    ///
    /// Sends `session/cancel` and, per ACP, resolves any permission request
    /// still awaiting the user with a cancelled outcome (so the agent's
    /// pending request is answered and the inline card stops showing buttons).
    pub(in crate::workspace) fn cancel_agent_turn(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        if let Some(ac) = self.agent_chat_content_mut_for_pane(pane_id) {
            if let Some(handle) = &ac.handle {
                handle.cancel();
            }
            cancel_pending_permission(ac);
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
        if let Some(card) = trailing_unresolved_permission(ac) {
            card.resolved = Some(daruda_acp::PermissionResolution::Chosen(option_id.clone()));
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

    /// Toggle the fold state of one block in an Agent chat pane. View
    /// dispatch only: the disclosure chevron / header click routes here.
    ///
    /// Resolves the `active` flag the same way `render` derives it, so the
    /// first click flips the *visible* state rather than re-deriving from a
    /// stale default. `Tool` is matched by id (active = `InProgress`); `Diff`
    /// is always `DefaultCollapsed`, so its derivation ignores `active` and we
    /// pass `false`.
    pub(in crate::workspace) fn toggle_agent_fold(
        &mut self,
        pane_id: PaneId,
        key: FoldKey,
        cx: &mut Context<Self>,
    ) {
        let Some(ac) = self.agent_chat_content_mut_for_pane(pane_id) else {
            return;
        };
        let active = match &key {
            FoldKey::Assistant(ix) | FoldKey::Thinking(ix) => {
                ac.items.get(*ix).map(is_active).unwrap_or(false)
            }
            FoldKey::Tool(id) => ac
                .items
                .iter()
                .find_map(|item| match item {
                    daruda_acp::ChatItem::ToolCall(tc) if tc.id == *id => Some(is_active(item)),
                    _ => None,
                })
                .unwrap_or(false),
            // Diff policy is DefaultCollapsed → derivation ignores `active`.
            FoldKey::Diff(_) => false,
        };
        ac.fold.toggle(key, active);
        cx.notify();
    }

    /// Expand or collapse every currently-visible foldable block in an Agent
    /// chat pane at once (the pane header's expand-all / collapse-all). View
    /// dispatch only.
    ///
    /// Builds the visible key set from `items`: each assistant / thinking item
    /// by index, each tool call by id plus one `Diff` key per diff it carries
    /// (the same `diff_editor_key` the renderer embeds with). User / permission
    /// / error items are not foldable and are skipped.
    pub(in crate::workspace) fn set_all_agent_folds(
        &mut self,
        pane_id: PaneId,
        expanded: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(ac) = self.agent_chat_content_mut_for_pane(pane_id) else {
            return;
        };
        let keys = collect_foldable_keys(&ac.items);
        ac.fold.set_all(keys, expanded);
        cx.notify();
    }

    /// Jump the conversation list to the bottom and re-engage follow mode.
    /// Backs the floating scroll-to-bottom button. View dispatch only.
    pub(in crate::workspace) fn agent_chat_scroll_to_bottom(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        let Some(ac) = self.agent_chat_content_mut_for_pane(pane_id) else {
            return;
        };
        ac.scroll_handle.scroll_to_bottom();
        ac.stick_to_bottom = true;
        cx.notify();
    }

    /// Recompute follow mode after the user scrolls the conversation list
    /// (wired to the list's `on_scroll_wheel`). Pins to the bottom when the user
    /// is at/near the live edge, releases otherwise — so streaming output
    /// auto-follows only while the user is already at the bottom, and the
    /// scroll-to-bottom button appears once they scroll up. Notifies only on a
    /// change so a scroll that stays in the same zone is cheap.
    pub(in crate::workspace) fn agent_chat_on_scroll(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        let Some(ac) = self.agent_chat_content_mut_for_pane(pane_id) else {
            return;
        };
        let now_at_bottom = at_bottom(&ac.scroll_handle);
        if ac.stick_to_bottom != now_at_bottom {
            ac.stick_to_bottom = now_at_bottom;
            cx.notify();
        }
    }

    /// Build the read-only diff editor entity for every tool-call file
    /// modification that does not yet have one. Called from the event pump
    /// after `items` is mutated, so the (cached) pane subtree shows the diff
    /// through the same editor the File viewer uses rather than the inline
    /// fallback.
    ///
    /// Keyed by `"{tool_call_id}#{diff_index}"` — one editor per file. A diff
    /// is converted to a `DiffEditorModel` purely (no GPUI), then the editor
    /// entity is created + configured inside a single window re-entry
    /// (`InputState::new` / `set_value` need a live `&mut Window`). Entities
    /// are never created in `render`; the renderer only embeds them.
    ///
    /// Build-once: keys are only filled when absent. The first `ToolCall` with
    /// empty `diffs` inserts no key, so a later `ToolCallUpdate` that fills the
    /// diffs still builds normally. Only an update that *changes the content* of
    /// an already-built diff would be stale — and the adapter does not emit such
    /// updates today (if it ever does, add a change check before the
    /// `contains_key` guard and rebuild on change).
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
                    .dedup(format!("agent_chat.diff_editor.theme_missing.{pane_id}"))
                    .build(),
            );
            return;
        };

        // Collect the pure work first; entity creation re-enters the window,
        // which can't happen while the immutable `items` borrow is live.
        let Some(ac) = self.agent_chat_content_for_pane(pane_id) else {
            return;
        };
        let mut pending: Vec<(String, String, DiffEditorModel, DiffStat)> = Vec::new();
        for item in &ac.items {
            let daruda_acp::ChatItem::ToolCall(tc) = item else {
                continue;
            };
            for (di, diff) in tc.diffs.iter().enumerate() {
                let key = diff_editor_key(&tc.id, di);
                if ac.diff_editors.contains_key(&key) {
                    continue;
                }
                let Some((model, stat)) =
                    build_diff_view_model(diff, syntax_theme, is_light, &colors)
                else {
                    continue;
                };
                let language = diff_editor_language(diff).to_owned();
                pending.push((key, language, model, stat));
            }
        }
        if pending.is_empty() {
            return;
        }

        for (key, language, model, stat) in pending {
            if let Some(editor) = create_diff_editor(cx, pane_id, &language, model)
                && let Some(ac) = self.agent_chat_content_mut_for_pane(pane_id)
            {
                // Cache the stat under the same key as the editor so the fold
                // summary (`+N −M`, Task 5) reads it back via `diff_editor_key`.
                // Stored only when the editor builds — a no-change diff yields
                // no editor and no stat (absent ≡ `0/0`).
                ac.diff_stats.insert(key.clone(), stat);
                ac.diff_editors.insert(key, editor);
            }
        }
    }

    /// Rasterize every ` ```mermaid ` fence in the conversation that does not
    /// yet have a cached bitmap (and isn't already being rendered). Mirrors
    /// [`Self::reconcile_diff_editors`]: collect the pure work first, then spawn
    /// each rasterization on the background executor (selkie is CPU-heavy and can
    /// panic), and re-enter the workspace to fill the cache + `cx.notify()` when
    /// it lands. Until then the fence renders via the default code block (the
    /// `code_block_render` hook returns `None` for an absent key).
    ///
    /// `dark` matches the diagram theme to the host appearance (dark UI → dark
    /// diagram) so edges stay visible; the caller derives it from the active
    /// theme (`dark = !is_light`). Theme-switch staleness — a cached raster keeps
    /// its original colour after a light/dark toggle — is out of scope here (no
    /// re-raster on theme change); the cache is only ever added to.
    fn reconcile_mermaid(&mut self, pane_id: PaneId, dark: bool, cx: &mut Context<Self>) {
        // Collect the not-yet-cached, not-in-flight sources first; the spawn
        // re-enters the workspace, which can't happen while the `items` borrow
        // is live.
        let Some(ac) = self.agent_chat_content_for_pane(pane_id) else {
            return;
        };
        let mut pending: Vec<(u64, String)> = Vec::new();
        for item in &ac.items {
            let Some(text) = chat_item_markdown(item) else {
                continue;
            };
            for source in mermaid_sources(text) {
                let key = mermaid_key(&source);
                if ac.mermaid_rasters.contains_key(&key)
                    || ac.mermaid_inflight.contains(&key)
                    || pending.iter().any(|(k, _)| *k == key)
                {
                    continue;
                }
                pending.push((key, source));
            }
        }
        if pending.is_empty() {
            return;
        }

        // Mark all pending keys in-flight before spawning so a second event
        // arriving before any task resolves doesn't re-spawn the same source.
        if let Some(ac) = self.agent_chat_content_mut_for_pane(pane_id) {
            for (key, _) in &pending {
                ac.mermaid_inflight.insert(*key);
            }
        }

        for (key, source) in pending {
            cx.spawn(async move |this, cx| {
                let raster = cx
                    .background_executor()
                    .spawn(async move {
                        let themed = mermaid_with_theme(&source, dark);
                        // selkie is a young reimplementation; guard against a
                        // panic on malformed input so one bad diagram can't take
                        // the executor down — on panic / error we drop it and the
                        // fence keeps the default code rendering.
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            selkie::render::render_text(&themed)
                                .ok()
                                .and_then(|svg| visual::rasterize_svg(&svg).ok())
                        }))
                        .ok()
                        .flatten()
                    })
                    .await;
                // SILENT-OK: workspace/window dropped before the raster resolved — nothing left to cache it on.
                let _ = this.update(cx, |ws, cx| {
                    let Some(ac) = ws.agent_chat_content_mut_for_pane(pane_id) else {
                        return;
                    };
                    ac.mermaid_inflight.remove(&key);
                    if let Some(raster) = raster {
                        ac.mermaid_rasters.insert(key, std::sync::Arc::new(raster));
                        cx.notify();
                    }
                });
            })
            .detach();
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

/// The visible foldable-key set for a conversation: each assistant / thinking
/// item by index, each tool call by id plus one `Diff` key per diff it carries
/// (the same `diff_editor_key` the renderer embeds with). User / permission /
/// error items are not foldable and contribute none. Single source of truth for
/// expand-all / collapse-all (`set_all_agent_folds`) and the coverage test.
fn collect_foldable_keys(items: &[daruda_acp::ChatItem]) -> Vec<FoldKey> {
    let mut keys: Vec<FoldKey> = Vec::new();
    for (ix, item) in items.iter().enumerate() {
        match item {
            daruda_acp::ChatItem::AssistantText { .. } => keys.push(FoldKey::Assistant(ix)),
            daruda_acp::ChatItem::Thinking { .. } => keys.push(FoldKey::Thinking(ix)),
            daruda_acp::ChatItem::ToolCall(tc) => {
                keys.push(FoldKey::Tool(tc.id.clone()));
                for di in 0..tc.diffs.len() {
                    keys.push(FoldKey::Diff(diff_editor_key(&tc.id, di)));
                }
            }
            daruda_acp::ChatItem::UserText(_)
            | daruda_acp::ChatItem::Permission(_)
            | daruda_acp::ChatItem::Error(_) => {}
        }
    }
    keys
}

/// Cache key for a tool call's `di`-th diff editor: one editor per file.
/// Shared with the renderer so the embed lookup matches the insert key.
pub(in crate::workspace) fn diff_editor_key(tool_call_id: &str, di: usize) -> String {
    format!("{tool_call_id}#{di}")
}

/// The markdown body of a chat item that can carry a ` ```mermaid ` fence —
/// assistant / thinking / user text. Tool / permission / error items carry no
/// markdown body and contribute none. Drives the mermaid scan.
fn chat_item_markdown(item: &daruda_acp::ChatItem) -> Option<&str> {
    match item {
        daruda_acp::ChatItem::AssistantText { text, .. }
        | daruda_acp::ChatItem::Thinking { text, .. } => Some(text),
        daruda_acp::ChatItem::UserText(text) => Some(text),
        daruda_acp::ChatItem::ToolCall(_)
        | daruda_acp::ChatItem::Permission(_)
        | daruda_acp::ChatItem::Error(_) => None,
    }
}

/// Stable cache key for a mermaid fence's source, shared between the rasterizer
/// (insert) and the renderer (lookup) so the embed matches what was cached.
/// `DefaultHasher` is process-stable, which is all the in-memory cache needs.
pub(in crate::workspace) fn mermaid_key(source: &str) -> u64 {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    hasher.finish()
}

/// Extract the source of every **closed** ` ```mermaid ` fence in `text`, in
/// document order. Only closed fences are returned: a still-streaming (never
/// terminated) trailing `mermaid` fence is skipped so a half-arrived diagram
/// isn't rasterized until it completes. Non-mermaid fences are ignored.
///
/// A mermaid fence opens on a line whose trimmed content is exactly ```` ```mermaid ````
/// (optionally with trailing spaces) and closes on the next line whose trimmed
/// content is ```` ``` ````. Leading indentation on the fence lines is tolerated;
/// the captured source keeps the lines between the fences verbatim.
fn mermaid_sources(text: &str) -> Vec<String> {
    let mut sources = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        if line.trim() != "```mermaid" {
            continue;
        }
        // Inside a mermaid fence — collect until the closing ``` line. If the
        // text ends first the fence is unterminated (still streaming): drop it.
        let mut body: Vec<&str> = Vec::new();
        let mut closed = false;
        for inner in lines.by_ref() {
            if inner.trim() == "```" {
                closed = true;
                break;
            }
            body.push(inner);
        }
        if closed {
            sources.push(body.join("\n"));
        }
    }
    sources
}

/// Added / removed line counts for one tool-call diff, used by the fold
/// summary (`+N −M`) shown when the diff editor is collapsed. Counted from
/// the *same* hunks that build the diff editor (see [`build_diff_view_model`]),
/// so the numbers match what the editor renders exactly — an edit reports the
/// changed lines, not a full delete-then-re-add. Cached alongside the editor in
/// `AgentChatContent.diff_stats`, keyed by [`diff_editor_key`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::workspace) struct DiffStat {
    pub(in crate::workspace) added: usize,
    pub(in crate::workspace) removed: usize,
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
/// diff-through-editor renderer consumes, plus the [`DiffStat`] for the same
/// diff. Pure / GPUI-free: builds the unified diff from `old_text`/`new_text`
/// (a created file has no `old_text` → empty old side), syntax-highlights and
/// word-diffs the hunks exactly as the File viewer's `load_diff` does, then
/// folds them into a [`DiffEditorModel`].
///
/// The stat is counted from those *same* hunks (via [`diff_stat_from_hunks`]),
/// so it matches the rendered editor line-for-line — a one-line edit reports
/// `added = 1, removed = 1`, never the full old/new line totals.
///
/// Returns `None` when the two sides are identical (no hunks → nothing to
/// render), so the caller leaves the inline fallback in place and records no
/// stat entry (absent ≡ `0/0`).
fn build_diff_view_model(
    diff: &DiffView,
    syntax_theme: &str,
    is_light: bool,
    colors: &DiffColors,
) -> Option<(DiffEditorModel, DiffStat)> {
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
    // Count add/remove from the parsed hunks before they are highlighted /
    // word-diffed (those passes only annotate, never reclassify lines), so the
    // stat is from the exact same diff that builds the editor below.
    let stat = diff_stat_from_hunks(&hunks);
    let ext = diff.path.extension_str();
    highlight_hunks(&mut hunks, ext, syntax_theme, is_light);
    apply_word_diff(&mut hunks);
    let rows = build_diff_rows(&hunks, false);
    Some((build_diff_editor_model(&rows, colors), stat))
}

/// Tally a [`DiffStat`] from parsed diff hunks. Pure / GPUI-free wrapper over
/// the File viewer's [`count_diff_stats`], which counts `DiffLine::Added` vs
/// `DiffLine::Removed` across the hunks — the same line classification the
/// editor rows are built from. A created file's hunks are all-added, so this
/// naturally yields `removed = 0`; a no-change diff produces no hunks and never
/// reaches here.
fn diff_stat_from_hunks(
    hunks: &[crate::workspace::main_area::file_view_pane::DiffHunk],
) -> DiffStat {
    let (added, removed) = crate::workspace::main_area::file_view_pane::count_diff_stats(hunks);
    DiffStat { added, removed }
}

/// Create + configure a read-only diff editor entity inside a single window
/// re-entry. Mirrors the File viewer's editor construction
/// (`multi_line` + `soft_wrap(false)` + `code_editor`) and the diff-config
/// it applies (`set_disabled(true)` for read-only + decorations + injected
/// highlight spans). Returns `None` if the owning window is gone.
fn create_diff_editor(
    cx: &mut Context<Workspace>,
    pane_id: PaneId,
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
                    .dedup(format!("agent_chat.diff_editor.window_gone.{pane_id}"))
                    .build(),
            );
            None
        }
    }
}

/// Whether a chat block is currently streaming / in progress — the `active`
/// input the fold derivation reads. A streaming text or thinking block, or a
/// tool call still `InProgress`, is active; everything else (settled text,
/// finished/failed tool calls, user / permission / error items) is not. Shared
/// by [`Workspace::toggle_agent_fold`] and the renderer so both derive the same
/// effective fold state.
pub(in crate::workspace) fn is_active(item: &daruda_acp::ChatItem) -> bool {
    use daruda_acp::{ChatItem, ToolStatusView};
    match item {
        ChatItem::AssistantText { streaming, .. } | ChatItem::Thinking { streaming, .. } => {
            *streaming
        }
        ChatItem::ToolCall(tc) => tc.status == ToolStatusView::InProgress,
        ChatItem::UserText(_) | ChatItem::Permission(_) | ChatItem::Error(_) => false,
    }
}

/// Whether a scroll view is at (or within `slack` of) the bottom. `offset_y`
/// is the scroll handle's current y offset (`<= 0`; more negative = scrolled
/// further down); `max_y` is the maximum scroll distance (`>= 0`). Content that
/// fits without scrolling (`max_y <= 0`) is trivially at the bottom. Pure so it
/// is unit-testable without a laid-out handle.
fn scroll_at_bottom(offset_y: f32, max_y: f32, slack: f32) -> bool {
    max_y <= 0.0 || (max_y + offset_y) <= slack
}

/// [`scroll_at_bottom`] applied to a live [`gpui::ScrollHandle`] with the
/// agent-chat slack. `offset().y` is negative when scrolled down and
/// `max_offset().y` is the bottom extent, so at the bottom `max + offset ≈ 0`.
fn at_bottom(handle: &gpui::ScrollHandle) -> bool {
    scroll_at_bottom(
        f32::from(handle.offset().y),
        f32::from(handle.max_offset().y),
        crate::ui::theme::AGENT_CHAT_SCROLL_BOTTOM_SLACK,
    )
}

/// The trailing not-yet-resolved permission card in `items`, if any. The
/// agent keeps a single permission request outstanding at a time and it is
/// always the most recent, so reverse-scan for the first unresolved card.
fn trailing_unresolved_permission(
    ac: &mut AgentChatContent,
) -> Option<&mut daruda_acp::PermissionItem> {
    ac.items.iter_mut().rev().find_map(|item| match item {
        daruda_acp::ChatItem::Permission(card) if card.resolved.is_none() => Some(card),
        _ => None,
    })
}

/// Cancel-drain the pane's pending permission request, if any: respond to the
/// agent with a `Cancelled` outcome and mark the trailing unresolved card
/// cancelled so its buttons disable. No-op when nothing is pending;
/// idempotent. ACP requires the client to resolve a pending permission with a
/// cancelled outcome on `session/cancel`; this also runs when a turn ends or
/// errors before the user decided, so no card is left stuck with live buttons.
fn cancel_pending_permission(ac: &mut AgentChatContent) {
    let Some(id) = ac.pending_permission.take() else {
        return;
    };
    if let Some(handle) = &ac.handle {
        handle.respond_permission(id, daruda_acp::PermissionDecision::Cancelled);
    }
    if let Some(card) = trailing_unresolved_permission(ac) {
        card.resolved = Some(daruda_acp::PermissionResolution::Cancelled);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_acp::{ChatItem, ToolCallItem};

    /// A syntax theme id every test reuses for the highlight passes.
    const TEST_SYNTAX_THEME: &str = "base16-ocean.dark";

    #[test]
    fn scroll_at_bottom_detects_bottom_top_and_slack() {
        // Content fits (no scroll) → trivially at bottom.
        assert!(scroll_at_bottom(0.0, 0.0, 24.0));
        // At the very bottom: max + offset == 0.
        assert!(scroll_at_bottom(-100.0, 100.0, 24.0));
        // Within slack of the bottom (10px from the edge, slack 24).
        assert!(scroll_at_bottom(-90.0, 100.0, 24.0));
        // At the top of a scrollable view → not at bottom.
        assert!(!scroll_at_bottom(0.0, 100.0, 24.0));
        // Scrolled up beyond slack (90px from the edge) → not at bottom.
        assert!(!scroll_at_bottom(-10.0, 100.0, 24.0));
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
        let (m, _) = build_diff_view_model(&d, TEST_SYNTAX_THEME, false, &diff_colors())
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
        let (m, _) = build_diff_view_model(&d, TEST_SYNTAX_THEME, false, &diff_colors())
            .expect("a created file produces an all-added hunk");
        assert!(m.text.contains("line one"));
        assert!(m.text.contains("line two"));
    }

    /// Identical sides yield no hunks, so the adapter returns `None` and the
    /// caller keeps the inline fallback.
    #[test]
    fn diff_view_model_none_when_unchanged() {
        let d = diff(Some("same\n"), "same\n", "same.txt");
        assert!(build_diff_view_model(&d, TEST_SYNTAX_THEME, false, &diff_colors()).is_none());
    }

    /// A simple one-line modification must report the *changed* line on each
    /// side — `added = 1, removed = 1` — not the file's total line counts
    /// (which would be 3/3 here). This is the whole point of counting from the
    /// diff hunks rather than `new.lines() vs old.lines()`.
    #[test]
    fn diff_stat_counts_changed_lines_not_totals() {
        let d = diff(
            Some("fn a() {}\nlet x = 1;\nfn b() {}\n"),
            "fn a() {}\nlet y = 2;\nfn b() {}\n",
            "src/lib.rs",
        );
        let (_, stat) = build_diff_view_model(&d, TEST_SYNTAX_THEME, false, &diff_colors())
            .expect("a modified file produces hunks");
        assert_eq!(
            stat,
            DiffStat {
                added: 1,
                removed: 1
            }
        );
    }

    /// A newly created file (`old_text == None`) diffs against an empty old
    /// side, so every line is an addition: `added = N, removed = 0`.
    #[test]
    fn diff_stat_new_file_is_all_added() {
        let d = diff(None, "line one\nline two\nline three\n", "new.txt");
        let (_, stat) = build_diff_view_model(&d, TEST_SYNTAX_THEME, false, &diff_colors())
            .expect("a created file produces an all-added hunk");
        assert_eq!(
            stat,
            DiffStat {
                added: 3,
                removed: 0
            }
        );
    }

    /// A pure deletion — the new side drops every line of the old — reports
    /// `added = 0, removed = N`, the mirror of the all-added created-file case.
    #[test]
    fn diff_stat_deleted_lines_are_all_removed() {
        let d = diff(Some("first\nsecond\n"), "", "old.rs");
        let (_, stat) = build_diff_view_model(&d, TEST_SYNTAX_THEME, false, &diff_colors())
            .expect("a fully-deleted file produces an all-removed hunk");
        assert_eq!(
            stat,
            DiffStat {
                added: 0,
                removed: 2
            }
        );
    }

    /// Identical sides produce no hunks → no editor and no stat. The cache
    /// simply has no entry (absent ≡ `0/0` for the fold summary), so there is
    /// nothing to assert beyond the `None` already covered above; this pins the
    /// pure tally directly on empty hunks for clarity.
    #[test]
    fn diff_stat_unchanged_is_zero() {
        assert_eq!(diff_stat_from_hunks(&[]), DiffStat::default());
    }

    /// The cache key is per-(tool-call, diff index) so two files in one tool
    /// call get distinct editors.
    #[test]
    fn diff_editor_keys_are_per_file() {
        assert_eq!(diff_editor_key("call-1", 0), "call-1#0");
        assert_ne!(diff_editor_key("call-1", 0), diff_editor_key("call-1", 1));
        assert_ne!(diff_editor_key("call-1", 0), diff_editor_key("call-2", 0));
    }

    /// A tool-call item with a given status and diff list, for `is_active`
    /// and key-collection coverage.
    fn tool_call(id: &str, status: daruda_acp::ToolStatusView, diffs: usize) -> ToolCallItem {
        ToolCallItem {
            id: id.to_owned(),
            title: "t".to_owned(),
            kind: daruda_acp::ToolKindView::Edit,
            status,
            diffs: (0..diffs)
                .map(|i| DiffView {
                    path: std::path::PathBuf::from(format!("f{i}.rs")),
                    old_text: None,
                    new_text: "x".to_owned(),
                })
                .collect(),
            output: Vec::new(),
            raw_input: None,
        }
    }

    /// `is_active` is true exactly while a block is streaming / in progress:
    /// streaming text & thinking, and an `InProgress` tool call. Everything
    /// else (settled text, finished/failed tool calls, user / permission /
    /// error items) is inactive — this drives the auto-collapse derivation.
    #[test]
    fn is_active_matches_streaming_and_in_progress() {
        use daruda_acp::ToolStatusView::*;
        assert!(is_active(&ChatItem::AssistantText {
            text: "a".to_owned(),
            streaming: true,
        }));
        assert!(!is_active(&ChatItem::AssistantText {
            text: "a".to_owned(),
            streaming: false,
        }));
        assert!(is_active(&ChatItem::Thinking {
            text: "t".to_owned(),
            streaming: true,
        }));
        assert!(!is_active(&ChatItem::Thinking {
            text: "t".to_owned(),
            streaming: false,
        }));
        assert!(is_active(&ChatItem::ToolCall(tool_call(
            "c1", InProgress, 0
        ))));
        assert!(!is_active(&ChatItem::ToolCall(tool_call("c1", Pending, 0))));
        assert!(!is_active(&ChatItem::ToolCall(tool_call(
            "c1", Completed, 0
        ))));
        assert!(!is_active(&ChatItem::ToolCall(tool_call("c1", Failed, 0))));
        // Non-foldable / inactive items.
        assert!(!is_active(&ChatItem::UserText("u".to_owned())));
        assert!(!is_active(&ChatItem::Error("e".to_owned())));
    }

    /// A single closed mermaid fence yields its verbatim body.
    #[test]
    fn mermaid_sources_extracts_a_closed_fence() {
        let text = "intro\n```mermaid\ngraph TD\nA-->B\n```\noutro";
        assert_eq!(mermaid_sources(text), vec!["graph TD\nA-->B".to_string()]);
    }

    /// Multiple closed fences are returned in document order.
    #[test]
    fn mermaid_sources_extracts_multiple_fences() {
        let text = "```mermaid\nA\n```\nmid\n```mermaid\nB\n```";
        assert_eq!(
            mermaid_sources(text),
            vec!["A".to_string(), "B".to_string()]
        );
    }

    /// An unterminated trailing fence (still streaming) is skipped — only the
    /// already-closed fence before it is returned.
    #[test]
    fn mermaid_sources_skips_unterminated_trailing_fence() {
        let text = "```mermaid\nA\n```\n```mermaid\nstill streaming";
        assert_eq!(mermaid_sources(text), vec!["A".to_string()]);
        // A lone unterminated fence yields nothing.
        assert!(mermaid_sources("```mermaid\ngraph TD").is_empty());
    }

    /// Non-mermaid fences (other languages, or none) are ignored.
    #[test]
    fn mermaid_sources_ignores_non_mermaid_fences() {
        let text = "```rust\nfn main() {}\n```\n```\nplain\n```";
        assert!(mermaid_sources(text).is_empty());
    }

    /// The cache key is stable per source and distinct across sources.
    #[test]
    fn mermaid_key_is_stable_and_distinct() {
        assert_eq!(
            mermaid_key("graph TD\nA-->B"),
            mermaid_key("graph TD\nA-->B")
        );
        assert_ne!(
            mermaid_key("graph TD\nA-->B"),
            mermaid_key("graph LR\nA-->B")
        );
    }

    /// The visible foldable-key set the expand-all / collapse-all op builds:
    /// assistant & thinking by index, each tool call by id plus one diff key
    /// per diff it carries; user / permission / error items contribute none.
    /// (Mirrors `set_all_agent_folds`'s key collection.)
    #[test]
    fn visible_fold_keys_cover_text_tools_and_diffs() {
        use daruda_acp::ToolStatusView::Completed;
        let items = [
            ChatItem::UserText("u".to_owned()),
            ChatItem::AssistantText {
                text: "a".to_owned(),
                streaming: false,
            },
            ChatItem::Thinking {
                text: "t".to_owned(),
                streaming: false,
            },
            ChatItem::ToolCall(tool_call("c1", Completed, 2)),
            ChatItem::Error("e".to_owned()),
        ];
        // Exercise the same collection the expand-all / collapse-all op uses, so
        // a new foldable kind is covered here automatically once the helper
        // handles it.
        let keys = collect_foldable_keys(&items);
        assert_eq!(
            keys,
            vec![
                FoldKey::Assistant(1),
                FoldKey::Thinking(2),
                FoldKey::Tool("c1".to_owned()),
                FoldKey::Diff("c1#0".to_owned()),
                FoldKey::Diff("c1#1".to_owned()),
            ]
        );
    }
}
