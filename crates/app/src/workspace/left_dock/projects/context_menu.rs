//! Right-click context menu items for a lane row.
//!
//! Captures all path / id / branch metadata by value so the resulting
//! `PopupMenuItem` closures are `'static` and free of borrows on the
//! row or snapshot they were built from.

use gpui::{App, ClickEvent, SharedString};

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::system_info::redact_home;
use daruda_store::project::{LaneId, LaneRef, ProjectId};

use crate::lane::availability::LaneAvailability;
use crate::surface::strings as surface_strings;
use crate::ui::PopupMenuItem;
use crate::workspace::render::ws_popup_menu_item;

/// Inputs for [`build_context_menu_items`]. Grouped into a struct so
/// the lane-row call site stays readable as per-lane flags accumulate
/// (mirrors `rows::ProjectHeaderArgs`). Every field is owned so the
/// resulting `PopupMenuItem` closures are `'static`.
pub(in crate::workspace) struct CtxMenuArgs {
    pub project_id: ProjectId,
    pub wt_id: LaneId,
    pub path_str: String,
    pub current_description: Option<String>,
    pub current_remote_cwd: Option<String>,
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
pub(in crate::workspace) fn build_context_menu_items(args: CtxMenuArgs) -> Vec<PopupMenuItem> {
    let CtxMenuArgs {
        project_id,
        wt_id,
        path_str,
        current_description,
        current_remote_cwd,
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
    let reveal_item = PopupMenuItem::new(surface_strings::ctx_reveal_in_finder()).on_click(
        move |_ev: &ClickEvent, _window, app_cx: &mut App| {
            let path = path_for_reveal.clone();
            let workspace = workspace_for_reveal.clone();
            // `open -R` reveals (selects) the path in Finder rather than
            // opening it, so this stays a direct `open` invocation — the
            // `open` crate only launches the default handler.
            app_cx
                .spawn(async move |cx| {
                    let reveal_path = path.clone();
                    let result = cx
                        .background_executor()
                        .spawn(async move {
                            std::process::Command::new("open")
                                .args(["-R", &reveal_path])
                                .spawn()
                                .map(|_| ())
                        })
                        .await;
                    if let Err(e) = result {
                        let report = ErrorReport::new("Reveal in Finder failed")
                            .severity(ErrorSeverity::Warning)
                            .from_error(&e)
                            .at(file!(), line!())
                            .with_context("path", redact_home(&path))
                            .dedup("files.reveal")
                            .build();
                        // SILENT-OK: workspace may drop before the reveal returns
                        let _ = workspace.update(cx, |ws, cx| ws.report_error(report, cx));
                    }
                })
                .detach();
        },
    );

    let copy_item = crate::workspace::render::ws_popup_clipboard_item(
        surface_strings::ctx_copy_path(),
        path_str.clone(),
    );

    let edit_description_item = ws_popup_menu_item(
        workspace.clone(),
        surface_strings::ctx_edit_description(),
        false,
        move |_ws, window, cx| {
            let current = current_description.clone();
            let callback_ws = cx.entity().downgrade();
            crate::workspace::dialog_helpers::open_single_field_dialog(
                callback_ws,
                surface_strings::edit_description_modal_title(),
                surface_strings::edit_description_placeholder(),
                current.as_deref(),
                move |workspace, value, _window, cx| {
                    workspace.set_lane_description(
                        LaneRef {
                            project: project_id,
                            lane: wt_id,
                        },
                        value,
                        cx,
                    );
                },
                window,
                cx,
            );
        },
    );

    let edit_remote_cwd_item = ws_popup_menu_item(
        workspace.clone(),
        surface_strings::ctx_edit_remote_cwd(),
        false,
        move |_ws, window, cx| {
            let current = current_remote_cwd.clone();
            let callback_ws = cx.entity().downgrade();
            crate::workspace::dialog_helpers::open_single_field_dialog(
                callback_ws,
                surface_strings::edit_remote_cwd_modal_title(),
                surface_strings::edit_remote_cwd_placeholder(),
                current.as_deref(),
                move |workspace, value, _window, cx| {
                    workspace.set_lane_remote_cwd(
                        LaneRef {
                            project: project_id,
                            lane: wt_id,
                        },
                        value,
                        cx,
                    );
                },
                window,
                cx,
            );
        },
    )
    // The setting only takes hold for panes created after this edit —
    // an already-created (but not yet connected) agent-chat pane keeps
    // whatever cwd it resolved at creation time. Surfaced as a hover
    // tooltip rather than new modal chrome, mirroring how the disabled
    // Merge item explains itself elsewhere in this menu.
    .tooltip(surface_strings::ctx_edit_remote_cwd_hint());

    let rename_item = ws_popup_menu_item(
        workspace.clone(),
        surface_strings::ctx_rename(),
        false,
        move |_ws, window, cx| {
            let current = current_name.clone();
            let callback_ws = cx.entity().downgrade();
            crate::workspace::dialog_helpers::open_single_field_dialog(
                callback_ws,
                surface_strings::rename_modal_title(),
                surface_strings::rename_placeholder(),
                current.as_deref(),
                move |workspace, value, _window, cx| {
                    workspace.set_lane_name(
                        LaneRef {
                            project: project_id,
                            lane: wt_id,
                        },
                        value,
                        cx,
                    );
                },
                window,
                cx,
            );
        },
    );

    let mut items = vec![
        reveal_item,
        copy_item,
        edit_description_item,
        edit_remote_cwd_item,
        rename_item,
    ];

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
        items.push(PopupMenuItem::separator());
        if availability == LaneAvailability::AccessDenied {
            // Disabled, informational only — no handler.
            items.push(
                PopupMenuItem::new(surface_strings::ctx_grant_full_disk_access()).disabled(true),
            );
        }
        items.push(ws_popup_menu_item(
            workspace.clone(),
            surface_strings::ctx_remove(),
            false,
            move |ws, window, cx| {
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
            },
        ));
        return items;
    }

    // "Merge into…" — only for git-backed lanes.
    if is_git {
        let merge_item = if is_detached {
            PopupMenuItem::new(surface_strings::ctx_merge_into())
                .disabled(true)
                .tooltip(surface_strings::ctx_merge_disabled_detached())
        } else if is_dirty {
            PopupMenuItem::new(surface_strings::ctx_merge_into())
                .disabled(true)
                .tooltip(surface_strings::ctx_merge_disabled_dirty())
        } else {
            // source_branch is guaranteed Some when is_git && !is_detached.
            let branch = source_branch.unwrap_or_default();
            ws_popup_menu_item(
                workspace.clone(),
                surface_strings::ctx_merge_into(),
                false,
                move |ws, window, cx| {
                    let branch = branch.clone();
                    let base_ref = base_ref.clone();
                    let workspace_weak = cx.entity().downgrade();
                    let src_path = source_path.clone();
                    let src_repo_root = source_repo_root.clone().unwrap_or_default();

                    // Build target list: other git worktrees with a
                    // branch — from the *menu's own* project, not the
                    // active one (lane ids collide across projects).
                    let target_options: Vec<super::merge_modal::TargetOption> = ws
                        .project_for(project_id)
                        .map(|p| p.lanes.as_slice())
                        .unwrap_or(&[])
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
                        let report = ErrorReport::new(surface_strings::merge_modal_no_targets())
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
                                project_id,
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
                },
            )
        };
        items.push(PopupMenuItem::separator());
        items.push(merge_item);
    }

    items
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(items: &[PopupMenuItem]) -> Vec<String> {
        items
            .iter()
            .filter_map(|i| match i {
                PopupMenuItem::Item { label, .. } => Some(label.to_string()),
                _ => None,
            })
            .collect()
    }

    fn disabled_labels(items: &[PopupMenuItem]) -> Vec<String> {
        items
            .iter()
            .filter_map(|i| match i {
                PopupMenuItem::Item {
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
    fn build(availability: LaneAvailability, is_git: bool, removable: bool) -> Vec<PopupMenuItem> {
        build_context_menu_items(CtxMenuArgs {
            project_id: 0,
            wt_id: 1,
            path_str: "/tmp/repo-feat".to_string(),
            current_description: None,
            current_remote_cwd: None,
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
