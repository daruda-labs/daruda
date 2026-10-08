//! User-initiated diagnostic export from the Help menu.

use daruda_store::observability::{
    error_report::{ErrorReport, ErrorSeverity},
    log_writer::LogWriter,
};
use gpui::App;

pub(crate) fn export(cx: &mut App) {
    let selected = cx.prompt_for_paths(gpui::PathPromptOptions {
        files: false,
        directories: true,
        multiple: false,
        prompt: Some(crate::surface::strings::menu::export_diagnostics().into()),
    });
    cx.spawn(async move |cx| {
        let selection = selected.await;
        let output = match selection {
            Ok(Ok(Some(paths))) if !paths.is_empty() => paths[0].join(format!(
                "daruda-diagnostics-{}.zip",
                chrono::Utc::now().format("%Y%m%dT%H%M%S%fZ")
            )),
            Ok(Err(error)) => {
                cx.update(|cx| report(error.root_cause(), cx));
                return;
            }
            Ok(_) | Err(_) => return,
        };
        let result = cx
            .background_executor()
            .spawn(async move {
                daruda_store::observability::diagnostics::export_current(
                    &output,
                    chrono::Utc::now(),
                )
                .map(|_| output)
            })
            .await;
        match result {
            Ok(output) => {
                if let Err(error) = open::that_detached(&output) {
                    cx.update(|cx| report(&error, cx));
                }
            }
            Err(error) => cx.update(|cx| report(error.as_ref(), cx)),
        }
    })
    .detach();
}

fn report(error: &dyn std::error::Error, cx: &mut App) {
    let report = ErrorReport::new(crate::surface::strings::menu::err_export_diagnostics())
        .severity(ErrorSeverity::Warning)
        .from_error(error)
        .at(file!(), line!())
        .dedup("diagnostics.export")
        .build();
    let mut pending = Some(report);
    crate::window_registry::WindowRegistry::for_each_workspace(cx, |ws, _, cx| {
        if let Some(report) = pending.take() {
            ws.report_error(report, cx);
        }
    });
    if let Some(report) = pending {
        LogWriter::log(report);
    }
}
