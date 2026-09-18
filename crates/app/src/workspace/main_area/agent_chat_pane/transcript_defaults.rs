//! The transcript-presentation defaults a chat pane starts on, as config
//! states them.

use daruda_config::{AgentDefinition, TAIL_WINDOW_DEFAULT};

use super::rows::tail::{StepWindow, TailWindow};
use super::view::ChatContentWidth;
use crate::transcript::display_filter::DisplayFilter;
use crate::transcript::fold_mode::FoldMode;

/// What a chat pane's presentation starts on, as the resolved config states it.
/// One type because these are always derived together and always applied
/// together — at pane creation and again on every live config reload — so a
/// further setting cannot be added to one path and forgotten in the other.
///
/// The three transcript axes are stated per-agent, because what reads well
/// depends on what the agent emits: one agent produces no reasoning at all,
/// another produces it constantly. An axis the entry does not state falls
/// straight to the built-in value — there is no layer between the two.
/// [`Self::content_width`] is the exception: it is app-wide, since how wide a
/// column reads is a property of the reader rather than of the agent.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::workspace) struct TranscriptDefaults {
    pub(super) tail: StepWindow,
    pub(super) fold_mode: FoldMode,
    pub(super) filter: DisplayFilter,
    pub(super) content_width: ChatContentWidth,
}

impl TranscriptDefaults {
    /// Resolve for the agent a pane runs under. `definition` is that agent's
    /// catalog entry, or `None` for an id no longer in the catalog — which
    /// resolves the same as an entry that states nothing of its own.
    /// `content_width` is app-wide, so it arrives already resolved — the
    /// Workspace holds the one mirror of `agent.use_reading_width`, the way it
    /// holds `syntax_theme`.
    pub(in crate::workspace) fn resolve(
        definition: Option<&AgentDefinition>,
        content_width: ChatContentWidth,
    ) -> Self {
        let fold_tokens: &[String] = definition
            .and_then(|d| d.fold_mode.as_deref())
            .unwrap_or(&[]);
        let tail = StepWindow {
            steps: TailWindow::last(
                definition
                    .and_then(|d| d.tail_window)
                    .unwrap_or(TAIL_WINDOW_DEFAULT),
            ),
            calls: TailWindow::last(
                definition
                    .and_then(|d| d.tail_window_calls)
                    .unwrap_or(TAIL_WINDOW_DEFAULT),
            ),
        };
        // An empty list is a real visible set (nothing on screen), so only the
        // absent key falls through to the unfiltered default.
        let filter = definition.and_then(|d| d.display_filter.as_ref());
        Self {
            tail,
            content_width,
            fold_mode: FoldMode::from_tokens(fold_tokens.iter().map(String::as_str)),
            filter: filter.map_or_else(DisplayFilter::default, |tokens| {
                DisplayFilter::from_stored(tokens)
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::display_filter::FilterFacet;
    use crate::transcript::fold_mode::FoldPreset;

    fn definition() -> AgentDefinition {
        AgentDefinition::claude_default()
    }

    /// The visible set with only thinking rows checked.
    fn thinking_only() -> DisplayFilter {
        DisplayFilter::from_tokens([FilterFacet::Thinking.token()])
    }

    /// No catalog entry at all and an entry that states nothing read alike:
    /// both mean "this agent has no opinion", which is the built-in value.
    #[test]
    fn an_entry_that_states_nothing_yields_the_built_in_defaults() {
        for entry in [None, Some(definition())] {
            let defaults = TranscriptDefaults::resolve(entry.as_ref(), ChatContentWidth::Reading);
            assert_eq!(defaults.tail, StepWindow::default());
            assert_eq!(defaults.fold_mode, FoldMode::default());
            assert_eq!(defaults.filter, DisplayFilter::default());
        }
    }

    #[test]
    fn every_axis_takes_the_value_the_entry_states() {
        let definition = AgentDefinition {
            tail_window: Some(3),
            tail_window_calls: Some(10),
            fold_mode: Some(vec!["expanded".to_string()]),
            display_filter: Some(vec![FilterFacet::Tools.token().to_string()]),
            ..definition()
        };
        let defaults = TranscriptDefaults::resolve(Some(&definition), ChatContentWidth::Reading);
        assert_eq!(
            defaults.tail,
            StepWindow {
                steps: TailWindow::Last(3),
                calls: TailWindow::Last(10),
            }
        );
        assert_eq!(defaults.fold_mode, FoldPreset::Expanded.mode());
        assert_eq!(
            defaults.filter,
            DisplayFilter::from_tokens([FilterFacet::Tools.token()])
        );
    }

    /// The axes are independent: stating one leaves the other two on the
    /// built-in value rather than dragging them along.
    #[test]
    fn one_axis_stated_leaves_the_others_built_in() {
        let definition = AgentDefinition {
            tail_window: Some(1),
            tail_window_calls: None,
            ..definition()
        };
        let defaults = TranscriptDefaults::resolve(Some(&definition), ChatContentWidth::Reading);
        assert_eq!(
            defaults.tail,
            StepWindow {
                steps: TailWindow::Last(1),
                // The axis's own two levels are independent too: an entry that
                // states the steps says nothing about the calls inside them.
                calls: TailWindow::All,
            },
            "the stated one"
        );
        assert_eq!(defaults.fold_mode, FoldMode::default(), "built-in");
        assert_eq!(defaults.filter, DisplayFilter::default(), "built-in");
    }

    /// Either level can be the only one stated.
    #[test]
    fn the_call_level_can_be_stated_alone() {
        let definition = AgentDefinition {
            tail_window: None,
            tail_window_calls: Some(5),
            ..definition()
        };
        let defaults = TranscriptDefaults::resolve(Some(&definition), ChatContentWidth::Reading);
        assert_eq!(
            defaults.tail,
            StepWindow {
                steps: TailWindow::All,
                calls: TailWindow::Last(5),
            }
        );
    }

    /// The filter's empty list is a value, not an absence: unchecking every box
    /// must not read back as "show everything".
    #[test]
    fn an_empty_filter_list_is_not_the_unfiltered_default() {
        let emptied = AgentDefinition {
            display_filter: Some(Vec::new()),
            ..definition()
        };
        let empty = TranscriptDefaults::resolve(Some(&emptied), ChatContentWidth::Reading).filter;
        let absent = TranscriptDefaults::resolve(None, ChatContentWidth::Reading).filter;
        assert_ne!(empty, absent);
        assert!(absent.shows_everything());
        assert!(
            FilterFacet::ALL
                .into_iter()
                .all(|facet| !empty.contains(facet)),
            "an empty list names an empty visible set"
        );
    }

    /// An empty `fold_mode` list resolves to the built-in matrix — unlike the
    /// filter, the fold reader starts from that matrix rather than from
    /// nothing, so the list has no "names an empty set" reading to protect.
    #[test]
    fn an_empty_fold_list_is_the_built_in_matrix() {
        let emptied = AgentDefinition {
            fold_mode: Some(Vec::new()),
            ..definition()
        };
        assert_eq!(
            TranscriptDefaults::resolve(Some(&emptied), ChatContentWidth::Reading).fold_mode,
            FoldMode::default()
        );
    }

    /// A stated value the pickers cannot name still resolves — the entry is
    /// the only layer, so a hand-written matrix has to survive the read.
    #[test]
    fn a_hand_written_fold_matrix_resolves() {
        let tuned = AgentDefinition {
            fold_mode: Some(vec![
                "summary".to_string(),
                "past.thinking=collapsed".to_string(),
            ]),
            ..definition()
        };
        let mode = TranscriptDefaults::resolve(Some(&tuned), ChatContentWidth::Reading).fold_mode;
        assert_eq!(mode.preset(), None, "a matrix, not a preset");
        assert_eq!(
            mode.rule(
                crate::transcript::fold_mode::TurnPosition::Past,
                crate::transcript::fold_mode::FoldBlock::Thinking,
            ),
            crate::transcript::fold_mode::BlockRule::Collapsed
        );
    }

    #[test]
    fn a_stated_filter_narrows_the_visible_set() {
        let narrowed = AgentDefinition {
            display_filter: Some(vec![FilterFacet::Thinking.token().to_string()]),
            ..definition()
        };
        assert_eq!(
            TranscriptDefaults::resolve(Some(&narrowed), ChatContentWidth::Reading).filter,
            thinking_only()
        );
    }
}
