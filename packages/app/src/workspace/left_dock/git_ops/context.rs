//! Git Changes view state owned by [`crate::workspace::Workspace`].
//!
//! Window-wide: the panel's input, focus and scroll, plus the locks and
//! git-dir watchers every git op shares. Per-lane git data lives in
//! `LaneScoped`.

use gpui::{
    AppContext, Context, FocusHandle, Subscription, UniformListScrollHandle, WeakEntity, Window,
};

use super::history::CommitMode;
use super::lock::GitLocks;
use super::watch::GitWatch;
use crate::workspace::{CommitChanges, Workspace};

pub(in crate::workspace) struct GitContext {
    /// Commit-message input panel rendered in the Git Changes footer.
    pub(in crate::workspace) commit_input: gpui::Entity<crate::ui::InputPanel>,
    /// Keeps the commit input's subscription alive with the Workspace.
    _commit_subscription: Subscription,
    /// Commit-button mode — see [`CommitMode`].
    pub(in crate::workspace) commit_mode: CommitMode,
    /// The two git locks this window holds — see [`GitLocks`].
    pub(in crate::workspace) locks: GitLocks,
    /// Git directories this window watches — see [`GitWatch`].
    pub(in crate::workspace) watch: GitWatch,
    /// Focus handle for the panel body. Bound to `key_context("GitChanges")`
    /// so the arrow / Space / Enter keybindings only fire when the panel
    /// holds focus — otherwise they fall through to terminal panes.
    pub(in crate::workspace) panel_focus: FocusHandle,
    /// Scroll handle for the file list — shared with the scrollbar overlay.
    pub(in crate::workspace) scroll_handle: UniformListScrollHandle,
}

impl GitContext {
    pub(in crate::workspace) fn new(
        workspace: &WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Self {
        let ws_commit = workspace.clone();
        let ws_amend = workspace.clone();
        let commit_input = cx.new(|cx| {
            crate::ui::InputPanel::new(crate::ui::InputPanelLayout::ActionsFloating, window, cx)
                .with_placeholder(
                    crate::surface::strings::git::commit_placeholder(),
                    window,
                    cx,
                )
                .with_borderless(cx)
                .with_focus_ring(false, cx)
                .with_action(
                    crate::ui::PanelAction::new(
                        "commit",
                        crate::surface::strings::git::commit_btn(),
                        crate::ui::PanelActionVariant::Primary,
                        move |_, window, cx| {
                            let _ = ws_commit.upgrade().map(|w| {
                                w.update(cx, |ws, cx| {
                                    ws.on_commit_changes(&CommitChanges, window, cx)
                                })
                            });
                        },
                    )
                    .with_dropdown_item(
                        crate::surface::strings::ctx::git_commit_amend(),
                        move |window, app_cx| {
                            if let Some(ws) = ws_amend.upgrade() {
                                ws.update(app_cx, |ws, cx| ws.on_commit_amend(window, cx));
                            }
                        },
                    ),
                )
        });
        let commit_subscription = cx.subscribe_in(
            &commit_input,
            window,
            |this, _, ev: &crate::ui::InputPanelEvent, window, cx| match ev {
                crate::ui::InputPanelEvent::Submit => {
                    this.on_commit_changes(&CommitChanges, window, cx);
                }
                crate::ui::InputPanelEvent::Changed => {}
            },
        );
        Self {
            commit_input,
            _commit_subscription: commit_subscription,
            commit_mode: CommitMode::default(),
            locks: GitLocks::default(),
            watch: GitWatch::default(),
            panel_focus: cx.focus_handle(),
            scroll_handle: UniformListScrollHandle::new(),
        }
    }
}
