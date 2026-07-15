//! Remove-lane confirmation dialog (Modal trait flavor).
//!
//! Self-contained modal entity. Owns its own focus, key dispatch,
//! and submit (background `git worktree remove`). On success the
//! workspace finalizes by switching off the removed lane (if
//! active) and dropping its runtime + entry.

use std::path::PathBuf;

use crate::ui::theme;
use daruda_store::project::LaneId;
use gpui::{
    App, ClickEvent, Context, FocusHandle, Focusable, IntoElement, KeyDownEvent, Render,
    SharedString, WeakEntity, Window, div, prelude::*, px,
};

use crate::ui::Disableable as _;
use crate::ui::WindowExt as _;
use crate::ui::{button, button_danger, checkbox};

use crate::workspace::ModalView;
use crate::workspace::Workspace;
use crate::workspace::lane_ops::RemoveWorktreePlan;

pub struct RemoveWorktreeModal {
    focus_handle: FocusHandle,
    target_id: LaneId,
    target_label: SharedString,
    target_path: SharedString,
    plan: RemoveWorktreePlan,
    /// Branch the lane is checked out at. `None` for default
    /// (non-git) lanes and detached-HEAD lanes — in both
    /// cases there is nothing to delete, so the checkbox is hidden.
    branch: Option<String>,
    /// User opted in to also `git branch -D <branch>` after the
    /// lane is removed. Off by default — branch removal is
    /// destructive and the user must explicitly ask for it.
    delete_branch_too: bool,
    error: Option<SharedString>,
    /// When true, re-submit uses `git worktree remove --force`.
    /// Set automatically when git rejects a dirty lane.
    allow_force: bool,
    submitting: bool,
    workspace: WeakEntity<Workspace>,
}

impl RemoveWorktreeModal {
    pub fn new(
        workspace: WeakEntity<Workspace>,
        target_id: LaneId,
        target_label: impl Into<SharedString>,
        target_path: impl Into<SharedString>,
        plan: RemoveWorktreePlan,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            target_id,
            target_label: target_label.into(),
            target_path: target_path.into(),
            plan,
            branch: None,
            delete_branch_too: false,
            error: None,
            allow_force: false,
            submitting: false,
            workspace,
        }
    }

    /// Set the branch name that the lane is checked out at.
    /// When `Some`, shows the "Also delete branch" checkbox.
    pub fn with_branch(mut self, branch: Option<String>) -> Self {
        self.branch = branch;
        self
    }

    pub(crate) fn toggle_delete_branch(&mut self, cx: &mut Context<Self>) {
        if self.branch.is_some() {
            self.delete_branch_too = !self.delete_branch_too;
            cx.notify();
        }
    }

    pub(crate) fn dismiss(&mut self, window: &mut Window, cx: &mut App) {
        window.close_dialog(cx);
    }

    /// Test-only accessors.
    #[cfg(test)]
    pub(crate) fn allow_force(&self) -> bool {
        self.allow_force
    }
    #[cfg(test)]
    pub(crate) fn target_id(&self) -> LaneId {
        self.target_id
    }
    #[cfg(test)]
    pub(crate) fn delete_branch_too(&self) -> bool {
        self.delete_branch_too
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.submitting {
            return;
        }
        self.submitting = true;
        cx.notify();

        let me = cx.entity().downgrade();
        let workspace = self.workspace.clone();
        let repo_root = self.plan.repo_root.clone();
        let path: PathBuf = self.plan.path.clone();
        let force = self.allow_force;
        let target_id = self.target_id;
        let active_project = workspace
            .upgrade()
            .map(|w| w.read(cx).active_ref().project)
            .unwrap_or_default();
        let target_ref = daruda_store::project::LaneRef {
            project: active_project,
            lane: target_id,
        };
        // Branch deletion only fires when the user explicitly opted
        // in *and* the lane actually has a named branch.
        let branch_to_delete = if self.delete_branch_too {
            self.branch.clone()
        } else {
            None
        };
        window
            .spawn(cx, async move |async_cx| {
                let result: Result<(), String> = async_cx
                    .background_executor()
                    .spawn(async move {
                        crate::lane::git::remove_lane(&repo_root, &path, force)
                            .map_err(|e| e.to_string())?;
                        // Branch removal is best-effort *after* the
                        // lane is gone — failure here surfaces as
                        // an inline error but the lane is already
                        // detached, so we don't roll back.
                        if let Some(b) = &branch_to_delete {
                            crate::lane::git::delete_branch(&repo_root, b).map_err(|e| {
                                format!("Lane removed, but `git branch -D {b}` failed: {e}")
                            })?;
                        }
                        Ok(())
                    })
                    .await;

                // SILENT-OK: workspace may drop after remove modal closes
                let _ = async_cx.update(|window, app_cx| {
                    let Some(me) = me.upgrade() else { return };
                    match result {
                        Ok(()) => {
                            if let Some(ws) = workspace.upgrade() {
                                ws.update(app_cx, |ws, cx| {
                                    ws.finalize_remove_lane(target_ref, window, cx);
                                });
                            }
                            me.update(app_cx, |modal, cx| {
                                modal.submitting = false;
                                modal.dismiss(window, cx);
                            });
                        }
                        Err(msg) => {
                            me.update(app_cx, |modal, cx| {
                                modal.submitting = false;
                                if msg.contains("modifications") || msg.contains("--force") {
                                    modal.allow_force = true;
                                }
                                modal.error = Some(msg.into());
                                cx.notify();
                            });
                        }
                    }
                });
            })
            .detach();
    }

    fn handle_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match ev.keystroke.key.as_str() {
            "escape" => {
                self.dismiss(window, cx);
                cx.stop_propagation();
            }
            "enter" => {
                self.submit(window, cx);
                cx.stop_propagation();
            }
            _ => {}
        }
    }
}

impl Focusable for RemoveWorktreeModal {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ModalView for RemoveWorktreeModal {}

impl Render for RemoveWorktreeModal {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = theme::current(cx);
        let muted_text = t.text_muted;
        let faint_text = t.text_subtle;
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(muted_text)
                    .child(SharedString::from(format!(
                        "Remove \"{}\" — this runs `git worktree remove`. The branch is kept; only the checkout directory is deleted.",
                        self.target_label
                    ))),
            )
            .child(
                div()
                    .text_size(px(theme::LANE_SUB_FONT_SIZE))
                    .text_color(faint_text)
                    .child(self.target_path.clone()),
            );

        // Optional checkbox row: "Also delete branch '<name>'".
        // Only shown when the lane actually has a branch
        // (Default kind / detached HEAD have nothing to delete).
        if let Some(branch) = self.branch.clone() {
            let label = SharedString::from(format!("Also delete branch \"{branch}\""));
            body = body.child(
                checkbox("remove-wt-also-delete-branch", label, 0)
                    .checked(self.delete_branch_too)
                    .on_click(cx.listener(|this, _: &bool, _, cx| {
                        this.toggle_delete_branch(cx);
                    })),
            );
        }

        // Error block: alert banner + optional --force hint, stacked
        // so the dialog body shows them as a single element.
        let error_block = self.error.as_ref().map(|msg| {
            let mut stack = div()
                .flex()
                .flex_col()
                .gap(px(theme::MODAL_PANEL_GAP))
                .child(crate::ui::alert::error("remove-lane-error", msg.clone()));
            if self.allow_force {
                stack = stack.child(
                    div()
                        .text_size(px(theme::LANE_SUB_FONT_SIZE))
                        .text_color(faint_text)
                        .child(
                            "The lane has uncommitted changes. Clicking Remove again will pass --force.",
                        ),
                );
            }
            stack
        });

        let confirm_label = if self.submitting {
            "Removing…"
        } else if self.allow_force {
            "Force Remove"
        } else {
            "Remove"
        };

        let footer = div()
            .flex()
            .flex_row()
            .justify_end()
            .gap(px(theme::MODAL_FOOTER_GAP))
            .mt(px(theme::MODAL_FOOTER_MARGIN_TOP))
            .child(button("remove-wt-cancel", "Cancel").on_click(cx.listener(
                |this, _: &ClickEvent, window, cx| {
                    this.dismiss(window, cx);
                },
            )))
            .child(
                button_danger("remove-wt-confirm", confirm_label)
                    .disabled(self.submitting)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.submit(window, cx);
                    })),
            );

        let mut panel = div()
            .flex()
            .flex_col()
            .key_context("RemoveWorktreeModal")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                this.handle_key(ev, window, cx);
            }))
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(body);
        if let Some(b) = error_block {
            panel = panel.child(b);
        }
        panel.child(footer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    fn build_modal(cx: &mut TestAppContext) -> gpui::Entity<RemoveWorktreeModal> {
        build_modal_with_branch(cx, Some("feat/sidebar".to_string()))
    }

    fn build_modal_with_branch(
        cx: &mut TestAppContext,
        branch: Option<String>,
    ) -> gpui::Entity<RemoveWorktreeModal> {
        crate::test_support::init_gpui_component(cx);
        let plan = RemoveWorktreePlan {
            path: PathBuf::from("/tmp/repo-feat"),
            repo_root: PathBuf::from("/tmp/repo"),
        };
        let wh = cx.add_window(|window, cx| {
            RemoveWorktreeModal::new(
                WeakEntity::new_invalid(),
                7,
                "feat/sidebar",
                "/tmp/repo-feat",
                plan,
                window,
                cx,
            )
            .with_branch(branch)
        });
        wh.root(cx).unwrap()
    }

    #[gpui::test]
    fn starts_with_target_and_no_error(cx: &mut TestAppContext) {
        let modal = build_modal(cx);
        modal.read_with(cx, |m, _| {
            assert_eq!(m.target_id(), 7);
            assert!(!m.allow_force());
            assert!(m.error.is_none());
            assert!(!m.submitting);
        });
    }

    #[gpui::test]
    fn delete_branch_defaults_off(cx: &mut TestAppContext) {
        // Branch deletion is destructive — must require explicit
        // user opt-in even when a branch is present.
        let modal = build_modal(cx);
        modal.read_with(cx, |m, _| assert!(!m.delete_branch_too()));
    }

    #[gpui::test]
    fn toggle_delete_branch_flips_when_branch_present(cx: &mut TestAppContext) {
        let modal = build_modal(cx);
        modal.update(cx, |m, cx| m.toggle_delete_branch(cx));
        modal.read_with(cx, |m, _| assert!(m.delete_branch_too()));
        modal.update(cx, |m, cx| m.toggle_delete_branch(cx));
        modal.read_with(cx, |m, _| assert!(!m.delete_branch_too()));
    }

    #[gpui::test]
    fn toggle_delete_branch_noop_when_branch_absent(cx: &mut TestAppContext) {
        // Default (non-git) or detached-HEAD lane → no branch
        // to delete → toggle must stay off so the modal doesn't
        // pretend it can clean something up.
        let modal = build_modal_with_branch(cx, None);
        modal.update(cx, |m, cx| m.toggle_delete_branch(cx));
        modal.read_with(cx, |m, _| assert!(!m.delete_branch_too()));
    }
}
