//! Right-click context menu items for a lane row.
//!
//! Captures all path / id / branch metadata by value so the resulting
//! `ContextMenuItem` closures are `'static` and free of borrows on the
//! row or snapshot they were built from.

use gpui::{App, MouseDownEvent, SharedString, Window};

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::project::{LaneId, LaneRef, ProjectId};

use crate::lane::availability::LaneAvailability;
use crate::surface::strings as surface_strings;
use crate::ui::ContextMenuItem;

/// Inputs for [`build_context_menu_items`]. Grouped into a struct so
/// the lane-row call site stays readable as per-lane flags accumulate
/// (mirrors `rows::ProjectHeaderArgs`). Every field is owned so the
/// resulting `ContextMenuItem` closures are `'static`.
pub(in crate::workspace) struct CtxMenuArgs {
    pub project_id: ProjectId,
    pub wt_id: LaneId,
    pub path_str: String,
    pub current_description: Option<String>,
    pub current_name: Option<String>,
    pub workspace: gpui::WeakEntity<crate::workspace::Workspace>,
    pub is_git: bool,
    pub is_detached: bool,
    pub is_dirty: bool,
    pub source_branch: Option<String>,
    pub base_ref: Option<String>,
    pub source_path: std::path::PathBuf,
    pub source_repo_root: Option<std::path::PathBuf>,
    /// Read-availability of the lane root. Non-`Present` hides git-only
    /// actions and surfaces a Remove affordance (plus a permission hint
    /// for `AccessDenied`).
    pub availability: LaneAvailability,
    /// Whether the lane is removable as a git worktree
    /// ([`crate::workspace::Workspace::lane_removable`]). Non-removable
    /// (main / default) lanes route Remove through the project-delete
    /// flow instead.
    pub removable: bool,
}

/// Build the context menu item list for a lane row right-click.
/// Captures path / id by value so the closures are `'static`.
pub(in crate::workspace) fn build_context_menu_items(args: CtxMenuArgs) -> Vec<ContextMenuItem> {
    let CtxMenuArgs {
        project_id,
        wt_id,
        path_str,
        current_description,
        current_name,
        workspace,
        is_git,
        is_detached,
        is_dirty,
        source_branch,
        base_ref,
        source_path,
        source_repo_root,
        availability,
        removable,
    } = args;
    let workspace_for_reveal = workspace.clone();
    let path_for_reveal = path_str.clone();
    let reveal_item = ContextMenuItem::new(
        surface_strings::ctx_reveal_in_finder(),
        move |_ev: &MouseDownEvent, _window, app_cx: &mut App| {
            let path = path_for_reveal.clone();
            let workspace = workspace_for_reveal.clone();
            app_cx
                .background_executor()
                .spawn(async move {
                    std::process::Command::new("open")
                        .args(["-R", &path])
                        .spawn()
                        .ok();
                })
                .detach();
            // Close the context menu.
            if let Some(ws) = workspace.upgrade() {
                ws.update(app_cx, |ws, cx| ws.close_context_menu(cx));
            }
        },
    );

    let workspace_for_copy = workspace.clone();
    let path_for_copy = path_str.clone();
    let copy_item = ContextMenuItem::new(
        surface_strings::ctx_copy_path(),
        move |_ev: &MouseDownEvent, _window, app_cx: &mut App| {
            app_cx.write_to_clipboard(gpui::ClipboardItem::new_string(path_for_copy.clone()));
            if let Some(ws) = workspace_for_copy.upgrade() {
                ws.update(app_cx, |ws, cx| ws.close_context_menu(cx));
            }
        },
    );

    let workspace_for_description = workspace.clone();
    let edit_description_item = ContextMenuItem::new(
        surface_strings::ctx_edit_description(),
        move |_ev: &MouseDownEvent, window: &mut Window, app_cx: &mut App| {
            if let Some(ws) = workspace_for_description.upgrade() {
                let current = current_description.clone();
                let callback_ws = workspace_for_description.clone();
                ws.update(app_cx, |ws, cx| {
                    ws.close_context_menu(cx);
                    crate::workspace::dialog_helpers::open_single_field_dialog(
                        callback_ws.clone(),
                        surface_strings::edit_description_modal_title(),
                        surface_strings::edit_description_placeholder(),
                        current.as_deref(),
                        move |workspace, value, _window, cx| {
                            workspace.set_lane_description(wt_id, value, cx);
                        },
                        window,
                        cx,
                    );
                });
            }
        },
    );

    let workspace_for_rename = workspace.clone();
    let rename_item = ContextMenuItem::new(
        surface_strings::ctx_rename(),
        move |_ev: &MouseDownEvent, window: &mut Window, app_cx: &mut App| {
            if let Some(ws) = workspace_for_rename.upgrade() {
                let current = current_name.clone();
                let callback_ws = workspace_for_rename.clone();
                ws.update(app_cx, |ws, cx| {
                    ws.close_context_menu(cx);
                    crate::workspace::dialog_helpers::open_single_field_dialog(
                        callback_ws.clone(),
                        surface_strings::rename_modal_title(),
                        surface_strings::rename_placeholder(),
                        current.as_deref(),
                        move |workspace, value, _window, cx| {
                            workspace.set_lane_name(wt_id, value, cx);
                        },
                        window,
                        cx,
                    );
                });
            }
        },
    );

    let mut items = vec![reveal_item, copy_item, edit_description_item, rename_item];

    // Inaccessible lane (Missing / AccessDenied): git-only actions
    // (Merge into…) are meaningless on a root that can't be read, so
    // hide them and offer Remove instead. AccessDenied additionally
    // surfaces a non-actionable hint pointing at the macOS permission
    // grant — we do NOT force-suggest delete there, because the
    // directory still exists and the user may just need to grant access.
    if availability != LaneAvailability::Present {
        let target = LaneRef {
            project: project_id,
            lane: wt_id,
        };
        let workspace_for_remove = workspace.clone();
        items.push(ContextMenuItem::separator());
        if availability == LaneAvailability::AccessDenied {
            // Disabled, informational only — no handler.
            items.push(
                ContextMenuItem::new(surface_strings::ctx_grant_full_disk_access(), |_, _, _| {})
                    .disabled(true),
            );
        }
        items.push(ContextMenuItem::new(
            surface_strings::ctx_remove(),
            move |_ev: &MouseDownEvent, window: &mut Window, app_cx: &mut App| {
                let Some(ws) = workspace_for_remove.upgrade() else {
                    return;
                };
                ws.update(app_cx, |ws, cx| {
                    ws.close_context_menu(cx);
                    if removable {
                        ws.open_remove_lane_modal(target, window, cx);
                    } else {
                        // Main / default lane stands in for the whole
                        // project. Route Remove through the project-delete
                        // chooser targeted by id — do NOT force focus onto
                        // this (inaccessible) lane, or cancelling would
                        // strand the user on a dead lane. Activation, if
                        // any, happens only once the user confirms inside
                        // the modal.
                        ws.open_delete_project_modal(project_id, window, cx);
                    }
                });
            },
        ));
        return items;
    }

    // "Merge into…" — only for git-backed lanes.
    if is_git {
        let merge_item = if is_detached {
            ContextMenuItem::new(surface_strings::ctx_merge_into(), |_, _, _| {})
                .disabled(true)
                .with_tooltip(surface_strings::ctx_merge_disabled_detached())
        } else if is_dirty {
            ContextMenuItem::new(surface_strings::ctx_merge_into(), |_, _, _| {})
                .disabled(true)
                .with_tooltip(surface_strings::ctx_merge_disabled_dirty())
        } else {
            // source_branch is guaranteed Some when is_git && !is_detached.
            let branch = source_branch.unwrap_or_default();
            let workspace_for_merge = workspace.clone();
            ContextMenuItem::new(
                surface_strings::ctx_merge_into(),
                move |_ev: &MouseDownEvent, window: &mut Window, app_cx: &mut App| {
                    let Some(ws) = workspace_for_merge.upgrade() else {
                        return;
                    };
                    let branch = branch.clone();
                    let base_ref = base_ref.clone();
                    let workspace_weak = workspace_for_merge.clone();
                    // Clone before ws.update so the outer Fn closure can
                    // be called again without violating the Fn constraint.
                    let src_path = source_path.clone();
                    let src_repo_root = source_repo_root.clone().unwrap_or_default();
                    ws.update(app_cx, |ws, cx| {
                        ws.close_context_menu(cx);

                        // Build target list: other git worktrees with a branch.
                        let target_options: Vec<super::merge_modal::TargetOption> = ws
                            .active_lanes()
                            .iter()
                            .filter(|w| w.id != wt_id)
                            .filter_map(|w| match &w.kind {
                                daruda_store::project::LaneKind::Git {
                                    branch: Some(b), ..
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
                                ErrorReport::new(surface_strings::merge_modal_no_targets())
                                    .severity(ErrorSeverity::Info)
                                    .at(file!(), line!())
                                    .dedup("lane.merge.no_targets")
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
        items.push(ContextMenuItem::separator());
        items.push(merge_item);
    }

    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(items: &[ContextMenuItem]) -> Vec<String> {
        items
            .iter()
            .filter_map(|i| match i {
                ContextMenuItem::Item { label, .. } => Some(label.to_string()),
                ContextMenuItem::Separator => None,
            })
            .collect()
    }

    fn disabled_labels(items: &[ContextMenuItem]) -> Vec<String> {
        items
            .iter()
            .filter_map(|i| match i {
                ContextMenuItem::Item {
                    label,
                    disabled: true,
                    ..
                } => Some(label.to_string()),
                _ => None,
            })
            .collect()
    }

    // The closures only *store* the weak workspace ref; they never
    // upgrade it at build time, so an invalid handle is safe here.
    fn build(
        availability: LaneAvailability,
        is_git: bool,
        removable: bool,
    ) -> Vec<ContextMenuItem> {
        build_context_menu_items(CtxMenuArgs {
            project_id: 0,
            wt_id: 1,
            path_str: "/tmp/repo-feat".to_string(),
            current_description: None,
            current_name: None,
            workspace: gpui::WeakEntity::new_invalid(),
            is_git,
            is_detached: false,
            is_dirty: false,
            source_branch: Some("feat/x".to_string()),
            base_ref: None,
            source_path: std::path::PathBuf::from("/tmp/repo-feat"),
            source_repo_root: Some(std::path::PathBuf::from("/tmp/repo")),
            availability,
            removable,
        })
    }

    #[test]
    fn present_git_lane_shows_merge_and_no_remove() {
        let items = build(LaneAvailability::Present, true, true);
        let labels = labels(&items);
        assert!(labels.contains(&surface_strings::ctx_merge_into()));
        assert!(!labels.contains(&surface_strings::ctx_remove()));
    }

    #[test]
    fn missing_lane_hides_merge_and_shows_remove() {
        let items = build(LaneAvailability::Missing, true, true);
        let labels = labels(&items);
        assert!(
            !labels.contains(&surface_strings::ctx_merge_into()),
            "git-only Merge must be hidden for a missing lane"
        );
        assert!(labels.contains(&surface_strings::ctx_remove()));
        // Missing offers no permission hint (only AccessDenied does).
        assert!(!labels.contains(&surface_strings::ctx_grant_full_disk_access()));
    }

    #[test]
    fn access_denied_lane_adds_disabled_permission_hint() {
        let items = build(LaneAvailability::AccessDenied, true, true);
        let labels = labels(&items);
        assert!(!labels.contains(&surface_strings::ctx_merge_into()));
        assert!(labels.contains(&surface_strings::ctx_remove()));
        assert!(
            disabled_labels(&items).contains(&surface_strings::ctx_grant_full_disk_access()),
            "AccessDenied must surface a disabled Full-Disk-Access hint"
        );
    }
}
