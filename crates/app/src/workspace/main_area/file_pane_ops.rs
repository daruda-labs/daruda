use gpui::{AppContext as _, Context, Window};

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};

use super::file_view_pane::{CharPos, CharSelection, FileViewMode, PaneFileContent, PaneFileView};
use super::pane::{FileContent, Pane, PaneContent, PaneSpawnError};
use super::pane_tree::{PaneId, PaneLayout};
use crate::workspace::Workspace;

impl Workspace {
    // ---- Focused-pane file-viewer accessors ----
    //
    // Each open file lives in its own `Pane` carrying
    // `PaneContent::File(FileContent)`; "the file viewer" — for
    // action handlers, key contexts, dock highlighting — is
    // whichever file pane currently has focus.

    pub(in crate::workspace) fn focused_file_view(&self) -> Option<&PaneFileView> {
        let id = self.main_area.focused_pane_id;
        self.main_area
            .panes
            .iter()
            .find(|p| p.id == id)
            .and_then(|p| p.file_view())
    }

    /// Focused pane's TerminalView, when the focused pane is a
    /// terminal. Used by command-history picker and other actions
    /// that target the currently-active terminal.
    pub(in crate::workspace) fn focused_terminal_view(
        &self,
    ) -> Option<&gpui::Entity<daruda_terminal::view::TerminalView>> {
        let id = self.main_area.focused_pane_id;
        self.main_area
            .panes
            .iter()
            .find(|p| p.id == id)
            .and_then(|p| p.terminal_view())
    }

    pub(in crate::workspace) fn focused_file_view_mut(&mut self) -> Option<&mut PaneFileView> {
        let id = self.main_area.focused_pane_id;
        self.main_area
            .panes
            .iter_mut()
            .find(|p| p.id == id)
            .and_then(|p| p.file_view_mut())
    }

    pub(in crate::workspace) fn focused_file_content(&self) -> Option<&FileContent> {
        let id = self.main_area.focused_pane_id;
        self.main_area
            .panes
            .iter()
            .find(|p| p.id == id)
            .and_then(|p| p.file_content())
    }

    pub(in crate::workspace) fn focused_file_content_mut(&mut self) -> Option<&mut FileContent> {
        let id = self.main_area.focused_pane_id;
        self.main_area
            .panes
            .iter_mut()
            .find(|p| p.id == id)
            .and_then(|p| p.file_content_mut())
    }

    /// Find any single-pane tab whose pane holds a file viewer.
    /// Returns `(tab_index, pane_id)` when found. Used by
    /// `open_pane_file_view` in preview-tab mode.
    pub(in crate::workspace) fn find_any_file_tab(&self) -> Option<(usize, PaneId)> {
        for (i, tab) in self.main_area.tabs.iter().enumerate() {
            if let PaneLayout::Pane(pane_id) = tab.layout
                && self
                    .main_area
                    .panes
                    .iter()
                    .any(|p| p.id == pane_id && p.file_view().is_some())
            {
                return Some((i, pane_id));
            }
        }
        None
    }

    /// Find an existing single-pane tab showing the given file
    /// (worktree + path + staged). Returns `(tab_index, pane_id)`.
    /// Used by `open_file_in_new_tab` to dedupe.
    pub(in crate::workspace) fn find_existing_file_tab(
        &self,
        worktree_id: daruda_store::project::WorktreeId,
        path: &std::path::Path,
        staged: bool,
    ) -> Option<(usize, PaneId)> {
        for (i, tab) in self.main_area.tabs.iter().enumerate() {
            if let PaneLayout::Pane(pane_id) = tab.layout
                && let Some(pane) = self.main_area.panes.iter().find(|p| p.id == pane_id)
                && let Some(fv) = pane.file_view()
                && fv.worktree_id == worktree_id
                && fv.path == path
                && fv.staged == staged
            {
                return Some((i, pane_id));
            }
        }
        None
    }

    /// Construct a file-viewer `Pane` (no tab side-effects). Allocates
    /// the pane id, creates a per-pane `InputState` for the find panel
    /// (and its subscription), and seeds `PaneFileView` with `Loading`.
    /// Caller is responsible for adding the pane + tab and kicking off
    /// `load_pane_file_content`.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::workspace) fn create_file_pane(
        &mut self,
        worktree_id: daruda_store::project::WorktreeId,
        path: std::path::PathBuf,
        staged: bool,
        file_status: Option<char>,
        view_mode: FileViewMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Pane {
        let pane_id = self.alloc_id();

        let cached_title = path
            .file_name()
            .map(|n| gpui::SharedString::from(n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| gpui::SharedString::from("(file)"));

        let search_input = cx.new(|cx_state| {
            crate::ui::InputState::new(window, cx_state)
                .placeholder(crate::surface::strings::FILE_VIEWER_SEARCH_PLACEHOLDER)
        });
        // The subscription is owned by `FileContent` and dropped with
        // the pane. Capture `pane_id` so the closure can locate the
        // right pane even when focus has moved.
        let search_subscription = cx.subscribe_in(
            &search_input,
            window,
            move |this, inp, ev: &crate::ui::InputEvent, _window, cx| match ev {
                crate::ui::InputEvent::Change => {
                    let query = inp.read(cx).value().to_string();
                    if let Some(pane) = this.main_area.panes.iter_mut().find(|p| p.id == pane_id)
                        && let Some(fv) = pane.file_view_mut()
                    {
                        fv.search_update_query(&query);
                    }
                    this.scroll_file_viewer_to_focused_match();
                    cx.notify();
                }
                crate::ui::InputEvent::PressEnter { .. } => {
                    if let Some(pane) = this.main_area.panes.iter_mut().find(|p| p.id == pane_id)
                        && let Some(fv) = pane.file_view_mut()
                    {
                        fv.search_next_match();
                    }
                    this.scroll_file_viewer_to_focused_match();
                    cx.notify();
                }
                _ => {}
            },
        );

        let focus_handle = cx.focus_handle();
        Pane {
            id: pane_id,
            content: PaneContent::File(FileContent {
                view: PaneFileView {
                    worktree_id,
                    path,
                    staged,
                    file_status,
                    content: PaneFileContent::Loading,
                    view_mode,
                    hide_unchanged: false,
                    char_selection: None,
                    char_anchor: None,
                    is_drag_selecting: false,
                    search: None,
                },
                scroll_handle: gpui::ScrollHandle::new(),
                search_input,
                focus_handle,
                _search_subscription: search_subscription,
                cached_title,
            }),
        }
    }

    /// View-dispatched mouse-down handler for the file viewer.
    /// Coordinate-to-byte conversion is done in the View; this method
    /// owns the resulting state transition (set anchor, set selection,
    /// possibly start drag). No-op if no file viewer is focused.
    pub(in crate::workspace) fn file_view_mouse_down(
        &mut self,
        hit: CharPos,
        shift: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(fv) = self.focused_file_view_mut() else {
            return;
        };
        if shift {
            let anchor = fv.char_anchor.unwrap_or(hit);
            fv.char_selection = Some(CharSelection {
                anchor,
                active: hit,
            });
        } else {
            fv.char_anchor = Some(hit);
            fv.char_selection = Some(CharSelection {
                anchor: hit,
                active: hit,
            });
            fv.is_drag_selecting = true;
        }
        cx.notify();
    }

    /// View-dispatched mouse-move/drag handler for the file viewer.
    /// `still_pressed` is true when the left button is held; `hovered`
    /// is true when the cursor is inside the row hitbox. Branch order
    /// matches the pre-refactor closure exactly:
    ///   1. drag in progress but button released → reset flag, notify
    ///   2. not drag-selecting OR cursor outside hitbox → noop
    ///   3. no anchor → noop
    ///   4. new selection differs from current → set, notify
    ///   5. otherwise → noop (no notify)
    pub(in crate::workspace) fn file_view_mouse_drag(
        &mut self,
        active: CharPos,
        still_pressed: bool,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(fv) = self.focused_file_view_mut() else {
            return;
        };
        if fv.is_drag_selecting && !still_pressed {
            fv.is_drag_selecting = false;
            cx.notify();
            return;
        }
        if !fv.is_drag_selecting || !hovered {
            return;
        }
        let Some(anchor) = fv.char_anchor else {
            return;
        };
        let new_sel = CharSelection { anchor, active };
        if fv.char_selection.as_ref() != Some(&new_sel) {
            fv.char_selection = Some(new_sel);
            cx.notify();
        }
    }

    /// Surface a pane-spawn failure on both the pinned status bar and
    /// the transient toast queue. Shared by `add_tab` and
    /// `split_focused_pane` to report failures with the same wording.
    pub(in crate::workspace) fn report_pane_error(
        &mut self,
        context: &str,
        err: PaneSpawnError,
        cx: &mut Context<Self>,
    ) {
        let msg = format!("{context} failed — {err}");
        self.last_error = Some(msg.clone().into());
        let report = ErrorReport::new(format!("Pane spawn failed: {context}"))
            .severity(ErrorSeverity::Error)
            .from_error(&err)
            .at(file!(), line!())
            .with_context("context", context)
            .dedup("pane.spawn")
            .build();
        self.report_error(report, cx);
    }
}
