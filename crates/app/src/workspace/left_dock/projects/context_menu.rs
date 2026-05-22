//! Context menu builder for a worktree row (left-dock worktrees view).
//!
//! Returns a closure compatible with [`crate::ui::ContextMenuExt`] —
//! chain `.context_menu(build_worktree_menu(...))` on the row element.
//! All path / id / branch metadata is captured by value so the closure
//! is `'static` and free of borrows on the row or snapshot.

use gpui::{App, Context, SharedString, Window};

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::project::WorktreeId;

use crate::surface::strings as surface_strings;
use crate::ui::{PopupMenu, PopupMenuItem, menu_builder};

#[allow(clippy::too_many_arguments)]
pub(in crate::workspace) fn build_worktree_menu(
    wt_id: WorktreeId,
    path_str: String,
    current_description: Option<String>,
    current_name: Option<String>,
    workspace: gpui::WeakEntity<crate::workspace::Workspace>,
    is_git: bool,
    is_detached: bool,
    is_dirty: bool,
    source_branch: Option<String>,
    base_ref: Option<String>,
    source_path: std::path::PathBuf,
    source_repo_root: Option<std::path::PathBuf>,
) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu {
    menu_builder(move |menu, _, _| {
        let path_for_reveal = path_str.clone();
        let reveal_item = PopupMenuItem::new(surface_strings::CTX_REVEAL_IN_FINDER).on_click(
            move |_, _, app_cx: &mut App| {
                let path = path_for_reveal.clone();
                app_cx
                    .background_executor()
                    .spawn(async move {
                        std::process::Command::new("open")
                            .args(["-R", &path])
                            .spawn()
                            .ok();
                    })
                    .detach();
            },
        );

        let path_for_copy = path_str.clone();
        let copy_item = PopupMenuItem::new(surface_strings::CTX_COPY_PATH).on_click(
            move |_, _, app_cx: &mut App| {
                app_cx.write_to_clipboard(gpui::ClipboardItem::new_string(path_for_copy.clone()));
            },
        );

        let ws_desc = workspace.clone();
        let current_desc = current_description.clone();
        let edit_description_item = PopupMenuItem::new(surface_strings::CTX_EDIT_DESCRIPTION)
            .on_click(move |_, window, app_cx| {
                let Some(ws) = ws_desc.upgrade() else {
                    return;
                };
                let current = current_desc.clone();
                let callback_ws = ws_desc.clone();
                ws.update(app_cx, |_, cx| {
                    crate::workspace::dialog_helpers::open_single_field_dialog(
                        callback_ws.clone(),
                        surface_strings::EDIT_DESCRIPTION_MODAL_TITLE,
                        surface_strings::EDIT_DESCRIPTION_PLACEHOLDER,
                        current.as_deref(),
                        move |workspace, value, _window, cx| {
                            workspace.set_worktree_description(wt_id, value, cx);
                        },
                        window,
                        cx,
                    );
                });
            });

        let ws_rename = workspace.clone();
        let current_nm = current_name.clone();
        let rename_item =
            PopupMenuItem::new(surface_strings::CTX_RENAME).on_click(move |_, window, app_cx| {
                let Some(ws) = ws_rename.upgrade() else {
                    return;
                };
                let current = current_nm.clone();
                let callback_ws = ws_rename.clone();
                ws.update(app_cx, |_, cx| {
                    crate::workspace::dialog_helpers::open_single_field_dialog(
                        callback_ws.clone(),
                        surface_strings::RENAME_MODAL_TITLE,
                        surface_strings::RENAME_PLACEHOLDER,
                        current.as_deref(),
                        move |workspace, value, _window, cx| {
                            workspace.set_worktree_name(wt_id, value, cx);
                        },
                        window,
                        cx,
                    );
                });
            });

        let mut menu = menu
            .item(reveal_item)
            .item(copy_item)
            .item(edit_description_item)
            .item(rename_item);

        if is_git {
            // PopupMenuItem has no tooltip API, so we can't show per-reason
            // disabled text (detached vs dirty). Both cases just appear greyed out.
            let merge_item = if is_detached || is_dirty {
                PopupMenuItem::new(surface_strings::CTX_MERGE_INTO).disabled(true)
            } else {
                let branch = source_branch.clone().unwrap_or_default();
                let ws_merge = workspace.clone();
                let base_ref = base_ref.clone();
                let src_path = source_path.clone();
                let src_repo_root = source_repo_root.clone();
                PopupMenuItem::new(surface_strings::CTX_MERGE_INTO).on_click(
                    move |_, window, app_cx| {
                        let Some(ws) = ws_merge.upgrade() else {
                            return;
                        };
                        let branch = branch.clone();
                        let base_ref = base_ref.clone();
                        let workspace_weak = ws_merge.clone();
                        let src_path = src_path.clone();
                        let src_repo_root = src_repo_root.clone().unwrap_or_default();
                        ws.update(app_cx, |ws, cx| {
                            let target_options: Vec<super::merge_modal::TargetOption> = ws
                                .active_worktrees()
                                .iter()
                                .filter(|w| w.id != wt_id)
                                .filter_map(|w| match &w.kind {
                                    daruda_store::project::WorktreeKind::Git {
                                        branch: Some(b),
                                        ..
                                    } => Some(super::merge_modal::TargetOption {
                                        wt_id: w.id,
                                        branch: b.clone(),
                                        wt_path: w.path.clone(),
                                    }),
                                    _ => None,
                                })
                                .collect();

                            if target_options.is_empty() {
                                let report =
                                    ErrorReport::new(surface_strings::MERGE_MODAL_NO_TARGETS)
                                        .severity(ErrorSeverity::Info)
                                        .at(file!(), line!())
                                        .dedup("worktree.merge.no_targets")
                                        .build();
                                ws.report_error(report, cx);
                                return;
                            }

                            crate::workspace::dialog_helpers::open_form_modal(
                                SharedString::from(format!("Merge \"{branch}\" into")),
                                None,
                                move |window, cx| {
                                    super::merge_modal::MergeModal::new(
                                        wt_id,
                                        branch.clone(),
                                        src_path,
                                        src_repo_root,
                                        target_options,
                                        base_ref.clone(),
                                        workspace_weak.clone(),
                                        window,
                                        cx,
                                    )
                                },
                                window,
                                cx,
                            );
                        });
                    },
                )
            };
            menu = menu.separator().item(merge_item);
        }

        menu
    })
}

#[cfg(test)]
mod tests {
    // `build_worktree_menu` returns a PopupMenu builder closure whose
    // handlers delegate to Workspace ops already covered by workspace
    // integration tests. Unit tests for the capture semantics and item
    // ordering live indirectly through those integration tests.
}
