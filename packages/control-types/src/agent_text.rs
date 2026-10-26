//! Bounding agent-authored text on its way out of the app.
//!
//! An agent writes its own session title and its own answers, and both travel
//! somewhere that did not author them — a phone message, another agent's tool
//! result. Every bound here is about that crossing: the text is unbounded and
//! untrusted at the source, so each consumer would otherwise have to defend
//! against it alone.
//!
//! One algorithm, two policies. A phone screen and an LLM context are not
//! bounded by the same thing, and a marker a person reads is localized while
//! one a model reads must not be — so [`elide_middle`] takes the budget and
//! the marker as arguments. The orchestrator's policy lives here beside it
//! (`bound_agent_text`, fixed English) because it has no other home; the
//! phone's lives at its call site in `telegram_ops`, which owns the localized
//! string.
//!
//! GPUI-free.

/// A session title is agent-authored and unbounded. Capped at the one place a
/// `ChatSummary` is built, so the bound is a property of the type rather than
/// of one adapter's renderer.
const TITLE_MAX_CHARS: usize = 80;

/// Leading/trailing characters [`bound_agent_text`] keeps. Larger than the
/// phone's budget because the consumer is an LLM context, not a screen, and
/// the point is to avoid a second call to read the part that was cut.
const ORCHESTRATOR_HEAD_CHARS: usize = 2000;
const ORCHESTRATOR_TAIL_CHARS: usize = 2000;

/// Fixed English, like every other value in a tool result: the consumer is a
/// model branching on content, not a person reading their own locale.
const TRUNCATION_MARKER: &str = "[...truncated by daruda...]";

/// Flatten an agent-authored title to one bounded line. Control characters
/// become spaces before the whitespace run is collapsed, so a multi-line title
/// cannot break the row it is rendered on, and no consumer has to defend
/// against one.
pub fn sanitize_title(raw: &str) -> Option<String> {
    let clean: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(TITLE_MAX_CHARS)
        .collect();
    (!clean.is_empty()).then_some(clean)
}

/// Keep `text` verbatim while it fits in `head + tail` characters; past that,
/// keep the two ends around `marker`.
///
/// The threshold is `head + tail` rather than its own parameter: a threshold
/// below the sum would elide a middle that was never there, and one above it
/// would drop characters the caller asked to keep. There is one value that is
/// not a bug, so it is derived instead of passed.
///
/// Counts `char`s, not bytes, so a multi-byte response never splits
/// mid-character. Line breaks are preserved — a caller wanting one line wants
/// [`sanitize_title`].
pub fn elide_middle(text: &str, head: usize, tail: usize, marker: &str) -> String {
    let Some((head_end, _)) = text.char_indices().nth(head) else {
        return text.to_string();
    };
    let tail_start = if tail == 0 {
        text.len()
    } else {
        let Some((start, _)) = text.char_indices().rev().nth(tail - 1) else {
            return text.to_string();
        };
        start
    };
    if head_end >= tail_start {
        return text.to_string();
    }
    let head = &text[..head_end];
    let tail = &text[tail_start..];
    format!("{head}\n{marker}\n{tail}")
}

/// Bound an agent's answer for a tool result. `None` for text that carries
/// nothing — a caller must not hand a model an empty field and let it guess
/// whether that means silence or absence.
///
/// Deliberately does *not* flatten: this is markdown prose, and a body whose
/// line breaks were collapsed would arrive as one unreadable paragraph.
pub fn bound_agent_text(raw: &str) -> Option<String> {
    if raw.trim().is_empty() {
        return None;
    }
    Some(elide_middle(
        raw,
        ORCHESTRATOR_HEAD_CHARS,
        ORCHESTRATOR_TAIL_CHARS,
        TRUNCATION_MARKER,
    ))
}
