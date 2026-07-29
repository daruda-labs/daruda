//! Branch-name derivation. Mirrors superset-desktop's
//! `deriveBranchName` — sanitize the title, truncate to 40 chars, then
//! append a ULID suffix so two tasks with the same title never collide.
//!
//! When the sanitized title is empty (user typed only whitespace or
//! reserved characters), fall back to `task-<ulid8>` so we always have
//! a usable branch name.

use daruda_core::git::sanitize_branch_name;

/// Maximum length of the title segment before the ULID suffix.
const MAX_TITLE_CHARS: usize = 40;

/// Number of leading ULID characters used as the disambiguating suffix.
const ULID_SUFFIX_CHARS: usize = 4;

/// Number of leading ULID characters used by the empty-title fallback.
const ULID_FALLBACK_CHARS: usize = 8;

/// Derive a stable branch name. The result is reused on Reopen / Retry —
/// never regenerated — so the lane path stays predictable.
///
/// - Sanitizes via `sanitize_branch_name`.
/// - Truncates the sanitized prefix to `MAX_TITLE_CHARS` *characters*
///   (not bytes — must be safe for non-ASCII titles).
/// - Appends `-<ulid[..4]>` (lowercased).
/// - Falls back to `task-<ulid[..8]>` when the sanitized prefix is empty.
pub fn derive_branch_name(title: &str, ulid: &str) -> String {
    let sanitized = sanitize_branch_name(title).unwrap_or_default();
    let truncated: String = sanitized.chars().take(MAX_TITLE_CHARS).collect();

    if truncated.is_empty() {
        let take = ULID_FALLBACK_CHARS.min(ulid.len());
        format!("task-{}", ulid[..take].to_lowercase())
    } else {
        let take = ULID_SUFFIX_CHARS.min(ulid.len());
        format!("{}-{}", truncated, ulid[..take].to_lowercase())
    }
}
