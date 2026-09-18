use gpui::{App, AppContext as _, Context, Window};

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};

use super::file_view_pane::{CharPos, FileViewMode, PaneFileView};
use super::pane::{FileContent, Pane, PaneContent, PaneSpawnError};
use super::pane_tree::{PaneId, PaneLayout};
use crate::path_ext::PathExt as _;
use crate::workspace::Workspace;

impl Workspace {
    // ---- Focused-pane file-viewer accessors ----
    //
    // Each open file lives in its own `Pane` carrying
    // `PaneContent::File(FileContent)`; "the file viewer" — for
    // action handlers, key contexts, dock highlighting — is
    // whichever file pane currently has focus.

    pub(in crate::workspace) fn focused_file_view(&self) -> Option<&PaneFileView> {
        let id = self.active_runtime().focused_pane_id;
        self.active_runtime()
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
        let id = self.active_runtime().focused_pane_id;
        self.active_runtime()
            .panes
            .iter()
            .find(|p| p.id == id)
            .and_then(|p| p.terminal_view())
    }

    /// Step into the already-open viewer when `path` is the file the focused
    /// pane is showing; `false` when it is not, leaving the caller to open it.
    ///
    /// This is what gives a left-dock panel's Enter two stages: the first
    /// opens the file and keeps the panel focused so the arrows keep working,
    /// the second walks into the viewer. Without it the panel's focus rule
    /// leaves no keyboard way in at all. Mirrors zed's `git_panel::open_diff`,
    /// which focuses the ProjectDiff when it already shows the selected entry.
    pub(in crate::workspace) fn step_into_open_file_view(
        &mut self,
        path: Option<std::path::PathBuf>,
        staged: bool,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(path) = path else {
            return false;
        };
        let open = self
            .focused_file_view()
            .map(|fv| (fv.lane_id, fv.path.clone(), fv.staged));
        let Some((lane_id, open_path, open_staged)) = open else {
            return false;
        };
        // `staged` is part of the identity for the same reason
        // `find_existing_file_tab` keys on it: the staged diff and the working
        // copy of one path are two different panes, and Enter on one must not
        // walk into the other.
        if lane_id != self.active.lane || open_path != path || open_staged != staged {
            return false;
        }
        let pane = self.active_runtime().focused_pane_id;
        // Entering is a commit — see `release_preview_tab_for_pane`.
        self.release_preview_tab_for_pane(pane);
        self.focus_pane(pane, window, cx);
        true
    }

    pub(in crate::workspace) fn focused_file_view_mut(&mut self) -> Option<&mut PaneFileView> {
        let id = self.active_runtime().focused_pane_id;
        self.active_runtime_mut()
            .panes
            .iter_mut()
            .find(|p| p.id == id)
            .and_then(|p| p.file_view_mut())
    }

    pub(in crate::workspace) fn focused_file_content(&self) -> Option<&FileContent> {
        let id = self.active_runtime().focused_pane_id;
        self.active_runtime()
            .panes
            .iter()
            .find(|p| p.id == id)
            .and_then(|p| p.file_content())
    }

    pub(in crate::workspace) fn focused_file_content_mut(&mut self) -> Option<&mut FileContent> {
        let id = self.active_runtime().focused_pane_id;
        self.active_runtime_mut()
            .panes
            .iter_mut()
            .find(|p| p.id == id)
            .and_then(|p| p.file_content_mut())
    }

    /// Index of the scratch tab — the one a left-dock preview opened and a
    /// later one may take over. `None` when there is none.
    ///
    /// Two things read this and must never disagree: the reuse decision in
    /// `open_pane_file_view`, and the tab strip, which renders this tab in
    /// italics so the user can see which one is about to be taken over.
    ///
    /// Unsaved edits take a tab out of the slot. That single rule does both
    /// jobs: skimming past an edited tab opens beside it instead of throwing
    /// the edits away, and the italic drops the moment the user types — so
    /// what the strip shows is exactly what the reuse will do.
    pub(in crate::workspace) fn preview_tab_index(&self, cx: &App) -> Option<usize> {
        // Multi-tab mode never reuses a tab, so nothing is replaceable and the
        // strip must not italicise one as though it were.
        if !self.file_viewer_preview_tab {
            return None;
        }
        let (i, pane_id) = self.preview_tab_slot()?;
        let pane = self
            .active_runtime()
            .panes
            .iter()
            .find(|p| p.id == pane_id)?;
        (!pane.is_dirty(cx)).then_some(i)
    }

    /// The recorded slot, before the unsaved-edits veto. Private on purpose —
    /// every consumer wants [`Self::preview_tab_index`]'s answer.
    fn preview_tab_slot(&self) -> Option<(usize, PaneId)> {
        let preview_id = self.active_runtime().preview_tab_id?;
        let (i, tab) = self
            .active_runtime()
            .tabs
            .iter()
            .enumerate()
            .find(|(_, t)| t.id == preview_id)?;
        let PaneLayout::Pane(pane_id) = tab.layout else {
            return None;
        };
        self.active_runtime()
            .panes
            .iter()
            .any(|p| p.id == pane_id && p.file_view().is_some())
            .then_some((i, pane_id))
    }

    /// The scratch tab and its pane, for `open_pane_file_view`'s reuse branch.
    pub(in crate::workspace) fn find_preview_file_tab(&self, cx: &App) -> Option<(usize, PaneId)> {
        self.preview_tab_index(cx)?;
        self.preview_tab_slot()
    }

    /// Find an existing single-pane tab showing the given file
    /// (lane + path + staged). Returns `(tab_index, pane_id)`.
    /// Used by `open_file_in_new_tab` to dedupe.
    pub(in crate::workspace) fn find_existing_file_tab(
        &self,
        lane_id: daruda_store::project::LaneId,
        path: &std::path::Path,
        staged: bool,
    ) -> Option<(usize, PaneId)> {
        for (i, tab) in self.active_runtime().tabs.iter().enumerate() {
            if let PaneLayout::Pane(pane_id) = tab.layout
                && let Some(pane) = self.active_runtime().panes.iter().find(|p| p.id == pane_id)
                && let Some(fv) = pane.file_view()
                && fv.lane_id == lane_id
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
        lane_id: daruda_store::project::LaneId,
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
            .unwrap_or_else(|| {
                gpui::SharedString::from(crate::surface::strings::file_viewer_untitled_tab_title())
            });

        let search_input = cx.new(|cx_state| {
            crate::ui::InputState::new(window, cx_state)
                .placeholder(crate::surface::strings::file_viewer_search_placeholder())
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
                    if let Some(pane) = this
                        .active_runtime_mut()
                        .panes
                        .iter_mut()
                        .find(|p| p.id == pane_id)
                        && let Some(fv) = pane.file_view_mut()
                    {
                        fv.search_update_query(&query);
                    }
                    this.scroll_file_viewer_to_focused_match(cx);
                    cx.notify();
                }
                crate::ui::InputEvent::PressEnter { .. } => {
                    if let Some(pane) = this
                        .active_runtime_mut()
                        .panes
                        .iter_mut()
                        .find(|p| p.id == pane_id)
                        && let Some(fv) = pane.file_view_mut()
                    {
                        fv.search_next_match();
                    }
                    this.scroll_file_viewer_to_focused_match(cx);
                    cx.notify();
                }
                _ => {}
            },
        );

        let focus_handle = cx.focus_handle();
        let language = crate::ui::highlighter::language_for_extension(path.extension_str());
        let editor_state = cx.new(|cx_state| {
            gpui_component::input::InputState::new(window, cx_state)
                .multi_line(true)
                .soft_wrap(false)
                .code_editor(language)
        });
        Pane {
            id: pane_id,
            content: PaneContent::File(FileContent {
                view: PaneFileView::loading(lane_id, path, staged, file_status, view_mode),
                scroll_handle: gpui::ScrollHandle::new(),
                search_input,
                focus_handle,
                _search_subscription: search_subscription,
                cached_title,
                editor_state,
                saved_text: String::new(),
                images: Default::default(),
            }),
        }
    }

    /// Save the focused file-view pane to disk (raw mode only).
    pub(in crate::workspace) fn save_focused_file_pane(&mut self, cx: &mut Context<Self>) {
        use super::file_view_pane::PaneFileContent;
        let Some(fc) = self.focused_file_content_mut() else {
            return;
        };
        if !matches!(fc.view.content, PaneFileContent::LoadedRaw)
            || fc.view.staged
            || !fc.view.path.is_absolute()
        {
            return;
        }
        let path = fc.view.path.clone();
        let text = fc.editor_state.read(cx).text().to_string();
        match std::fs::write(&path, text.as_bytes()) {
            Ok(()) => {
                if let Some(fc) = self.focused_file_content_mut() {
                    fc.saved_text = text;
                }
                cx.notify();
            }
            Err(e) => {
                let report = ErrorReport::new(crate::surface::strings::error_save_file_failed(
                    &path.display().to_string(),
                ))
                .severity(ErrorSeverity::Error)
                .from_error(&e)
                .build();
                self.report_error(report, cx);
            }
        }
    }

    /// View-dispatched mouse-down handler for the file viewer.
    /// Coordinate-to-byte conversion is done in the View; the state
    /// transition lives on `PaneFileView::handle_mouse_down`. No-op
    /// when no file viewer is focused.
    pub(super) fn file_view_mouse_down(
        &mut self,
        hit: CharPos,
        shift: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(fv) = self.focused_file_view_mut() else {
            return;
        };
        fv.handle_mouse_down(hit, shift);
        cx.notify();
    }

    /// View-dispatched mouse-move/drag handler. State transition lives
    /// on `PaneFileView::handle_mouse_drag`; we only forward the result
    /// to `cx.notify()` when the model actually changed.
    pub(super) fn file_view_mouse_drag(
        &mut self,
        active: CharPos,
        still_pressed: bool,
        hovered: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(fv) = self.focused_file_view_mut() else {
            return;
        };
        if fv.handle_mouse_drag(active, still_pressed, hovered) {
            cx.notify();
        }
    }

    /// Settle a live file-view selection drag (char or markdown block).
    /// Shared by the workspace mouse-up and the missed-release branch of
    /// the root `on_mouse_move`: a button released outside the window
    /// never reaches the bubble-phase mouse-up, and a markdown block's own
    /// bubble handler only fires when the cursor re-enters over that very
    /// block — re-entering over the body padding or another region would
    /// otherwise leave the selection stuck `InProgress`. The root move
    /// handler spans the whole window, so routing the settle through here
    /// catches the release wherever the cursor lands. No-op when no drag
    /// is in progress (`end_selection_drag` is idempotent).
    pub(in crate::workspace) fn end_file_selection_drag(&mut self, cx: &mut Context<Self>) {
        if let Some(fv) = self.focused_file_view_mut()
            && fv.end_selection_drag()
        {
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
        let msg = crate::surface::strings::error_pane_spawn_status(context, &err.to_string());
        self.last_error = Some(msg.clone().into());
        let report = ErrorReport::new(crate::surface::strings::error_pane_spawn_failed(context))
            .severity(ErrorSeverity::Error)
            .from_error(&err)
            .at(file!(), line!())
            .with_context("context", context)
            .dedup("pane.spawn")
            .build();
        self.report_error(report, cx);
    }
}
