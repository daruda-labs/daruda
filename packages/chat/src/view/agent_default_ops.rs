//! Saving a pane's own transcript choice as its agent's default.
//!
//! The host's [`AgentDefaultWriter`] lands it in `[[agents]]`, whose observers
//! reseed every pane still following that agent. The pane that saved stops
//! being an override: it keeps what is on screen, now as config's value.

use std::rc::Rc;

use daruda_config::{TAIL_WINDOW_DEFAULT, TranscriptDefault};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::{App, Context};

use super::super::pane_choice::PaneChoice;
use super::super::rows::tail::StepWindow;
use super::AgentChatView;
use daruda_transcript::display_filter::DisplayFilter;
use daruda_transcript::fold_mode::FoldMode;

/// Writes `value` onto the catalog entry `agent_id` launches from. `Err`
/// says why it did not land, for the pane to report.
pub type AgentDefaultWriter = Rc<dyn Fn(&str, TranscriptDefault, &mut App) -> Result<(), String>>;

impl AgentChatView {
    /// Write this pane's fold matrix as its agent's default.
    pub(crate) fn save_fold_mode_as_agent_default(&mut self, cx: &mut Context<Self>) {
        let mode = self.fold.mode();
        if !self.write_agent_default(TranscriptDefault::FoldMode(fold_tokens(mode)), cx) {
            return;
        }
        self.defaults.fold_mode = mode;
        self.fold.follow_current_mode();
        self.after_agent_default_saved(cx);
    }

    /// Write this pane's visible set as its agent's default.
    pub(crate) fn save_display_filter_as_agent_default(&mut self, cx: &mut Context<Self>) {
        let filter = self.display_filter.value();
        if !self.write_agent_default(TranscriptDefault::DisplayFilter(filter_tokens(filter)), cx) {
            return;
        }
        self.defaults.filter = filter;
        self.display_filter = PaneChoice::Seeded(filter);
        self.after_agent_default_saved(cx);
    }

    /// Write both of this pane's activity windows as its agent's default.
    pub(crate) fn save_tail_window_as_agent_default(&mut self, cx: &mut Context<Self>) {
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
        let writer = self.agent_default_writer.clone();
        match writer(&self.agent_id, value, cx) {
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
        cx.emit(super::AgentChatEvent::ReportError(report));
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
    use daruda_transcript::display_filter::FilterFacet;
    use daruda_transcript::fold_mode::FoldPreset;

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

    use std::cell::RefCell;

    use super::super::super::rows::tail::{TailLevel, TailWindow};
    use super::super::tests::make_test_view_with_writer;

    type Written = Rc<RefCell<Vec<(String, TranscriptDefault)>>>;

    /// A view whose writer records each write, and refuses an agent with no
    /// entry the way the settings store does.
    fn open(cx: &mut gpui::TestAppContext) -> (gpui::WindowHandle<AgentChatView>, Written) {
        let written = Written::default();
        let log = written.clone();
        let writer: super::AgentDefaultWriter = Rc::new(move |agent_id, value, _| {
            if agent_id == "gone" {
                return Err(format!("no enabled agent entry has id `{agent_id}`"));
            }
            log.borrow_mut().push((agent_id.to_owned(), value));
            Ok(())
        });
        (make_test_view_with_writer(cx, writer), written)
    }

    /// A pane's own fold matrix is written as its agent's `fold_mode`, and the
    /// pane goes back to following it with nothing on screen moving.
    #[gpui::test]
    fn saving_the_fold_mode_writes_the_agent_entry_and_follows_it(cx: &mut gpui::TestAppContext) {
        let (window, written) = open(cx);
        let mode = FoldPreset::Expanded.mode();
        window
            .update(cx, |view, window, cx| {
                view.set_fold_mode(mode, window, cx);
                assert!(!view.fold.mode_choice().is_following());
                view.save_fold_mode_as_agent_default(cx);
                assert_eq!(view.fold.mode_choice(), PaneChoice::Seeded(mode));
                assert_eq!(view.defaults.fold_mode, mode);
            })
            .unwrap();
        assert_eq!(
            *written.borrow(),
            [(
                "claude".to_owned(),
                TranscriptDefault::FoldMode(Some(mode.tokens()))
            )]
        );
    }

    #[gpui::test]
    fn saving_the_filter_writes_the_visible_set(cx: &mut gpui::TestAppContext) {
        let (window, written) = open(cx);
        let filter = window
            .update(cx, |view, _, cx| {
                view.toggle_display_facet(FilterFacet::Thinking, cx);
                let filter = view.display_filter.value();
                view.save_display_filter_as_agent_default(cx);
                assert_eq!(view.display_filter, PaneChoice::Seeded(filter));
                filter
            })
            .unwrap();
        let [(_, TranscriptDefault::DisplayFilter(Some(stored)))] = &written.borrow()[..] else {
            panic!("one cut set is written: {:?}", written.borrow());
        };
        assert_eq!(DisplayFilter::from_stored(stored), filter);
    }

    /// Both windows are written together, and a level on the built-in size
    /// writes no key.
    #[gpui::test]
    fn saving_the_range_writes_both_windows(cx: &mut gpui::TestAppContext) {
        let (window, written) = open(cx);
        let steps = window
            .update(cx, |view, _, cx| {
                view.set_tail_window(TailLevel::Calls, TailWindow::Last(3), cx);
                view.save_tail_window_as_agent_default(cx);
                assert!(view.tail_steps.is_following() && view.tail_calls.is_following());
                assert_eq!(view.tail_calls.value(), TailWindow::Last(3));
                view.tail_steps.value()
            })
            .unwrap();
        assert_eq!(
            written.borrow()[0].1,
            TranscriptDefault::TailWindows {
                steps: window_size(steps.size()),
                calls: Some(3),
            }
        );
    }

    /// When the write is refused, the pane keeps its override rather than
    /// claiming a default config does not hold.
    #[gpui::test]
    fn a_pane_whose_agent_is_gone_keeps_its_override(cx: &mut gpui::TestAppContext) {
        let (window, written) = open(cx);
        window
            .update(cx, |view, window, cx| {
                view.agent_id = "gone".into();
                view.set_fold_mode(FoldPreset::Expanded.mode(), window, cx);
                view.save_fold_mode_as_agent_default(cx);
                assert!(!view.fold.mode_choice().is_following());
                assert_eq!(view.defaults.fold_mode, FoldMode::default());
            })
            .unwrap();
        assert!(written.borrow().is_empty());
    }
}
