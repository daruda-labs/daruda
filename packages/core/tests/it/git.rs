//! Public API tests for git.

use daruda_core::git::*;

#[test]
fn accepts_typical_branch_names() {
    assert_eq!(sanitize_branch_name("main").as_deref(), Some("main"));
    assert_eq!(
        sanitize_branch_name("feat/sidebar").as_deref(),
        Some("feat/sidebar")
    );
    assert_eq!(sanitize_branch_name("v1.2.3").as_deref(), Some("v1.2.3"));
    // Surrounding whitespace is trimmed, not rejected.
    assert_eq!(
        sanitize_branch_name("  fix-123  ").as_deref(),
        Some("fix-123")
    );
}

#[test]
fn reports_which_rule_failed() {
    let err = |s: &str| validate_branch_name(s).unwrap_err();
    assert_eq!(err(""), BranchNameRule::Empty);
    assert_eq!(err("   "), BranchNameRule::Empty);
    assert_eq!(err("foo..bar"), BranchNameRule::DoubleDot);
    assert_eq!(err(".."), BranchNameRule::DoubleDot);
    assert_eq!(err("/leading"), BranchNameRule::EdgeSlash);
    assert_eq!(err("trailing/"), BranchNameRule::EdgeSlash);
    assert_eq!(err(".hidden"), BranchNameRule::EdgeDot);
    assert_eq!(err("trailing."), BranchNameRule::EdgeDot);
    assert_eq!(err("has space"), BranchNameRule::Space);
    assert_eq!(err("bell\u{7}"), BranchNameRule::ControlChar);
    assert_eq!(err("has:colon"), BranchNameRule::Reserved(':'));
    assert_eq!(err("has~tilde"), BranchNameRule::Reserved('~'));
    assert_eq!(err("has\\slash"), BranchNameRule::Reserved('\\'));
}

#[test]
fn sanitize_agrees_with_validate() {
    // The two entry points must never disagree on acceptance — that
    // divergence is what splitting them into separate copies caused.
    for s in [
        "main", "feat/x", "", "  ", "a..b", "/x", "x/", ".x", "x.", "a b", "a:b", "v1.2.3",
    ] {
        assert_eq!(
            sanitize_branch_name(s).is_some(),
            validate_branch_name(s).is_ok(),
            "disagreement on {s:?}"
        );
    }
}
