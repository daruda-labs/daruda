//! The transcript-presentation defaults a chat pane starts on, resolved over
//! the per-agent and app-wide config layers.

use daruda_config::{AgentConfig, AgentDefinition};

use super::display_filter::DisplayFilter;
use super::fold_mode::FoldMode;
use super::rows::tail::TailWindow;

/// Tail window, fold mode and display filter as the resolved config states
/// them. One type because the three are always derived together and always
/// applied together — at pane creation and again on every live config reload —
/// so a fourth setting cannot be added to one path and forgotten in the other.
///
/// Each axis resolves per-agent first and falls through to `[agent]`, because
/// what reads well depends on what the agent emits: one agent produces no
/// reasoning at all, another produces it constantly, and a single app-wide
/// value fits neither.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::workspace) struct TranscriptDefaults {
    pub(in crate::workspace) tail: TailWindow,
    pub(in crate::workspace) fold_mode: FoldMode,
    pub(in crate::workspace) filter: DisplayFilter,
}

impl TranscriptDefaults {
    /// Resolve for the agent a pane runs under. `definition` is that agent's
    /// catalog entry, or `None` for an id no longer in the catalog — which
    /// resolves the same as an entry that states no override of its own.
    pub(in crate::workspace) fn resolve(
        agent: &AgentConfig,
        definition: Option<&AgentDefinition>,
    ) -> Self {
        let fold_tokens = definition
            .and_then(|d| d.fold_mode.as_ref())
            .unwrap_or(&agent.fold_mode);
        let tail = definition
            .and_then(|d| d.tail_window)
            .unwrap_or(agent.tail_window);
        // An empty list is a real visible set (nothing on screen), so only the
        // absent key falls through to the unfiltered default.
        let filter = definition
            .and_then(|d| d.display_filter.as_ref())
            .or(agent.display_filter.as_ref());
        Self {
            tail: TailWindow::last(tail),
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
    use crate::workspace::main_area::agent_chat_pane::display_filter::FilterFacet;
    use crate::workspace::main_area::agent_chat_pane::fold_mode::FoldPreset;

    fn definition() -> AgentDefinition {
        AgentDefinition::claude_default()
    }

    /// The visible set with only thinking rows checked.
    fn thinking_only() -> DisplayFilter {
        DisplayFilter::from_tokens([FilterFacet::Thinking.token()])
    }

    #[test]
    fn an_untouched_config_yields_the_built_in_defaults() {
        let defaults = TranscriptDefaults::resolve(&AgentConfig::default(), None);
        assert_eq!(defaults.tail, TailWindow::All);
        assert_eq!(defaults.fold_mode, FoldMode::default());
        assert_eq!(defaults.filter, DisplayFilter::default());
    }

    #[test]
    fn every_axis_falls_through_to_the_agent_section() {
        let agent = AgentConfig {
            tail_window: 5,
            fold_mode: vec!["summary".to_string()],
            display_filter: Some(vec![FilterFacet::Thinking.token().to_string()]),
            ..AgentConfig::default()
        };
        // Once with no catalog entry at all, once with an entry that states
        // nothing: both mean "this agent has no opinion".
        for definition in [None, Some(definition())] {
            let defaults = TranscriptDefaults::resolve(&agent, definition.as_ref());
            assert_eq!(defaults.tail, TailWindow::Last(5));
            assert_eq!(defaults.fold_mode, FoldPreset::Summary.mode());
            assert_eq!(defaults.filter, thinking_only());
        }
    }

    #[test]
    fn every_axis_prefers_the_agents_own_value() {
        let agent = AgentConfig {
            tail_window: 5,
            fold_mode: vec!["summary".to_string()],
            display_filter: Some(vec![FilterFacet::Thinking.token().to_string()]),
            ..AgentConfig::default()
        };
        let definition = AgentDefinition {
            tail_window: Some(3),
            fold_mode: Some(vec!["expanded".to_string()]),
            display_filter: Some(vec![FilterFacet::Tools.token().to_string()]),
            ..definition()
        };
        let defaults = TranscriptDefaults::resolve(&agent, Some(&definition));
        assert_eq!(defaults.tail, TailWindow::Last(3));
        assert_eq!(defaults.fold_mode, FoldPreset::Expanded.mode());
        assert_eq!(
            defaults.filter,
            DisplayFilter::from_tokens([FilterFacet::Tools.token()])
        );
    }

    #[test]
    fn one_axis_set_per_agent_leaves_the_others_following() {
        let agent = AgentConfig {
            tail_window: 5,
            fold_mode: vec!["summary".to_string()],
            display_filter: Some(vec![FilterFacet::Thinking.token().to_string()]),
            ..AgentConfig::default()
        };
        let definition = AgentDefinition {
            tail_window: Some(1),
            ..definition()
        };
        let defaults = TranscriptDefaults::resolve(&agent, Some(&definition));
        assert_eq!(defaults.tail, TailWindow::Last(1), "the agent's own");
        assert_eq!(defaults.fold_mode, FoldPreset::Summary.mode(), "app-wide");
        assert_eq!(defaults.filter, thinking_only(), "app-wide");
    }

    /// A config written before the per-agent keys existed: `[agent]` alone
    /// still decides, and the axis that never had a key stays unfiltered.
    #[test]
    fn a_pre_per_agent_config_resolves_exactly_as_before() {
        let toml_str = "[agent]\ntail_window = 3\nfold_mode = [\"summary\"]\n";
        let config: daruda_config::Config = toml::from_str(toml_str).expect("deserialize");
        let definitions = config.resolved_agents();
        let defaults = TranscriptDefaults::resolve(&config.agent, definitions.first());
        assert_eq!(defaults.tail, TailWindow::Last(3));
        assert_eq!(defaults.fold_mode, FoldPreset::Summary.mode());
        assert_eq!(defaults.filter, DisplayFilter::default());
    }

    /// The filter's empty list is a value, not an absence: unchecking every box
    /// must not read back as "show everything".
    #[test]
    fn an_empty_filter_list_is_not_the_unfiltered_default() {
        let agent = AgentConfig {
            display_filter: Some(Vec::new()),
            ..AgentConfig::default()
        };
        let empty = TranscriptDefaults::resolve(&agent, None).filter;
        let absent = TranscriptDefaults::resolve(&AgentConfig::default(), None).filter;
        assert_ne!(empty, absent);
        assert!(absent.shows_everything());
        assert!(
            FilterFacet::ALL
                .into_iter()
                .all(|facet| !empty.contains(facet)),
            "an empty list names an empty visible set"
        );

        // And the same distinction on the per-agent layer, over an app-wide
        // value it has to be able to override *down* to nothing.
        let agent = AgentConfig {
            display_filter: Some(vec![FilterFacet::Thinking.token().to_string()]),
            ..AgentConfig::default()
        };
        let definition = AgentDefinition {
            display_filter: Some(Vec::new()),
            ..definition()
        };
        assert_eq!(
            TranscriptDefaults::resolve(&agent, Some(&definition)).filter,
            empty
        );
        let following = AgentDefinition {
            display_filter: None,
            ..definition
        };
        assert_eq!(
            TranscriptDefaults::resolve(&agent, Some(&following)).filter,
            thinking_only()
        );
    }
}
