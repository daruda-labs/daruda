use daruda_control_types::agent_text::{bound_agent_text, elide_middle, sanitize_title};

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
fn elision_accepts_empty_sides_and_oversized_budgets() {
    for (head, tail, expected) in [
        (0, 2, "\nCUT\né😀"),
        (2, 0, "한e\nCUT\n"),
        (0, 0, "\nCUT\n"),
        (usize::MAX, 1, "한eé😀"),
        (1, usize::MAX, "한eé😀"),
        (usize::MAX, usize::MAX, "한eé😀"),
    ] {
        assert_eq!(elide_middle("한eé😀", head, tail, "CUT"), expected);
    }
    assert_eq!(elide_middle("", 0, 0, "CUT"), "");
}

#[test]
fn a_bounded_body_carries_its_marker_in_band() {
    let long = "x".repeat(4001);
    let out = bound_agent_text(&long).expect("non-empty");
    assert!(
        out.contains("[...truncated by daruda...]"),
        "truncation must be self-describing, not a sibling bool: {out:.80}"
    );
    assert_eq!(out.chars().filter(|c| *c == 'x').count(), 4000);
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
    assert_eq!(clean.chars().count(), 80);
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
