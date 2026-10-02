//! Saving a pane's own transcript choice as its agent's default.
//!
//! The write lands in `[[agents]]` through the settings store, whose observers
//! reseed every pane still following that agent. The pane that saved stops
//! being an override: it keeps what is on screen, now as config's value.

use daruda_config::{SettingsPatch, TAIL_WINDOW_DEFAULT, TranscriptDefault};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::{BorrowAppContext as _, Context};

use super::super::pane_choice::PaneChoice;
use super::super::rows::tail::StepWindow;
use super::AgentChatView;
use crate::settings_store::SettingsStore;
use crate::transcript::display_filter::DisplayFilter;
use crate::transcript::fold_mode::FoldMode;

impl AgentChatView {
    /// Write this pane's fold matrix as its agent's default.
    pub(in crate::workspace) fn save_fold_mode_as_agent_default(&mut self, cx: &mut Context<Self>) {
        let mode = self.fold.mode();
        if !self.write_agent_default(TranscriptDefault::FoldMode(fold_tokens(mode)), cx) {
            return;
        }
        self.defaults.fold_mode = mode;
        self.fold.follow_current_mode();
        self.after_agent_default_saved(cx);
    }

    /// Write this pane's visible set as its agent's default.
    pub(in crate::workspace) fn save_display_filter_as_agent_default(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        let filter = self.display_filter.value();
        if !self.write_agent_default(TranscriptDefault::DisplayFilter(filter_tokens(filter)), cx) {
            return;
        }
        self.defaults.filter = filter;
        self.display_filter = PaneChoice::Seeded(filter);
        self.after_agent_default_saved(cx);
    }

    /// Write both of this pane's activity windows as its agent's default.
    pub(in crate::workspace) fn save_tail_window_as_agent_default(
        &mut self,
        cx: &mut Context<Self>,
    ) {
        let tail = StepWindow {
            steps: self.tail_steps.value(),
            calls: self.tail_calls.value(),
        };
        let value = TranscriptDefault::TailWindows {
            steps: window_size(tail.steps.size()),
            calls: window_size(tail.calls.size()),
        };
        if !self.write_agent_default(value, cx) {
            return;
        }
        self.defaults.tail = tail;
        self.tail_steps = PaneChoice::Seeded(tail.steps);
        self.tail_calls = PaneChoice::Seeded(tail.calls);
        self.after_agent_default_saved(cx);
    }

    /// The value on screen is unchanged, so nothing reprojects: only the
    /// follow state moved, which the bar's pin and the panel footer show.
    fn after_agent_default_saved(&mut self, cx: &mut Context<Self>) {
        cx.notify();
        self.persist_pane_prefs(cx);
    }

    /// Persist `value` on the entry this pane's agent launches from. `false`
    /// leaves the pane as it was — still an override — and says why.
    fn write_agent_default(&self, value: TranscriptDefault, cx: &mut Context<Self>) -> bool {
        let catalog = daruda_config::with_transcript_default(
            &SettingsStore::global(cx).user().agents,
            &self.agent_id,
            value,
        );
        let result = match catalog {
            Some(agents) => cx.update_global::<SettingsStore, _>(|store, _| {
                store.apply_patch(SettingsPatch::AgentCatalog(agents))
            }),
            None => Err(format!("no enabled agent entry has id `{}`", self.agent_id)),
        };
        match result {
            Ok(()) => true,
            Err(detail) => {
                self.report_agent_default_failure(detail, cx);
                false
            }
        }
    }

    fn report_agent_default_failure(&self, detail: String, cx: &mut Context<Self>) {
        let report = ErrorReport::new("Saving the agent default failed")
            .severity(ErrorSeverity::Error)
            .message(detail)
            .with_context("agent", self.agent_id.clone())
            .at(file!(), line!())
            .dedup("agent_chat.save_agent_default")
            .build();
        let window_handle = self.window_handle;
        // Deferred: the toast belongs to the Workspace, which may be the one
        // updating this view right now.
        cx.defer(move |cx| {
            let Some(workspace) =
                crate::window_registry::WindowRegistry::workspace_for_window(window_handle, cx)
            else {
                daruda_store::observability::log_writer::LogWriter::log(report);
                return;
            };
            let fallback = report.clone();
            if workspace
                .update(cx, |ws, cx| ws.report_error(report, cx))
                .is_err()
            {
                daruda_store::observability::log_writer::LogWriter::log(fallback);
            }
        });
    }
}

/// The stored form of a fold matrix: nothing for the built-in.
fn fold_tokens(mode: FoldMode) -> Option<Vec<String>> {
    (mode != FoldMode::default()).then(|| mode.tokens())
}

/// The stored form of a visible set: nothing for the unfiltered default.
fn filter_tokens(filter: DisplayFilter) -> Option<Vec<String>> {
    (filter != DisplayFilter::default())
        .then(|| filter.tokens().into_iter().map(str::to_owned).collect())
}

/// The stored form of one window: nothing for the built-in size.
fn window_size(size: u8) -> Option<u8> {
    (size != TAIL_WINDOW_DEFAULT).then_some(size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::display_filter::FilterFacet;
    use crate::transcript::fold_mode::FoldPreset;

    /// The built-in on every axis writes no key, the same rule Settings keeps.
    #[test]
    fn the_built_in_value_stores_nothing() {
        assert_eq!(fold_tokens(FoldMode::default()), None);
        assert_eq!(filter_tokens(DisplayFilter::default()), None);
        assert_eq!(window_size(TAIL_WINDOW_DEFAULT), None);
    }

    /// Anything else stores tokens the axis reads back unchanged.
    #[test]
    fn a_chosen_value_round_trips_through_its_tokens() {
        let mode = FoldPreset::Expanded.mode().with_collapse_history(true);
        let stored = fold_tokens(mode).expect("not the built-in");
        assert_eq!(
            FoldMode::from_tokens(stored.iter().map(String::as_str)),
            mode
        );

        let filter = DisplayFilter::default().toggled(FilterFacet::Thinking);
        let stored = filter_tokens(filter).expect("not the built-in");
        assert_eq!(DisplayFilter::from_stored(&stored), filter);

        let other = TAIL_WINDOW_DEFAULT.wrapping_add(2);
        assert_eq!(window_size(other), Some(other));
    }

    use super::super::super::rows::tail::{TailLevel, TailWindow};
    use super::super::tests::make_test_view;

    fn claude(cx: &gpui::App) -> daruda_config::AgentDefinition {
        SettingsStore::global(cx)
            .user()
            .resolved_agents()
            .into_iter()
            .find(|agent| agent.id == "claude")
            .expect("the built-in catalog launches claude")
    }

    fn open(cx: &mut gpui::TestAppContext) -> gpui::WindowHandle<AgentChatView> {
        cx.update(SettingsStore::init);
        make_test_view(cx)
    }

    /// A pane's own fold matrix becomes its agent's `fold_mode`, and the pane
    /// goes back to following it with nothing on screen moving.
    #[gpui::test]
    fn saving_the_fold_mode_writes_the_agent_entry_and_follows_it(cx: &mut gpui::TestAppContext) {
        let window = open(cx);
        let mode = FoldPreset::Expanded.mode();
        window
            .update(cx, |view, window, cx| {
                view.set_fold_mode(mode, window, cx);
                assert!(!view.fold.mode_choice().is_following());
                view.save_fold_mode_as_agent_default(cx);
                assert_eq!(view.fold.mode_choice(), PaneChoice::Seeded(mode));
                assert_eq!(view.defaults.fold_mode, mode);
                assert_eq!(claude(cx).fold_mode, Some(mode.tokens()));
            })
            .unwrap();
    }

    #[gpui::test]
    fn saving_the_filter_writes_the_visible_set(cx: &mut gpui::TestAppContext) {
        let window = open(cx);
        window
            .update(cx, |view, _, cx| {
                view.toggle_display_facet(FilterFacet::Thinking, cx);
                let filter = view.display_filter.value();
                view.save_display_filter_as_agent_default(cx);
                assert_eq!(view.display_filter, PaneChoice::Seeded(filter));
                let stored = claude(cx).display_filter.expect("a cut set is stated");
                assert_eq!(DisplayFilter::from_stored(&stored), filter);
            })
            .unwrap();
    }

    /// Both windows are written together, and a level on the built-in size
    /// writes no key.
    #[gpui::test]
    fn saving_the_range_writes_both_windows(cx: &mut gpui::TestAppContext) {
        let window = open(cx);
        window
            .update(cx, |view, _, cx| {
                view.set_tail_window(TailLevel::Calls, TailWindow::Last(3), cx);
                view.save_tail_window_as_agent_default(cx);
                assert!(view.tail_steps.is_following() && view.tail_calls.is_following());
                assert_eq!(view.tail_calls.value(), TailWindow::Last(3));
                let agent = claude(cx);
                assert_eq!(agent.tail_window_calls, Some(3));
                assert_eq!(
                    agent.tail_window,
                    window_size(view.tail_steps.value().size())
                );
            })
            .unwrap();
    }

    /// With no entry to write, the pane keeps its override rather than
    /// claiming a default config does not hold.
    #[gpui::test]
    fn a_pane_whose_agent_is_gone_keeps_its_override(cx: &mut gpui::TestAppContext) {
        let window = open(cx);
        window
            .update(cx, |view, window, cx| {
                view.agent_id = "gone".into();
                view.set_fold_mode(FoldPreset::Expanded.mode(), window, cx);
                view.save_fold_mode_as_agent_default(cx);
                assert!(!view.fold.mode_choice().is_following());
                assert_eq!(view.defaults.fold_mode, FoldMode::default());
            })
            .unwrap();
    }
}
