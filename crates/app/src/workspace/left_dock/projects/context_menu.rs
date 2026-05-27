//! Right-click context menu items for a lane row.
//!
//! Captures all path / id / branch metadata by value so the resulting
//! `ContextMenuItem` closures are `'static` and free of borrows on the
//! row or snapshot they were built from.

use gpui::{App, MouseDownEvent, SharedString, Window};

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::project::LaneId;

use crate::surface::strings as surface_strings;
use crate::ui::ContextMenuItem;

/// Build the context menu item list for a lane row right-click.
/// Captures path / id by value so the closures are `'static`.
#[allow(clippy::too_many_arguments)]
pub(in crate::workspace) fn build_context_menu_items(
    wt_id: LaneId,
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
) -> Vec<ContextMenuItem> {
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
