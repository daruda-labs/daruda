//! The orchestrator's window: one `Workspace` with no project, holding one
//! agent-chat pane.
//!
//! It is a real `Workspace` because everything an ACP chat needs is
//! `impl Workspace` — connecting, sending a prompt, the status pulse that
//! detects a settled turn, the Telegram relay, error reporting. A window
//! without one renders but can do none of it.
//!
//! Having no project is not a special case: `Workspace::new` is already the
//! project-less constructor, and `snapshot_for_disk` returns `None` on an
//! empty project list — which is exactly the non-persistence this wants.

use gpui::{AnyWindowHandle, App, AppContext as _, WindowOptions};

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;

use crate::orchestrator::config::ResolvedOrchestrator;
use crate::telegram::bridge::PaneRef;
use crate::window_registry::WindowRegistry;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum OpenError {
    /// The orchestrator's own working directory could not be created.
    CwdUnavailable(String),
    /// GPUI refused the window.
    WindowFailed(String),
    /// The window opened but the chat pane could not be seeded, so there is
    /// nothing to send a prompt to.
    NoChatPane,
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CwdUnavailable(e) => write!(f, "orchestrator cwd unavailable: {e}"),
            Self::WindowFailed(e) => write!(f, "orchestrator window failed: {e}"),
            Self::NoChatPane => write!(f, "orchestrator window has no chat pane"),
        }
    }
}

/// `default_data_dir()/orchestrator`. The orchestrator's capability is its
/// tools, not the filesystem — reading and editing code is a lane agent's job
/// — so it is rooted somewhere with nothing in it.
pub(crate) fn cwd() -> Result<std::path::PathBuf, OpenError> {
    let dir = daruda_store::persistence::default_data_dir().join("orchestrator");
    std::fs::create_dir_all(&dir).map_err(|e| OpenError::CwdUnavailable(e.to_string()))?;
    Ok(dir)
}

/// Open the orchestrator's window, seed its chat pane, and return that pane.
///
/// Replaces any window already in the slot rather than stacking: a second one
/// would hold a second session competing for the same phone conversation.
pub(crate) fn open(resolved: &ResolvedOrchestrator, cx: &mut App) -> Result<PaneRef, OpenError> {
    if let Some((handle, _)) = WindowRegistry::orchestrator(cx) {
        WindowRegistry::clear_orchestrator(cx);
        crate::windows::try_update_workspace_window(
            handle,
            cx,
            "orchestrator.replace",
            |window, _cx| window.remove_window(),
        );
    }
    let cwd = cwd()?;
    let config = crate::settings_store::SettingsStore::global(cx).user_arc();
    let opts = WindowOptions {
        // Opened by a phone command, not by the person at the keyboard — so it
        // must not take their focus. `show` still puts it on screen.
        focus: false,
        ..crate::windows::build_window_options(&config)
    };

    let window = cx
        .open_window(opts, |window, cx| {
            let workspace = cx.new(|cx| {
                crate::workspace::Workspace::new(
                    &config,
                    daruda_store::persistence::default_data_dir(),
                    window,
                    cx,
                )
            });
            cx.new(|cx| gpui_component::Root::new(workspace, window, cx))
        })
        .map_err(|e| OpenError::WindowFailed(e.to_string()))?;
    let handle: AnyWindowHandle = window.into();

    match seed(handle, resolved, cwd, cx) {
        Some(pane) => Ok(pane),
        None => {
            // No chat pane means future `/daruda` calls would keep reusing an
            // unusable window.
            WindowRegistry::clear_orchestrator(cx);
            crate::windows::try_update_workspace_window(
                handle,
                cx,
                "orchestrator.unseeded",
                |window, _cx| window.remove_window(),
            );
            Err(OpenError::NoChatPane)
        }
    }
}

/// Claim and seed the freshly opened orchestrator window.
fn seed(
    handle: AnyWindowHandle,
    resolved: &ResolvedOrchestrator,
    cwd: std::path::PathBuf,
    cx: &mut App,
) -> Option<PaneRef> {
    let weak = WindowRegistry::workspace_for_window(handle, cx)?;
    // Register before seeding so a failure below can tear the window down.
    WindowRegistry::register_orchestrator(handle, weak.clone(), cx);
    let workspace = weak.upgrade()?;
    match cx.update_window(handle, |_root, window, cx_w| {
        workspace.update(cx_w, |ws, cx| {
            ws.seed_orchestrator_chat_pane(
                resolved.agent_id.clone(),
                cwd,
                resolved.account,
                window,
                cx,
            )?;
            ws.orchestrator_chat_pane()
        })
    }) {
        Ok(pane) => pane,
        Err(e) => {
            log_open_failure(&OpenError::WindowFailed(e.to_string()));
            None
        }
    }
}

/// Record a failed open. The caller answers the phone with its own wording, so
/// this only makes sure the concrete reason reaches the log.
pub(crate) fn log_open_failure(error: &OpenError) {
    LogWriter::log(
        ErrorReport::new("Orchestrator window failed to open")
            .severity(ErrorSeverity::Error)
            .at(file!(), line!())
            .with_context("reason", error.to_string())
            .dedup("orchestrator.open")
            .build(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_open_failure_has_a_diagnostic() {
        for case in [
            OpenError::CwdUnavailable("permission denied".into()),
            OpenError::WindowFailed("no display".into()),
            OpenError::NoChatPane,
        ] {
            assert!(!case.to_string().is_empty(), "{case:?}");
        }
    }

    #[test]
    fn the_cwd_is_created_under_the_profile_data_dir() {
        let dir = cwd().expect("cwd");
        assert!(dir.is_dir());
        assert!(dir.starts_with(daruda_store::persistence::default_data_dir()));
        assert_eq!(
            dir.file_name().and_then(|n| n.to_str()),
            Some("orchestrator")
        );
    }
}
