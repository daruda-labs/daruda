//! The final textual answer at the end of a CLI transcript.

use std::path::Path;

/// The answer is normally the last line or close to it; the budget only has
/// to fit that line plus a few trailing entries. It bounds the settle-time
/// read when no answer exists in a transcript that has grown to gigabytes.
const RESULT_TAIL_BYTES: u64 = 8 * 1024 * 1024;

pub(super) fn read(path: &Path) -> Option<String> {
    let text = daruda_agent::jsonl::answer::last_assistant_text(path, RESULT_TAIL_BYTES).ok()??;
    daruda_control_types::agent_text::bound_agent_text(&text)
}

#[cfg(test)]
pub(super) mod tests;
