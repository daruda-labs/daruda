//! Turn cancel/settle, permission responses, fold/plan/mode toggles, and
//! session teardown/reset for one Agent chat pane. The Stop path and the
//! `/clear`-style resets live here since they all end in the same shared
//! settle/teardown core.

use daruda_acp::{
    ChatItem, PermissionDecision, PermissionKindView, cancel_pending_tools, finalize_streaming,
};
use gpui::{Context, Window};

use super::super::agent_chat_helpers::{
    cancel_pending_permission, collect_foldable_keys, fold_context, fold_key_item_index,
    permission_card_mut,
};
use super::super::agent_chat_ops::model_select;
use super::super::fold::FoldKey;
use super::super::pane_choice::PaneChoice;
use super::super::reconcile::ReconcileScope;
use super::super::rows::RowKind;
use super::super::rows::tail::TailWindow;
use super::super::session_config::SessionConfig;
use super::super::transcript_defaults::TranscriptDefaults;
use super::super::window_access::WindowAccess;
use super::{ActivityOptionsTab, AgentChatView, AgentSessionStatus, Turn, TurnOutcome};
use crate::transcript::display_filter::{DisplayFilter, FilterFacet};
use crate::transcript::fold_mode::{FoldMode, FoldPreset, TurnPosition};

impl AgentChatView {
    /// Stop the active turn: send `session/cancel` *and* end the turn locally
    /// right now, without waiting for the agent's stop reason. `cancel` is
    /// only cooperative — a hung/dead agent may never send the `Cancelled`
    /// `TurnEnded`, leaving the turn pulsing forever otherwise. A later
    /// `TurnEnded` for this turn is idempotent.
    pub(in crate::workspace) fn cancel_turn(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = &self.handle {
            handle.cancel();
        }
        // Settle the turn *locally and immediately* — responsive and hung-safe: a
        // connected agent that never returns the `cancelled` `TurnEnded` (or
        // returns it much later) can't leave the pane stuck busy. When a live
        // foreground turn is being cancelled, stash its `Stopped` (fired at the
        // busy→idle edge by `cancel_agent_turn`'s reconcile) and open the cancel
        // window (`cancel_in_flight`): until the cancel is acked, a re-prompt
        // buffers *client-side* (see `send_prompt_text`) rather than racing onto
        // the wire, so it can't be misattributed to the cancelled turn's ack and
        // a second Stop can still clear it. A trailing-subagent Stop (turn already
        // idle) neither stashes nor opens the window: the foreground turn's
        // already-captured outcome is preserved and fires when the tools settle.
        if self.queue.turn.is_in_flight() {
            self.activity.pending_completion = Some(TurnOutcome::Stopped);
            self.activity.cancel_in_flight = true;
            // Mark where the run was cut — the adapter's live path drops the
            // SDK's own interrupt entry while its replay lets it through, so
            // without this push a stop is invisible until restore. See
            // `daruda_acp::ChatItem::Interrupted`.
            //
            // Gated on a live turn to match when the SDK actually records one:
            // every writer is reached only while a request is being processed —
            // four inside the query generator (abort, query error, pre-stop
            // hook) plus the slash-command expansion's `AbortError` catch
            // (`cli.js`, claude-agent-sdk 2.1.44). An interrupt with nothing in
            // progress writes nothing.
            //
            // `handle.cancel()` above is deliberately *not* gated the same way:
            // Escape's third branch cancels a trailing background subagent with
            // the turn already idle, and that is a real cancel that records no
            // entry. Marking it would invent a row the replay cannot reproduce.
            self.items.push(ChatItem::Interrupted);
        }
        self.settle_turn();
        // Stop can be the only terminal transition a live resource link gets;
        // the agent is allowed to never acknowledge the cooperative cancel.
        self.reconcile_tool_images(&ReconcileScope::All, cx);
        self.clear_telegram_first_response_watch();
        // Stop cancels the running turn but PRESERVES the queue: move everything
        // buffered before this Stop into the parked queue rather than dropping
        // it. Parked prompts do NOT auto-drain — they live outside
        // `pending_prompts`, so the cancel-ack / `TurnEnded` pump is a no-op on
        // them. The user resumes them explicitly (the queue strip's Resume
        // button → `resume_queue`) or discards them (a second Esc / clear-all).
        // A prompt typed *after* Stop is pushed onto the now-empty live queue and
        // (idle + connected) sends immediately as a fresh turn, ahead of the
        // parked queue. `append` preserves FIFO order and stacks after any items
        // parked by a prior Stop.
        self.queue
            .paused_prompts
            .append(&mut self.queue.pending_prompts);
        self.queue.editing_prompt = None;
        // `settle_turn` mutated items (streaming → done, running tools →
        // cancelled, pending card → resolved), changing fold visibility and row
        // heights, so reproject and remeasure before notifying.
        self.rebuild_rows();
        self.list_state.remeasure();
        cx.notify();
    }

    /// Drop the stop marker back to the end of the transcript if anything landed
    /// behind it. Called on the cancel ack — the point after which no further
    /// chunk for the cancelled turn can arrive.
    ///
    /// Stop pushes the marker immediately (it has to: the ack may never come),
    /// but a chunk already on the wire still lands after it — nothing gates
    /// `AcpEvent::Update` on the open cancel window, and the adapter's stream
    /// path has no cancelled check of its own. Left in place that text reads as
    /// a promptless turn under the marker, since [`agent_run`] treats the marker
    /// as a run boundary; and the SDK persists the assistant message *before*
    /// its interrupt entry, so a restored pane would order it the other way.
    ///
    /// This is the one structural removal in the pane besides `items.clear()`,
    /// so it shifts item indices — but only for items *after* the marker, which
    /// by construction arrived in the window between Stop and its ack. The one
    /// thing that can already point there is a fold the user opened on that
    /// content while the ack was outstanding; it lands on the neighbouring key
    /// and self-corrects on the next toggle. Everything else is keyed by id
    /// (permissions, subagents) or recomputed by `rebuild_rows`.
    ///
    /// [`agent_run`]: super::super::agent_chat_helpers::agent_run
    pub(super) fn settle_stop_marker(&mut self) {
        if matches!(self.items.last(), Some(ChatItem::Interrupted)) {
            return;
        }
        let Some(ix) = self
            .items
            .iter()
            .rposition(|item| matches!(item, ChatItem::Interrupted))
        else {
            return;
        };
        self.items.remove(ix);
        self.items.push(ChatItem::Interrupted);
    }

    /// End the current turn locally and settle every still-live item. Model-only
    /// (no rows/notify) so the three call paths (Stop, `TurnEnded`, `Error`)
    /// share one settle sequence and can't drift. Idempotent.
    pub(super) fn settle_turn(&mut self) {
        self.queue.turn = Turn::Idle;
        self.settle_items();
        // Everything the turn produced is delivered by the completion relay, so
        // reset the post-turn baseline to the current assistant-text count; only
        // messages that arrive *after* this settle count as a follow-up.
        self.snap_post_turn_baseline();
    }

    /// Settle every still-live item without touching `turn`. [`Self::settle_turn`]
    /// is this plus `turn = Idle`; [`Self::cancel_turn`] calls this alone so
    /// the turn stays in-flight until its real `TurnEnded` drives the drain.
    fn settle_items(&mut self) {
        finalize_streaming(&mut self.items);
        cancel_pending_tools(&mut self.items);
        cancel_pending_permission(self);
    }

    /// Resolve the permission request `request_id` with the chosen option:
    /// mark that card resolved (by id, not position — several may be
    /// outstanding), send the decision, drop the id from the index. No-op if
    /// already answered or cancelled.
    pub(in crate::workspace) fn respond_permission(
        &mut self,
        request_id: u64,
        option_id: String,
        kind: PermissionKindView,
        cx: &mut Context<Self>,
    ) {
        if !self.pending_permissions.remove(&request_id) {
            return;
        }
        // Mark the card that carries this request id resolved so its buttons
        // disable and the choice shows — the others stay live.
        if let Some(card) = permission_card_mut(self, request_id) {
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
        if let Some(handle) = &self.handle {
            handle.respond_permission(request_id, decision);
        }
        // The card is now resolved, so it no longer force-stays-visible under a
        // collapsed response: reproject so it folds back into the process
        // immediately. It also renders shorter (buttons → outcome line), so
        // remeasure for the reflow (item count unchanged).
        self.rebuild_rows();
        self.list_state.remeasure();
        cx.notify();
    }

    /// Put `key` in a known state instead of flipping whatever it is in.
    ///
    /// `toggle_fold` reads the *visible* state, so a capture that toggles
    /// blindly shoots the inverse whenever a user's fold matrix already put the
    /// block the other way — and a screenshot has no way to report that it did.
    /// Reads the same source `toggle_fold` reads, so it is exact rather than a
    /// guess about the configured default.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn set_fold_for_shot(
        &mut self,
        key: FoldKey,
        expanded: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.fold.is_expanded(&key, fold_context(&key, &self.items)) != expanded {
            self.toggle_fold(key, window, cx);
        }
    }

    pub(in crate::workspace) fn toggle_fold(
        &mut self,
        key: FoldKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Resolve the context via the shared `fold_context` — the same source
        // `rows::project` uses to derive the default collapsed state — so the
        // first click flips the *visible* state rather than a stale re-derivation.
        let ctx = fold_context(&key, &self.items);
        // Resolve before the key moves into `fold.toggle` below: a nested fold
        // (a tool card's own body, one of its diffs, its raw-input disclosure,
        // or a nested subagent card) only changes its *owning row's* rendered
        // height in place — no `RenderRow::hidden` flip anywhere — so
        // `rebuild_rows`'s hidden-range diff can't see it and falls back to
        // remeasuring the tail, leaving that row's cached height stale (content
        // clips or overlaps the next row). Remeasure it explicitly whenever we
        // can resolve which row owns the toggled key.
        let item_ix = fold_key_item_index(&key, &self.items);
        // A card's own fold decides whether its body — and so its embed editors —
        // is on screen at all. Narrow the follow-up reconcile to that card;
        // response and group folds only flip row visibility, leaving each card's
        // own state (and its embeds) untouched.
        let embed_scope = match &key {
            FoldKey::Tool(id) | FoldKey::Subagent(id) => Some(ReconcileScope::Tool(id.clone())),
            FoldKey::Assistant(_)
            | FoldKey::Thinking(_)
            | FoldKey::Response(_)
            | FoldKey::ToolGroup(_)
            | FoldKey::ThinkingGroup(_)
            | FoldKey::Tail(_)
            | FoldKey::ToolGroupTail(_)
            | FoldKey::Filtered(_)
            | FoldKey::Diff(_)
            // A card's step boundary changes which children render, but every
            // child's embeds are judged by that child's own fold key alone
            // (`tool_body_on_screen`), so a withheld child's editors already
            // exist and nothing has to be rebuilt. Same shape as the raw-input
            // disclosure: an in-place body height change, which is what makes
            // `fold_key_item_index`'s targeted remeasure the operative fix.
            | FoldKey::SubagentTail(_)
            | FoldKey::ToolRawInput(_) => None,
        };
        self.fold.toggle(key, ctx);
        let reconciled = embed_scope.is_some();
        if let Some(scope) = embed_scope {
            self.reconcile_embeds_after_fold(&scope, &mut WindowAccess::Live(window), cx);
        }
        // A fold change flips row `hidden` flags (and may collapse a group):
        // reproject + reflow the affected span.
        self.rebuild_rows();
        if reconciled {
            // The reconcilers end in `remeasure()` — a *Proportional* re-anchor —
            // and the targeted call below only overrides `pending_scroll` when the
            // toggled row happens to be the scroll top. Reading history and
            // expanding a card would otherwise shift the viewport. Re-derive the
            // same full span through the Absolute-anchored API instead, matching
            // `apply_event`'s tool-update / turn-settled remeasures.
            let n = self.rows.len();
            self.list_state.remeasure_items(0..n);
        } else if let Some(item_ix) = item_ix
            && let Some(row_ix) = self
                .rows
                .iter()
                .position(|r| matches!(r.kind, RowKind::AgentItem(ix) if ix == item_ix))
        {
            self.list_state.remeasure_items(row_ix..row_ix + 1);
        }
        cx.notify();
    }

    /// Expand or collapse every currently-visible foldable block at once (the
    /// fold toolbar's expand-all / collapse-all).
    pub(in crate::workspace) fn set_all_folds(
        &mut self,
        expanded: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keys = collect_foldable_keys(&self.items);
        self.fold.set_all(keys, expanded);
        // Every card's fold just moved, so every card's embeds may need building
        // (expand-all) or releasing (collapse-all).
        self.reconcile_embeds_after_fold(&ReconcileScope::All, &mut WindowAccess::Live(window), cx);
        // Bulk expand/collapse flips many row `hidden` flags: reproject + reflow.
        self.rebuild_rows();
        // Unlike a single `toggle_fold`, this can change dozens of rows' inner
        // content height at once (every tool card's body, every diff, every
        // raw-input disclosure) with no per-row hidden flip to key a targeted
        // remeasure off of — a full remeasure is the correct (if broader) fix
        // here, same as `respond_permission`'s reflow.
        self.list_state.remeasure();
        cx.notify();
    }

    /// Test-only hook: append items and reproject, so a test outside this module
    /// can stand a transcript up without a session. Mirrors what an `AcpEvent`
    /// arrival ends in.
    #[cfg(test)]
    pub(in crate::workspace) fn seed_items_for_test(
        &mut self,
        items: impl IntoIterator<Item = daruda_acp::ChatItem>,
        cx: &mut Context<Self>,
    ) {
        self.items.extend(items);
        self.reproject(cx);
    }

    /// Re-derive the row projection and reflow the list. The one sequence every
    /// transcript-preference change ends in, since all of them feed `project`.
    fn reproject(&mut self, cx: &mut Context<Self>) {
        self.rebuild_rows();
        self.list_state.remeasure();
        cx.notify();
    }

    /// Get this pane's transcript preferences onto disk as they now stand. The
    /// snapshot writes each axis from its `chosen()`, so this is equally the
    /// way a reset *erases* a stored override. Deferred because the save
    /// re-enters the workspace, which is still mid-update while the chip
    /// handler that called this runs.
    fn persist_pane_prefs(&self, cx: &mut Context<Self>) {
        let window_handle = self.window_handle;
        cx.defer(move |cx| {
            if let Some(workspace) =
                crate::window_registry::WindowRegistry::workspace_for_window(window_handle, cx)
            {
                // SILENT-OK: the window may close before the deferred save runs.
                let _ = workspace.update(cx, |ws, cx| ws.mutate_durable(cx, |_, _| {}));
            }
        });
    }

    /// Toggle this pane between pane-wide wrapping and the configured reading
    /// column. Width changes can reflow every row, so invalidate all list
    /// measurements and persist the pane-local preference.
    pub(in crate::workspace) fn toggle_content_width(&mut self, cx: &mut Context<Self>) {
        self.content_width = self.content_width.toggle();
        self.list_state.remeasure();
        cx.notify();
        self.persist_pane_prefs(cx);
    }

    /// Follow a reloaded config: every transcript preference the user has not
    /// picked for this pane moves to the new default, and the ones they did
    /// pick stay. The single site that applies all three, so a pane open across
    /// a config edit ends up where a freshly restored one would. Nothing is
    /// persisted — a seed is not a choice.
    pub(in crate::workspace) fn reseed_transcript_defaults(
        &mut self,
        defaults: &TranscriptDefaults,
        cx: &mut Context<Self>,
    ) {
        // Remembered so a later reset can hand an axis back to *this* default
        // without the view resolving config on its own.
        self.defaults = *defaults;
        let before = (self.tail, self.fold.mode(), self.display_filter);
        self.tail.reseed(defaults.tail);
        self.fold.reseed_mode(defaults.fold_mode);
        self.display_filter.reseed(defaults.filter);
        if before == (self.tail, self.fold.mode(), self.display_filter) {
            return;
        }
        // A reseeded fold matrix moves every card's derived default, so the cards
        // it just opened owe the same embed pass a fold click does — see
        // [`Self::reconcile_embeds_after_fold`]. By handle, not a live borrow:
        // this arrives from `apply_config`'s global observer, which fires from
        // `flush_effects` with the window already back in `App::windows`.
        if self.fold.mode() != before.1 {
            self.reconcile_embeds_after_fold(
                &ReconcileScope::All,
                &mut WindowAccess::ByHandle(self.window_handle),
                cx,
            );
        }
        // A reveal names a row the old filter hid, so it cannot outlive it.
        self.fold.clear_filter_reveals();
        // All three feed the projection, so the transcript has to be re-derived
        // before the pane paints again.
        self.reproject(cx);
    }

    /// Replace the conversation with a fixed transcript — a `--screenshot`
    /// scenario or a `--replay-acp-log` capture — then run the same aftermath a
    /// live event does. The one seeding entry point, so no caller can leave
    /// `rows`, the virtualized list or the embed caches out of step with
    /// `items`; without the embed pass a seeded pane renders diffs and verbatim
    /// output through fallbacks no live session takes.
    #[cfg(feature = "devtools")]
    pub(in crate::workspace) fn seed_transcript(
        &mut self,
        items: Vec<daruda_acp::ChatItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.items = items;
        // A seeded pane stands in for a session it does not have. Parking it out
        // of `Idle` here is what keeps `maybe_connect_agent_chat` from spawning
        // a real adapter behind it on first focus. `handle` stays `None`, which
        // is what the send path already checks, so the pane reads as connected
        // and behaves as inert rather than half-live.
        self.status = AgentSessionStatus::Connected;
        // Seeding runs inside the opener's window update cycle — see [`WindowAccess`].
        let mut access = WindowAccess::Live(window);
        self.reconcile_all_embeds(&mut access, cx);
        self.reproject(cx);
    }

    /// Seed a transcript captured while the foreground turn is still active.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn seed_working_transcript(
        &mut self,
        items: Vec<daruda_acp::ChatItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let now = std::time::Instant::now();
        self.queue.turn = Turn::InFlight { started_at: now };
        let _ = self.reconcile_activity(now);
        self.seed_transcript(items, window, cx);
    }

    pub(in crate::workspace) fn set_tail_window(
        &mut self,
        tail: TailWindow,
        cx: &mut Context<Self>,
    ) {
        // Choosing the seeded value still detaches the pane from config.
        let choice = PaneChoice::Chosen(tail);
        let choice_changed = self.tail != choice;
        let reveal_changed = self.fold.clear_tail_reveals();
        if !choice_changed && !reveal_changed {
            return;
        }
        self.tail = choice;
        self.reproject(cx);
        // A cleared reveal is transient, so only a new choice is worth a save.
        if choice_changed {
            self.persist_pane_prefs(cx);
        }
    }

    /// Hand the tail axis back to config. Not a pick of the default's value:
    /// the pane follows every later config edit again, and the save below
    /// clears the stored override rather than writing a new one.
    pub(in crate::workspace) fn reset_tail_window(&mut self, cx: &mut Context<Self>) {
        let before = self.tail;
        self.tail.reset(self.defaults.tail);
        let reveal_changed = self.fold.clear_tail_reveals();
        if before == self.tail && !reveal_changed {
            return;
        }
        self.reproject(cx);
        if before != self.tail {
            self.persist_pane_prefs(cx);
        }
    }

    pub(in crate::workspace) fn set_fold_mode(
        &mut self,
        mode: FoldMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.fold.chosen_mode() == Some(mode) {
            return;
        }
        let current = self.fold.mode();
        self.fold_editor.remember(current, mode);
        self.fold.set_mode(mode);
        self.reconcile_embeds_after_mode_move(current, window, cx);
        self.reproject(cx);
        self.persist_pane_prefs(cx);
    }

    /// Rebuild the embeds a fold-*mode* move put on (or took off) the screen.
    ///
    /// A mode carries the derived default for every card at once, so unlike
    /// `toggle_fold` there is no single card to scope to — this is the same debt
    /// `set_all_folds` pays, for the same reason (see
    /// [`Self::reconcile_embeds_after_fold`]). Gated on the *effective* matrix
    /// actually moving: picking a value equal to the configured default only
    /// changes whether the pane is following it, which no card's body can see.
    fn reconcile_embeds_after_mode_move(
        &mut self,
        before: FoldMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.fold.mode() == before {
            return;
        }
        self.reconcile_embeds_after_fold(&ReconcileScope::All, &mut WindowAccess::Live(window), cx);
    }

    /// Pick a segment of the fold editor's preset strip. `None` is the `Custom`
    /// segment: it re-selects the remembered hand-edited matrix, and is a no-op
    /// when there is none (the strip disables it in that state).
    pub(in crate::workspace) fn select_fold_preset(
        &mut self,
        preset: Option<FoldPreset>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(mode) = self.fold_editor.segment_target(preset) else {
            return;
        };
        self.set_fold_mode(mode, window, cx);
    }

    /// Hand the fold axis back to config — see [`Self::reset_tail_window`].
    pub(in crate::workspace) fn reset_fold_mode(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let before = self.fold.mode_choice();
        // Leaving a hand-edited matrix, same as picking a preset does — the two
        // sit side by side in the panel, so only one of them keeping the work
        // would be a surprise.
        let current = self.fold.mode();
        self.fold_editor
            .remember_before_reset(current, self.defaults.fold_mode);
        self.fold.reset_mode(self.defaults.fold_mode);
        if before == self.fold.mode_choice() {
            return;
        }
        self.reconcile_embeds_after_mode_move(current, window, cx);
        self.reproject(cx);
        self.persist_pane_prefs(cx);
    }

    /// Record the pane width measured during the last paint.
    pub(in crate::workspace) fn set_pane_width(
        &mut self,
        width: gpui::Pixels,
        cx: &mut Context<Self>,
    ) {
        let was_compact = self.activity_bar_is_compact(cx);
        self.pane_width = Some(width);
        if self.activity_bar_is_compact(cx) != was_compact {
            cx.notify();
        }
    }

    /// Whether transcript controls should collapse into one popover. The
    /// threshold is font-dependent (the chips it has to fit are text), so it
    /// comes from `theme` rather than a bare constant.
    pub(in crate::workspace) fn activity_bar_is_compact(&self, cx: &gpui::App) -> bool {
        self.pane_width
            .is_some_and(|w| f32::from(w) <= crate::ui::theme::agent_chat_compact_options_w(cx))
    }

    pub(in crate::workspace) fn set_fold_editor_turn(
        &mut self,
        turn: TurnPosition,
        cx: &mut Context<Self>,
    ) {
        if self.fold_editor.set_turn(turn) {
            cx.notify();
        }
    }

    pub(in crate::workspace) fn set_activity_options_tab(
        &mut self,
        tab: ActivityOptionsTab,
        cx: &mut Context<Self>,
    ) {
        if self.activity_options_tab != tab {
            self.activity_options_tab = tab;
            cx.notify();
        }
    }

    pub(in crate::workspace) fn toggle_display_facet(
        &mut self,
        facet: FilterFacet,
        cx: &mut Context<Self>,
    ) {
        self.set_display_filter(self.display_filter.value().toggled(facet), cx);
    }

    /// Hand the filter axis back to config — see [`Self::reset_tail_window`].
    pub(in crate::workspace) fn reset_display_filter(&mut self, cx: &mut Context<Self>) {
        let before = self.display_filter;
        self.display_filter.reset(self.defaults.filter);
        let reveal_changed = self.fold.clear_filter_reveals();
        if before == self.display_filter && !reveal_changed {
            return;
        }
        self.reproject(cx);
        if before != self.display_filter {
            self.persist_pane_prefs(cx);
        }
    }

    /// Turn a whole parented filter section (`Prose`, `Tools`) on or off — what
    /// its tri-state parent checkbox does.
    pub(in crate::workspace) fn set_filter_section(
        &mut self,
        parent: FilterFacet,
        selected: bool,
        cx: &mut Context<Self>,
    ) {
        let next = self.display_filter.value().with_section(parent, selected);
        self.set_display_filter(next, cx);
    }

    fn set_display_filter(&mut self, filter: DisplayFilter, cx: &mut Context<Self>) {
        let choice = PaneChoice::Chosen(filter);
        let choice_changed = self.display_filter != choice;
        let reveal_changed = self.fold.clear_filter_reveals();
        if !choice_changed && !reveal_changed {
            return;
        }
        self.display_filter = choice;
        self.reproject(cx);
        // A cleared reveal is transient, so only a new choice is worth a save.
        if choice_changed {
            self.persist_pane_prefs(cx);
        }
    }

    /// Collapse / expand the bottom plan region. The plan is a derived render
    /// of `plan` (full-replaced by the agent), so this only flips the local
    /// presentation flag and notifies — no model rebuild needed.
    pub(in crate::workspace) fn toggle_plan_collapsed(&mut self, cx: &mut Context<Self>) {
        self.plan_collapsed = !self.plan_collapsed;
        cx.notify();
    }

    /// Dismiss the plan region: drop the entries so it renders nothing. Pure
    /// local presentation reset — a later `PlanChanged` repopulates it
    /// (expanded, since `plan_collapsed` resets too).
    pub(in crate::workspace) fn dismiss_plan(&mut self, cx: &mut Context<Self>) {
        self.plan.clear();
        self.plan_collapsed = false;
        cx.notify();
    }

    /// Switch the active session mode: show the pick immediately, then ask the
    /// agent over the live handle (no-op when the handle is absent). A
    /// `ModeChanged` replaces the whole state if the agent disagrees.
    pub(in crate::workspace) fn set_mode(&mut self, mode_id: String, cx: &mut Context<Self>) {
        self.session_config
            .set_current_mode_optimistically(mode_id.clone());
        self.last_known_mode_id = Some(mode_id.clone());
        if let Some(h) = &self.handle {
            h.set_mode(mode_id);
        }
        cx.notify();
    }

    /// Change a select config option on the *user's* behalf (the config
    /// chips): show the pick immediately, ask the agent over the live handle,
    /// and remember a `Model`-category pick as this pane's own model. The
    /// reply replaces the whole option set via `ConfigOptionsChanged`.
    pub(in crate::workspace) fn set_config_option(
        &mut self,
        config_id: String,
        value: daruda_acp::ConfigValueView,
        cx: &mut Context<Self>,
    ) {
        // Only the model axis is remembered, and only when this call names it.
        let picked_model = match &value {
            daruda_acp::ConfigValueView::Id(picked) => {
                model_select(&self.session_config.config_options)
                    .filter(|(option_id, _, _)| *option_id == config_id.as_str())
                    .map(|_| picked.clone())
            }
            daruda_acp::ConfigValueView::Bool(_) => None,
        };
        if let Some(model) = picked_model {
            self.last_known_model_id = Some(model);
        }
        self.send_config_option(config_id, value, cx);
    }

    /// The protocol half both entry points share: show the value immediately,
    /// then ask the agent over the live handle (no-op when absent).
    fn send_config_option(
        &mut self,
        config_id: String,
        value: daruda_acp::ConfigValueView,
        cx: &mut Context<Self>,
    ) {
        self.session_config
            .set_option_value_optimistically(&config_id, &value);
        if let Some(h) = &self.handle {
            h.set_config_option(config_id, value);
        }
        cx.notify();
    }

    /// Shared teardown for a `/clear` reset and a post-`Error` retry: drop the
    /// live handle and event pump, wipe the conversation model + every runtime
    /// cache. Does NOT touch `session_id`/`restoring`/`status` — the two
    /// callers differ there (fresh conversation vs. resume via `session/load`).
    fn teardown_transient_session_state(&mut self) {
        cancel_pending_permission(self);
        self.handle = None;
        self._event_pump = None;
        self.items.clear();
        self.queue.pending_prompts.clear();
        self.queue.paused_prompts.clear();
        self.queue.editing_prompt = None;
        self.pending_permissions.clear();
        self.telegram_first_response_watch = None;
        self.queue.turn = Turn::Idle;
        self.activity.subagent_last_activity.clear();
        self.activity.activity_started_at = None;
        self.activity.was_busy = false;
        self.activity.pending_completion = None;
        self.activity.cancel_in_flight = false;
        self.session_usage = None;
        self.assets.clear();
        self.fold.clear_overrides();
        self.session_config = SessionConfig::default();
        self.plan.clear();
        self.plan_collapsed = false;
        self.session_title = None;
        self.session_updated_at = None;
    }

    /// Full local reset for `/clear`: teardown, then also drop the persisted
    /// session id. The caller supersedes this with a fresh `connect_agent_chat`
    /// right after, so the view never sits handle-less for a render.
    pub(in crate::workspace) fn reset_for_new_session(&mut self, cx: &mut Context<Self>) {
        self.teardown_transient_session_state();
        // Clear the persisted id so a restart resumes the fresh session, not
        // the cleared conversation (Connected re-persists the new id).
        self.session_id = None;
        self.restoring = false;
        // `teardown_transient_session_state` already cleared `items`, so this
        // resets the baseline to 0 — a stray post-turn update queued before
        // teardown can't relay stale text into the fresh session.
        self.snap_post_turn_baseline();
        self.status = AgentSessionStatus::Connecting;
        self.rebuild_rows(); // diff-splices list_state down to 0 rows
        cx.notify();
    }

    /// Reconnect after a terminal `Error` without losing the conversation:
    /// same teardown as [`Self::reset_for_new_session`] but keeps `session_id`,
    /// so the reconnect resumes via `session/load` and replays the history.
    pub(in crate::workspace) fn retry_for_reconnect(&mut self, cx: &mut Context<Self>) {
        self.teardown_transient_session_state();
        self.restoring = self.session_id.is_some();
        self.status = AgentSessionStatus::Connecting;
        self.rebuild_rows();
        cx.notify();
    }

    /// Jump the conversation list to the bottom and re-engage `Tail` follow —
    /// the list re-arms tail-following once it lands at the end.
    pub(in crate::workspace) fn scroll_to_bottom(&mut self, cx: &mut Context<Self>) {
        self.list_state.scroll_to_end();
        cx.notify();
    }
}
