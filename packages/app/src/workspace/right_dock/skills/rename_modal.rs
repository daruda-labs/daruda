//! Rename-skill prompt — a validated single-field dialog asking for a new
//! directory name. A name the Create rules refuse stays in the dialog with the
//! reason under it; a passing one goes to `persist::rename_skill`.

use std::path::PathBuf;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::system_info::redact_home;
use gpui::{Context, Window};

use crate::agent::skills::{NameError, SkillScope, persist, validate_name};
use crate::surface::strings;
use crate::workspace::Workspace;
use crate::workspace::dialog_helpers::open_validated_single_field_dialog;

pub fn open_rename_skill_modal(
    ws: &mut Workspace,
    scope: SkillScope,
    dir: PathBuf,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    if !scope.is_writable() || ws.library_context().config.is_none() {
        return;
    }
    let workspace = cx.weak_entity();
    let current_name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_string();
    let dir_for_callback = dir.clone();

    open_validated_single_field_dialog(
        workspace,
        strings::common::btn_rename(),
        "new-name",
        Some(&current_name),
        move |ws, value, _window, cx| {
            let dir = dir_for_callback.clone();
            let Some(new_name) = value else {
                return Err(strings::common::name_required().into());
            };
            // The name it already has: nothing to rename, and not a duplicate.
            if dir.file_name() == Some(std::ffi::OsStr::new(&new_name)) {
                return Ok(());
            }
            match validate_name(&new_name) {
                Ok(()) => {}
                Err(NameError::Empty) => return Err(strings::common::name_required().into()),
                Err(NameError::TooLong { .. }) => {
                    return Err(strings::skills::name_too_long().into());
                }
                Err(NameError::InvalidChar { .. }) => {
                    return Err(strings::skills::name_invalid().into());
                }
                Err(NameError::InvalidLeading { .. }) => {
                    return Err(strings::skills::name_leading().into());
                }
                Err(NameError::DuplicateInScope { .. }) => unreachable!(),
            }
            if dir.parent().is_some_and(|p| p.join(&new_name).exists()) {
                return Err(strings::skills::name_duplicate().into());
            }
            match persist::rename_skill(&dir, &new_name) {
                Ok(_) => {
                    ws.refresh_library(cx);
                }
                Err(e) => {
                    let report =
                        ErrorReport::new(crate::surface::strings::error::skill_rename_failed())
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
            Ok(())
        },
        window,
        cx,
    );
}
