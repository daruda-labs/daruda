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
pub(crate) fn sanitize_title(raw: &str) -> Option<String> {
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
pub(crate) fn elide_middle(text: &str, head: usize, tail: usize, marker: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= head + tail {
        return text.to_string();
    }
    let head: String = chars[..head].iter().collect();
    let tail: String = chars[chars.len() - tail..].iter().collect();
    format!("{head}\n{marker}\n{tail}")
}

/// Bound an agent's answer for a tool result. `None` for text that carries
/// nothing — a caller must not hand a model an empty field and let it guess
/// whether that means silence or absence.
///
/// Deliberately does *not* flatten: this is markdown prose, and a body whose
/// line breaks were collapsed would arrive as one unreadable paragraph.
pub(crate) fn bound_agent_text(raw: &str) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elide_middle_keeps_short_text_verbatim() {
        assert_eq!(elide_middle("abc", 2, 2, "…"), "abc");
        // Exactly at the budget is still verbatim: there is no middle to cut.
        assert_eq!(elide_middle("abcd", 2, 2, "…"), "abcd");
    }

    #[test]
    fn elide_middle_keeps_both_ends_around_the_marker() {
        assert_eq!(elide_middle("abcdef", 2, 2, "CUT"), "ab\nCUT\nef");
    }

    /// A byte-indexed implementation panics or splits a character here.
    #[test]
    fn elide_middle_counts_chars_not_bytes() {
        let text = "가나다라마바";
        let out = elide_middle(text, 2, 2, "CUT");
        assert_eq!(out, "가나\nCUT\n마바");
        // Every retained char survived whole.
        assert!(out.chars().all(|c| c == '\n' || "가나마바CUT".contains(c)));
    }

    #[test]
    fn a_bounded_body_carries_its_marker_in_band() {
        let long = "x".repeat(ORCHESTRATOR_HEAD_CHARS + ORCHESTRATOR_TAIL_CHARS + 1);
        let out = bound_agent_text(&long).expect("non-empty");
        assert!(
            out.contains(TRUNCATION_MARKER),
            "truncation must be self-describing, not a sibling bool: {out:.80}"
        );
        assert_eq!(
            out.chars().filter(|c| *c == 'x').count(),
            ORCHESTRATOR_HEAD_CHARS + ORCHESTRATOR_TAIL_CHARS
        );
    }

    #[test]
    fn a_body_under_the_budget_is_untouched() {
        let body = "line one\n\nline two";
        assert_eq!(bound_agent_text(body).as_deref(), Some(body));
    }

    /// The difference from [`sanitize_title`]: prose keeps its shape.
    #[test]
    fn a_bounded_body_keeps_its_line_breaks() {
        let body = "## heading\n\n- one\n- two";
        let out = bound_agent_text(body).expect("non-empty");
        assert_eq!(out.matches('\n').count(), body.matches('\n').count());
    }

    #[test]
    fn a_blank_body_is_absent() {
        assert_eq!(bound_agent_text(""), None);
        assert_eq!(bound_agent_text("   \n\t  "), None);
    }

    #[test]
    fn a_title_is_capped_and_stripped_of_control_characters() {
        let raw = format!("line one\nline two\t{}", "x".repeat(200));
        let clean = sanitize_title(&raw).expect("non-empty");
        assert_eq!(clean.chars().count(), TITLE_MAX_CHARS);
        assert!(!clean.contains('\n') && !clean.contains('\t'));
    }

    #[test]
    fn a_short_title_is_untouched_and_a_blank_one_is_absent() {
        assert_eq!(
            sanitize_title("restore invariants").as_deref(),
            Some("restore invariants")
        );
        assert_eq!(sanitize_title("   \n\t  "), None);
        assert_eq!(sanitize_title(""), None);
    }
}
