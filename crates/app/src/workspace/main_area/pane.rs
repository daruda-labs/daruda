//! Pane and TabEntry types + PTY lifecycle management.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use daruda_store::tasks::TaskId;
use daruda_terminal::TerminalSession;
use daruda_terminal::ux::strings as term_strings;
use daruda_terminal::view::{TerminalInput, TerminalLayout, TerminalView};
use gpui::{
    App, Context, Entity, FocusHandle, ScrollHandle, SharedString, Subscription, Task, Window,
    prelude::*,
};
use portable_pty::MasterPty;

use super::file_view_pane::PaneFileView;
use crate::path_ext::PathExt;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::{PaneId, PaneLayout};
use daruda_terminal::pty::{PtyConfig, PtyError, spawn_pty};

/// Errors that can occur while creating a pane.
#[derive(Debug)]
pub(in crate::workspace) enum PaneSpawnError {
    /// PTY open or shell spawn failure.
    Pty(PtyError),
    /// Terminal VT (ghostty_vt) initialization failure.
    Vt(daruda_terminal::VtError),
}

impl std::fmt::Display for PaneSpawnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PaneSpawnError::Pty(e) => write!(f, "PTY: {e}"),
            PaneSpawnError::Vt(e) => write!(f, "VT init: {e}"),
        }
    }
}

impl std::error::Error for PaneSpawnError {}

/// Per-pane content kind. `Terminal` is the PTY-backed shell view;
/// `File` is the pane-area file viewer (raw / preview / diff). Future
/// kinds (chat, diagnostics) plug in here without touching every
/// workspace caller — `Pane` only ever exposes match-dispatched
/// accessors (`title`, `cwd`, `focus_handle`, `render_into`, `resize`).
///
/// `large_enum_variant` is allowed because Box-ing the bigger variant
/// would add an indirection on every render walk for negligible
/// stack-size savings — the enum is owned per-pane (one allocation
/// either way), and the path-hot reads (`title`, `focus_handle`) stay
/// cheaper without the heap hop.
#[allow(clippy::large_enum_variant)]
pub(in crate::workspace) enum PaneContent {
    Terminal(TerminalContent),
    File(FileContent),
    TaskEditPane(TaskEditContent),
}

/// PTY-backed terminal content. Owns the `TerminalView` entity, the
/// PTY master (resize / drop), the stdout-poll task, and the cached
/// title / cwd that OSC 0/2 + OSC 7 scanners feed back to the workspace
/// for tab strip + status bar rendering.
pub(in crate::workspace) struct TerminalContent {
    pub(in crate::workspace) view: Entity<TerminalView>,
    /// `None` when the pane was created via stub (test builds).
    pub(in crate::workspace) master: Option<Arc<dyn MasterPty + Send>>,
    pub(in crate::workspace) cached_title: SharedString,
    /// Cached cwd (OSC 7) — `None` until the shell first reports it.
    pub(in crate::workspace) cached_cwd: Option<PathBuf>,
    pub(in crate::workspace) _stdout_task: Task<()>,
    /// Listens for `TerminalViewEvent`s emitted by the view (e.g. OSC
    /// 1337 attention requests) and dispatches them to platform APIs
    /// gated by `[notifications]` config. Dropped with the pane.
    pub(in crate::workspace) _view_event_subscription: Subscription,
    /// Outgoing channel into the PTY's writer thread. Cloned from
    /// `stdin_tx` at pane-spawn time so `Workspace::send_to_pane`
    /// (R-14, skill invocation, future macros) can write into the
    /// same channel as the user's
    /// keystrokes, e.g. to dispatch a `claude --dangerously-skip-permissions
    /// "$(cat …)"` command line at task start. `None` for stub panes.
    pub(in crate::workspace) pty_input_tx: Option<mpsc::Sender<Vec<u8>>>,
}

/// File-viewer content. Each open file lives in its own `Pane`, owning
/// its body scroll handle, find-panel input, and search subscription
/// so multiple file viewers can coexist (e.g. across tabs / splits)
/// without sharing state. The data shape (`PaneFileView`) stays
/// GPUI-free; the renderer reads it and produces the element.
pub(in crate::workspace) struct FileContent {
    pub(in crate::workspace) view: PaneFileView,
    /// Body scroll handle. Reset to a fresh handle whenever the view
    /// mode changes so the body always starts at the top.
    pub(in crate::workspace) scroll_handle: ScrollHandle,
    /// Find-panel text input. Each pane owns its own.
    pub(in crate::workspace) search_input: Entity<crate::ui::InputState>,
    /// Pane-level focus handle. Used for `Cmd+W` close routing and
    /// non-search key handling. The find-panel uses the input's own
    /// focus handle when open.
    pub(in crate::workspace) focus_handle: FocusHandle,
    /// Keeps the per-pane `InputEvent` subscription alive; dropped
    /// with the pane.
    pub(in crate::workspace) _search_subscription: Subscription,
    /// Tab title — file basename. Set at construction.
    pub(in crate::workspace) cached_title: SharedString,
    /// Code-editor state for raw file editing.
    pub(in crate::workspace) editor_state: Entity<gpui_component::input::InputState>,
    /// Text that was last saved to disk — for dirty comparison.
    pub(in crate::workspace) saved_text: String,
}

/// Markdown-form editor pane for a single Task — replaces the old
/// Create / Edit modals (R-19 / I-1). Lives at the same `PaneLayout::Pane`
/// level as Terminal and File so users can split a TaskEdit alongside
/// a running shell. `task_id = None` means this is a draft (R-19 / I-7):
/// nothing is persisted to `tasks.json` until the user presses
/// `[Save Draft]` or `[Start]`, and the layout serializer skips drafts
/// so they don't survive a session restart.
pub(in crate::workspace) struct TaskEditContent {
    pub(in crate::workspace) task_id: Option<TaskId>,
    pub(in crate::workspace) title_input: Entity<crate::ui::InputState>,
    pub(in crate::workspace) branch_input: Entity<crate::ui::InputState>,
    /// `true` once the user has manually edited the branch field —
    /// further title changes stop auto-deriving the branch so we
    /// don't trample the override (R-19 / I-12).
    pub(in crate::workspace) branch_override: bool,
    pub(in crate::workspace) branch_validation: BranchValidation,
    /// Dropdown that maps user lane picks back to
    /// `Task::base_worktree_path`. The empty-string sentinel value
    /// means "no explicit base — start_task will branch from the
    /// project's active lane at run time"; every other value is
    /// the absolute path of a registered lane, matching the
    /// `Task::base_worktree_path: Option<PathBuf>` schema. Sits in
    /// the focus chain between the prompt editor and the notes
    /// editor (R-19 / C-1 review note).
    pub(in crate::workspace) base_select: Entity<crate::ui::select::SelectState>,
    /// Prompt is stored in `gpui_component::input::InputState` with
    /// `code_editor("markdown")` so it gets line numbers + syntax
    /// highlight (R-20 / I-2). The entity is shared with the renderer
    /// via `crate::ui::markdown_editor(&state)`.
    pub(in crate::workspace) prompt_state: Entity<gpui_component::input::InputState>,
    pub(in crate::workspace) notes_state: Entity<gpui_component::input::InputState>,
    pub(in crate::workspace) auto_execute: bool,
    pub(in crate::workspace) focus_handle: FocusHandle,
    pub(in crate::workspace) cached_title: SharedString,
    /// Baseline snapshot for dirty comparison (R-25 / I-8). Reset to
    /// `current_snapshot()` after every successful save.
    pub(in crate::workspace) saved_snapshot: TaskEditSnapshot,
    pub(in crate::workspace) _subscriptions: Vec<Subscription>,
    /// FS watcher on `<lane>/.daruda/task-<branch>.md`. `None`
    /// when the task is still in `Backlog` (no lane yet) or the
    /// file didn't exist at pane-open time. Dropped with the pane —
    /// `PromptFileWatcherHandle` shuts down the underlying threads.
    pub(in crate::workspace) _prompt_watcher:
        Option<crate::workspace::main_area::prompt_watcher::PromptFileWatcherHandle>,
    /// GPUI-side pump that polls the watcher's debounced channel and
    /// dispatches `handle_prompt_file_changed` (R-20). Dropped with
    /// the pane.
    pub(in crate::workspace) _prompt_pump: Option<Task<()>>,
    /// Trailing `[+ Add subtask…]` row input (R-21). `Submit` (Enter)
    /// dispatches `Workspace::add_subtask` and clears the buffer for
    /// the next entry; the input stays focused so the user can chain
    /// additions.
    pub(in crate::workspace) new_subtask_input: Entity<crate::ui::InputState>,
    /// `Some(subtask_id)` while the user has the matching row in
    /// inline-rename mode (double-click → Enter / blur commits the
    /// new title; Escape cancels). The rename input itself is reused
    /// across rows so only one entity exists, which sidesteps
    /// composition-state churn when switching rename targets mid-IME.
    pub(in crate::workspace) editing_subtask: Option<String>,
    pub(in crate::workspace) editing_subtask_input: Entity<crate::ui::InputState>,
    /// Scroll handle for the form-body absolute scroll container.
    /// `vertical_scrollbar(&handle)` on the relative parent renders
    /// the visible thumb; `track_scroll(&handle)` on the scroll
    /// container hooks up cursor + wheel + scrollbar drag together.
    pub(in crate::workspace) body_scroll_handle: ScrollHandle,
}

/// Result of running `validate_branch` over the current branch-input
/// text. Drives the disabled state of `[Save Draft]` / `[Start]` and
/// the inline red-border + reason label under the field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum BranchValidation {
    /// Empty input → Save will auto-derive from title at submit time.
    Empty,
    /// Passes git ref-name rules.
    Valid,
    /// Fails one of the git ref-name rules. The `reason` is the
    /// short human-readable cause displayed under the field.
    Invalid { reason: SharedString },
}

impl BranchValidation {
    /// Whether the field is currently in an unrecoverable invalid
    /// state — i.e. `Save Draft` / `Start` must stay disabled and the
    /// branch input should render with a red border. `Empty` is *not*
    /// invalid; it just defers the resolution to auto-derive at save
    /// time.
    pub(in crate::workspace) fn is_invalid(&self) -> bool {
        matches!(self, BranchValidation::Invalid { .. })
    }
}

/// Plain-data dirty-comparison baseline for a TaskEdit pane. Lives
/// on `TaskEditContent::saved_snapshot` and is recomputed via
/// `current_snapshot()` on every dirty check / save.
///
/// Newline normalisation (CRLF → LF) lives in `current_snapshot()` so
/// disk files written by external editors don't show as dirty just
/// because of line-ending differences (R-25 risk note).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::workspace) struct TaskEditSnapshot {
    pub(in crate::workspace) title: String,
    pub(in crate::workspace) branch: String,
    pub(in crate::workspace) prompt: String,
    pub(in crate::workspace) notes: String,
    pub(in crate::workspace) auto_execute: bool,
    /// Empty string ↔ `Task::base_worktree_path == None`; non-empty ↔
    /// `Some(PathBuf::from(s))`. Plain `String` (not `Option<String>`)
    /// keeps the dirty-comparison `==` path trivial — the user-facing
    /// sentinel is `""` either way.
    pub(in crate::workspace) base_value: String,
}

/// CRLF → LF normaliser used by both the renderer's snapshot builder
/// and the save path so dirty comparisons never trip on line-ending
/// differences (R-25 risk note).
pub(in crate::workspace) fn normalize_newlines(s: &str) -> String {
    s.replace("\r\n", "\n")
}

impl TaskEditContent {
    /// Build a fresh snapshot of the form's current state for dirty
    /// comparison (R-25). The two markdown editors and the title /
    /// branch inputs are read through their entity handles; newline
    /// endings are normalised so a CRLF disk reload doesn't read as
    /// a user edit.
    pub(in crate::workspace) fn current_snapshot(&self, cx: &App) -> TaskEditSnapshot {
        TaskEditSnapshot {
            title: self.title_input.read(cx).text().to_string(),
            branch: self.branch_input.read(cx).text().to_string(),
            prompt: normalize_newlines(self.prompt_state.read(cx).text().to_string().as_str()),
            notes: normalize_newlines(self.notes_state.read(cx).text().to_string().as_str()),
            auto_execute: self.auto_execute,
            base_value: self
                .base_select
                .read(cx)
                .selected_value()
                .map(|v| v.to_string())
                .unwrap_or_default(),
        }
    }

    /// True when the current form values differ from the last saved
    /// snapshot. The save / discard paths reset `saved_snapshot` to
    /// the value they wrote, so a successful save clears the flag.
    pub(in crate::workspace) fn is_dirty(&self, cx: &App) -> bool {
        self.current_snapshot(cx) != self.saved_snapshot
    }
}

pub(in crate::workspace) struct Pane {
    pub(in crate::workspace) id: PaneId,
    pub(in crate::workspace) content: PaneContent,
}

/// Cache key capturing all view settings that affect `cell_dimensions()`.
/// Adding a new font attribute here forces the cache lookup to account for it.
#[derive(Hash, PartialEq, Eq)]
pub(in crate::workspace) struct FontMetricsKey {
    font_size_bits: u32,
    v_spacing_bits: u32,
    h_spacing_bits: u32,
    font_family_hash: u64,
}

impl FontMetricsKey {
    pub(in crate::workspace) fn from_view(v: &TerminalView) -> Self {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let font = v.font();
        font.family.hash(&mut h);
        format!("{:?}", font.fallbacks).hash(&mut h);
        format!("{:?}", font.features).hash(&mut h);
        format!("{:?}", font.weight).hash(&mut h);
        format!("{:?}", font.style).hash(&mut h);
        Self {
            font_size_bits: v.font_size().to_bits(),
            v_spacing_bits: v.vertical_spacing().to_bits(),
            h_spacing_bits: v.horizontal_spacing().to_bits(),
            font_family_hash: h.finish(),
        }
    }
}

impl PaneContent {
    /// Focus handle for the outer pane-wrapper div's `track_focus` call.
    /// Returns `Some` for content variants rendered as a plain div tree
    /// (no inner GPUI `Entity` that manages its own `track_focus`), and
    /// `None` for variants whose inner `Entity<T>` calls `track_focus` in
    /// its own `Render` implementation.
    ///
    /// **Recipe 7 checklist** — when adding a new variant:
    /// - `Some(&handle)` → content rendered as a plain `div()` tree.
    /// - `None` → content is a GPUI `Entity<T>` whose `T::render` calls
    ///   `track_focus` itself (like `Terminal` via `TerminalView`).
    pub(in crate::workspace) fn wrapper_focus_handle(&self) -> Option<&FocusHandle> {
        match self {
            PaneContent::Terminal(_) => None,
            PaneContent::File(f) => Some(&f.focus_handle),
            PaneContent::TaskEditPane(te) => Some(&te.focus_handle),
        }
    }
}

impl Pane {
    /// Title shown in the tab strip, pane header, and status bar.
    pub(in crate::workspace) fn title(&self) -> SharedString {
        match &self.content {
            PaneContent::Terminal(t) => t.cached_title.clone(),
            PaneContent::File(f) => f.cached_title.clone(),
            PaneContent::TaskEditPane(te) => te.cached_title.clone(),
        }
    }

    /// Filesystem cwd if the content tracks one (Terminal: from OSC 7;
    /// File: the file's parent directory). The Files-view "show parent
    /// of focused file" affordance reuses this. TaskEdit panes don't
    /// have a meaningful cwd until the task transitions to `Running`
    /// and a lane is materialised — return `None` so dock
    /// affordances skip TaskEdit panes.
    pub(in crate::workspace) fn cwd(&self) -> Option<&Path> {
        match &self.content {
            PaneContent::Terminal(t) => t.cached_cwd.as_deref(),
            PaneContent::File(f) => f.view.path.parent(),
            PaneContent::TaskEditPane(_) => None,
        }
    }

    /// Last path component of `cwd`, for compact display in
    /// tab / header / status bar.
    pub(in crate::workspace) fn display_cwd(&self) -> Option<SharedString> {
        cwd_basename(self.cwd())
    }

    /// Focus handle the pane gives to the window when activated.
    /// File panes return their pane-level handle; the find-panel input
    /// has its own handle that takes priority while the panel is open
    /// (search uses `track_focus(&search_input.focus_handle())`).
    pub(in crate::workspace) fn focus_handle(&self, cx: &App) -> FocusHandle {
        match &self.content {
            PaneContent::Terminal(t) => t.view.read(cx).focus_handle().clone(),
            PaneContent::File(f) => f.focus_handle.clone(),
            PaneContent::TaskEditPane(te) => te.focus_handle.clone(),
        }
    }

    /// Typed accessor for sites that legitimately need the
    /// `TerminalView` (font settings broadcast in `apply_config`,
    /// macro panel `send_input`, terminal-only tests, the layout
    /// walker rendering a Terminal pane). Returns `None` when the
    /// content is not a terminal.
    pub(in crate::workspace) fn terminal_view(&self) -> Option<&Entity<TerminalView>> {
        match &self.content {
            PaneContent::Terminal(t) => Some(&t.view),
            PaneContent::File(_) | PaneContent::TaskEditPane(_) => None,
        }
    }

    /// Write `bytes` directly to the pane's PTY stdin. Returns `true`
    /// when the channel accepted the buffer; `false` for non-terminal
    /// panes, stub panes (no `pty_input_tx`), or a writer thread that
    /// has already shut down. Used by `Workspace::send_to_pane` to
    /// dispatch task-level commands (e.g. `claude --dangerously-skip-permissions`)
    /// and skill invocations.
    pub(in crate::workspace) fn send_input(&self, bytes: &[u8]) -> bool {
        match &self.content {
            PaneContent::Terminal(t) => match &t.pty_input_tx {
                Some(tx) => tx.send(bytes.to_vec()).is_ok(),
                None => false,
            },
            PaneContent::File(_) | PaneContent::TaskEditPane(_) => false,
        }
    }

    /// Immutable accessor for the file-viewer state.
    pub(in crate::workspace) fn file_content(&self) -> Option<&FileContent> {
        match &self.content {
            PaneContent::File(f) => Some(f),
            PaneContent::Terminal(_) | PaneContent::TaskEditPane(_) => None,
        }
    }

    /// Mutable accessor for the file-viewer state. Used by action
    /// handlers that want to update the focused pane's file viewer
    /// (search, scroll, mode toggle).
    pub(in crate::workspace) fn file_content_mut(&mut self) -> Option<&mut FileContent> {
        match &mut self.content {
            PaneContent::File(f) => Some(f),
            PaneContent::Terminal(_) | PaneContent::TaskEditPane(_) => None,
        }
    }

    /// Immutable accessor for the TaskEdit pane state. Used by
    /// `Workspace::find_task_edit_pane`, dirty-check helpers (R-25),
    /// and the layout serializer to skip draft panes (R-19 / B-5).
    pub(in crate::workspace) fn task_edit_content(&self) -> Option<&TaskEditContent> {
        match &self.content {
            PaneContent::TaskEditPane(te) => Some(te),
            PaneContent::Terminal(_) | PaneContent::File(_) => None,
        }
    }

    /// Mutable counterpart to `task_edit_content`. Used by save /
    /// validation / watcher callbacks that need to flip
    /// `branch_override`, refresh `saved_snapshot`, or re-render the
    /// pane after a state mutation.
    pub(in crate::workspace) fn task_edit_content_mut(&mut self) -> Option<&mut TaskEditContent> {
        match &mut self.content {
            PaneContent::TaskEditPane(te) => Some(te),
            PaneContent::Terminal(_) | PaneContent::File(_) => None,
        }
    }

    /// True when the pane holds unsaved user edits. Terminal / File
    /// panes are never dirty — `false` rules them out of the close
    /// prompt entirely (R-25 / I-8). TaskEdit panes diff the form
    /// state against `saved_snapshot`.
    pub(in crate::workspace) fn is_dirty(&self, cx: &App) -> bool {
        match &self.content {
            PaneContent::Terminal(_) => false,
            PaneContent::File(f) => {
                use super::file_view_pane::PaneFileContent;
                !f.view.staged
                    && matches!(f.view.content, PaneFileContent::LoadedRaw { .. })
                    && f.editor_state.read(cx).text().to_string() != f.saved_text
            }
            PaneContent::TaskEditPane(te) => te.is_dirty(cx),
        }
    }

    /// True when the pane's `save` path is meaningful for the user.
    pub(in crate::workspace) fn can_save(&self, cx: &App) -> bool {
        match &self.content {
            PaneContent::Terminal(_) => false,
            PaneContent::File(f) => {
                use super::file_view_pane::PaneFileContent;
                !f.view.staged
                    && matches!(f.view.content, PaneFileContent::LoadedRaw { .. })
                    && f.view.path.is_absolute()
            }
            PaneContent::TaskEditPane(te) => {
                !matches!(te.branch_validation, BranchValidation::Invalid { .. })
                    && !te.title_input.read(cx).value().trim().is_empty()
            }
        }
    }

    /// True when the tab strip should paint a small `●` next to the
    /// pane's title to signal unsaved edits (R-25 / Zed tab indicator).
    pub(in crate::workspace) fn tab_dirty_dot(&self, cx: &App) -> bool {
        self.is_dirty(cx)
    }

    /// Convenience accessor — file viewer data only.
    pub(in crate::workspace) fn file_view(&self) -> Option<&PaneFileView> {
        self.file_content().map(|f| &f.view)
    }

    /// Mutable convenience accessor for the file viewer data.
    pub(in crate::workspace) fn file_view_mut(&mut self) -> Option<&mut PaneFileView> {
        self.file_content_mut().map(|f| &mut f.view)
    }

    /// Apply OSC-derived state (title from OSC 0/2, cwd from OSC 7)
    /// from the stdout-poll task. Returns `true` when at least one
    /// cached field changed so the caller can guard `cx.notify` and
    /// avoid spamming the render tree on idempotent OSC repeats.
    /// No-op when the pane is not a terminal.
    pub(in crate::workspace) fn update_cached_terminal(
        &mut self,
        new_title: String,
        new_cwd: Option<PathBuf>,
    ) -> bool {
        match &mut self.content {
            PaneContent::Terminal(t) => t.update_cached(new_title, new_cwd),
            PaneContent::File(_) | PaneContent::TaskEditPane(_) => false,
        }
    }

    /// Compute grid dimensions and propagate to the underlying
    /// transport. Per-content dispatch — Terminal resizes the PTY +
    /// view; File content has no grid to resize and returns `true`
    /// (counts as "measured") so workspace doesn't keep retrying.
    pub(in crate::workspace) fn resize(
        &self,
        avail_w: f32,
        avail_h: f32,
        pane_header_h: f32,
        cache: &mut std::collections::HashMap<FontMetricsKey, TerminalLayout>,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> bool {
        match &self.content {
            PaneContent::Terminal(t) => {
                t.resize_to_fit(avail_w, avail_h, pane_header_h, cache, window, cx)
            }
            PaneContent::File(_) | PaneContent::TaskEditPane(_) => true,
        }
    }
}

impl TerminalContent {
    /// Update OSC-derived title / cwd in place. Returns `true` iff a
    /// field actually changed, so the caller can scope `cx.notify` to
    /// real updates and skip idempotent OSC repeats.
    fn update_cached(&mut self, new_title: String, new_cwd: Option<PathBuf>) -> bool {
        let mut changed = false;
        if self.cached_title.as_ref() != new_title {
            self.cached_title = SharedString::from(new_title);
            changed = true;
        }
        if self.cached_cwd != new_cwd {
            self.cached_cwd = new_cwd;
            changed = true;
        }
        changed
    }

    fn resize_to_fit(
        &self,
        avail_w: f32,
        avail_h: f32,
        pane_header_h: f32,
        cache: &mut std::collections::HashMap<FontMetricsKey, TerminalLayout>,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> bool {
        let key = FontMetricsKey::from_view(self.view.read(cx));
        let layout = cache.get(&key).copied().or_else(|| {
            let m = self.view.read(cx).cell_layout(window)?;
            cache.insert(key, m);
            Some(m)
        });

        let Some(layout) = layout else { return false };
        // ghostty_vt render paths are undefined for a 1-column terminal
        // (mirrors Zed's `cell_width * 2` minimum guard).
        let cols = layout.cols(avail_w).max(2);
        let rows = layout.rows((avail_h - pane_header_h).max(1.0));

        if let Some(master) = &self.master {
            let _ = master.resize(portable_pty::PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
        self.view
            .update(cx, |view, cx| view.resize_terminal(cols, rows, cx));
        true
    }
}

/// Dispatch one `TerminalViewEvent` from `pane_id` into the matching
/// platform call, gated by the `[notifications]` config and by the
/// "skip focused pane" rule (which itself only applies when daruda
/// is the foreground app).
fn handle_view_event(
    workspace: &mut Workspace,
    pane_id: PaneId,
    event: &daruda_terminal::TerminalViewEvent,
) {
    use crate::platform;
    use crate::surface::{constants::APP_NAME, strings as s};
    use daruda_terminal::{NotificationRequest, TerminalViewEvent};

    // The focused pane is silenced only when daruda itself is the
    // foreground app — backgrounded notifications always surface
    // because the user, by definition, is not looking at the pane.
    let suppressed_by_focus = workspace.notifications.skip_focused_pane
        && platform::attention::is_app_active()
        && workspace.main_area.focused_pane_id == pane_id;

    match event {
        TerminalViewEvent::AttentionRequested(kind) => {
            // macOS auto-suppresses dock bounce when the app is
            // foreground, so we don't apply the focused-pane rule
            // here — the kernel of attention is "tell the user
            // something happened in the background".
            if workspace.notifications.attention_enabled {
                platform::attention::apply(*kind);
            }
        }
        TerminalViewEvent::NotificationRequested(req) => {
            if suppressed_by_focus {
                return;
            }
            match req {
                NotificationRequest::Osc9 { body } => {
                    if workspace.notifications.osc9_enabled {
                        platform::notifications::show(APP_NAME, body);
                    }
                }
                NotificationRequest::Osc777 { title, body } => {
                    if workspace.notifications.osc777_enabled {
                        platform::notifications::show(title, body);
                    }
                }
            }
        }
        TerminalViewEvent::CommandFinishedAfter { elapsed } => {
            if !workspace.notifications.long_running_enabled {
                return;
            }
            let threshold =
                std::time::Duration::from_secs(workspace.notifications.long_running_threshold_secs);
            if *elapsed < threshold {
                return;
            }
            if suppressed_by_focus {
                return;
            }
            let body = s::format_duration_compact(*elapsed);
            let title = s::notification_long_running_title();
            platform::notifications::show(&title, &body);
        }
    }
}

/// Last path component (basename) of a filesystem path.
pub(in crate::workspace) fn cwd_basename(cwd: Option<&std::path::Path>) -> Option<SharedString> {
    let cwd = cwd?;
    let name = cwd.file_name()?.to_string_lossy().into_owned();
    if name.is_empty() {
        None
    } else {
        Some(SharedString::from(name))
    }
}

#[allow(dead_code)]
pub(in crate::workspace) struct TabEntry {
    pub(in crate::workspace) id: u64,
    pub(in crate::workspace) layout: PaneLayout,
    pub(in crate::workspace) last_focused_pane: PaneId,
    /// User-set title (Window > Edit Tab Title…). `None` means the
    /// tab strip falls back to the focused pane's auto-derived title.
    pub(in crate::workspace) user_label: Option<SharedString>,
}

/// All the cwd sources `resolve_default_cwd` chooses between.
/// Named-fields struct so callers can't transpose `active_lane`
/// and `project_root` (both `Option<PathBuf>`); the compiler now
/// catches a mistake that previously silently picked the wrong tier.
#[derive(Debug, Default)]
pub(in crate::workspace) struct CwdCandidates {
    /// The pane the user currently has focus on. Only consulted when
    /// the workspace has `inherit_cwd` on.
    pub focused_pane: Option<PathBuf>,
    /// The active lane's filesystem path. The "always preserve
    /// 1 lane = 1 cwd" tier — wins whenever the focused-pane
    /// path is unavailable or `inherit_cwd` is off.
    pub active_lane: Option<PathBuf>,
    /// Umbrella project root. Last-resort fallback for legacy /
    /// non-lane workspaces; in the steady state it is shadowed
    /// by `active_lane` because every Workspace bootstraps at
    /// least one lane.
    pub project_root: Option<PathBuf>,
}

/// $HOME directory as a last-resort cwd when no workspace-level path is
/// available. Returns `None` only when `HOME` is unset or points at a
/// non-existent path (unusual but possible in sandboxed environments).
fn home_dir() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    home.is_accessible_dir().then_some(home)
}

/// Pure resolver for the cwd a new pane should spawn at. Keeps the
/// "1 lane = 1 cwd" invariant: when no live focused-pane cwd is
/// available, the active lane's path wins over the project root.
///
/// Priority:
/// 1) `candidates.focused_pane` (when `inherit_cwd` is true) — copy
///    from the pane the user is currently looking at,
/// 2) `candidates.active_lane` — keeps `Cmd+T` from a lane
///    pinned inside that lane even before the new shell sends
///    OSC 7,
/// 3) `candidates.project_root` — last-resort fallback for
///    non-lane workspaces.
///
/// The previous resolver fell through to `project_root` ahead of
/// the active lane path, which silently spawned new shells at
/// the main repo root from inside a `daruda-feat-x` lane —
/// breaking isolation for fresh starts, restored sessions before
/// OSC 7 landed, and any `Cmd+T` issued before the focused pane had
/// a reported cwd.
pub(in crate::workspace) fn resolve_default_cwd(
    inherit_cwd: bool,
    candidates: CwdCandidates,
) -> Option<PathBuf> {
    if inherit_cwd && let Some(cwd) = candidates.focused_pane {
        return Some(cwd);
    }
    candidates.active_lane.or(candidates.project_root)
}

impl Workspace {
    /// Default cwd for a new pane. Thin wrapper that gathers the
    /// candidates from `Workspace` state and delegates to
    /// [`resolve_default_cwd`].
    pub(in crate::workspace) fn default_cwd_for_new_pane(&self) -> Option<PathBuf> {
        let candidates = CwdCandidates {
            focused_pane: self
                .main_area
                .panes
                .iter()
                .find(|p| p.id == self.main_area.focused_pane_id)
                .and_then(|p| p.cwd().map(Path::to_path_buf)),
            active_lane: self.active_lane().map(|w| w.path.clone()),
            project_root: self.active_project().map(|p| p.root.clone()),
        };
        resolve_default_cwd(self.inherit_cwd, candidates).or_else(home_dir)
    }

    pub(in crate::workspace) fn create_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Pane, PaneSpawnError> {
        let cwd = self.default_cwd_for_new_pane();
        self.create_pane_with_cwd(cwd, window, cx)
    }

    /// Like `create_pane` but forces a specific initial cwd. Used by
    /// session restore so each restored pane starts at the directory
    /// it last tracked, independent of the focused-pane / project
    /// inheritance rules.
    pub(in crate::workspace) fn create_pane_with_cwd(
        &mut self,
        cwd: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Pane, PaneSpawnError> {
        // Propagate the workspace's terminal config so every pane
        // starts with the same font_size / spacing. Zoom actions
        // diverge each view's runtime font_size individually.
        let config = self.terminal_config;
        // `shell_program` carries the effective shell from
        // `apply_config` (user `[shell]` plus any project override).
        // Falls back to `PtyConfig::default()`'s `$SHELL`/`/bin/zsh`
        // resolution when neither layer pins a program.
        let mut pty_config = PtyConfig {
            cwd,
            ..PtyConfig::default()
        };
        if let Some(program) = self.shell_program.as_deref() {
            pty_config.shell = program.to_string();
        }
        let handle = spawn_pty(&pty_config).map_err(PaneSpawnError::Pty)?;
        let pty_pid = handle.child_pid;
        let (stdin_tx, stdout_rx, exit_rx, error_rx, master) = handle.into_parts();
        // Keep a sibling sender so non-keyboard callers
        // (Workspace::send_to_pane) can write into the same PTY
        // without going through the TerminalView's keyboard handler.
        // The original `stdin_tx` is moved into the TerminalInput
        // closure below, so we clone before that.
        let pty_input_tx = stdin_tx.clone();

        // Create the VT session up front so its error can propagate out
        // of create_pane rather than panicking inside the cx.new closure.
        let session = TerminalSession::new(config).map_err(PaneSpawnError::Vt)?;

        let pane_id = self.alloc_id();

        let font_family = self.font_family.clone();
        let view = cx.new(|cx| {
            let focus_handle = cx.focus_handle();
            let input = TerminalInput::new(move |bytes| {
                let _ = stdin_tx.send(bytes.to_vec());
            });
            let mut tv = TerminalView::new_with_input(session, focus_handle, input);
            tv.set_font(daruda_terminal::terminal_font_with_family(&font_family));
            tv
        });

        let workspace_entity = cx.entity().clone();
        let stdout_task = Self::spawn_stdout_poll(
            view.clone(),
            stdout_rx,
            exit_rx,
            error_rx,
            workspace_entity,
            pane_id,
            window,
            cx,
        );

        // Bridge `TerminalViewEvent`s into Workspace-side platform calls
        // (dock attention, notifications, …). The view emits; the
        // Workspace gates by config and dispatches. `pane_id` is
        // captured by the closure so the focus-gate can identify
        // which pane raised the event without needing entity equality.
        let captured_pane_id = pane_id;
        let view_event_sub = cx.subscribe(
            &view,
            move |this, _view, event: &daruda_terminal::TerminalViewEvent, _cx| {
                handle_view_event(this, captured_pane_id, event);
            },
        );

        // Register the pane's shell PID with the PTY tracker so the
        // sysinfo poller can find `claude` descendants of this pane
        // and bind them back to its session_id (Phase E). Stub
        // panes (no pty_pid) skip registration.
        if let Some(pid) = pty_pid {
            self.claude.pty_tracker.register(pane_id, pid);
        }

        Ok(Pane {
            id: pane_id,
            content: PaneContent::Terminal(TerminalContent {
                view,
                master,
                cached_title: term_strings::FALLBACK_TITLE.into(),
                cached_cwd: None,
                _stdout_task: stdout_task,
                _view_event_subscription: view_event_sub,
                pty_input_tx: Some(pty_input_tx),
            }),
        })
    }

    #[allow(clippy::too_many_arguments)] // PTY plumbing — splitting wraps tax callers more than it saves.
    fn spawn_stdout_poll(
        view: Entity<TerminalView>,
        stdout_rx: mpsc::Receiver<Vec<u8>>,
        exit_rx: mpsc::Receiver<()>,
        error_rx: mpsc::Receiver<daruda_store::observability::error_report::ErrorReport>,
        workspace: Entity<Workspace>,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        window.spawn(cx, async move |cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;

                let mut batch = Vec::new();
                while let Ok(chunk) = stdout_rx.try_recv() {
                    batch.extend_from_slice(&chunk);
                }

                // Drain PTY thread errors (writer/reader death) on
                // every tick so the user sees them within one frame
                // (D5). We enrich each report with pane id + cwd so
                // the user can tell which session died — the PTY
                // threads themselves are GPUI-free and don't have
                // access to the workspace's cached cwd.
                let mut errors = Vec::new();
                while let Ok(report) = error_rx.try_recv() {
                    errors.push(report);
                }
                if !errors.is_empty() {
                    let workspace_for_errors = workspace.clone();
                    // If the workspace window has dropped before this
                    // drain catches up (process exit, last-tab-close
                    // race) the toast pipeline becomes unreachable —
                    // the per-report `report_error` calls would
                    // surface as toasts otherwise. Surface the path
                    // loss on NDJSON so we don't lose track of the
                    // fact that errors were dropped (Iron Law: no
                    // silent failure).
                    let update_result = cx.update(|_, app_cx| {
                        workspace_for_errors.update(app_cx, |ws, cx| {
                            let cwd = ws
                                .main_area
                                .panes
                                .iter()
                                .find(|p| p.id == pane_id)
                                .and_then(|p| p.cwd())
                                .map(daruda_store::observability::system_info::redact_home);
                            for mut report in errors {
                                report
                                    .context
                                    .insert("pane".to_string(), pane_id.to_string());
                                if let Some(cwd) = cwd.clone() {
                                    report.context.insert("cwd".to_string(), cwd);
                                }
                                ws.report_error(report, cx);
                            }
                        });
                    });
                    if let Err(e) = update_result {
                        daruda_store::observability::log_writer::LogWriter::log(
                            daruda_store::observability::error_report::ErrorReport::new(
                                "Pane background errors could not reach the workspace toast layer",
                            )
                            .severity(
                                daruda_store::observability::error_report::ErrorSeverity::Warning,
                            )
                            .at(file!(), line!())
                            .with_context("pane", pane_id.to_string())
                            .with_context("error", format!("{e}"))
                            .dedup("pane.background_error.update_failed")
                            .build(),
                        );
                    }
                }

                // Treat both "sender signaled" and "sender gone" as
                // shell termination so a panicking waiter thread never
                // leaves the pane stuck with a dead shell.
                let exited = matches!(
                    exit_rx.try_recv(),
                    Ok(()) | Err(mpsc::TryRecvError::Disconnected)
                );

                if !batch.is_empty() {
                    let ok = cx.update(|_, cx| {
                        view.update(cx, |this, cx| {
                            this.queue_output_bytes(&batch, cx);
                            // Flush so terminal_title/terminal_cwd reflect the bytes
                            // we just queued — render() may not run before the
                            // workspace.update below reads them.
                            this.flush_pending_output(cx);
                        });
                        let v = view.read(cx);
                        let title = v.terminal_title().to_string();
                        let cwd = v.terminal_cwd().map(PathBuf::from);
                        workspace.update(cx, |ws, cx| {
                            if let Some(pane) =
                                ws.main_area.panes.iter_mut().find(|p| p.id == pane_id)
                                && pane.update_cached_terminal(title, cwd)
                            {
                                cx.notify();
                            }
                        });
                    });
                    if ok.is_err() {
                        break;
                    }
                }

                if exited {
                    // Read the config flag now, before any sibling
                    // spawn, so the read and the later update never
                    // share a closure — avoids any ambiguity with
                    // GPUI's reentrant-update guard.
                    let should_close = cx
                        .update(|_, app_cx| workspace.read(app_cx).mirrors.close_pane_on_exit)
                        .unwrap_or(false);

                    if should_close {
                        // Self-drop hazard: calling `close_pane_by_id`
                        // inline would remove our own Pane and drop
                        // this very Task mid-poll. A sibling task runs
                        // after we return, so our future completes
                        // before its owner is freed.
                        let workspace_close = workspace.clone();
                        cx.spawn(async move |cx| {
                            // SILENT-OK: pane owner may drop during async task cleanup
                            let _ = cx.update(|window, app_cx| {
                                workspace_close.update(app_cx, |ws, cx| {
                                    ws.close_pane_by_id(pane_id, window, cx);
                                });
                            });
                        })
                        .detach();
                    }
                    break;
                }
            }
        })
    }

    pub(in crate::workspace) fn focus_pane(
        &self,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(pane) = self.main_area.panes.iter().find(|p| p.id == pane_id) {
            let handle = pane.focus_handle(cx);
            handle.focus(window, cx);
        }
    }
}
