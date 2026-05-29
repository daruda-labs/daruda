//! Background git-status / diff / commit / push operations for the
//! Git Changes dock view.
//!
//! All git CLI calls run on `background_executor` so the UI thread never
//! blocks. State mutations come back via `cx.spawn(|this, cx| ...)` where
//! `this` is `WeakEntity<Workspace>` auto-injected by `Context::spawn`.
//!
//! Methods are split across submodules by responsibility:
//!
//! - [`status`] — `git status` refresh + commit-button sync + repo-root lookup.
//! - [`file_view`] — pane-area file viewer open / mode / scroll.
//! - [`index`] — `git add` / `git restore --staged` / discard.
//! - [`history`] — commit / amend / push / pull / fetch.
//! - [`nav`] — Git Changes dock keyboard cursor + dir collapse.
//! - [`init`] — `git init` for non-git worktrees.

pub(in crate::workspace) mod file_view;
pub(in crate::workspace) mod history;
pub(in crate::workspace) mod index;
pub(in crate::workspace) mod init;
pub(in crate::workspace) mod nav;
pub(in crate::workspace) mod status;

/// Map a git status character to a single-letter display symbol. Lifted
/// from the old `git_changes::status_display` so the left-dock Files
/// view (W-7f) can show the same badge with one source of truth.
pub(in crate::workspace) fn git_status_symbol(ch: char) -> &'static str {
    match ch {
        'M' => "M",
        // Untracked files render as additions in the left dock.
        'A' | '?' => "A",
        'D' => "D",
        'R' => "R",
        'C' => "C",
        'U' => "U",
        _ => "·",
    }
}

/// Map a git status character and staged flag to a display colour.
/// Used by both the left dock file list and the pane-area file viewer toolbar.
/// Reads from the live `DarudaTheme` Global so colours flip on
/// light-mode switch.
pub(in crate::workspace) fn git_status_color(ch: char, staged: bool, cx: &gpui::App) -> gpui::Hsla {
    use crate::ui::theme;
    let t = theme::current(cx);
    match ch {
        'M' | 'D' => {
            if staged {
                theme::GIT_STAGED
            } else {
                theme::GIT_MODIFIED
            }
        }
        'A' | 'R' | 'C' => theme::GIT_STAGED,
        '?' => theme::GIT_UNTRACKED,
        'U' => theme::GIT_MODIFIED,
        _ => t.faint_text,
    }
}
