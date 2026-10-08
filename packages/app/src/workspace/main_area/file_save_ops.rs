//! Writing a raw file pane back to disk. A save first checks that the file
//! still holds what the pane loaded, since an agent may have edited it since;
//! overwriting that silently would drop the agent's work.

use gpui::{Context, Window};

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};

use super::file_view_pane::PaneFileContent;
use super::file_view_pane::file_content::disk_holds;
use super::pane::FileContent;
use super::pane_tree::PaneId;
use crate::surface::strings;
use crate::workspace::Workspace;

/// What writing a file pane back to disk came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) enum FileSaveOutcome {
    Saved,
    /// The file no longer holds the text the pane loaded or last wrote.
    ChangedOnDisk,
    /// The write failed; already reported.
    Failed,
    /// Not a buffer that saves: a read-only view, or the pane is gone.
    NotSavable,
}

impl Workspace {
    /// ⌘S on the focused file pane.
    pub(in crate::workspace) fn save_focused_file_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pane_id = self.active_runtime().focused_pane_id;
        self.save_file_pane_or_ask(pane_id, false, window, cx);
    }

    /// Save `pane_id`, asking first when the file changed on disk.
    /// `close_after` closes the pane once the buffer is settled — written, or
    /// given up in favour of the disk copy.
    pub(in crate::workspace) fn save_file_pane_or_ask(
        &mut self,
        pane_id: PaneId,
        close_after: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.write_file_pane(pane_id, false, cx) {
            FileSaveOutcome::Saved if close_after => self.close_pane_by_id(pane_id, window, cx),
            FileSaveOutcome::ChangedOnDisk => {
                self.ask_about_save_conflict(pane_id, close_after, window, cx)
            }
            FileSaveOutcome::Saved | FileSaveOutcome::Failed | FileSaveOutcome::NotSavable => {}
        }
    }

    /// Write `pane_id`'s buffer to its file. Without `overwrite` a file that
    /// changed on disk is left alone and reported as
    /// [`FileSaveOutcome::ChangedOnDisk`]. Searches every lane, so a close
    /// prompt can save a pane outside the active one.
    pub(in crate::workspace) fn write_file_pane(
        &mut self,
        pane_id: PaneId,
        overwrite: bool,
        cx: &mut Context<Self>,
    ) -> FileSaveOutcome {
        let Some(fc) = self.file_content_by_id_mut(pane_id) else {
            return FileSaveOutcome::NotSavable;
        };
        if !fc.view.holds_editable_buffer() || !fc.view.path.is_absolute() {
            return FileSaveOutcome::NotSavable;
        }
        let path = fc.view.path.clone();
        if !overwrite && !disk_holds(&path, &fc.saved_text) {
            return FileSaveOutcome::ChangedOnDisk;
        }
        let text = fc.editor_state.read(cx).text().to_string();
        match std::fs::write(&path, text.as_bytes()) {
            Ok(()) => {
                fc.saved_text = text;
                cx.notify();
                FileSaveOutcome::Saved
            }
            Err(e) => {
                let report =
                    ErrorReport::new(strings::error::save_file_failed(path.display().to_string()))
                        .severity(ErrorSeverity::Error)
                        .from_error(&e)
                        .build();
                self.report_error(report, cx);
                FileSaveOutcome::Failed
            }
        }
    }

    fn file_content_by_id_mut(&mut self, pane_id: PaneId) -> Option<&mut FileContent> {
        self.main_area
            .pane_mut(pane_id)
            .and_then(|p| p.file_content_mut())
    }

    fn ask_about_save_conflict(
        &mut self,
        pane_id: PaneId,
        close_after: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(fc) = self.file_content_by_id_mut(pane_id) else {
            return;
        };
        let name = fc
            .view
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| fc.view.path.display().to_string());
        let heading = strings::file_viewer::save_conflict_heading(&name);
        let detail = strings::file_viewer::save_conflict_detail();
        let overwrite = strings::file_viewer::save_conflict_overwrite();
        let reload = strings::file_viewer::save_conflict_reload();
        let cancel = strings::common::btn_cancel();
        let receiver = window.prompt(
            gpui::PromptLevel::Warning,
            &heading,
            Some(&detail),
            &[overwrite.as_str(), reload.as_str(), cancel.as_str()],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            let Ok(answer) = receiver.await else {
                return;
            };
            // SILENT-OK: the window may close before the answer arrives
            let _ = this.update_in(cx, |this, window, cx| match answer {
                0 => {
                    let saved = this.write_file_pane(pane_id, true, cx) == FileSaveOutcome::Saved;
                    if saved && close_after {
                        this.close_pane_by_id(pane_id, window, cx);
                    }
                }
                1 if close_after => this.close_pane_by_id(pane_id, window, cx),
                1 => this.reload_file_pane(pane_id, cx),
                _ => {}
            });
        })
        .detach();
    }

    /// Drop the buffer and read the file again.
    fn reload_file_pane(&mut self, pane_id: PaneId, cx: &mut Context<Self>) {
        let Some(fc) = self.file_content_by_id_mut(pane_id) else {
            return;
        };
        fc.view.content = PaneFileContent::Loading;
        self.load_pending_file_panes(cx);
    }
}
