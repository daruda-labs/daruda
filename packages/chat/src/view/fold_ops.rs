//! Fold toggles, the recent-steps tail windows and the fold mode / preset
//! controls of one Agent chat pane.

use gpui::{Context, Window};

use super::super::agent_chat_helpers::{fold_context, fold_key_item_index};
use super::super::fold::FoldKey;
use super::super::pane_choice::PaneChoice;
use super::super::reconcile::ReconcileScope;
use super::super::rows::tail::{StepWindow, TailLevel, TailWindow};
use super::super::rows::{collect_foldable_keys, item_row};
use super::super::window_access::WindowAccess;
use super::AgentChatView;
use super::list_sync::{CollapseAnchor, ListSync};
use daruda_transcript::fold_mode::{FoldMode, FoldPreset, TurnPosition};

impl AgentChatView {
    /// Collapse from a card footer and recover its owning transcript row.
    pub(crate) fn collapse_card(
        &mut self,
        key: FoldKey,
        anchor: CollapseAnchor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let context = fold_context(&key, &self.items, &self.live_units);
        if !self.fold.is_expanded(&key, context) {
            return;
        }
        let item = fold_key_item_index(&key, &self.items);
        let row_top = item
            .and_then(|ix| item_row(&self.rows, ix))
            .and_then(|row| self.list_state.bounds_for_item(row))
            .map(|bounds| bounds.origin.y);
        let offset = anchor.offset_to_reveal(row_top, self.list_state.viewport_bounds().origin.y);
        self.set_fold_expanded(key, false, window, cx);
        if let Some(row) = item.and_then(|ix| item_row(&self.rows, ix))
            && let Some(offset_in_item) = offset
            && self.list_state.logical_scroll_top().item_ix >= row
        {
            self.list_state.scroll_to(gpui::ListOffset {
                item_ix: row,
                offset_in_item,
            });
        }
    }

    /// Seed the context meter for a transcript without a live session.
    #[cfg(feature = "screenshot")]
    pub fn seed_usage_for_shot(&mut self, cx: &mut Context<Self>) {
        self.session_usage = Some(daruda_acp::UsageView {
            used: 53_000,
            size: 200_000,
            cost: None,
            rate_limit: None,
        });
        cx.notify();
    }

    /// Set a deterministic capture state independent of the configured defaults.
    #[cfg(feature = "screenshot")]
    pub fn set_fold_for_shot(
        &mut self,
        key: FoldKey,
        expanded: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_fold_expanded(key, expanded, window, cx);
    }

    /// Every prompt in the conversation has had its turn settle, so a long
    /// one may fold. Only at a settle edge or a restore, both of which
    /// remeasure every row after.
    pub(super) fn settle_prompts(&mut self) {
        if let Some(ix) = self
            .items
            .iter()
            .rposition(|item| matches!(item, daruda_acp::ChatItem::UserText(_)))
        {
            self.fold.settle_prompts_through(ix);
        }
    }

    pub(crate) fn toggle_fold(
        &mut self,
        key: FoldKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let context = fold_context(&key, &self.items, &self.live_units);
        let expanded = !self.fold.is_expanded(&key, context);
        self.set_fold_expanded(key, expanded, window, cx);
    }

    pub(crate) fn set_fold_expanded(
        &mut self,
        key: FoldKey,
        expanded: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Projection and user actions resolve the same effective defaults.
        let ctx = fold_context(&key, &self.items, &self.live_units);
        if !self.fold.set_expanded(key.clone(), expanded, ctx) {
            return;
        }
        // Nested folds change their owning row's height without flipping any
        // row's hidden flag. Name that row so remeasurement cannot miss it.
        let item_ix = fold_key_item_index(&key, &self.items);
        // A card's own fold decides whether its body — and so its embed editors —
        // is on screen at all. Narrow the follow-up reconcile to that card;
        // response and group folds only flip row visibility, leaving each card's
        // own state (and its embeds) untouched.
        let embed_scope = match &key {
            FoldKey::Compaction(ix) => match self.items.get(*ix) {
                Some(daruda_acp::ChatItem::Compaction(c)) => Some(ReconcileScope::Tool(c.output_key())),
                _ => None,
            },
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
            | FoldKey::ToolRawInput(_)
            | FoldKey::UserPrompt(_) => None,
        };
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
            self.resync_all_row_heights("fold-toggle-wide");
        } else if let Some(item_ix) = item_ix
            && let Some(row_ix) = item_row(&self.rows, item_ix)
        {
            self.apply_list_sync(ListSync::Rows(row_ix..row_ix + 1), "fold-toggle-row");
        }
        cx.notify();
    }

    /// Expand or collapse every currently-visible foldable block at once (the
    /// fold toolbar's expand-all / collapse-all).
    pub fn set_all_folds(&mut self, expanded: bool, window: &mut Window, cx: &mut Context<Self>) {
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
        self.resync_all_row_heights("fold-all");
        cx.notify();
    }

    /// Both levels' resolved windows, as the projection takes them.
    pub(super) fn step_windows(&self) -> StepWindow {
        StepWindow {
            steps: self.tail_steps.value(),
            calls: self.tail_calls.value(),
        }
    }

    /// One level's choice, by level — the tests assert per level, and a match
    /// duplicated there could disagree with the one below.
    #[cfg(test)]
    pub(super) fn tail_choice_for_test(&self, level: TailLevel) -> PaneChoice<TailWindow> {
        match level {
            TailLevel::Steps => self.tail_steps,
            TailLevel::Calls => self.tail_calls,
        }
    }

    fn tail_choice_mut(&mut self, level: TailLevel) -> &mut PaneChoice<TailWindow> {
        match level {
            TailLevel::Steps => &mut self.tail_steps,
            TailLevel::Calls => &mut self.tail_calls,
        }
    }

    /// Pin one level's window, independently of the other level.
    pub fn set_tail_window(&mut self, level: TailLevel, tail: TailWindow, cx: &mut Context<Self>) {
        // Choosing the seeded value still detaches the pane from config.
        let choice = PaneChoice::Chosen(tail);
        let slot = self.tail_choice_mut(level);
        let choice_changed = *slot != choice;
        *slot = choice;
        let reveal_changed = self.fold.clear_tail_reveals(level);
        if !choice_changed && !reveal_changed {
            return;
        }
        self.reproject(cx);
        // A cleared reveal is transient, so only a new choice is worth a save.
        if choice_changed {
            self.persist_pane_prefs(cx);
        }
    }

    pub fn set_fold_mode(&mut self, mode: FoldMode, window: &mut Window, cx: &mut Context<Self>) {
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

    pub(crate) fn reset_tail_window(&mut self, cx: &mut Context<Self>) {
        self.tail_steps = PaneChoice::Seeded(self.defaults.tail.steps);
        self.tail_calls = PaneChoice::Seeded(self.defaults.tail.calls);
        for level in TailLevel::ALL {
            self.fold.clear_tail_reveals(level);
        }
        self.reproject(cx);
        self.persist_pane_prefs(cx);
    }

    pub(crate) fn toggle_fold_editor_tools(&mut self, turn: TurnPosition, cx: &mut Context<Self>) {
        self.fold_editor.toggle_tools(turn);
        cx.notify();
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
    pub(crate) fn select_fold_preset(
        &mut self,
        preset: Option<FoldPreset>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(mode) = self.fold_editor.segment_target(preset, self.fold.mode()) else {
            return;
        };
        self.set_fold_mode(mode, window, cx);
    }

    /// Hand the fold axis back to config — see [`Self::reset_tail_window`].
    pub(crate) fn reset_fold_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
}
