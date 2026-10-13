//! Close requests for tabs and panes that first check for unsaved file edits.

use crate::workspace::Workspace;
use crate::workspace::dirty_items::DirtyItem;
use crate::workspace::main_area::pane_tree::PaneId;
use gpui::{Context, Window};

impl Workspace {
    /// Close every tab in `indices`, asking first about running work, then
    /// with one summary modal about unsaved edits. `indices` must be in
    /// descending order.
    pub(in crate::workspace) fn request_close_tabs_bulk(
        &mut self,
        indices: Vec<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tabs = &self.active_runtime().tabs;
        let pane_ids: Vec<PaneId> = indices
            .iter()
            .filter_map(|&i| tabs.get(i))
            .flat_map(|tab| tab.layout.pane_ids())
            .collect();
        let tab_ids: Vec<u64> = indices
            .iter()
            .filter_map(|&i| tabs.get(i).map(|tab| tab.id))
            .collect();
        let lane = self.active;
        self.confirm_stopping_then(&pane_ids, window, cx, move |this, window, cx| {
            // The prompt may have been up while tabs moved or the lane
            // changed; close what was asked about, found again by id.
            if this.active != lane {
                return;
            }
            let mut indices: Vec<usize> = tab_ids
                .iter()
                .filter_map(|id| this.tab_index_by_id(*id))
                .collect();
            indices.sort_unstable_by(|a, b| b.cmp(a));
            this.close_tabs_checking_edits(indices, window, cx)
        });
    }

    fn tab_index_by_id(&self, tab_id: u64) -> Option<usize> {
        self.active_runtime()
            .tabs
            .iter()
            .position(|tab| tab.id == tab_id)
    }

    fn close_tabs_checking_edits(
        &mut self,
        indices: Vec<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dirty: Vec<DirtyItem> = indices
            .iter()
            .filter_map(|&i| self.active_runtime().tabs.get(i))
            .flat_map(|tab| tab.layout.pane_ids().into_iter())
            .filter_map(|id| {
                let pane = self.active_runtime().panes.iter().find(|p| p.id == id)?;
                DirtyItem::of_pane(pane, cx)
            })
            .collect();

        if dirty.is_empty() {
            for i in &indices {
                self.close_tab_at(*i, window, cx);
            }
            return;
        }

        let detail = crate::workspace::dirty_items::dirty_listing(&dirty);

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
            cx,
        );

        cx.spawn_in(window, async move |this, cx| {
            let Ok(answer) = receiver.await else {
                return;
            };
            // SILENT-OK: user may close window before save-dialog answer arrives
            let _ = this.update_in(cx, |this, window, cx| match answer {
                0 if this.commit_dirty_items_with_failure_toast(&dirty, window, cx) => {
                    for i in &indices {
                        this.close_tab_at(*i, window, cx);
                    }
                }
                1 => {
                    for i in &indices {
                        this.close_tab_at(*i, window, cx);
                    }
                }
                _ => {} // Cancel
            });
        })
        .detach();
    }

    /// Close a whole tab, asking first about running work, then with a single
    /// 3-button prompt about its dirty panes — Save all / Discard all /
    /// Cancel.
    pub(in crate::workspace) fn request_close_tab(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.active_runtime().tabs.get(index) else {
            return;
        };
        // Closing the orchestrator's tab only hides it; nothing stops.
        if self.is_orchestrator_tab(tab) {
            self.close_tab_at(index, window, cx);
            return;
        }
        let pane_ids = tab.layout.pane_ids();
        let tab_id = tab.id;
        let lane = self.active;
        self.confirm_stopping_then(&pane_ids, window, cx, move |this, window, cx| {
            // Found again by id: the tab may have moved while the prompt was up.
            if this.active != lane {
                return;
            }
            if let Some(index) = this.tab_index_by_id(tab_id) {
                this.close_tab_checking_edits(index, window, cx)
            }
        });
    }

    fn close_tab_checking_edits(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.active_runtime().tabs.get(index) else {
            return;
        };

        let dirty: Vec<DirtyItem> = tab
            .layout
            .pane_ids()
            .iter()
            .filter_map(|&id| {
                let pane = self.active_runtime().panes.iter().find(|p| p.id == id)?;
                DirtyItem::of_pane(pane, cx)
            })
            .collect();

        if dirty.is_empty() {
            self.close_tab_at(index, window, cx);
            return;
        }

        let detail = crate::workspace::dirty_items::dirty_listing(&dirty);

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
            cx,
        );

        cx.spawn_in(window, async move |this, cx| {
            let Ok(answer) = receiver.await else {
                return;
            };
            // SILENT-OK: user may close window before save-dialog answer arrives
            let _ = this.update_in(cx, |this, window, cx| match answer {
                0 if this.commit_dirty_items_with_failure_toast(&dirty, window, cx) => {
                    this.close_tab_at(index, window, cx);
                }
                1 => this.close_tab_at(index, window, cx),
                _ => {} // Cancel
            });
        })
        .detach();
    }

    /// Public close entry point: asks first about running work, then walks
    /// pane content through the dirty-prompt before delegating to
    /// `close_pane_by_id`.
    pub(in crate::workspace::main_area) fn request_close_pane(
        &mut self,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_stopping_then(&[pane_id], window, cx, move |this, window, cx| {
            this.close_pane_checking_edits(pane_id, window, cx)
        });
    }

    fn close_pane_checking_edits(
        &mut self,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pane) = self.active_runtime().panes.iter().find(|p| p.id == pane_id) else {
            return;
        };

        if !pane.is_dirty(cx) {
            self.close_pane_by_id(pane_id, window, cx);
            return;
        }

        let title = pane.title(cx);
        let can_save = pane.can_save();
        let is_file = pane.file_content().is_some();

        let heading = crate::surface::strings::task::edit_save_prompt(&title);
        let save_label = crate::surface::strings::common::btn_save();
        let btn_discard = crate::surface::strings::task::edit_discard();
        let btn_cancel = crate::surface::strings::common::btn_cancel();
        let buttons = [
            save_label.as_str(),
            btn_discard.as_str(),
            btn_cancel.as_str(),
        ];

        let receiver = window.prompt(gpui::PromptLevel::Warning, &heading, None, &buttons, cx);

        cx.spawn_in(window, async move |this, cx| {
            let Ok(answer) = receiver.await else {
                return;
            };
            // SILENT-OK: user may close window before save-dialog answer arrives
            let _ = this.update_in(cx, |this, window, cx| match answer {
                // can_save=false means the form is invalid. Leave the pane open.
                0 if can_save && is_file => this.save_file_pane_or_ask(pane_id, true, window, cx),
                0 => {}
                1 => this.close_pane_by_id(pane_id, window, cx),
                _ => {} // Cancel
            });
        })
        .detach();
    }
}
