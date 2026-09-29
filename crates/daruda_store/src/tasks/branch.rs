//! Branch-name derivation: slugify the title, truncate to 40 characters,
//! and append a random ULID suffix to disambiguate identical titles.
//!
//! When the sanitized title is empty (user typed only whitespace or
//! reserved characters), fall back to `task-<ulid8>` so we always have
//! a usable branch name.

/// Maximum length of the title segment before the ULID suffix.
const MAX_TITLE_CHARS: usize = 40;

/// The ULID tail contains randomness; its prefix only contains time.
const ULID_SUFFIX_CHARS: usize = 8;

/// Derive a stable branch name. The result is reused on Reopen / Retry —
/// never regenerated — so the lane path stays predictable.
///
/// - Converts title words into a lowercase, hyphen-separated slug.
/// - Truncates the sanitized prefix to `MAX_TITLE_CHARS` *characters*
///   (not bytes — must be safe for non-ASCII titles).
/// - Appends the last eight ULID characters (lowercased).
/// - Falls back to `task-<suffix>` when the slug is empty.
pub fn derive_branch_name(title: &str, ulid: &str) -> String {
    let slug = title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join("-")
        .to_lowercase();
    let truncated: String = slug.chars().take(MAX_TITLE_CHARS).collect();
    let prefix = truncated.trim_end_matches('-');
    let suffix: String = ulid
        .chars()
        .rev()
        .take(ULID_SUFFIX_CHARS)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!(
        "{}-{}",
        if prefix.is_empty() { "task" } else { prefix },
        suffix.to_lowercase()
    )
}
