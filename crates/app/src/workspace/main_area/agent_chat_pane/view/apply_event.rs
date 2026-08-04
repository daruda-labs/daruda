//! Fold one `AcpEvent` into the chat model + status (`apply_event`, the
//! per-pane ACP reducer), plus its immediate aftermath: row reprojection
//! (`rebuild_rows`), the replay-abort / still-connecting status queries, and
//! the debug remeasure trace. Kept together since `apply_event` is the sole
//! production caller of `rebuild_rows` (tests call it directly too).

use daruda_acp::{AcpEvent, ChatItem, apply_update_with, permission_item, touched_tool_id};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::project::PaneCwd;
use gpui::Context;

use super::super::agent_chat_helpers::apply_info_field;
use super::super::rows::{RowKind, project};
use super::{
    ActivityState, AgentChatView, AgentSessionStatus, TelegramFirstResponseEffect,
    TelegramWatchAction, TurnOutcome, debug_list_trace_enabled,
};
use crate::surface::strings as s;

impl AgentChatView {
    /// Fold a single [`AcpEvent`] into the chat model + status, reconcile the
    /// diff editors / mermaid diagrams the new content needs, then notify.
    /// Called by the Workspace pump on the foreground for every event.
    pub(in crate::workspace) fn apply_event(
        &mut self,
        event: AcpEvent,
        syntax_theme: &str,
        is_light: bool,
        cx: &mut Context<Self>,
    ) -> TelegramFirstResponseEffect {
        // Session errors surface inline in the status banner (the `Error` arm
        // below sets `status`), so this only records to the NDJSON log — no
        // toast. A toast here is pure noise: it duplicates the banner, and on
        // cold restore the auto-connect of any errored session would pop one
        // per pane on startup.
        if let AcpEvent::Error(message) = &event {
            let report = ErrorReport::new("ACP session error")
                .severity(ErrorSeverity::Error)
                .with_context("detail", message.clone())
                .at(file!(), line!())
                .dedup("agent_chat.session_error")
                .build();
            daruda_store::observability::log_writer::LogWriter::log(report);
        }
        // Non-fatal advisory: log at Warning severity; session stays live.
        if let AcpEvent::Notice(message) = &event {
            let report = ErrorReport::new("ACP session notice")
                .severity(ErrorSeverity::Warning)
                .with_context("detail", message.clone())
                .at(file!(), line!())
                .dedup(format!("agent_chat.notice.{}", self.pane_id))
                .build();
            daruda_store::observability::log_writer::LogWriter::log(report);
        }

        // What this event changed, to gate the expensive full-conversation
        // reconciles below. Only an `Update` carrying tool/text content sets
        // these; every other event leaves both false.
        let mut touched_tool = false;
        let mut touched_text = false;
        // Set only when a `session/load` replay just finished (the `Connected`
        // reply cleared `restoring`), so the tail runs the single catch-up.
        let mut finished_restore = false;
        // Set when a turn just settled (natural completion or a session error).
        // The turn's streamed rows may have changed height via the *trailing*
        // async markdown reparse — `TextView` re-parses ~one debounce after the
        // final chunk, off the outer list's per-chunk remeasure path — so their
        // cached heights can be stale. The tail forces a full remeasure to
        // re-derive them (defensive: prevents a stale streaming height from
        // lingering and inflating the scroll geometry into an oversized gap).
        let mut turn_settled = false;
        // Returned to the Workspace pump after the model mutation, so it can
        // send the first phone-visible reply without letting this self-owned
        // pane reach into Workspace/Telegram state.
        let mut telegram_first_response_effect = TelegramFirstResponseEffect::None;
        // Decided per-arm below, applied once in the tail — see
        // `TelegramWatchAction`.
        let mut telegram_watch_action = TelegramWatchAction::None;

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
            AcpEvent::Connected {
                session_id,
                modes,
                config_options,
                capabilities,
            } => {
                self.status = AgentSessionStatus::Connected;
                if let Some(state) = &modes {
                    self.last_known_mode_id = Some(state.current.clone());
                }
                self.session_config.modes = modes;
                self.session_config.config_options = config_options;
                self.session_capabilities = capabilities;
                // A real resume (`session/load`) returns the same id we asked to
                // load; a resume the agent couldn't load was downgraded to a fresh
                // `session/new` with a NEW id. `self.restoring` was set
                // optimistically at connect and can't tell those apart, so decide
                // by id match — otherwise a downgraded resume keeps the prior
                // session's title and skips fresh-session setup. Compare before
                // overwriting `session_id`.
                let resumed =
                    self.restoring && self.session_id.as_deref() == Some(session_id.as_str());
                self.restoring = false;
                // A resume's replayed `session/update`s already populated `items`
                // by this point (see the comment above) — sync the baseline now
                // so those replayed messages don't later look like a background
                // follow-up. A fresh session's `items` is still empty, so this is
                // a no-op there.
                self.snap_post_turn_baseline();
                // Record the live session id so it persists — and so a later
                // launch resumes this session instead of starting fresh.
                self.session_id = Some(session_id);
                if resumed {
                    // Resume (`session/load`): the replayed `session/update`s
                    // already populated the conversation, plan, and title before
                    // this reply — keep them. Let the tail run the single
                    // coalesced catch-up.
                    finished_restore = true;
                } else {
                    // Fresh session (`session/new`): clear stale plan/title so
                    // they don't flash before the new agent sends its first
                    // updates, and drop the prior session's subagent activity so
                    // its timestamps can't hold the new session's badge "busy".
                    self.plan.clear();
                    self.session_title = None;
                    self.session_updated_at = None;
                    self.plan_collapsed = false;
                    self.session_usage = None;
                    self.activity.subagent_last_activity.clear();
                    // Reset the activity-span tracker so a prior session's edge
                    // state / captured outcome can't leak into the fresh one.
                    self.activity.activity_started_at = None;
                    self.activity.was_busy = false;
                    self.activity.pending_completion = None;
                    self.activity.cancel_in_flight = false;
                    // A fresh session gets its own chance to report a dropped-
                    // output mismatch instead of inheriting the prior session's
                    // silence.
                    self.warned_dropped_terminal_output = false;
                }
                self.pump_pending_prompt(cx);
            }
            AcpEvent::ConfigOptionsChanged(options) => {
                self.session_config.config_options = options;
            }
            AcpEvent::UsageChanged(usage) => {
                self.session_usage = Some(usage);
            }
            AcpEvent::ModeChanged { state } => {
                self.last_known_mode_id = Some(state.current.clone());
                self.session_config.modes = Some(state);
            }
            AcpEvent::Update(update) => {
                // Fold protocol traffic through this pane's per-agent strategy
                // (selected from its catalog id) so vendor-specific `_meta` is
                // read the way that agent emits it. See `daruda_acp::adapter`.
                let adapter = daruda_acp::adapter::adapter_for(&self.agent_id);
                let effect = apply_update_with(&mut self.items, &update, adapter.as_ref());
                touched_tool = effect.touched_tool;
                touched_text = effect.touched_text;
                // A settled tool call whose only content was an embedded
                // terminal handle we couldn't resolve from any channel — the
                // card is left empty. `daruda_acp` has no logger of its own, so
                // the drop is reported back through the effect and logged here.
                // Logged once per pane per session: this indicates a systemic
                // adapter contract mismatch (every command would trip it), so
                // one line is enough to diagnose. The `dedup` key is a grep tag
                // only — `LogWriter::log` writes unconditionally; it does not
                // itself suppress repeats.
                if effect.dropped_terminal_output && !self.warned_dropped_terminal_output {
                    self.warned_dropped_terminal_output = true;
                    let report = ErrorReport::new("Tool output dropped")
                        .severity(ErrorSeverity::Warning)
                        .message(
                            "A shell tool reported its result as an embedded terminal with no \
                             output on any side channel, so the card is empty.",
                        )
                        .with_context("agent", self.agent_id.clone())
                        .at(file!(), line!())
                        .dedup(format!(
                            "agent_chat.dropped_terminal_output.{}",
                            self.agent_id
                        ))
                        .build();
                    daruda_store::observability::log_writer::LogWriter::log(report);
                }
                // Post-turn (background) activity: an update that touches text or a
                // tool while no turn is in flight and we're not replaying a load.
                // Stamp the quiescence clock; the pulse tick relays the settled
                // follow-up (Claude reports background completion here, with no
                // TurnEnded to trigger the normal completion relay).
                if (touched_text || touched_tool)
                    && !self.queue.turn.is_in_flight()
                    && !self.restoring
                {
                    self.activity.post_turn_dirty_at = Some(std::time::Instant::now());
                }
                // Bump the subagent (parent) whose child just produced this
                // tool-call event, so its run span stays "active" across the
                // gaps between the subagent's sequential child calls. Only child
                // tools carry a `parent_tool_id`; a top-level tool has none, so
                // nothing is bumped for the turn's own (foreground) work.
                if let Some(tool_id) = touched_tool_id(&update) {
                    let parent = self.items.iter().rev().find_map(|it| match it {
                        ChatItem::ToolCall(tc) if tc.id == tool_id => tc.parent_tool_id.clone(),
                        _ => None,
                    });
                    if let Some(parent) = parent {
                        self.activity
                            .subagent_last_activity
                            .insert(parent, std::time::Instant::now());
                    }
                }
                telegram_watch_action = TelegramWatchAction::CheckUpdate;
            }
            AcpEvent::PermissionRequested { id, request } => {
                let item = permission_item(id, &request, &self.items);
                self.items.push(item);
                self.pending_permissions.insert(id);
                telegram_watch_action = TelegramWatchAction::Clear;
            }
            AcpEvent::TurnEnded { .. } | AcpEvent::TurnFailed(_)
                if self.activity.cancel_in_flight =>
            {
                // The terminal signal (a `cancelled` `TurnEnded`, or a
                // `TurnFailed` if the prompt errored as the cancel raced it) for a
                // turn a Stop already settled locally — its `Stopped` fired at
                // cancel time. Don't re-settle, re-complete, or push an error
                // item: just close the cancel window and drain any re-prompt the
                // user buffered while it was open (as a fresh turn). A buffered
                // re-prompt was never put on the wire (see `send_prompt_text`'s
                // `cancel_in_flight` guard), so nothing raced this ack and a
                // second Stop could still have cleared it.
                self.activity.cancel_in_flight = false;
                telegram_watch_action = TelegramWatchAction::Clear;
                self.pump_pending_prompt(cx);
            }
            AcpEvent::TurnEnded {
                completed_normally, ..
            } => {
                // Settle the turn: finalize streaming, cancel any tool the agent
                // left non-terminal (e.g. a `Cancelled` stop reason), and drain a
                // still-pending permission so no card keeps live buttons.
                self.settle_turn();
                turn_settled = true;
                telegram_watch_action = TelegramWatchAction::Finish;
                // Capture the outcome; it fires only when the pane settles
                // busy→idle (via `reconcile_activity`), which may trail this
                // `end_turn` while trailing subagents finish.
                self.activity.pending_completion = Some(if completed_normally {
                    TurnOutcome::Completed
                } else {
                    TurnOutcome::Stopped
                });
                // Drain the next buffered prompt (if any) now that the turn
                // completed — one per `TurnEnded`, so the queue advances a single
                // turn at a time and `turn.is_in_flight()` keeps tracking exactly
                // one live turn. A no-op when nothing is buffered or on a cancelled
                // turn whose queue is empty. Pumping here (not inside
                // `settle_turn`) is deliberate: `settle_turn` is also the Stop /
                // `Error` teardown, and pumping from all three would double-drain
                // (a cancelled turn's later idempotent `TurnEnded` re-runs
                // settle) — only a natural completion should advance the queue.
                self.pump_pending_prompt(cx);
                // Auto-collapse the plan region so the completed checklist
                // recedes to a one-line summary. The next `PlanChanged` will
                // re-expand it (see below).
                if !self.plan.is_empty() {
                    self.plan_collapsed = true;
                }
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
            AcpEvent::Notice(_) => {
                // Logged above; no status change.
            }
            AcpEvent::TurnFailed(message) => {
                // A single `session/prompt` failed (e.g. the adapter hit a usage
                // / session limit → `-32603`), but the ACP connection is alive —
                // the error was a normal JSON-RPC response, not a transport
                // failure. So unlike the terminal `Error` arm below, DO NOT set
                // `status = Error` and DO NOT drop the handle: keep the session
                // Connected and usable so the user can re-prompt (e.g. once the
                // limit resets) without reconnecting.
                //
                // Surface the failure inline in the conversation, then settle the
                // turn exactly as a `TurnEnded` would — otherwise a streaming
                // block stays `streaming: true` and an `InProgress` tool stays
                // live, so the rollup glyph blinks forever and the footer reads
                // `Running` after the turn is already over.
                self.items.push(ChatItem::Error(message));
                // (A `TurnFailed` while `cancel_in_flight` is handled by the
                // guarded arm above; here the turn was not being cancelled.)
                self.settle_turn();
                turn_settled = true;
                telegram_watch_action = TelegramWatchAction::Finish;
                // Capture the errored outcome; it fires (notification +
                // backing-task done) on the busy→idle settle edge that
                // `reconcile_activity` detects, same as a normal completion.
                self.activity.pending_completion = Some(TurnOutcome::Errored);
                // Advance the prompt queue one turn like a natural completion, so
                // a prompt the user buffered while this turn ran still runs. No-op
                // when nothing is buffered (the common single-prompt case).
                self.pump_pending_prompt(cx);
            }
            AcpEvent::Error(message) => {
                let error_message = match &self.cwd {
                    Some(PaneCwd::Remote(_)) => {
                        format!(
                            "{}\n\n{}",
                            message,
                            s::agent_chat_remote_connect_error_hint()
                        )
                    }
                    _ => message,
                };
                self.status = AgentSessionStatus::Error(error_message);
                // A session-level error terminates every outstanding turn,
                // including any cancel we were still awaiting an ack for — close
                // the cancel window so a post-reconnect turn isn't misread.
                self.activity.cancel_in_flight = false;
                // A load that fails mid-replay must still render whatever was
                // replayed — release the coalescing gate so the tail rebuilds.
                self.restoring = false;
                // Whatever replayed before the failure is now the baseline —
                // it was already delivered by the replay itself, not a
                // background follow-up.
                self.snap_post_turn_baseline();
                // A mid-turn failure must settle the turn like a Stop would —
                // otherwise a streaming block stays `streaming: true` and an
                // `InProgress` tool stays live, so the rollup glyph blinks
                // forever and the response bar reads `Running` after the session
                // is already dead.
                self.settle_turn();
                turn_settled = true;
                telegram_watch_action = TelegramWatchAction::Finish;
                // Capture the failure outcome; it fires on the busy→idle settle
                // edge (via `reconcile_activity`), same as a normal completion.
                self.activity.pending_completion = Some(TurnOutcome::Errored);
                // The session is dead with no reconnect path, so any buffered
                // prompts can never be delivered — drop both the live queue and
                // a parked queue (a Stop may have parked one before this Error)
                // rather than leaving them to be pumped or shown with a Resume
                // button that can't send (they were never echoed, so nothing
                // dangles in the transcript).
                self.queue.pending_prompts.clear();
                self.queue.paused_prompts.clear();
                self.queue.editing_prompt = None;
                // Drop the now-dead handle. The connection task has ended (this
                // `Error` is its terminal signal), so its command channel is
                // closed — a lingering `Some(handle)` would let `send_prompt_text`
                // send into a dead channel (silently dropped) and mark a turn
                // in-flight that never ends, stranding the pane on a phantom
                // "Working". With `None`, a post-error prompt buffers instead of
                // stranding. (Distinct from `TurnFailed`, which keeps the handle:
                // there the connection is still alive.)
                self.handle = None;
            }
        }
        // Single dispatch point for every arm's `telegram_watch_action` above
        // — see `TelegramWatchAction`. Safe to run unconditionally after the
        // match: `Finish`'s "a final streaming text is resolved first"
        // behavior (see `finish_telegram_first_response_watch`) needs
        // `settle_turn`'s finalize to have already run, which it has by now
        // since every arm that sets `Finish` also calls `settle_turn` earlier
        // in its own body.
        match telegram_watch_action {
            TelegramWatchAction::None => {}
            TelegramWatchAction::Clear => self.clear_telegram_first_response_watch(),
            TelegramWatchAction::CheckUpdate => {
                if let Some(outcome) = self.take_telegram_first_response() {
                    telegram_first_response_effect = TelegramFirstResponseEffect::Relay(outcome);
                }
            }
            TelegramWatchAction::Finish => {
                telegram_first_response_effect = self.finish_telegram_first_response_watch();
            }
        }
        // Gate the full-conversation reconciles on what the event actually
        // changed: diff editors only when a tool call moved, mermaid raster
        // whenever either tool or message content moved. Running these on every
        // streamed chunk would rescan the whole `items` vec per chunk — O(n²)
        // over a long turn.
        if touched_tool {
            self.reconcile_diff_editors(syntax_theme, is_light, cx);
            // Tool-output images arrive on a `ToolCall`/`ToolCallUpdate`
            // (`touched_tool`), never on `touched_text` (that flag is set only
            // for assistant/thinking/user message text) — gate here, not next
            // to `reconcile_mermaid` below, or an image-only tool update would
            // never get scanned.
            self.reconcile_tool_images(cx);
            // Verbatim output blocks arrive on the same tool events as the
            // images above, so the same gate applies.
            self.reconcile_output_editors(cx);
        }
        // Mermaid fences arrive in message text AND in tool `Text` output blocks
        // (a tool writing/reading a .md file), so both flags trigger the scan.
        // `dark` must be `host_is_dark` — the same source the theme observer and
        // the render hook use for the cache key; `!is_light` here keyed by the
        // terminal-bg lightness and permanently missed the cache whenever the two
        // disagreed (the raster content itself is themed from `DarudaTheme`).
        if touched_text || touched_tool {
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
        if self.restoring {
            return telegram_first_response_effect;
        }
        // Reproject rows + sync the virtualized list. `FollowMode::Tail` keeps
        // the bottom pinned while streaming — no manual scroll needed.
        self.rebuild_rows();
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
        if touched_tool {
            let n = self.rows.len();
            self.list_state.remeasure_items(0..n);
            self.trace_list_sync("tool-update", 0, n, n);
        }
        // Re-measure after a structural settle so no row keeps a stale streaming
        // height. Two triggers, two anchor policies:
        // (a) a `session/load` replay just spliced many rows at once — force a
        //     full `remeasure()` so the list has heights for all of them before
        //     the paint. A cold restore anchors to the tail, so the proportional
        //     re-anchor `remeasure()` performs is irrelevant here.
        // (b) a turn just settled — its streamed rows may have changed height via
        //     the trailing async markdown reparse. Re-derive every row's height,
        //     but through the span API (`remeasure_items`, Absolute anchor) rather
        //     than `remeasure()` (Proportional): if the user has scrolled back to
        //     read history, a Proportional re-anchor shifts their viewport when the
        //     anchored row's height changes, whereas Absolute keeps it fixed.
        // Cheap: at most once per restore / turn.
        if finished_restore {
            self.list_state.remeasure();
        }
        if turn_settled {
            // Span is all rows and the count is unchanged, so `to` and `prev_rows`
            // both equal the current row count.
            let n = self.rows.len();
            self.list_state.remeasure_items(0..n);
            self.trace_list_sync("turn-settled", 0, n, n);
        }
        cx.notify();
        telegram_first_response_effect
    }

    /// Release a stuck replay gate: if a misbehaving adapter closes the event
    /// stream mid-load without a `Connected`/`Error` to clear `restoring`, the
    /// accumulated items would never project. The pump calls this once its
    /// loop exits so whatever arrived still renders. No-op when not restoring.
    pub(in crate::workspace) fn abort_restore(&mut self, cx: &mut Context<Self>) {
        if self.restoring {
            self.restoring = false;
            // Whatever arrived before the stream closed is now the baseline —
            // it was already delivered by the (aborted) replay, not a
            // background follow-up.
            self.snap_post_turn_baseline();
            self.rebuild_rows();
            self.list_state.remeasure();
            cx.notify();
        }
    }

    /// End-of-stream safety-net predicate: true while `status` is a
    /// non-terminal connecting state. Normally the stream never closes before
    /// `Connected`/`Error` fires, but a connection task that panics (or is
    /// dropped before its `Err` path runs) closes it silently, stranding the
    /// pane on "Connecting…" forever with no retry affordance. When this is
    /// true, the pump feeds a real `AcpEvent::Error` through `apply_event`
    /// instead of setting `status` directly, so the failure gets the exact
    /// same handling as any other terminal error.
    pub(in crate::workspace) fn is_still_connecting(&self) -> bool {
        matches!(
            self.status,
            AgentSessionStatus::PreparingRuntime(_)
                | AgentSessionStatus::Connecting
                | AgentSessionStatus::Handshaking(_)
        )
    }

    /// Recompute the projected render rows from `items` + `fold` and sync the
    /// virtualized `list` to them. The single rebuild site; call after any
    /// `items` or `fold` mutation. Diffs old vs new rows by `same_slot`:
    /// structural changes `splice` from the first divergent slot (scroll above
    /// it preserved); same slots & count (a fold flip or streamed tail growth)
    /// `remeasure_items` over just the changed span with an `Absolute` anchor
    /// so reading history during streaming never drifts.
    pub(super) fn rebuild_rows(&mut self) {
        let old = std::mem::take(&mut self.rows);
        // The inline working indicator means "answering" — suppress it while
        // blocked on a permission prompt (the card + footer already say so).
        let awaiting_response = matches!(self.activity_state(), ActivityState::Working);
        self.rows = project(&self.items, &self.fold, awaiting_response);

        if let Some(at) = old
            .iter()
            .zip(&self.rows)
            .position(|(a, b)| !a.same_slot(b))
        {
            self.list_state.splice(at..old.len(), self.rows.len() - at);
            self.trace_list_sync("splice-divergent", at, self.rows.len(), old.len());
            return;
        }
        if old.len() != self.rows.len() {
            let at = old.len().min(self.rows.len());
            self.list_state.splice(at..old.len(), self.rows.len() - at);
            self.trace_list_sync("splice-count", at, self.rows.len(), old.len());
            return;
        }
        // Same slots & count: only `hidden` flipped or item content grew.
        // Remeasure the span whose `hidden` changed; if none did, a streamed
        // tail grow → remeasure the last row.
        let (lo, hi) = old
            .iter()
            .zip(&self.rows)
            .enumerate()
            .filter(|(_, (a, b))| a.hidden != b.hidden)
            .fold((usize::MAX, 0usize), |(lo, hi), (i, _)| {
                (lo.min(i), hi.max(i))
            });
        if lo == usize::MAX {
            // No `hidden` flip: a streamed chunk grew a row's content in place.
            // During an active turn the *last* row is a fixed-height
            // `WorkingIndicator` (`rows::project` pins it to the tail), so the row
            // that actually grew is the last non-indicator row. Remeasuring only
            // `n-1` would re-measure the indicator and leave the grown content row
            // at its stale (shorter) cached height — which then inflates the
            // scroll geometry the moment that row scrolls into the overdraw zone
            // (the intermittent oversized-gap bug). Remeasure from the last
            // content row through the end so both the grown row and the indicator
            // are covered.
            let n = self.rows.len();
            if n > 0 {
                let start = self.rows[..n]
                    .iter()
                    .rposition(|r| !matches!(r.kind, RowKind::WorkingIndicator))
                    .unwrap_or(n - 1);
                self.list_state.remeasure_items(start..n);
                self.trace_list_sync("tail-grow", start, n, old.len());
            }
        } else {
            self.list_state.remeasure_items(lo..hi + 1);
            self.trace_list_sync("hidden-span", lo, hi + 1, old.len());
        }
    }

    /// Trace one list-sync decision (splice or remeasure) to the NDJSON log,
    /// silent unless `DARUDA_DEBUG_AGENT_LIST` is set. Kept compiled in
    /// (near-zero cost off) to capture the sync timeline on recurrence of the
    /// intermittent oversized-gap bug (a row's height changes without a
    /// matching remeasure). `prev_rows` is the count *before* this sync, so a
    /// splice's count delta is visible in the trace.
    fn trace_list_sync(&self, branch: &str, from: usize, to: usize, prev_rows: usize) {
        if !debug_list_trace_enabled() {
            return;
        }
        daruda_store::observability::log_writer::LogWriter::log(
            ErrorReport::new("agent-chat list sync")
                .severity(ErrorSeverity::Info)
                .with_context("pane", self.pane_id.to_string())
                .with_context("branch", branch.to_string())
                .with_context("from", from.to_string())
                .with_context("to", to.to_string())
                .with_context("rows", self.rows.len().to_string())
                .with_context("prev_rows", prev_rows.to_string())
                .at(file!(), line!())
                .dedup("agent_chat.list_sync_trace")
                .build(),
        );
    }
}
