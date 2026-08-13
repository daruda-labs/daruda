//! Delete confirmation modal — routes through
//! [`crate::workspace::dialog_helpers::open_confirm_dialog`] so every
//! confirm-style dialog in the app shares one opener (consistency
//! with lane / panels delete confirmations).

use std::path::PathBuf;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::system_info::redact_home;
use gpui::{Context, Window};

use crate::agent::skills::{SkillScope, persist};
use crate::surface::strings;
use crate::ui::dialog::ButtonVariant;
use crate::workspace::Workspace;
use crate::workspace::dialog_helpers::open_confirm_dialog;

pub fn open_delete_skill_confirm(
    _ws: &mut Workspace,
    scope: SkillScope,
    dir: PathBuf,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    // Plugin scope is read-only. The renderer hides the × button on
    // plugin rows, but guard here too so a stale dispatch (e.g. from
    // a keybinding the user wires up later) cannot delete marketplace
    // files.
    if !scope.is_writable() {
        return;
    }
    let workspace = cx.weak_entity();
    let body = format!(
        "{}\n\n{}",
        strings::skills_delete_body_prefix(),
        dir.display(),
    );

    open_confirm_dialog(
        strings::skills_delete_title(),
        body,
        strings::skills_button_delete(),
        ButtonVariant::Danger,
        move |_, _window, app_cx| {
            if let Some(ws) = workspace.upgrade() {
                let dir = dir.clone();
                ws.update(app_cx, |ws, cx| {
                    if let Err(e) = persist::delete_skill(&dir) {
                        let report =
                            ErrorReport::new(crate::surface::strings::error_skill_delete_failed())
                                .severity(ErrorSeverity::Error)
                                .from_error(&e)
                                .at(file!(), line!())
                                .with_context("path", redact_home(&dir))
                                .dedup("skills.delete")
                                .build();
                        ws.report_error(report, cx);
                        return;
                    }
                    ws.refresh_skills_watcher(cx);
                });
            }
        },
        window,
        cx,
    );
}
