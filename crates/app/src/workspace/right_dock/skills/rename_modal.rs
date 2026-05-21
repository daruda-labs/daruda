//! Rename-skill prompt — wraps `SingleFieldModal` to ask the user for
//! a new directory name, validates against the same regex used by
//! Create, and calls `persist::rename_skill` on submit.

use std::path::PathBuf;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::system_info::redact_home;
use gpui::{Context, Window};

use crate::agent::skills::{NameError, SkillScope, persist, validate_name};
use crate::surface::strings;
use crate::workspace::Workspace;
use crate::workspace::dialog_helpers::open_single_field_dialog;

fn report_validation(
    ws: &mut Workspace,
    msg: impl Into<String>,
    dedup: &'static str,
    cx: &mut Context<Workspace>,
) {
    let report = ErrorReport::new(msg)
        .severity(ErrorSeverity::Info)
        .at(file!(), line!())
        .dedup(dedup)
        .build();
    ws.report_error(report, cx);
}

pub fn open_rename_skill_modal(
    _ws: &mut Workspace,
    scope: SkillScope,
    dir: PathBuf,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    let workspace = cx.weak_entity();
    let current_name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    let dir_for_callback = dir.clone();

    open_single_field_dialog(
        workspace,
        strings::skills_button_rename(),
        "new-name",
        Some(&current_name),
        move |ws, value, _window, cx| {
            let dir = dir_for_callback.clone();
            let Some(new_name) = value.map(|s| s.trim().to_string()) else {
                return;
            };
            if new_name.is_empty() {
                report_validation(ws, strings::skills_name_empty(), "skills.rename.empty", cx);
                return;
            }
            match validate_name(&new_name) {
                Ok(()) => {}
                Err(NameError::Empty) => {
                    report_validation(ws, strings::skills_name_empty(), "skills.rename.empty", cx);
                    return;
                }
                Err(NameError::TooLong { .. }) => {
                    report_validation(
                        ws,
                        strings::skills_name_too_long(),
                        "skills.rename.too_long",
                        cx,
                    );
                    return;
                }
                Err(NameError::InvalidChar { .. }) => {
                    report_validation(
                        ws,
                        strings::skills_name_invalid(),
                        "skills.rename.invalid",
                        cx,
                    );
                    return;
                }
                Err(NameError::InvalidLeading { .. }) => {
                    report_validation(
                        ws,
                        strings::skills_name_leading(),
                        "skills.rename.leading",
                        cx,
                    );
                    return;
                }
                Err(NameError::DuplicateInScope { .. }) => unreachable!(),
            }
            let worktree = ws.active_worktree_root();
            if cx
                .global::<crate::agent::skills::SkillsState>()
                .name_exists(scope, &new_name, worktree.as_deref())
            {
                report_validation(
                    ws,
                    strings::skills_name_duplicate(),
                    "skills.rename.duplicate",
                    cx,
                );
                return;
            }
            match persist::rename_skill(&dir, &new_name) {
                Ok(_) => {
                    ws.refresh_skills_watcher(cx);
                }
                Err(e) => {
                    let report = ErrorReport::new("Skill rename failed")
                        .severity(ErrorSeverity::Error)
                        .from_error(&e)
                        .at(file!(), line!())
                        .with_context("path", redact_home(&dir))
                        .with_context("new_name", new_name.clone())
                        .dedup("skills.rename")
                        .build();
                    ws.report_error(report, cx);
                }
            }
        },
        window,
        cx,
    );
}
