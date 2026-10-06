//! Git naming rules, as pure predicates.
//!
//! Only what can be decided from a string — no process, no repository. Git
//! itself is the authority (`git check-ref-format`), so this is a preflight
//! filter: it lets a form reject bad input before shelling out, and it is
//! deliberately stricter than git in places (a plain space is legal in some
//! git contexts but hostile in every shell that will see the name).
//!
//! Callers that only need a yes/no use [`sanitize_branch_name`]; callers that
//! must tell the user *why* use [`validate_branch_name`] and render the
//! returned [`BranchNameRule`] in their own words. Keeping both on one rule
//! walk is the point — the two used to be separate copies that had to be kept
//! in step by hand.

/// The git ref-name rule an input breaks, in the order they are checked.
///
/// A caller renders these; this crate holds no user-facing text (it sits
/// below the app's i18n layer).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchNameRule {
    /// Nothing but whitespace.
    Empty,
    /// Contains `..`, which git reads as a range operator.
    DoubleDot,
    /// Starts or ends with `/`.
    EdgeSlash,
    /// Starts or ends with `.`.
    EdgeDot,
    /// Contains an ASCII control character.
    ControlChar,
    /// Contains a space — legal for git in some contexts, hostile to shells.
    Space,
    /// Contains a character git reserves or a shell would expand.
    Reserved(char),
}

/// Characters rejected on sight: git-reserved (`:` `~` `^` `?` `*` `[`) plus
/// the backslash, which no shell quoting round-trips cleanly.
const RESERVED: [char; 7] = [':', '~', '^', '?', '*', '[', '\\'];

/// Check `raw` against the branch-name rules, returning the trimmed name or
/// the first rule it breaks.
pub fn validate_branch_name(raw: &str) -> Result<&str, BranchNameRule> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(BranchNameRule::Empty);
    }
    if trimmed.contains("..") {
        return Err(BranchNameRule::DoubleDot);
    }
    if trimmed.starts_with('/') || trimmed.ends_with('/') {
        return Err(BranchNameRule::EdgeSlash);
    }
    if trimmed.starts_with('.') || trimmed.ends_with('.') {
        return Err(BranchNameRule::EdgeDot);
    }
    for c in trimmed.chars() {
        if c.is_control() {
            return Err(BranchNameRule::ControlChar);
        }
        if c == ' ' {
            return Err(BranchNameRule::Space);
        }
        if RESERVED.contains(&c) {
            return Err(BranchNameRule::Reserved(c));
        }
    }
    Ok(trimmed)
}

/// The trimmed branch name when `raw` passes every rule, else `None`.
/// The caller decides what to do on rejection (form validation, fallback
/// name, …).
pub fn sanitize_branch_name(raw: &str) -> Option<String> {
    validate_branch_name(raw).ok().map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn every_reserved_character_is_rejected() {
        for c in RESERVED {
            assert_eq!(
                validate_branch_name(&format!("a{c}b")),
                Err(BranchNameRule::Reserved(c)),
                "{c:?} must be rejected"
            );
        }
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
}
