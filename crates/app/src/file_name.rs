//! What a name must avoid to be a file name on every platform daruda runs
//! on — not just the one writing it.
//!
//! A name daruda writes into a repository (a flow file, a worktree's
//! directory) travels with the checkout, so it has to hold on Windows even
//! when written on macOS: saved on NTFS, `fix: login.yaml` becomes an
//! alternate data stream on `fix`, and `C:ship.yaml` a drive-relative path
//! outside the folder it was meant for.

/// Characters Windows refuses in a file name, beyond the two separators.
const RESERVED_CHARS: [char; 7] = ['<', '>', ':', '"', '|', '?', '*'];

/// Device names Windows reserves whatever follows the first dot.
const RESERVED_STEMS: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// What in-place character stands in for one a file name cannot hold.
const REPLACEMENT: char = '-';

/// Why `name` cannot be a file name everywhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NameProblem {
    /// A `/` or `\` would make it a path, not a name.
    Separator,
    /// A character, ending, or device name Windows will not take.
    Unportable,
}

pub(crate) fn problem(name: &str) -> Option<NameProblem> {
    if name.contains(['/', '\\']) {
        return Some(NameProblem::Separator);
    }
    let bad_char = name
        .chars()
        .any(|c| c.is_control() || RESERVED_CHARS.contains(&c));
    // Windows strips a trailing dot or space, so the name would not round-trip.
    let bad_end = name.ends_with(['.', ' ']);
    (bad_char || bad_end || has_reserved_stem(name)).then_some(NameProblem::Unportable)
}

/// `name` with whatever [`problem`] objects to replaced, for a name derived
/// from something that is not a file name — a branch name, say.
pub(crate) fn sanitized(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c.is_control() || RESERVED_CHARS.contains(&c) {
                REPLACEMENT
            } else {
                c
            }
        })
        .collect();
    while out.ends_with(['.', ' ']) {
        out.pop();
    }
    if has_reserved_stem(&out) {
        out.push(REPLACEMENT);
    }
    out
}

fn has_reserved_stem(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).trim_end();
    RESERVED_STEMS
        .iter()
        .any(|reserved| stem.eq_ignore_ascii_case(reserved))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn separators_and_windows_reserved_names_are_refused() {
        assert_eq!(problem("a/b"), Some(NameProblem::Separator));
        assert_eq!(problem("a\\b"), Some(NameProblem::Separator));
        assert_eq!(problem("fix: login.yaml"), Some(NameProblem::Unportable));
        assert_eq!(problem("C:ship.yaml"), Some(NameProblem::Unportable));
        assert_eq!(problem("a|b"), Some(NameProblem::Unportable));
        assert_eq!(problem("con.yaml"), Some(NameProblem::Unportable));
        assert_eq!(problem("trailing."), Some(NameProblem::Unportable));
        assert_eq!(problem("tab\there"), Some(NameProblem::Unportable));
        assert_eq!(problem("ship it.yaml"), None);
        assert_eq!(problem("console.yaml"), None);
        assert_eq!(problem("릴리스 준비.yaml"), None);
    }

    #[test]
    fn a_sanitized_name_has_no_problem_left() {
        for raw in ["feat/a|b", "fix: \"x\"", "nul", "wip.", "a<b>c?*"] {
            let clean = sanitized(raw);
            assert_eq!(problem(&clean), None, "{raw} -> {clean}");
        }
        assert_eq!(sanitized("feat/login"), "feat-login");
        assert_eq!(sanitized("nul"), "nul-");
    }
}
