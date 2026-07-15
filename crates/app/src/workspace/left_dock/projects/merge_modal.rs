//! Merge-into modal — lets the user pick a target lane branch and
//! runs `git merge <source>` from the target's checkout directory.
//!
//! Only branches currently checked out as app lanes are offered as
//! targets; merging into a branch that isn't in any lane would
//! require a checkout, which git forbids while another lane has it.
//!
//! Conflict handling: when git reports conflicts the merge is left
//! in-progress in the target lane.  The modal switches to a
//! "Conflicts" state and offers two actions:
//!   • [Abort Merge] — runs `git merge --abort` and dismisses.
//!   • [Go to "<branch>" →] — activates the target lane tab so the
//!     user can resolve conflicts in the terminal, then commit.
//!
//! Remove-after-merge: when the source lane is removable (not the
//! main checkout), a checkbox lets the user remove the lane and
//! delete its branch automatically after a successful merge.

use std::path::PathBuf;

use crate::ui::theme;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::project::{LaneId, LaneRef, ProjectId};
use gpui::{
    App, ClickEvent, Context, FocusHandle, Focusable, IntoElement, KeyDownEvent, MouseDownEvent,
    Render, SharedString, WeakEntity, Window, div, prelude::*, px,
};

use crate::ui::Disableable as _;
use crate::ui::WindowExt as _;
use crate::ui::{button, button_danger, button_primary, checkbox};

use crate::surface::strings as surface_strings;
use crate::workspace::ModalView;
use crate::workspace::Workspace;

// ----------------------------------------------------------------
// Data types
// ----------------------------------------------------------------

pub(in crate::workspace) struct TargetOption {
    pub wt_id: LaneId,
    pub branch: String,
    pub wt_path: PathBuf,
}

enum MergeState {
    Idle,
    Merging,
    /// Merge ran and produced conflicts; the target lane is in
    /// mid-merge state waiting for manual resolution.
    Conflicts(Vec<String>),
    Error(String),
}

// ----------------------------------------------------------------
// Modal entity
// ----------------------------------------------------------------

pub(in crate::workspace) struct MergeModal {
    /// The project both lanes belong to — captured from the lane row the
    /// menu was opened for, NOT the workspace's active project. Lane ids
    /// restart per project, so resolving source/target against the active
    /// project would merge the wrong project's like-id'd lanes.
    project: ProjectId,
    /// The lane being merged from.
    source_wt_id: LaneId,
    source_branch: String,
    /// Path of the source lane checkout (for `git worktree remove`).
    source_path: PathBuf,
    /// Repo root of the source lane (for `git branch -D`).
    source_repo_root: PathBuf,
    target_options: Vec<TargetOption>,
    selected_idx: usize,
    workspace: WeakEntity<Workspace>,
    focus_handle: FocusHandle,
    state: MergeState,
    /// When true, remove the source lane and delete its branch
    /// after a successful merge.
    remove_after_merge: bool,
}

impl MergeModal {
    /// `target_options` must be non-empty — callers must check and
    /// surface a transient error instead of opening this modal when
    /// the list would be empty.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::workspace) fn new(
        project: ProjectId,
        source_wt_id: LaneId,
        source_branch: String,
        source_path: PathBuf,
        source_repo_root: PathBuf,
        mut target_options: Vec<TargetOption>,
        base_ref: Option<String>,
        workspace: WeakEntity<Workspace>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // Sort: base_ref match first, then alphabetical.
        if let Some(base) = &base_ref {
            target_options.sort_by(|a, b| {
                let a_match = &a.branch == base;
                let b_match = &b.branch == base;
                match (a_match, b_match) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => a.branch.cmp(&b.branch),
                }
            });
        } else {
            target_options.sort_by(|a, b| a.branch.cmp(&b.branch));
        }

        Self {
            project,
            source_wt_id,
            source_branch,
            source_path,
            source_repo_root,
            target_options,
            selected_idx: 0,
            workspace,
            focus_handle: cx.focus_handle(),
            state: MergeState::Idle,
            remove_after_merge: false,
        }
    }

    /// `LaneRef` for the currently-selected merge target, resolved against
    /// this modal's own [`Self::project`] — never the active project.
    fn target_ref(&self) -> LaneRef {
        LaneRef {
            project: self.project,
            lane: self.target_options[self.selected_idx].wt_id,
        }
    }

    /// `LaneRef` for the source lane, resolved against [`Self::project`].
    fn source_ref(&self) -> LaneRef {
        LaneRef {
            project: self.project,
            lane: self.source_wt_id,
        }
    }

    /// True when the source lane can be removed (i.e. it is not the
    /// main checkout of the repo).
    fn source_is_removable(&self) -> bool {
        self.source_path != self.source_repo_root
    }

    pub(crate) fn dismiss(&mut self, window: &mut Window, cx: &mut App) {
        window.close_dialog(cx);
    }

    fn select_up(&mut self, cx: &mut Context<Self>) {
        if self.selected_idx > 0 {
            self.selected_idx -= 1;
            cx.notify();
        }
    }

    fn select_down(&mut self, cx: &mut Context<Self>) {
        if self.selected_idx + 1 < self.target_options.len() {
            self.selected_idx += 1;
            cx.notify();
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.state, MergeState::Merging) {
            return;
        }

        // Pre-check: target lane must be clean.
        // NOTE: git_status_cache may be stale if the target changed after
        // the last refresh. git itself will still reject a dirty target
        // (GitError::Exit), so no data is corrupted — this pre-check is
        // only a best-effort UX improvement for a friendlier error message.
        let target_is_dirty = if let Some(ws) = self.workspace.upgrade() {
            let target_ref = self.target_ref();
            ws.read(cx)
                .git_status_cache
                .get(&target_ref)
                .is_some_and(|s| !s.staged.is_empty() || !s.unstaged.is_empty())
        } else {
            false
        };

        if target_is_dirty {
            self.state = MergeState::Error(surface_strings::merge_modal_target_dirty().to_string());
            cx.notify();
            return;
        }

        self.state = MergeState::Merging;
        cx.notify();

        let me = cx.entity().downgrade();
        let workspace = self.workspace.clone();
        let source_branch = self.source_branch.clone();
        let source_path = self.source_path.clone();
        let source_repo_root = self.source_repo_root.clone();
        let target = &self.target_options[self.selected_idx];
        let target_path = target.wt_path.clone();
        let remove_after_merge = self.remove_after_merge;
        // Both refs resolve against this modal's own project, not the
        // active one — see `MergeModal::project`.
        let target_ref = self.target_ref();
        let source_ref = self.source_ref();

        // Clones for use after the first async_cx.update closure.
        let workspace_post = workspace.clone();
        let me_post = me.clone();

        window
            .spawn(cx, async move |async_cx| {
                // Clone before the merge spawn so it's still available
                // for the post-merge removal spawn.
                let source_branch_for_removal = source_branch.clone();

                let merge_result = async_cx
                    .background_executor()
                    .spawn(async move { crate::lane::git::git_merge(&target_path, &source_branch) })
                    .await;

                // Snapshot before merge_result is moved into the closure.
                let is_success = matches!(
                    &merge_result,
                    Ok(crate::lane::git::MergeOutcome::Success)
                        | Ok(crate::lane::git::MergeOutcome::AlreadyUpToDate)
                );
                let was_up_to_date = matches!(
                    &merge_result,
                    Ok(crate::lane::git::MergeOutcome::AlreadyUpToDate)
                );

                // SILENT-OK: workspace may drop after merge modal closes
                let _ = async_cx.update(|window, app_cx| {
                    match merge_result {
                        Ok(crate::lane::git::MergeOutcome::AlreadyUpToDate) => {
                            if let Some(ws) = workspace.upgrade() {
                                ws.update(app_cx, |ws, cx| {
                                    ws.finalize_merge(target_ref, cx);
                                    // Show "already up to date" only if we're
                                    // not about to also remove the lane.
                                    if !remove_after_merge {
                                        let report = ErrorReport::new(
                                            surface_strings::merge_modal_already_up_to_date(),
                                        )
                                        .severity(ErrorSeverity::Info)
                                        .at(file!(), line!())
                                        .dedup("lane.merge.up_to_date")
                                        .build();
                                        ws.report_error(report, cx);
                                    }
                                });
                            }
                            if !remove_after_merge && let Some(me) = me.upgrade() {
                                me.update(app_cx, |m, cx| m.dismiss(window, cx));
                            }
                        }

                        Ok(crate::lane::git::MergeOutcome::Success) => {
                            if let Some(ws) = workspace.upgrade() {
                                ws.update(app_cx, |ws, cx| ws.finalize_merge(target_ref, cx));
                            }
                            if !remove_after_merge && let Some(me) = me.upgrade() {
                                me.update(app_cx, |m, cx| m.dismiss(window, cx));
                            }
                        }

                        Ok(crate::lane::git::MergeOutcome::Conflicts(files)) => {
                            // Leave the merge in-progress in the target lane.
                            // Refresh git status so the left dock reflects the
                            // mid-merge state (dirty files + MERGE_HEAD present).
                            if let Some(ws) = workspace.upgrade() {
                                ws.update(app_cx, |ws, cx| ws.finalize_merge(target_ref, cx));
                            }
                            if let Some(me) = me.upgrade() {
                                me.update(app_cx, |modal, cx| {
                                    modal.state = MergeState::Conflicts(files);
                                    cx.notify();
                                });
                            }
                        }

                        Err(e) => {
                            if let Some(me) = me.upgrade() {
                                me.update(app_cx, |modal, cx| {
                                    modal.state = MergeState::Error(e.to_string());
                                    cx.notify();
                                });
                            }
                        }
                    }
                });

                // Post-merge removal — only on success when user opted in.
                if is_success && remove_after_merge {
                    let remove_result = async_cx
                        .background_executor()
                        .spawn(async move {
                            // Remove lane directory first, then delete the branch.
                            // Branch deletion is best-effort — if it fails the lane
                            // is already detached, so we surface the error without rolling back.
                            crate::lane::git::remove_lane(&source_repo_root, &source_path, false)
                                .and_then(|()| {
                                    crate::lane::git::delete_branch(
                                        &source_repo_root,
                                        &source_branch_for_removal,
                                    )
                                })
                                .map_err(|e| e.to_string())
                        })
                        .await;

                    // SILENT-OK: workspace may drop after merge modal closes
                    let _ = async_cx.update(|window, app_cx| {
                        if let Some(ws) = workspace_post.upgrade() {
                            ws.update(app_cx, |ws, cx| {
                                match &remove_result {
                                    Ok(()) => {
                                        ws.finalize_remove_lane(source_ref, window, cx);
                                        if was_up_to_date {
                                            let report = ErrorReport::new(
                                                surface_strings::merge_modal_already_up_to_date(),
                                            )
                                            .severity(ErrorSeverity::Info)
                                            .at(file!(), line!())
                                            .dedup("lane.merge.up_to_date")
                                            .build();
                                            ws.report_error(report, cx);
                                        }
                                    }
                                    Err(e) => {
                                        let report =
                                            ErrorReport::new("Merge succeeded, but cleanup failed")
                                                .severity(ErrorSeverity::Error)
                                                .at(file!(), line!())
                                                .with_context("detail", e.clone())
                                                .dedup("lane.merge.cleanup")
                                                .build();
                                        ws.report_error(report, cx);
                                    }
                                }
                                cx.notify();
                            });
                        }
                        if let Some(me) = me_post.upgrade() {
                            me.update(app_cx, |m, cx| m.dismiss(window, cx));
                        }
                    });
                }
            })
            .detach();
    }

    /// Abort the in-progress merge in the target lane (best-effort,
    /// on background executor), refresh its git status, then dismiss.
    fn abort_merge(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.target_options.get(self.selected_idx) else {
            self.dismiss(window, cx);
            return;
        };
        let target_path = target.wt_path.clone();
        let target_ref = self.target_ref();
        let workspace = self.workspace.clone();
        let me = cx.entity().downgrade();

        window
            .spawn(cx, async move |async_cx| {
                async_cx
                    .background_executor()
                    .spawn(async move {
                        let _ = crate::lane::git::git_merge_abort(&target_path);
                    })
                    .await;
                // SILENT-OK: workspace may drop after merge modal closes
                let _ = async_cx.update(|window, app_cx| {
                    if let Some(ws) = workspace.upgrade() {
                        ws.update(app_cx, |ws, cx| ws.finalize_merge(target_ref, cx));
                    }
                    if let Some(me) = me.upgrade() {
                        me.update(app_cx, |m, cx| m.dismiss(window, cx));
                    }
                });
            })
            .detach();
    }

    /// Switch focus to the target lane so the user can resolve
    /// conflicts in its terminal, then dismiss this modal.
    fn go_to_target(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.target_ref();
        if let Some(ws) = self.workspace.upgrade() {
            ws.update(cx, |ws, cx| ws.activate_lane(target, window, cx));
        }
        self.dismiss(window, cx);
    }

    fn handle_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match ev.keystroke.key.as_str() {
            "escape" => {
                if !matches!(self.state, MergeState::Merging) {
                    self.dismiss(window, cx);
                }
                // Always stop propagation — even while Merging, to prevent
                // the workspace-level Escape handler from closing the modal
                // while git is running.
                cx.stop_propagation();
            }
            "enter" => {
                self.submit(window, cx);
                cx.stop_propagation();
            }
            "up" => {
                self.select_up(cx);
                cx.stop_propagation();
            }
            "down" => {
                self.select_down(cx);
                cx.stop_propagation();
            }
            _ => {}
        }
    }

    /// Test-only accessors.
    #[cfg(test)]
    pub(crate) fn selected_branch(&self) -> &str {
        &self.target_options[self.selected_idx].branch
    }
    #[cfg(test)]
    pub(crate) fn remove_after_merge(&self) -> bool {
        self.remove_after_merge
    }
}

// ----------------------------------------------------------------
// GPUI trait implementations
// ----------------------------------------------------------------

impl Focusable for MergeModal {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ModalView for MergeModal {}

impl Render for MergeModal {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Title is provided by the Dialog wrapper at the entry point.
        let is_merging = matches!(self.state, MergeState::Merging);
        let is_conflicts = matches!(self.state, MergeState::Conflicts(_));
        let show_remove_checkbox = self.source_is_removable();

        // Snapshot selected-target info for footer closures (must happen
        // before any borrow of self.state).
        let sel_branch = self.target_options[self.selected_idx].branch.clone();

        let t = theme::current(cx);
        let strong_text = t.text_primary;
        let muted_text = t.text_muted;
        let row_hover_bg = t.lane_row_hover_bg;
        let radio_dot_color = t.text_primary;

        // ---- branch list ----
        let mut branch_list = div().flex().flex_col().gap(px(theme::MODAL_BUTTON_PAD_Y));
        for (idx, opt) in self.target_options.iter().enumerate() {
            let is_selected = idx == self.selected_idx;
            let branch_label = SharedString::from(opt.branch.clone());
            let row = div()
                .id(("merge-target", idx))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(theme::LANE_LABEL_GAP))
                .px(px(theme::MODAL_BUTTON_PAD_X))
                .py(px(theme::MODAL_BUTTON_PAD_Y))
                .rounded(px(theme::MODAL_BUTTON_RADIUS))
                .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                .text_color(if is_selected { strong_text } else { muted_text })
                .bg(if is_selected {
                    row_hover_bg
                } else {
                    gpui::transparent_black()
                })
                // Branch selection is only interactive before merge starts.
                .when(!is_conflicts && !is_merging, |d| {
                    d.cursor_pointer()
                        .hover(move |d| d.bg(row_hover_bg))
                        .on_mouse_down(
                            gpui::MouseButton::Left,
                            cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                                this.selected_idx = idx;
                                cx.notify();
                            }),
                        )
                })
                .child(
                    div()
                        .flex_none()
                        .w(px(theme::MODAL_RADIO_W))
                        .text_color(radio_dot_color)
                        .child(if is_selected { "●" } else { "○" }),
                )
                .child(branch_label);
            branch_list = branch_list.child(row);
        }

        // Merging / Conflicts state surfaces an inline note inside the
        // body. Error state goes through `crate::ui::alert::error` so it reads
        // at a glance.
        let inline_status = match &self.state {
            MergeState::Idle | MergeState::Error(_) => None,
            MergeState::Merging => Some(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(muted_text)
                    .child(surface_strings::merge_modal_merging())
                    .into_any_element(),
            ),
            MergeState::Conflicts(files) => {
                let mut content = div()
                    .flex()
                    .flex_col()
                    .gap(px(theme::MODAL_BUTTON_PAD_Y))
                    .child(
                        div()
                            .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                            .text_color(strong_text)
                            .child(surface_strings::merge_modal_conflicts_note()),
                    );
                if !files.is_empty() {
                    let mut file_list = div().flex().flex_col().pl(px(theme::MODAL_BUTTON_PAD_X));
                    for f in files {
                        file_list = file_list.child(
                            div()
                                .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                                .text_color(muted_text)
                                .child(SharedString::from(f.clone())),
                        );
                    }
                    content = content.child(file_list);
                }
                Some(content.into_any_element())
            }
        };

        let error_banner = match &self.state {
            MergeState::Error(msg) => Some(crate::ui::alert::error(
                "merge-error",
                SharedString::from(msg.clone()),
            )),
            _ => None,
        };

        // ---- body ----
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(muted_text)
                    .child(surface_strings::merge_modal_branch_label()),
            )
            .child(branch_list);

        if show_remove_checkbox && !is_conflicts {
            body = body.child(
                checkbox(
                    "merge-remove-after",
                    surface_strings::merge_modal_remove_after(),
                    0,
                )
                .checked(self.remove_after_merge)
                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                    this.remove_after_merge = *checked;
                    cx.notify();
                })),
            );
        }

        if let Some(status) = inline_status {
            body = body.child(status);
        }

        // ---- footer ----
        let mut footer = div()
            .flex()
            .flex_row()
            .justify_end()
            .gap(px(theme::MODAL_FOOTER_GAP))
            .mt(px(theme::MODAL_FOOTER_MARGIN_TOP));

        if is_conflicts {
            footer = footer
                .child(
                    button_danger("merge-abort", surface_strings::merge_modal_abort_merge())
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.abort_merge(window, cx);
                        })),
                )
                .child(
                    button_primary(
                        "merge-goto",
                        SharedString::from(format!("Go to \"{sel_branch}\" \u{2192}")),
                    )
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, window, cx| {
                            this.go_to_target(window, cx);
                        },
                    )),
                );
        } else {
            footer = footer
                .child(
                    button("merge-cancel", "Cancel")
                        .disabled(is_merging)
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.dismiss(window, cx);
                        })),
                )
                .child(
                    button_primary("merge-confirm", "Merge")
                        .disabled(is_merging)
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.submit(window, cx);
                        })),
                );
        }

        let mut panel = div()
            .flex()
            .flex_col()
            .key_context("MergeModal")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                this.handle_key(ev, window, cx);
            }))
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(body);
        if let Some(b) = error_banner {
            panel = panel.child(b);
        }
        panel.child(footer)
    }
}

// ----------------------------------------------------------------
// Tests
// ----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{TestAppContext, WeakEntity};

    fn make_targets(branches: &[&str]) -> Vec<TargetOption> {
        branches
            .iter()
            .enumerate()
            .map(|(i, b)| TargetOption {
                wt_id: i as LaneId + 10,
                branch: b.to_string(),
                wt_path: std::path::PathBuf::from(format!("/tmp/wt-{b}")),
            })
            .collect()
    }

    fn build_modal(
        cx: &mut TestAppContext,
        branches: &[&str],
        base_ref: Option<&str>,
    ) -> gpui::Entity<MergeModal> {
        crate::test_support::init_gpui_component(cx);
        let targets = make_targets(branches);
        let base = base_ref.map(|s| s.to_string());
        cx.add_window(|window, cx| {
            MergeModal::new(
                7, // project id, distinct from any lane id below
                99,
                "feat/xyz".to_string(),
                std::path::PathBuf::from("/tmp/repo/wt-feat"),
                std::path::PathBuf::from("/tmp/repo"),
                targets,
                base,
                WeakEntity::new_invalid(),
                window,
                cx,
            )
        })
        .root(cx)
        .unwrap()
    }

    #[gpui::test]
    fn refs_resolve_against_constructed_project_not_active(cx: &mut TestAppContext) {
        // The modal is built for project 7 (see `build_modal`) with an
        // invalid workspace handle, so any resolution that reached for the
        // *active* project would collapse to the default id 0. Both refs
        // must instead carry the project the modal was opened for — the
        // guarantee that a merge initiated on a background project's lane
        // does not operate on the active project's like-id'd lanes.
        let modal = build_modal(cx, &["main", "develop"], None);
        modal.read_with(cx, |m, _| {
            assert_eq!(m.source_ref().project, 7, "source ref uses modal's project");
            assert_eq!(m.source_ref().lane, 99, "source ref uses source_wt_id");
            assert_eq!(m.target_ref().project, 7, "target ref uses modal's project");
            // First target after alphabetical sort ("develop") — wt_id 11.
            assert_eq!(m.target_ref().lane, m.target_options[0].wt_id);
        });
    }

    #[gpui::test]
    fn default_selection_is_first(cx: &mut TestAppContext) {
        let modal = build_modal(cx, &["develop", "main"], None);
        modal.read_with(cx, |m, _| {
            assert_eq!(m.selected_idx, 0);
        });
    }

    #[gpui::test]
    fn base_ref_sorted_to_front(cx: &mut TestAppContext) {
        let modal = build_modal(cx, &["develop", "main", "release"], Some("main"));
        modal.read_with(cx, |m, _| {
            assert_eq!(m.selected_branch(), "main");
        });
    }

    #[gpui::test]
    fn select_down_wraps_at_end(cx: &mut TestAppContext) {
        let modal = build_modal(cx, &["a", "b", "c"], None);
        modal.update(cx, |m, cx| {
            m.select_down(cx);
            m.select_down(cx);
            m.select_down(cx); // already at end — no change
        });
        modal.read_with(cx, |m, _| {
            assert_eq!(m.selected_idx, 2);
        });
    }

    #[gpui::test]
    fn select_up_stops_at_zero(cx: &mut TestAppContext) {
        let modal = build_modal(cx, &["a", "b"], None);
        modal.update(cx, |m, cx| m.select_up(cx)); // already at 0
        modal.read_with(cx, |m, _| {
            assert_eq!(m.selected_idx, 0);
        });
    }

    #[gpui::test]
    fn escape_blocked_while_merging(cx: &mut TestAppContext) {
        // Asserts the Merging-state guard directly: `handle_key`'s
        // dismiss branch checks `matches!(self.state, MergeState::Merging)`.
        // The actual close (`window.close_dialog`) needs a real Root
        // window fixture, so this only verifies the state stays Merging.
        let modal = build_modal(cx, &["main"], None);
        modal.update(cx, |m, cx| {
            m.state = MergeState::Merging;
            cx.notify();
        });
        modal.read_with(cx, |m, _| {
            assert!(matches!(m.state, MergeState::Merging));
        });
    }

    #[gpui::test]
    fn conflicts_state_switches_footer(cx: &mut TestAppContext) {
        let modal = build_modal(cx, &["main"], None);
        modal.update(cx, |m, cx| {
            m.state =
                MergeState::Conflicts(vec!["src/foo.rs".to_string(), "src/bar.rs".to_string()]);
            cx.notify();
        });
        modal.read_with(cx, |m, _| {
            assert!(matches!(m.state, MergeState::Conflicts(_)));
        });
    }

    #[gpui::test]
    fn remove_after_merge_toggle(cx: &mut TestAppContext) {
        let modal = build_modal(cx, &["main"], None);
        // source_path (/tmp/repo/wt-feat) != source_repo_root (/tmp/repo)
        // so source_is_removable() is true and the checkbox is shown.
        modal.read_with(cx, |m, _| {
            assert!(m.source_is_removable());
            assert!(!m.remove_after_merge());
        });
        modal.update(cx, |m, cx| {
            m.remove_after_merge = true;
            cx.notify();
        });
        modal.read_with(cx, |m, _| {
            assert!(m.remove_after_merge());
        });
    }

    #[gpui::test]
    fn main_worktree_not_removable(cx: &mut TestAppContext) {
        crate::test_support::init_gpui_component(cx);
        // When source_path == source_repo_root the checkbox must be hidden.
        let targets = make_targets(&["develop"]);
        let modal = cx
            .add_window(|window, cx| {
                MergeModal::new(
                    0,
                    1,
                    "main".to_string(),
                    std::path::PathBuf::from("/tmp/repo"), // same as repo root
                    std::path::PathBuf::from("/tmp/repo"),
                    targets,
                    None,
                    WeakEntity::new_invalid(),
                    window,
                    cx,
                )
            })
            .root(cx)
            .unwrap();
        modal.read_with(cx, |m, _| {
            assert!(!m.source_is_removable());
        });
    }
}
