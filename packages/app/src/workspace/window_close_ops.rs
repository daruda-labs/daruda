use gpui::{App, Context, Window};

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};

use crate::workspace::close_guard_ops::prompt_stop_running;
use crate::workspace::dirty_items::{DirtyItem, DirtyTarget, dirty_listing};
use crate::workspace::main_area::file_save_ops::FileSaveOutcome;
use crate::workspace::main_area::pane_tree::PaneId;
use crate::workspace::{CloseWindow, Workspace};

impl Workspace {
    /// The app-drawn caption button's close. `window.remove_window()` sets a
    /// flag the teardown reads; it never calls the platform should-close
    /// callback, so taking that shortcut would drop a dirty task-edit draft
    /// without asking. Routing through the same body the hook runs keeps one
    /// answer for "may this window close".
    pub(in crate::workspace) fn on_close_window(
        &mut self,
        _: &CloseWindow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Deferred out of this listener: the gate reads and updates the
        // Workspace, which is checked out for as long as the listener runs.
        let weak = cx.entity().downgrade();
        window.defer(cx, move |window, cx| {
            if Self::may_close_window(&weak, window, cx) {
                window.remove_window();
            }
        });
    }

    /// Register the platform `on_window_should_close` callback that
    /// holds the window open while the batch close prompt runs. The
    /// `WindowRuntime::close_in_flight` flag guards against the callback
    /// firing again while the prompt is on screen.
    pub(in crate::workspace) fn install_window_close_hook(
        weak: gpui::WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak_for_hook = weak.clone();
        window.on_window_should_close(cx, move |window, app| {
            Self::may_close_window(&weak_for_hook, window, app)
        });
    }

    /// `true` when the window may close now; `false` when a prompt is on
    /// screen and will close it later. Shared by the platform callback and
    /// the app-drawn close button.
    pub(in crate::workspace) fn may_close_window(
        weak: &gpui::WeakEntity<Workspace>,
        window: &mut Window,
        app: &mut App,
    ) -> bool {
        Self::may_close_window_then(weak, AfterClose::Stay, false, window, app)
    }

    /// The `Quit` action's handler. Deferred: ⌘Q, the menu and the palette
    /// all arrive inside the focused window's update, where gpui has that
    /// window checked out and the sweep could not reach it. The Dock's Quit
    /// and logout never reach it: gpui_macos answers only
    /// `applicationWillTerminate:`, which cannot hold the quit.
    pub(crate) fn request_quit(cx: &mut App) {
        cx.defer(Self::quit_after_closing_windows);
    }

    /// ⌘Q: put every workspace window through the gate its close button
    /// uses, then quit. A window that raises a prompt stops the sweep; its
    /// answer, once that window has closed, runs the sweep again.
    pub(crate) fn quit_after_closing_windows(cx: &mut App) {
        for handle in crate::window_registry::WindowRegistry::all_handles(cx) {
            let Some(weak) =
                crate::window_registry::WindowRegistry::workspace_for_window(handle, cx)
            else {
                continue;
            };
            let may_close = handle.update(cx, |_, window, app| {
                Self::may_close_window_then(&weak, AfterClose::Quit, false, window, app)
            });
            match may_close {
                Ok(true) => {}
                Ok(false) => return,
                // Removed since the registry listed it (the sweep runs
                // deferred, so no window is checked out here).
                Err(_) => {}
            }
        }
        cx.quit();
    }

    fn may_close_window_then(
        weak: &gpui::WeakEntity<Workspace>,
        after: AfterClose,
        running_confirmed: bool,
        window: &mut Window,
        app: &mut App,
    ) -> bool {
        let Some(ws) = weak.upgrade() else {
            return true;
        };
        // Settings is a body-level view, not a pane, so the dirty-pane
        // sweep below cannot see what one of its fields is holding. Land
        // it through the same funnel Escape and the back button use; a
        // write that failed holds the window open around its banner.
        if !ws.update(app, |this, cx| this.commit_settings_edits(window, cx)) {
            return false;
        }
        if ws.read(app).window_runtime.close_in_flight {
            return false;
        }
        if !running_confirmed {
            let running = {
                let this = ws.read(app);
                this.running_pane_titles(&this.all_pane_ids(), app)
            };
            if !running.is_empty() {
                ws.update(app, |this, _| this.window_runtime.close_in_flight = true);
                let receiver = prompt_stop_running(&running, window, app);
                let weak = weak.clone();
                window
                    .spawn(app, async move |cx| {
                        let answer = receiver.await.unwrap_or(1);
                        // SILENT-OK: the window may be gone before the answer arrives
                        let _ = cx.update(|window, app| {
                            if let Some(ws) = weak.upgrade() {
                                ws.update(app, |this, _| {
                                    this.window_runtime.close_in_flight = false
                                });
                            }
                            if answer == 0
                                && Self::may_close_window_then(&weak, after, true, window, app)
                            {
                                finish_close(after, window, app);
                            }
                        });
                    })
                    .detach();
                return false;
            }
        }

        let dirty = ws.read(app).collect_dirty_items(app);
        if dirty.is_empty() {
            return true;
        }
        ws.update(app, |this, _| {
            this.window_runtime.close_in_flight = true;
        });

        let detail = dirty_listing(&dirty);

        let prompt_heading = crate::surface::strings::task::batch_close_heading();
        let prompt_save = crate::surface::strings::task::batch_save_all();
        let prompt_discard = crate::surface::strings::task::batch_discard_all();
        let prompt_cancel = crate::surface::strings::common::btn_cancel();
        let receiver = window.prompt(
            gpui::PromptLevel::Warning,
            &prompt_heading,
            Some(&detail),
            &[
                prompt_save.as_str(),
                prompt_discard.as_str(),
                prompt_cancel.as_str(),
            ],
            app,
        );

        let weak_inner = weak.clone();
        window
            .spawn(app, async move |cx| {
                let answer = receiver.await.unwrap_or(2);
                // SILENT-OK: workspace may drop during async save-dialog wait
                let _ = weak_inner.update_in(cx, |this, window, cx| {
                    this.window_runtime.close_in_flight = false;
                    match answer {
                        0 => {
                            if this.commit_dirty_items_with_failure_toast(&dirty, window, cx) {
                                finish_close(after, window, cx);
                            }
                        }
                        1 => finish_close(after, window, cx),
                        _ => {} // Cancel — leave the window open
                    }
                });
            })
            .detach();

        false
    }

    /// Save every dirty file, task, and flow editor. Return `false`, with one
    /// dedup'd warning toast naming them, when
    /// any stayed unsaved: the caller then keeps them open rather than
    /// dropping the edits. A file that changed on disk counts as unsaved.
    pub(in crate::workspace) fn commit_dirty_items_with_failure_toast(
        &mut self,
        dirty: &[DirtyItem],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut failed: Vec<gpui::SharedString> = Vec::new();
        for item in dirty {
            let saved = match item.target {
                DirtyTarget::Pane(pane_id) => self.save_pane_for_close(pane_id, window, cx),
                DirtyTarget::TaskEditor(id) => self.commit_task_editor(id, window, cx).is_some(),
            };
            if !saved {
                failed.push(item.title.clone());
            }
        }
        if failed.is_empty() {
            return true;
        }
        let listing = failed
            .iter()
            .map(|t| t.as_ref())
            .collect::<Vec<_>>()
            .join(", ");
        let report = ErrorReport::new(crate::surface::strings::task::batch_save_failed_title())
            .severity(ErrorSeverity::Warning)
            .message(crate::surface::strings::task::batch_save_failed_detail(
                failed.len(),
                &listing,
            ))
            .at(file!(), line!())
            .dedup("tasks.batch_save")
            .build();
        self.report_error(report, cx);
        false
    }

    fn save_pane_for_close(
        &mut self,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if let Some((path, view)) = self
            .main_area
            .pane(pane_id)
            .and_then(|pane| pane.flow_graph_content())
            .map(|graph| (graph.path.clone(), graph.view.clone()))
        {
            return self.save_flow_editor(&path, view, window, cx);
        }
        self.write_file_pane(pane_id, false, cx) == FileSaveOutcome::Saved
    }

    /// Every piece of unsaved work in the window — closing it drops the
    /// parked lanes' panes too. Used by the window-close batch prompt to
    /// summarise pending edits in one modal.
    pub(in crate::workspace) fn collect_dirty_items(&self, cx: &App) -> Vec<DirtyItem> {
        let panes = self
            .main_area
            .runtimes
            .values()
            .flat_map(|rt| rt.panes.iter())
            .filter_map(|p| DirtyItem::of_pane(p, cx));
        let editor = self
            .pages
            .tasks
            .detail
            .iter()
            .filter_map(|d| DirtyItem::of_task_editor(d.id, &d.editor, cx));
        panes.chain(editor).collect()
    }
}

/// What a close that a prompt held open does once it lands.
#[derive(Debug, Clone, Copy)]
enum AfterClose {
    Stay,
    Quit,
}

fn finish_close(after: AfterClose, window: &mut Window, cx: &mut App) {
    window.remove_window();
    if let AfterClose::Quit = after {
        // Deferred: the answer runs inside this window's own update, and the
        // sweep re-enters every workspace.
        cx.defer(Workspace::quit_after_closing_windows);
    }
}
