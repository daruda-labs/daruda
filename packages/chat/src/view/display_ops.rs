//! Transcript presentation: reprojection, content width, config reseeding,
//! the display filter, activity-bar options and the plan region.

use gpui::Context;

use super::super::pane_choice::PaneChoice;
use super::super::reconcile::ReconcileScope;
use super::super::transcript_defaults::TranscriptDefaults;
use super::super::window_access::WindowAccess;
use super::list_sync::ListSync;
use super::{ActivityOptionsTab, AgentChatView};
use daruda_transcript::display_filter::{DisplayFilter, FilterFacet};

impl AgentChatView {
    /// Test-only hook: append items and reproject, so a test outside this module
    /// can stand a transcript up without a session. Mirrors what an `AcpEvent`
    /// arrival ends in.
    #[cfg(any(test, feature = "test-support"))]
    pub fn seed_items_for_test(
        &mut self,
        items: impl IntoIterator<Item = daruda_acp::ChatItem>,
        cx: &mut Context<Self>,
    ) {
        self.items.extend(items);
        self.reproject(cx);
    }

    /// Re-derive the row projection and reflow the list. The one sequence every
    /// transcript-preference change ends in, since all of them feed `project`.
    pub(super) fn reproject(&mut self, cx: &mut Context<Self>) {
        self.rebuild_rows();
        self.resync_all_row_heights("reproject");
        cx.notify();
    }

    /// Get this pane's transcript preferences onto disk as they now stand. The
    /// snapshot writes each axis from its `chosen()`, so this is equally the
    /// way a reset *erases* a stored override. The host saves on the event,
    /// after the chip handler that called this has released the workspace.
    pub(super) fn persist_pane_prefs(&self, cx: &mut Context<Self>) {
        cx.emit(super::AgentChatEvent::PrefsChanged);
    }

    /// Toggle this pane between pane-wide wrapping and the configured reading
    /// column. Width changes can reflow every row, so invalidate all list
    /// measurements and persist the pane-local preference.
    pub(crate) fn toggle_content_width(&mut self, cx: &mut Context<Self>) {
        self.content_width = self.content_width.toggle();
        // Using the toggle is what takes this pane off `use_reading_width` —
        // including a toggle that lands back on the configured value, which is
        // still a statement about this pane.
        self.content_width_chosen = true;
        self.apply_list_sync(ListSync::EveryRow, "width-toggle");
        cx.notify();
        self.persist_pane_prefs(cx);
    }

    /// Follow a reloaded config: every presentation preference the user has not
    /// picked for this pane moves to the new default, and the ones they did
    /// pick stay. The single site that applies them all, so a pane open across
    /// a config edit ends up where a freshly restored one would. Nothing is
    /// persisted — a seed is not a choice.
    pub fn reseed_transcript_defaults(
        &mut self,
        defaults: &TranscriptDefaults,
        cx: &mut Context<Self>,
    ) {
        // Remembered so a later reset can hand an axis back to *this* default
        // without the view resolving config on its own. The summary style has
        // no pane override, so render reads it straight from here.
        let summary_changed = self.defaults.tool_summary != defaults.tool_summary;
        self.defaults = *defaults;
        let before = (
            self.tail_steps,
            self.tail_calls,
            self.fold.mode(),
            self.display_filter,
            self.content_width,
        );
        self.tail_steps.reseed(defaults.tail.steps);
        self.tail_calls.reseed(defaults.tail.calls);
        self.fold.reseed_mode(defaults.fold_mode);
        self.display_filter.reseed(defaults.filter);
        if !self.content_width_chosen {
            self.content_width = defaults.content_width;
        }
        if before
            == (
                self.tail_steps,
                self.tail_calls,
                self.fold.mode(),
                self.display_filter,
                self.content_width,
            )
        {
            if summary_changed {
                cx.notify();
            }
            return;
        }
        // A width change reflows every row, so the cached heights are stale —
        // the same invalidation the pane's own toggle does.
        if self.content_width != before.4 {
            self.apply_list_sync(ListSync::EveryRow, "width-pref");
        }
        // A reseeded fold matrix moves every card's derived default, so the cards
        // it just opened owe the same embed pass a fold click does — see
        // [`Self::reconcile_embeds_after_fold`]. By handle, not a live borrow:
        // this arrives from `apply_config`'s global observer, which fires from
        // `flush_effects` with the window already back in `App::windows`.
        if self.fold.mode() != before.2 {
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

    /// Record the pane width measured during the last paint.
    pub(crate) fn set_pane_width(&mut self, width: gpui::Pixels, cx: &mut Context<Self>) {
        let was_compact = self.activity_bar_is_compact(cx);
        self.pane_width = Some(width);
        if self.activity_bar_is_compact(cx) != was_compact {
            cx.notify();
        }
    }

    /// Whether transcript controls should collapse into one popover. The
    /// threshold is font-dependent (the chips it has to fit are text), so it
    /// comes from `theme` rather than a bare constant.
    pub(crate) fn activity_bar_is_compact(&self, cx: &gpui::App) -> bool {
        self.pane_width
            .is_some_and(|w| f32::from(w) <= daruda_ui::theme::agent_chat_compact_options_w(cx))
    }

    /// Open or shut one Visible-items section's child rows. Presentation only:
    /// the filter is not touched.
    pub(crate) fn toggle_filter_editor_section(
        &mut self,
        axis: daruda_transcript::display_filter::FilterAxis,
        cx: &mut Context<Self>,
    ) {
        self.filter_editor.toggle(axis);
        cx.notify();
    }

    pub(crate) fn toggle_fold_editor_history_rules(&mut self, cx: &mut Context<Self>) {
        self.fold_editor.toggle_history_rules();
        cx.notify();
    }

    pub(crate) fn set_activity_options_tab(
        &mut self,
        tab: ActivityOptionsTab,
        cx: &mut Context<Self>,
    ) {
        if self.activity_options_tab != tab {
            self.activity_options_tab = tab;
            cx.notify();
        }
    }

    pub fn toggle_display_facet(&mut self, facet: FilterFacet, cx: &mut Context<Self>) {
        self.set_display_filter(self.display_filter.value().toggled(facet), cx);
    }

    /// Hand the filter axis back to config — see [`Self::reset_tail_window`].
    pub(crate) fn reset_display_filter(&mut self, cx: &mut Context<Self>) {
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
    pub(crate) fn set_filter_section(
        &mut self,
        parent: FilterFacet,
        selected: bool,
        cx: &mut Context<Self>,
    ) {
        let next = self.display_filter.value().with_section(parent, selected);
        self.set_display_filter(next, cx);
    }

    /// Pin the filter to `filter` whatever the pane was restored with, so a
    /// capture shows the same cut on every machine.
    #[cfg(feature = "screenshot")]
    pub fn set_display_filter_for_shot(&mut self, filter: DisplayFilter, cx: &mut Context<Self>) {
        self.set_display_filter(filter, cx);
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
    pub(crate) fn toggle_plan_collapsed(&mut self, cx: &mut Context<Self>) {
        self.plan_collapsed = !self.plan_collapsed;
        cx.notify();
    }

    /// Dismiss the plan region: drop the entries so it renders nothing. Pure
    /// local presentation reset — a later `PlanChanged` repopulates it
    /// (expanded, since `plan_collapsed` resets too).
    pub(crate) fn dismiss_plan(&mut self, cx: &mut Context<Self>) {
        self.plan.clear();
        self.plan_collapsed = false;
        cx.notify();
    }
}
