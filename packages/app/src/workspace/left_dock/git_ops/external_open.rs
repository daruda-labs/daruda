//! Handing a file to an app outside daruda — the preferred external
//! editor, else the OS default handler.

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::system_info::redact_home;
use daruda_store::project::LaneRef;
use gpui::Context;

use crate::workspace::Workspace;

mod launcher;
mod request;

impl Workspace {
    /// Open `path` from lane `lane_id` using the system default
    /// application. Runs the `open` command on a background thread so the
    /// UI thread is never blocked.
    ///
    /// `path` may be either lane-relative (Files left-dock convention) or
    /// absolute (Git Changes left-dock uses repo-root-relative paths and joins
    /// against repo_root before calling). `Path::join` returns the absolute
    /// argument unchanged, so the same code handles both cases.
    ///
    /// Launches `self.mirrors.preferred_editor` (`daruda_config::editor` preset name,
    /// Settings → External Editor) when set and recognized; an empty or
    /// unrecognized preference falls back to the OS default handler, same as
    /// before that setting existed.
    pub(in crate::workspace) fn open_file_externally(
        &mut self,
        lane: LaneRef,
        path: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        let Some(wt) = self.lane_for(lane) else {
            return;
        };
        let full_path = wt.path.join(&path);
        let preset = daruda_config::external_editor_preset(&self.mirrors.preferred_editor);
        self.spawn_external_open(full_path, preset, cx);
    }

    /// Open `path` in the OS default handler, ignoring the external-editor
    /// preference: for an image, a PDF or a directory the editor is the wrong
    /// tool, and the OS already knows the right one.
    pub(in crate::workspace) fn open_path_with_system_default(
        &mut self,
        path: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        self.spawn_external_open(path, None, cx);
    }

    /// The file viewer's way out of its binary placeholder: the file it is
    /// showing, handed to the OS default handler. `path` follows the viewer's
    /// own convention (absolute, or lane-relative for legacy state), same as
    /// [`Self::open_file_externally`].
    pub(in crate::workspace) fn open_lane_file_with_system_default(
        &mut self,
        target: LaneRef,
        path: std::path::PathBuf,
        cx: &mut Context<Self>,
    ) {
        let Some(wt) = self.lane_for(target) else {
            return;
        };
        let full_path = wt.path.join(&path);
        self.spawn_external_open(full_path, None, cx);
    }

    /// `open::that_detached` (the no-preset path) launches the default
    /// handler without blocking on it; the launcher waits on its own
    /// short-lived launcher commands so a failed candidate is detected.
    fn spawn_external_open(
        &mut self,
        full_path: std::path::PathBuf,
        preset: Option<&'static daruda_config::ExternalEditorPreset>,
        cx: &mut Context<Self>,
    ) {
        let request = request::prepare(full_path, preset, cx);

        crate::workspace::spawn_helpers::spawn_bg_work_and_mutate(
            cx,
            move || request.run(),
            |ws, (full_path, result), cx| {
                if let Err(e) = result {
                    let report = ErrorReport::new(
                        crate::surface::strings::error::open_file_external_failed(),
                    )
                    .severity(ErrorSeverity::Warning)
                    .from_error(&e)
                    .at(file!(), line!())
                    .with_context("path", redact_home(&full_path))
                    .dedup("files.open_external")
                    .build();
                    ws.report_error(report, cx);
                }
            },
        )
        .detach();
    }

    #[cfg(test)]
    pub(in crate::workspace) fn take_external_open_requests(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Vec<request::Request> {
        request::take_requests(cx)
    }
}
