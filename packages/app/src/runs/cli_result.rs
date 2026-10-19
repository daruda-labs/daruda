//! The final textual answer in a bounded CLI transcript tail.

use std::path::Path;

const RESULT_TAIL_LINES: usize = 400;

pub(super) fn read(path: &Path) -> Option<String> {
    use daruda_agent::jsonl::parser::{MessageContent, SessionEntry};
    let lines = daruda_agent::jsonl::tail::read_last_n_lines(path, RESULT_TAIL_LINES).ok()?;
    let entries = lines
        .iter()
        .map(|line| serde_json::from_str::<SessionEntry>(line))
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    entries.into_iter().rev().find_map(|entry| {
        let SessionEntry::Assistant { base, message } = entry else {
            return None;
        };
        if base.is_sidechain == Some(true) {
            return None;
        }
        let text = message
            .content
            .into_iter()
            .filter_map(|content| match content {
                MessageContent::Text { text } => Some(text),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        daruda_core::text::bound_agent_text(&text)
    })
}

#[cfg(test)]
pub(super) mod tests;
