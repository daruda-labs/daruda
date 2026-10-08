//! Display a startup failure before workspace globals or persistence are ready.

use gpui::{AppContext as _, Application, EmptyView, PromptLevel, QuitMode, WindowOptions};

pub(crate) fn show(detail: String) {
    use crate::surface::strings as s;

    Application::with_platform(gpui_platform::current_platform(false))
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(move |cx| {
            let opened = cx.open_window(WindowOptions::default(), |window, cx| {
                let close = s::common::btn_close();
                let answer = window.prompt(
                    PromptLevel::Critical,
                    &s::startup::failure_heading(),
                    Some(&detail),
                    &[close.as_str()],
                    cx,
                );
                cx.spawn(async move |cx| {
                    // Closing either the prompt or its host window ends startup.
                    let _ = answer.await;
                    cx.update(|cx| cx.quit());
                })
                .detach();
                cx.new(|_| EmptyView)
            });
            if let Err(error) = opened {
                let report = daruda_store::observability::error_report::ErrorReport::new(
                    "Could not display startup failure",
                )
                .from_error::<dyn std::error::Error>(error.as_ref())
                .build();
                // The original failure was already persisted before opening UI.
                if let Err(error) =
                    daruda_store::observability::log_writer::LogWriter::log_sync(&report)
                {
                    crate::platform::report_error(
                        "startup.failure.display",
                        "Cannot persist display failure",
                        &error,
                    );
                }
                cx.quit();
            }
        });
}
