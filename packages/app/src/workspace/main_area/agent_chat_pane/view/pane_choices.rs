//! The choices a user made on one pane, as one value: the view hands it
//! out to be saved and takes it back on restore, so the workspace owns
//! the file format without reaching into the view's fields.

use crate::transcript::display_filter::DisplayFilter;
use crate::transcript::fold_mode::FoldMode;

use super::super::pane_choice::PaneChoice;
use super::super::rows::tail::TailWindow;
use super::{AgentChatView, ChatContentWidth};

/// Each axis is `None` while the pane still follows config, so an
/// untouched pane keeps tracking a config change after a restart.
#[derive(Clone, Debug, Default, PartialEq)]
pub(in crate::workspace) struct ChatPaneChoices {
    pub(in crate::workspace) mode_id: Option<String>,
    pub(in crate::workspace) model_id: Option<String>,
    pub(in crate::workspace) content_width: Option<ChatContentWidth>,
    pub(in crate::workspace) tail_steps: Option<TailWindow>,
    pub(in crate::workspace) tail_calls: Option<TailWindow>,
    pub(in crate::workspace) display_filter: Option<DisplayFilter>,
    pub(in crate::workspace) fold_mode: Option<FoldMode>,
}

impl AgentChatView {
    pub(in crate::workspace) fn pane_choices(&self) -> ChatPaneChoices {
        ChatPaneChoices {
            mode_id: self.picked_mode_id.clone(),
            model_id: self.picked_model_id.clone(),
            content_width: self.content_width_chosen.then_some(self.content_width),
            tail_steps: self.tail_steps.chosen(),
            tail_calls: self.tail_calls.chosen(),
            display_filter: self.display_filter.chosen(),
            fold_mode: self.fold.chosen_mode(),
        }
    }

    /// Re-apply saved choices over the constructor's config seeds. The
    /// mode and model picks are taken as given — the lazy connect asks
    /// for them over the agent's defaults.
    pub(in crate::workspace) fn restore_pane_choices(&mut self, choices: ChatPaneChoices) {
        self.picked_mode_id = choices.mode_id;
        self.picked_model_id = choices.model_id;
        if let Some(width) = choices.content_width {
            self.content_width = width;
            self.content_width_chosen = true;
        }
        if let Some(tail) = choices.tail_steps {
            self.tail_steps = PaneChoice::Chosen(tail);
        }
        if let Some(tail) = choices.tail_calls {
            self.tail_calls = PaneChoice::Chosen(tail);
        }
        if let Some(filter) = choices.display_filter {
            self.display_filter = PaneChoice::Chosen(filter);
        }
        if let Some(mode) = choices.fold_mode {
            self.fold.set_mode(mode);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::make_test_view;
    use super::*;

    #[gpui::test]
    fn an_untouched_pane_saves_no_choice(cx: &mut gpui::TestAppContext) {
        let view = make_test_view(cx);
        view.read_with(cx, |v, _| {
            assert_eq!(v.pane_choices(), ChatPaneChoices::default());
        })
        .unwrap();
    }

    #[gpui::test]
    fn restored_choices_are_what_the_pane_saves_next(cx: &mut gpui::TestAppContext) {
        let view = make_test_view(cx);
        let choices = ChatPaneChoices {
            mode_id: Some("plan".into()),
            model_id: Some("opus".into()),
            content_width: Some(ChatContentWidth::Full),
            tail_steps: Some(TailWindow::Last(3)),
            tail_calls: Some(TailWindow::All),
            display_filter: Some(DisplayFilter::default()),
            fold_mode: Some(FoldMode::default()),
        };
        view.update(cx, |v, _, _| v.restore_pane_choices(choices.clone()))
            .unwrap();
        view.read_with(cx, |v, _| assert_eq!(v.pane_choices(), choices))
            .unwrap();
    }
}
