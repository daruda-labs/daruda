//! Writing one transcript axis into the `[[agents]]` entry a chat pane runs
//! under — how a pane's own choice becomes its agent's default.

use super::entry::{AgentEntry, AgentSource};

/// One transcript axis as stored on an agent entry. `None` drops the key, which
/// is how a value equal to the built-in is written: an absent key and a key
/// stating the built-in resolve alike, and the shorter keeps the file clean.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptDefault {
    FoldMode(Option<Vec<String>>),
    DisplayFilter(Option<Vec<String>>),
    /// Both windows together: the range axis is one setting with two levels.
    TailWindows {
        steps: Option<u8>,
        calls: Option<u8>,
    },
}

impl AgentEntry {
    /// Store `value` on this entry — as an override on a preset reference, or
    /// on the definition a custom entry carries.
    pub fn set_transcript_default(&mut self, value: TranscriptDefault) {
        match &mut self.source {
            AgentSource::Preset { overrides, .. } => match value {
                TranscriptDefault::FoldMode(tokens) => overrides.fold_mode = tokens,
                TranscriptDefault::DisplayFilter(tokens) => overrides.display_filter = tokens,
                TranscriptDefault::TailWindows { steps, calls } => {
                    overrides.tail_window = steps;
                    overrides.tail_window_calls = calls;
                }
            },
            AgentSource::Custom(definition) => match value {
                TranscriptDefault::FoldMode(tokens) => definition.fold_mode = tokens,
                TranscriptDefault::DisplayFilter(tokens) => definition.display_filter = tokens,
                TranscriptDefault::TailWindows { steps, calls } => {
                    definition.tail_window = steps;
                    definition.tail_window_calls = calls;
                }
            },
        }
    }
}

/// The catalog with `value` written on the entry that launches `agent_id`, or
/// `None` when no switched-on entry resolves to that id — the one the pane runs
/// under, since a disabled or unlaunchable entry never starts a pane.
pub fn with_transcript_default(
    entries: &[AgentEntry],
    agent_id: &str,
    value: TranscriptDefault,
) -> Option<Vec<AgentEntry>> {
    let index = entries.iter().position(|entry| {
        entry.enabled
            && entry
                .resolve()
                .is_some_and(|definition| definition.id == agent_id)
    })?;
    let mut next = entries.to_vec();
    next[index].set_transcript_default(value);
    Some(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentDefinition, PresetOverrides};

    fn tokens(values: &[&str]) -> Option<Vec<String>> {
        Some(values.iter().map(|v| v.to_string()).collect())
    }

    fn claude_reference() -> AgentEntry {
        AgentEntry::preset_with(
            "claude-acp",
            PresetOverrides {
                id: Some("claude".into()),
                ..PresetOverrides::default()
            },
        )
    }

    #[test]
    fn a_preset_reference_takes_the_value_as_an_override() {
        let next = with_transcript_default(
            &[claude_reference()],
            "claude",
            TranscriptDefault::FoldMode(tokens(&["summary", "last.diff=expanded"])),
        )
        .expect("claude resolves");
        let definition = next[0].resolve().unwrap();
        assert_eq!(
            definition.fold_mode,
            tokens(&["summary", "last.diff=expanded"])
        );
        assert_eq!(next[0].preset_id(), Some("claude-acp"), "still a reference");
    }

    #[test]
    fn a_custom_entry_takes_the_value_on_its_definition() {
        let custom = AgentEntry::custom(AgentDefinition::claude_default());
        let next = with_transcript_default(
            &[custom],
            "claude",
            TranscriptDefault::TailWindows {
                steps: Some(5),
                calls: None,
            },
        )
        .unwrap();
        let definition = next[0].resolve().unwrap();
        assert_eq!(definition.tail_window, Some(5));
        assert_eq!(definition.tail_window_calls, None);
    }

    #[test]
    fn writing_none_drops_the_key_and_leaves_the_other_axes() {
        let mut entry = claude_reference();
        entry.set_transcript_default(TranscriptDefault::DisplayFilter(tokens(&["prose"])));
        entry.set_transcript_default(TranscriptDefault::TailWindows {
            steps: None,
            calls: Some(3),
        });
        entry.set_transcript_default(TranscriptDefault::DisplayFilter(None));
        let definition = entry.resolve().unwrap();
        assert_eq!(definition.display_filter, None);
        assert_eq!(definition.tail_window_calls, Some(3), "untouched axis kept");
    }

    #[test]
    fn only_the_switched_on_entry_for_the_id_is_written() {
        let off = claude_reference().disabled();
        let on = claude_reference();
        let codex = AgentEntry::preset("codex-acp");
        let next = with_transcript_default(
            &[off.clone(), on, codex.clone()],
            "claude",
            TranscriptDefault::DisplayFilter(tokens(&["tools"])),
        )
        .unwrap();
        assert_eq!(next[0], off, "the disabled entry never started the pane");
        assert_eq!(
            next[1].resolve().unwrap().display_filter,
            tokens(&["tools"])
        );
        assert_eq!(next[2], codex);
    }

    #[test]
    fn an_id_no_entry_launches_writes_nothing() {
        assert_eq!(
            with_transcript_default(
                &[claude_reference()],
                "gone",
                TranscriptDefault::FoldMode(None)
            ),
            None
        );
    }
}
