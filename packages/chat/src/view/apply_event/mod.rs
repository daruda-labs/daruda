//! ACP event dispatch and its ordered phone, resource and row reconciliation tail.

use daruda_acp::{AcpEvent, permission_item};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::Context;

use super::super::agent_chat_helpers::apply_info_field;
use super::super::reconcile::ReconcileScope;
use super::super::window_access::WindowAccess;
use super::list_sync::ListSync;
use super::{AgentChatView, AgentSessionStatus, PhoneAckEffect, PhoneTurnAction, Replay};

mod content;
mod lifecycle;
mod rows;

/// Changes collected while folding one event. These gates describe independent
/// resource invalidations, not a persistent product state.
struct EventChanges {
    touched_tool: bool,
    touched_text: bool,
    compaction_index: Option<usize>,
    reconcile_scope: ReconcileScope,
    finished_restore: bool,
    turn_settled: bool,
    phone_turn_action: PhoneTurnAction,
}

impl Default for EventChanges {
    fn default() -> Self {
        Self {
            touched_tool: false,
            touched_text: false,
            compaction_index: None,
            reconcile_scope: ReconcileScope::All,
            finished_restore: false,
            turn_settled: false,
            phone_turn_action: PhoneTurnAction::None,
        }
    }
}

impl AgentChatView {
    /// Fold a single [`AcpEvent`] into the chat model + status, reconcile the
    /// diff editors / mermaid diagrams the new content needs, then notify.
    /// Called by the Workspace pump on the foreground for every event.
    pub fn apply_event(
        &mut self,
        event: AcpEvent,
        syntax_theme: &str,
        is_light: bool,
        cx: &mut Context<Self>,
    ) -> PhoneAckEffect {
        // Record the Workspace-resolved theme so a later fold expand can
        // materialize diff embeds without an event to carry it.
        self.set_syntax_theme(syntax_theme);
        // Session errors surface inline in the status banner (the `Error` arm
        // below sets `status`), so this only records to the NDJSON log — no
        // toast. A toast here is pure noise: it duplicates the banner, and on
        // cold restore the auto-connect of any errored session would pop one
        // per pane on startup.
        if let AcpEvent::Error(failure) = &event {
            let report = ErrorReport::new("ACP session error")
                .severity(ErrorSeverity::Error)
                .with_context("detail", failure.message().to_owned())
                .at(file!(), line!())
                .dedup("agent_chat.session_error")
                .build();
            daruda_store::observability::log_writer::LogWriter::log(report);
        }
        let mut changes = EventChanges::default();
        // Returned to the host only after mutation, phone handling and reconciliation.
        let mut telegram_first_response_effect = PhoneAckEffect::None;

        match event {
            AcpEvent::ConnectProgress(phase) => {
                // Guard against a stale progress event arriving after the
                // connect already resolved (Connected/Error) — same shape as
                // the NodeProgress drain's guard in `connect_agent_chat`.
                if matches!(
                    self.status,
                    AgentSessionStatus::Connecting | AgentSessionStatus::Handshaking(_)
                ) {
                    self.set_handshaking(phase, cx);
                }
            }
            // Arrives before any update, so a resume's replayed conversation is
            // already mapped under the right dialect. `Connected` restates it.
            AcpEvent::AgentIdentified { program } => self.agent_program = program,
            AcpEvent::Connected {
                session_id,
                modes,
                config_options,
                capabilities,
                // Dropped: a re-login runs the command its *account domain*
                // defines, so nothing here reads what the agent advertised.
                // These matter once the host offers the agent's own resolved
                // command, or the metered Console sign-in as a second choice —
                // and the agent re-advertises them on every connect, so the
                // pane loses nothing by picking them up only then.
                login_methods: _,
                program,
            } => {
                self.status = AgentSessionStatus::Connected;
                self.agent_program = program;
                self.session_config.modes = modes;
                self.session_config.config_options = config_options;
                self.session_capabilities = capabilities;
                self.finish_connected(session_id, &mut changes, cx);
            }
            AcpEvent::ConfigOptionsChanged(options) => {
                self.session_config.config_options = options;
            }
            AcpEvent::UsageChanged(usage) => {
                self.session_usage = Some(usage.carrying_cost_from(self.session_usage.as_ref()));
            }
            AcpEvent::ModeChanged { state } => {
                self.session_config.modes = Some(state);
            }
            AcpEvent::Compaction { session_id, update } => {
                self.apply_compaction(&session_id, &update, &mut changes);
            }
            AcpEvent::Update(update) => {
                self.apply_content_update(&update, &mut changes);
            }
            AcpEvent::PermissionRequested { id, request } => {
                let item = permission_item(id, &request, &self.items);
                self.items.push(item);
                self.pending_permissions.insert(id);
                changes.phone_turn_action = PhoneTurnAction::Answer;
            }
            AcpEvent::TurnEnded { .. } | AcpEvent::TurnFailed(_)
                if self.queue.turn.awaiting_cancel_ack() =>
            {
                self.apply_cancel_ack(&mut changes, cx);
            }
            AcpEvent::TurnEnded {
                stop_reason,
                completed_normally,
                usage,
            } => {
                self.apply_turn_ended(&stop_reason, completed_normally, usage, &mut changes, cx);
            }
            AcpEvent::AvailableCommandsChanged(commands) => {
                self.session_config.available_commands = commands;
            }
            // Full-replace the plan. Auto-expand when the plan content actually
            // changes (new turn's plan arrived) so the user sees the fresh
            // checklist. Mid-turn re-deliveries of the same entries keep the
            // current collapsed/expanded state to avoid fighting the user.
            AcpEvent::PlanChanged(entries) => {
                if self.plan != entries {
                    self.plan_collapsed = false;
                }
                self.plan = entries;
            }
            AcpEvent::SessionInfoChanged { title, updated_at } => {
                apply_info_field(&mut self.session_title, title);
                apply_info_field(&mut self.session_updated_at, updated_at);
            }
            AcpEvent::Notice(_) | AcpEvent::SessionNotice { .. } | AcpEvent::LegacyDelegation => {
                // Advisory only: the session stays live, so nothing here
                // changes. `Workspace::report_agent_notice` is what puts it
                // in front of the user.
            }
            AcpEvent::ConfigOptionRejected { .. } => {
                // Nothing to do here: the same refusal also arrives as a
                // `Notice`, which is the wording the user reads. This variant
                // exists for a consumer that *required* the change — a flow
                // node pinned to that model has to fail, where a chip the
                // user flipped just stays put.
            }
            AcpEvent::TurnFailed(failure) => {
                self.apply_turn_failed(failure, &mut changes, cx);
            }
            AcpEvent::Error(failure) => {
                self.apply_session_error(failure, &mut changes, cx);
            }
        }
        // Single dispatch point for every arm's `changes.phone_turn_action` above
        // — see `PhoneTurnAction`. Safe to run unconditionally after the
        // match: `Finish`'s "a final streaming text is resolved first"
        // behavior (see `finish_phone_turn`) needs
        // `settle_turn`'s finalize to have already run, which it has by now
        // since every arm that sets `Finish` also calls `settle_turn` earlier
        // in its own body.
        match changes.phone_turn_action {
            PhoneTurnAction::None => {}
            PhoneTurnAction::Clear => self.clear_phone_turn(),
            PhoneTurnAction::Answer => self.answer_phone_turn_for_permission(),
            PhoneTurnAction::CheckUpdate => {
                if let Some(outcome) = self.take_phone_first_response() {
                    telegram_first_response_effect = PhoneAckEffect::Relay(outcome);
                }
            }
            PhoneTurnAction::Finish => {
                telegram_first_response_effect = self.finish_phone_turn();
            }
        }
        // Gate the reconciles on what the event actually changed: diff editors
        // only when a tool call moved, mermaid raster whenever either tool or
        // message content moved. The gate alone is not enough — a `ToolCallUpdate`
        // fires per streamed chunk, so an unscoped pass would still re-fingerprint
        // every diff in the conversation per chunk, O(n²) over a long turn. Hence
        // `changes.reconcile_scope`, which narrows each pass to the call that moved.
        if changes.touched_tool {
            // An ACP event lands from a spawned task, outside any window update
            // cycle, so the window is resolved from the stored handle — see
            // [`WindowAccess`].
            let mut access = WindowAccess::ByHandle(self.window_handle);
            self.reconcile_diff_editors(
                syntax_theme,
                is_light,
                &changes.reconcile_scope,
                &mut access,
                cx,
            );
            // Tool-output images arrive on a `ToolCall`/`ToolCallUpdate`
            // (`changes.touched_tool`), never on `changes.touched_text` (that flag is set only
            // for assistant/thinking/user message text) — gate here, not next
            // to `reconcile_mermaid` below, or an image-only tool update would
            // never get scanned.
            self.reconcile_tool_images(&changes.reconcile_scope, cx);
            // Verbatim output blocks arrive on the same tool events as the
            // images above, so the same gate applies.
            self.reconcile_output_editors(&changes.reconcile_scope, &mut access, cx);
        }
        if changes.compaction_index.is_some() {
            let mut access = WindowAccess::ByHandle(self.window_handle);
            self.reconcile_tool_images(&changes.reconcile_scope, cx);
            self.reconcile_output_editors(&changes.reconcile_scope, &mut access, cx);
        }
        // A turn may finish without a final tool update, and a replay closes
        // with no `TurnEnded` at all. Either settle changes Pending/InProgress
        // calls to terminal, making their resource images eligible for the
        // first time, so revisit them even though this event did not set
        // `changes.touched_tool`.
        if changes.turn_settled || changes.finished_restore {
            self.reconcile_tool_images(&ReconcileScope::All, cx);
        }
        // Mermaid fences arrive in message text AND in tool `Text` output blocks
        // (a tool writing/reading a .md file), so both flags trigger the scan.
        // `dark` must be `host_is_dark` — the same source the theme observer and
        // the render hook use for the cache key; `!is_light` here keyed by the
        // terminal-bg lightness and permanently missed the cache whenever the two
        // disagreed (the raster content itself is themed from `DarudaTheme`).
        if changes.touched_text || changes.touched_tool {
            self.reconcile_mermaid(Self::host_is_dark(cx), cx);
        }
        // During a `session/load` replay the adapter streams the whole prior
        // conversation as many `session/update`s before the `Connected` reply.
        // The reconciles above still run per-event (so diff editors / mermaid
        // for replayed content are built as they arrive), but the row rebuild +
        // repaint is deferred until the `Connected` reply releases the gate — one
        // catch-up instead of a rebuild per replayed event. (The per-event
        // reconciles are unchanged, so replay cost is no better than live-
        // streaming the same events; this only removes the redundant rebuilds.)
        if self.replay.is_loading() {
            return telegram_first_response_effect;
        }
        // Reproject rows + sync the virtualized list. `FollowMode::Tail` keeps
        // the bottom pinned while streaming — no manual scroll needed.
        self.rebuild_rows();
        if let Some(row) = changes
            .compaction_index
            .and_then(|ix| super::super::rows::item_row(&self.rows, ix))
        {
            self.apply_list_sync(ListSync::Rows(row..row + 1), "compaction-update");
        }
        // A tool update can mutate a non-tail card in place (status, output,
        // raw-output fallback, diff body, or a nested subagent child rendered
        // inside its parent). `rebuild_rows` only knows row slots, not which
        // card body changed, so its same-slot path remeasures the streaming tail.
        // Force a full tool-event remeasure to avoid stale row-height cache for
        // mid-list tool cards. `ToolCallUpdate` fires per streamed chunk, so use
        // `remeasure_items` (Absolute anchor) rather than `remeasure()`
        // (Proportional) — same reasoning as the turn-settled case below: a
        // Proportional re-anchor would shift the viewport on every chunk if the
        // user has scrolled back to read history.
        if changes.touched_tool {
            self.resync_all_row_heights("tool-update");
        }
        // Re-measure after a structural settle so no row keeps a stale streaming
        // height. Two triggers, two anchor policies:
        // (a) a replay gate just closed, typically having spliced many rows at
        //     once — force a full `remeasure()` so the list has heights for all
        //     of them before the paint. A cold restore anchors to the tail, so
        //     the proportional re-anchor `remeasure()` performs is irrelevant
        //     here, and a downgraded load with nothing spliced measures nothing.
        // (b) a turn just settled — its streamed rows may have changed height via
        //     the trailing async markdown reparse. Re-derive every row's height,
        //     but through the span API (`remeasure_items`, Absolute anchor) rather
        //     than `remeasure()` (Proportional): if the user has scrolled back to
        //     read history, a Proportional re-anchor shifts their viewport when the
        //     anchored row's height changes, whereas Absolute keeps it fixed.
        // Cheap: at most once per restore / turn.
        if changes.finished_restore {
            self.apply_list_sync(ListSync::EveryRow, "replay-closed");
        }
        if changes.turn_settled {
            self.resync_all_row_heights("turn-settled");
        }
        cx.notify();
        telegram_first_response_effect
    }

    /// Release a stuck replay gate: if a misbehaving adapter closes the event
    /// stream mid-load without a `Connected`/`Error` to close it, the
    /// accumulated items would never project. The pump calls this once its
    /// loop exits so whatever arrived still renders. No-op when not replaying.
    pub fn abort_restore(&mut self, cx: &mut Context<Self>) {
        if self.replay.is_loading() {
            self.replay = Replay::Live;
            // Same settle the `Connected` exit owes (see there), and the same
            // revisit: no event follows a closed stream, so a call this settle
            // just made terminal would otherwise wait for a fold toggle.
            self.settle_run_state(cx);
            self.settle_prompts();
            self.reconcile_tool_images(&ReconcileScope::All, cx);
            self.rebuild_rows();
            self.apply_list_sync(ListSync::EveryRow, "stream-eof");
            cx.notify();
        }
    }

    /// Clean EOF while idle has no pending request to fail. A closed event
    /// stream must still retire the handle and offer a reconnect.
    pub fn needs_disconnect_error(&self) -> bool {
        matches!(
            self.status,
            AgentSessionStatus::PreparingRuntime(_)
                | AgentSessionStatus::Connecting
                | AgentSessionStatus::Handshaking(_)
                | AgentSessionStatus::Connected
        )
    }
}

#[cfg(test)]
mod tests;
