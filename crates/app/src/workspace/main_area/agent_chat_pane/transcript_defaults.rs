//! The `[agent]` transcript-presentation defaults a chat pane starts on.

use daruda_config::AgentConfig;

use super::fold_mode::FoldMode;
use super::rows::tail::TailWindow;

/// Tail window and fold mode as the resolved config states them. One type
/// because the two are always derived together and always applied together —
/// at pane creation and again on every live config reload — so a third setting
/// cannot be added to one path and forgotten in the other.
///
/// The display filter is deliberately absent: it is a one-off investigative act
/// on one pane, not a durable taste, so it has no config key to follow.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::workspace) struct TranscriptDefaults {
    pub(in crate::workspace) tail: TailWindow,
    pub(in crate::workspace) fold_mode: FoldMode,
}

impl TranscriptDefaults {
    pub(in crate::workspace) fn from_config(agent: &AgentConfig) -> Self {
        Self {
            tail: TailWindow::last(agent.tail_window),
            fold_mode: FoldMode::from_tokens(agent.fold_mode.iter().map(String::as_str)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::main_area::agent_chat_pane::fold_mode::FoldPreset;

    #[test]
    fn an_untouched_config_yields_the_built_in_defaults() {
        let defaults = TranscriptDefaults::from_config(&AgentConfig::default());
        assert_eq!(defaults.tail, TailWindow::All);
        assert_eq!(defaults.fold_mode, FoldMode::default());
    }

    #[test]
    fn every_setting_is_read_from_the_same_agent_section() {
        let agent = AgentConfig {
            tail_window: 5,
            fold_mode: vec!["summary".to_string()],
            ..AgentConfig::default()
        };
        let defaults = TranscriptDefaults::from_config(&agent);
        assert_eq!(defaults.tail, TailWindow::Last(5));
        assert_eq!(defaults.fold_mode, FoldPreset::Summary.mode());
    }
}
