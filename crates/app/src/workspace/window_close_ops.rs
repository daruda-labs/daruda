use gpui::{App, Context, Window};

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};

use crate::workspace::Workspace;
use crate::workspace::main_area::pane::PaneContent;
use crate::workspace::main_area::pane_tree::PaneId;

impl Workspace {
    /// Register the platform `on_window_should_close` callback that
    /// holds the window open while the R-25 batch prompt runs. The
    /// `window_close_in_flight` flag guards against the callback
    /// firing again while the prompt is on screen.
    pub(in crate::workspace) fn install_window_close_hook(
        weak: gpui::WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak_for_hook = weak.clone();
        window.on_window_should_close(cx, move |window, app| {
            let Some(ws) = weak_for_hook.upgrade() else {
                return true;
            };
            let dirty = ws.read(app).collect_dirty_pane_descriptors(app);
            if dirty.is_empty() {
                return true;
            }
            if ws.read(app).window_close_in_flight {
                return false;
            }
            ws.update(app, |this, _| {
                this.window_close_in_flight = true;
            });

            let detail = dirty
                .iter()
                .map(|(_, t, draft)| {
                    if *draft {
                        format!("• {} (new task)", t)
                    } else {
                        format!("• {}", t)
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");

            let receiver = window.prompt(
                gpui::PromptLevel::Warning,
                crate::surface::strings::TAB_CLOSE_BATCH_HEADING,
                Some(&detail),
                &[
                    crate::surface::strings::TAB_CLOSE_BATCH_SAVE_ALL,
                    crate::surface::strings::TAB_CLOSE_BATCH_DISCARD_ALL,
                    crate::surface::strings::TASK_EDIT_CANCEL,
                ],
                app,
            );

            let weak_inner = weak_for_hook.clone();
            window
                .spawn(app, async move |cx| {
                    let answer = receiver.await.unwrap_or(2);
                    // SILENT-OK: workspace may drop during async save-dialog wait
                    let _ = weak_inner.update_in(cx, |this, window, cx| {
                        this.window_close_in_flight = false;
                        match answer {
                            0 => {
                                this.commit_dirty_panes_with_failure_toast(&dirty, cx);
                                window.remove_window();
                            }
                            1 => window.remove_window(),
                            _ => {} // Cancel — leave the window open
                        }
                    });
                })
                .detach();

            false
        });
    }

    /// Walk every entry in `dirty` and call `commit_task_edit_pane`.
    /// Surfaces a single dedup'd warning toast naming panes whose
    /// commit returned `None`.
    pub(in crate::workspace) fn commit_dirty_panes_with_failure_toast(
        &mut self,
        dirty: &[(PaneId, gpui::SharedString, bool)],
        cx: &mut Context<Self>,
    ) {
        let mut failed: Vec<gpui::SharedString> = Vec::new();
        for (pane_id, title, _is_draft) in dirty {
            if self.commit_task_edit_pane(*pane_id, cx).is_none() {
                failed.push(title.clone());
            }
        }
        if failed.is_empty() {
            return;
        }
        let listing = failed
            .iter()
            .map(|t| t.as_ref())
            .collect::<Vec<_>>()
            .join(", ");
        let report = ErrorReport::new(crate::surface::strings::TASK_BATCH_SAVE_FAILED_TITLE)
            .severity(ErrorSeverity::Warning)
            .message(format!(
                "{} pane(s) had invalid input and were not saved: {}",
                failed.len(),
                listing,
            ))
            .at(file!(), line!())
            .dedup("tasks.batch_save")
            .build();
        self.report_error(report, cx);
    }

    /// Snapshot of every dirty TaskEdit pane. Used by the window-close
    /// batch prompt (R-25) to summarise pending edits in one modal.
    /// Returns `(pane_id, title, is_draft)` triples.
    pub(in crate::workspace) fn collect_dirty_pane_descriptors(
        &self,
        cx: &App,
    ) -> Vec<(PaneId, gpui::SharedString, bool)> {
        self.main_area
            .panes
            .iter()
            .filter_map(|p| {
                if !p.is_dirty(cx) {
                    return None;
                }
                let is_draft = matches!(
                    &p.content,
                    PaneContent::TaskEditPane(te) if te.task_id.is_none()
                );
                Some((p.id, p.title(), is_draft))
            })
            .collect()
    }
}
