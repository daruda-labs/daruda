//! Branch-name preflight filter — duplicates the implementation in
//! `app/src/workspace/worktree_ops.rs::sanitize_branch_name` so this
//! GPUI-free crate stays decoupled from the app crate. The two
//! definitions must stay byte-identical; `tests::sanitize_*` exercises
//! the same matrix the app side does.
//!
//! Rules enforced (subset of `git-check-ref-format`):
//! - non-empty after trim
//! - no `..`
//! - no leading / trailing `/`
//! - no leading / trailing `.`
//! - no shell-hostile / protocol-reserved chars: ` `, `:`, `~`, `^`,
//!   `?`, `*`, `[`, `\\`
//! - no control chars

/// Returns `Some(trimmed)` when the input is a valid branch fragment,
/// otherwise `None`. The caller decides what to do on rejection (modal
/// validation, fallback name, etc).
pub fn sanitize_branch_name(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.contains("..")
        || trimmed.starts_with('/')
        || trimmed.ends_with('/')
        || trimmed.starts_with('.')
        || trimmed.ends_with('.')
    {
        return None;
    }
    let bad = [' ', ':', '~', '^', '?', '*', '[', '\\'];
    if trimmed.chars().any(|c| bad.contains(&c) || c.is_control()) {
        return None;
    }
    Some(trimmed.to_string())
}
