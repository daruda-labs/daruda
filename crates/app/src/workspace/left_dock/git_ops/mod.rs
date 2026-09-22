//! Background git operations for the Git Changes dock view. All git CLI
//! calls run on `background_executor` so the UI thread never blocks; state
//! mutations return via `Context::spawn` with a `WeakEntity<Workspace>`.
//!
//! Submodules by responsibility: [`status`] (refresh + commit-button sync +
//! repo-root), [`file_view`] (pane-area file viewer), [`index`]
//! (stage/unstage/discard), [`history`] (commit/amend/push/pull/fetch),
//! [`nav`] (keyboard cursor + dir collapse), [`init`] (`git init`), [`lock`]
//! (the exclusive-op locks every one of them claims), [`watch`] (the git-dir
//! watchers that catch writes daruda did not make).

pub(super) mod file_view;
pub(super) mod history;
pub(super) mod index;
pub(super) mod init;
pub(in crate::workspace) mod lock;
pub(super) mod nav;
pub(super) mod status;
pub(in crate::workspace) mod watch;

/// Map a git status character to a single-letter display symbol. Shared by
/// the Git Changes dock and the Files view so both badges match.
pub(super) fn git_status_symbol(ch: char) -> &'static str {
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

/// Map a git status character and staged flag to a display colour. Used by
/// the left-dock file list and the file viewer toolbar. Reads the live
/// `DarudaTheme` Global so colours flip on light-mode switch.
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
        _ => t.text_subtle,
    }
}
