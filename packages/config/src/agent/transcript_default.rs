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
