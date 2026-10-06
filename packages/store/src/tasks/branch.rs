//! Default task branch name: `task-` plus a random suffix cut from a ULID.
//!
//! The title plays no part: a slug of free text (Hangul, spaces,
//! punctuation) reads badly as a ref and would change under every keystroke.

/// The ULID tail contains randomness; its prefix only contains time.
const ULID_SUFFIX_CHARS: usize = 8;

/// `task-<last eight ULID characters, lowercased>`. Stable for a given id,
/// so a task's default branch never changes once assigned.
pub fn branch_name_for(ulid: &str) -> String {
    let suffix: String = ulid
        .chars()
        .rev()
        .take(ULID_SUFFIX_CHARS)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("task-{}", suffix.to_lowercase())
}

/// A name with a fresh random suffix, for a form that has no task id yet.
pub fn random_branch_name() -> String {
    branch_name_for(&ulid::Ulid::new().to_string())
}
