//! Retryable delete confirmation that retains its target and error on failure.

use std::path::PathBuf;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::system_info::redact_home;
use gpui::{Context, SharedString, Window};

use super::super::delete_modal;
use crate::agent::skills::{SkillScope, persist};
use crate::surface::strings;
use crate::workspace::Workspace;

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
        strings::skills::delete_body_prefix(),
        dir.display(),
    );

    delete_modal::open(
        strings::skills::delete_title(),
        body,
        move |_window, app_cx| {
            let Some(ws) = workspace.upgrade() else {
                return Err(strings::common::form_target_gone().into());
            };
            {
                let dir = dir.clone();
                ws.update(app_cx, |ws, cx| {
                    if let Err(e) = persist::delete_skill(&dir) {
                        let message = SharedString::from(e.to_string());
                        let report =
                            ErrorReport::new(crate::surface::strings::error::skill_delete_failed())
                                .severity(ErrorSeverity::Error)
                                .from_error(&e)
                                .at(file!(), line!())
                                .with_context("path", redact_home(&dir))
                                .dedup("skills.delete")
                                .build();
                        ws.report_error(report, cx);
                        return Err(message);
                    }
                    ws.refresh_skills_watcher(cx);
                    Ok(())
                })
            }
        },
        window,
        cx,
    );
}
