//! Pane and TabEntry types + PTY lifecycle management.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use daruda_store::project::PaneCwd;
use daruda_store::tasks::{TaskAgentSurface, TaskId};
use daruda_terminal::ux::strings as term_strings;
use daruda_terminal::view::{TerminalInput, TerminalLayout, TerminalView};
use daruda_terminal::{TerminalDims, TerminalSession};
use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use futures::future::Either;
use gpui::{
    App, Context, Entity, FocusHandle, Focusable as _, ScrollHandle, SharedString, Subscription,
    Task, Window, prelude::*,
};
use portable_pty::MasterPty;

use super::agent_chat_pane::view::AgentChatView;
use super::file_view_pane::PaneFileView;
use crate::agent::account::PreparedAccount;
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

/// Per-pane content kind. `Pane` only ever exposes match-dispatched
/// accessors (`title`, `cwd`, `focus_handle`, `resize`), so new kinds plug
/// in here without touching every caller.
///
/// `large_enum_variant` is allowed: the enum is owned per-pane (one
/// allocation either way), so Box-ing the big variant only adds a heap hop
/// to the path-hot reads for negligible stack savings.
#[allow(clippy::large_enum_variant)]
pub(in crate::workspace) enum PaneContent {
    Terminal(TerminalContent),
    File(FileContent),
    TaskEditPane(TaskEditContent),
    AgentChat(AgentChatContent),
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
    /// Outgoing channel into the PTY's writer thread, cloned from `stdin_tx`
    /// so `Workspace::send_to_pane` (skills, macros) can write into the same
    /// channel as the user's keystrokes. `None` for stub panes.
    pub(in crate::workspace) pty_input_tx: Option<mpsc::Sender<Vec<u8>>>,
    /// Wakes the stdout poll out of its idle backoff (see
    /// `stdout_poll_interval`) so output following a PTY write is
    /// drained at the fast interval. Poked by the keyboard path
    /// (`TerminalInput` closure) and by `Pane::send_input`.
    pub(in crate::workspace) poke_tx: UnboundedSender<()>,
    /// Account this pane's shell was spawned under (see
    /// [`daruda_store::accounts::AccountSelection`]). Cached here (it never
    /// changes after construction) purely so the layout serializer can
    /// persist it — re-resolving the actual config dir at spawn is
    /// [`super::pane::resolve_pane_account`]'s job, not a runtime read
    /// of this field.
    pub(in crate::workspace) account: daruda_store::accounts::AccountSelection,
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

/// Markdown-form editor pane for a single Task. Lives at the same
/// `PaneLayout::Pane` level as Terminal and File so users can split a
/// TaskEdit alongside a running shell. `task_id = None` means this is a
/// draft: nothing is persisted to `tasks.json` until the user presses
/// `[Save Draft]` or `[Start]`, and the layout serializer skips drafts
/// so they don't survive a session restart.
pub(in crate::workspace) struct TaskEditContent {
    pub(in crate::workspace) task_id: Option<TaskId>,
    pub(in crate::workspace) title_input: Entity<crate::ui::InputState>,
    pub(in crate::workspace) branch_input: Entity<crate::ui::InputState>,
    /// `true` once the user has manually edited the branch field —
    /// further title changes stop auto-deriving the branch so we
    /// don't trample the override.
    pub(in crate::workspace) branch_override: bool,
    pub(in crate::workspace) branch_validation: BranchValidation,
    /// Dropdown mapping lane picks to `Task::base_worktree_path`. The
    /// empty-string sentinel means "no explicit base — branch from the
    /// active lane at run time"; every other value is the absolute path of a
    /// registered lane. Sits in the focus chain between prompt and notes.
    pub(in crate::workspace) base_select: Entity<crate::ui::select::SelectState>,
    /// Prompt editor state (`code_editor("markdown")` for line numbers +
    /// syntax highlight), shared with the renderer via
    /// `crate::ui::markdown_editor(&state)`.
    pub(in crate::workspace) prompt_state: Entity<gpui_component::input::InputState>,
    pub(in crate::workspace) notes_state: Entity<gpui_component::input::InputState>,
    pub(in crate::workspace) auto_execute: bool,
    /// Execution surface the task will run on when started — mirrors
    /// `Task::agent_surface`. Terminal CLI (default) or in-app Agent
    /// chat (ACP). Flipped in-place by the form's surface selector, the
    /// same plain-data pattern as `auto_execute`.
    pub(in crate::workspace) agent_surface: TaskAgentSurface,
    pub(in crate::workspace) focus_handle: FocusHandle,
    pub(in crate::workspace) cached_title: SharedString,
    /// Baseline snapshot for dirty comparison. Reset to
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
    /// dispatches `handle_prompt_file_changed`. Dropped with
    /// the pane.
    pub(in crate::workspace) _prompt_pump: Option<Task<()>>,
    /// Trailing `[+ Add subtask…]` row input. `Submit` (Enter)
    /// dispatches `Workspace::add_subtask` and clears the buffer for
    /// the next entry; the input stays focused so the user can chain
    /// additions.
    pub(in crate::workspace) new_subtask_input: Entity<crate::ui::InputState>,
    /// `Some(subtask_id)` while that row is in inline-rename mode (Enter /
    /// blur commits, Escape cancels). One shared rename input is reused
    /// across rows to avoid IME composition-state churn when switching rows.
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
/// because of line-ending differences.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::workspace) struct TaskEditSnapshot {
    pub(in crate::workspace) title: String,
    pub(in crate::workspace) branch: String,
    pub(in crate::workspace) prompt: String,
    pub(in crate::workspace) notes: String,
    pub(in crate::workspace) auto_execute: bool,
    pub(in crate::workspace) agent_surface: TaskAgentSurface,
    /// Empty string ↔ `Task::base_worktree_path == None`; non-empty ↔
    /// `Some(PathBuf::from(s))`. Plain `String` (not `Option<String>`)
    /// keeps the dirty-comparison `==` path trivial — the user-facing
    /// sentinel is `""` either way.
    pub(in crate::workspace) base_value: String,
}

/// CRLF → LF normaliser used by both the renderer's snapshot builder
/// and the save path so dirty comparisons never trip on line-ending
/// differences.
pub(in crate::workspace) fn normalize_newlines(s: &str) -> String {
    s.replace("\r\n", "\n")
}

impl TaskEditContent {
    /// Build a fresh snapshot of the form's current state for dirty
    /// comparison. The two markdown editors and the title /
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
            agent_surface: self.agent_surface,
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

/// Pane-level handle to an Agent chat pane. A thin wrapper over the
/// self-owned [`AgentChatView`] entity (which holds the model + UI state and
/// renders itself), mirroring how [`TerminalContent`] wraps `TerminalView`.
///
/// Unlike `TerminalContent` / `FileContent` / `TaskEditContent`, there is
/// no cached title here: the view's `session_title` and `items` (first-prompt
/// fallback) can change from several independent internal paths (event pump,
/// prompt echo, session reset), so a write-once cache drifts stale — `/clear`
/// in an existing pane is one repro. `Pane::title()` reads the entity live
/// via `cx` instead, the same way `Pane::is_dirty` / `Pane::can_save` already
/// read `f.editor_state` / `te.title_input` live for File / TaskEdit content.
pub(in crate::workspace) struct AgentChatContent {
    /// The self-owned chat view entity. Embedded by the pane walker via
    /// `AnyView::cached(..)`, so its `cx.notify()` dirties only its subtree.
    pub(in crate::workspace) view: Entity<AgentChatView>,
    /// Lane working directory the agent session is rooted at. `None` when the
    /// pane was opened without a resolvable lane cwd. Cached here (it never
    /// changes after construction) so `Pane::cwd()` stays cx-free and can hand
    /// back a borrow; the view holds its own copy for connect / persistence.
    pub(in crate::workspace) cwd: Option<PaneCwd>,
    /// Account this pane's ACP session runs under (see
    /// [`daruda_store::accounts::AccountSelection`]). Cached here so the
    /// layout serializer can persist it cx-free; `connect_agent_chat`
    /// resolves the actual config dir from it at connect time via
    /// [`super::pane::resolve_pane_account`].
    pub(in crate::workspace) account: daruda_store::accounts::AccountSelection,
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
            // The `AgentChatView` entity tracks its own focus handle in its
            // `Render` impl (like `Terminal` via `TerminalView`), so the
            // wrapper div must not double-track it.
            PaneContent::AgentChat(_) => None,
        }
    }
}

impl Pane {
    /// Title shown in the tab strip, pane header, and status bar. AgentChat
    /// content has no cache to go stale (see [`AgentChatContent`]'s doc
    /// comment): it reads the live session title, falling back to the first
    /// prompt, then to "New {agent name}" — the same precedence the in-pane
    /// activity bar uses (`agent_chat_pane::agent_chat_helpers::activity_bar_title`).
    pub(in crate::workspace) fn title(&self, cx: &App) -> SharedString {
        match &self.content {
            PaneContent::Terminal(t) => t.cached_title.clone(),
            PaneContent::File(f) => f.cached_title.clone(),
            PaneContent::TaskEditPane(te) => te.cached_title.clone(),
            PaneContent::AgentChat(ac) => {
                let v = ac.view.read(cx);
                super::agent_chat_pane::agent_chat_helpers::activity_bar_title(
                    v.session_title.as_deref(),
                    &v.items,
                )
                .map(SharedString::from)
                .unwrap_or_else(|| {
                    crate::surface::strings::new_agent_chat_named(&v.agent_name).into()
                })
            }
        }
    }

    /// Filesystem cwd if the content tracks one (Terminal: from OSC 7;
    /// File: the file's parent directory). The Files-view "show parent
    /// of focused file" affordance reuses this. TaskEdit panes don't
    /// have a meaningful cwd until the task transitions to `Running`
    /// and a lane is materialised — return `None` so dock
    /// affordances skip TaskEdit panes.
    ///
    /// **AgentChat is `None` for `PaneCwd::Remote`.** This is a local
    /// filesystem accessor — every consumer expects a real path on this
    /// machine. A remote pane's cwd string leaking through would reach
    /// `spawn_pty`'s existence check, silently fall back to `$HOME`, and skip
    /// the lane-local fallback tier.
    pub(in crate::workspace) fn cwd(&self) -> Option<&Path> {
        match &self.content {
            PaneContent::Terminal(t) => t.cached_cwd.as_deref(),
            PaneContent::File(f) => f.view.path.parent(),
            PaneContent::TaskEditPane(_) => None,
            PaneContent::AgentChat(ac) => ac.cwd.as_ref().and_then(PaneCwd::as_local),
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
            PaneContent::AgentChat(ac) => ac.view.read(cx).focus_handle.clone(),
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
            PaneContent::File(_) | PaneContent::TaskEditPane(_) | PaneContent::AgentChat(_) => None,
        }
    }

    /// The account a Terminal pane runs under, in persisted form
    /// (`None` = system default), or `None` for any other content kind.
    /// Used by the layout serializer to persist
    /// `SerializedLayout::Leaf::account_id`.
    pub(in crate::workspace) fn terminal_account_id(
        &self,
    ) -> Option<daruda_store::accounts::AccountId> {
        match &self.content {
            PaneContent::Terminal(t) => t.account.to_persisted(),
            PaneContent::File(_) | PaneContent::TaskEditPane(_) | PaneContent::AgentChat(_) => None,
        }
    }

    /// The account selection for either account-tracking pane kind
    /// (Terminal or AgentChat). `None` means the pane kind doesn't track an
    /// account at all (File/TaskEdit — the status bar hides the slot rather
    /// than showing a misleading "System"); `Some(sel)` is the pane's own
    /// [`AccountSelection`](daruda_store::accounts::AccountSelection). Single
    /// accessor behind the status bar's account slot and the account-switcher
    /// handler, so the two don't each re-derive this match.
    pub(in crate::workspace) fn account_selection(
        &self,
    ) -> Option<daruda_store::accounts::AccountSelection> {
        match &self.content {
            PaneContent::Terminal(t) => Some(t.account),
            PaneContent::AgentChat(ac) => Some(ac.account),
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
                Some(tx) => {
                    let sent = tx.send(bytes.to_vec()).is_ok();
                    if sent {
                        // Wake the stdout poll so the write's output is
                        // drained at the fast interval.
                        let _ = t.poke_tx.unbounded_send(());
                    }
                    sent
                }
                None => false,
            },
            PaneContent::File(_) | PaneContent::TaskEditPane(_) | PaneContent::AgentChat(_) => {
                false
            }
        }
    }

    /// Immutable accessor for the file-viewer state.
    pub(in crate::workspace) fn file_content(&self) -> Option<&FileContent> {
        match &self.content {
            PaneContent::File(f) => Some(f),
            PaneContent::Terminal(_) | PaneContent::TaskEditPane(_) | PaneContent::AgentChat(_) => {
                None
            }
        }
    }

    /// Mutable accessor for the file-viewer state. Used by action
    /// handlers that want to update the focused pane's file viewer
    /// (search, scroll, mode toggle).
    pub(in crate::workspace) fn file_content_mut(&mut self) -> Option<&mut FileContent> {
        match &mut self.content {
            PaneContent::File(f) => Some(f),
            PaneContent::Terminal(_) | PaneContent::TaskEditPane(_) | PaneContent::AgentChat(_) => {
                None
            }
        }
    }

    /// Immutable accessor for the TaskEdit pane state. Used by
    /// `Workspace::find_task_edit_pane`, dirty-check helpers,
    /// and the layout serializer to skip draft panes.
    pub(in crate::workspace) fn task_edit_content(&self) -> Option<&TaskEditContent> {
        match &self.content {
            PaneContent::TaskEditPane(te) => Some(te),
            PaneContent::Terminal(_) | PaneContent::File(_) | PaneContent::AgentChat(_) => None,
        }
    }

    /// Mutable counterpart to `task_edit_content`. Used by save /
    /// validation / watcher callbacks that need to flip
    /// `branch_override`, refresh `saved_snapshot`, or re-render the
    /// pane after a state mutation.
    pub(in crate::workspace) fn task_edit_content_mut(&mut self) -> Option<&mut TaskEditContent> {
        match &mut self.content {
            PaneContent::TaskEditPane(te) => Some(te),
            PaneContent::Terminal(_) | PaneContent::File(_) | PaneContent::AgentChat(_) => None,
        }
    }

    /// Immutable accessor for the AgentChat pane wrapper. Used by the layout
    /// serializer to persist the anchored lane cwd (cx-free).
    pub(in crate::workspace) fn agent_chat_content(&self) -> Option<&AgentChatContent> {
        match &self.content {
            PaneContent::AgentChat(ac) => Some(ac),
            PaneContent::Terminal(_) | PaneContent::File(_) | PaneContent::TaskEditPane(_) => None,
        }
    }

    /// Mutable counterpart to `agent_chat_content`. Used by session restore
    /// (`Workspace::rebuild_layout`) to patch the freshly-built pane's
    /// `account_id` from the persisted `SerializedAgentChatContent` — the
    /// constructor path itself has no override to seed with.
    pub(in crate::workspace) fn agent_chat_content_mut(&mut self) -> Option<&mut AgentChatContent> {
        match &mut self.content {
            PaneContent::AgentChat(ac) => Some(ac),
            PaneContent::Terminal(_) | PaneContent::File(_) | PaneContent::TaskEditPane(_) => None,
        }
    }

    /// The AgentChat pane's view entity. Used by the Workspace ops + pump (via
    /// `Workspace::agent_chat_view`) to drive the session, and by the snapshot
    /// builder to read `turn.is_in_flight()`. Mutation goes through `view.update`,
    /// which notifies the view's own cached subtree.
    pub(in crate::workspace) fn agent_chat_view(&self) -> Option<&Entity<AgentChatView>> {
        match &self.content {
            PaneContent::AgentChat(ac) => Some(&ac.view),
            PaneContent::Terminal(_) | PaneContent::File(_) | PaneContent::TaskEditPane(_) => None,
        }
    }

    /// True when the pane holds unsaved user edits. Terminal / File
    /// panes are never dirty — `false` rules them out of the close
    /// prompt entirely. TaskEdit panes diff the form
    /// state against `saved_snapshot`.
    pub(in crate::workspace) fn is_dirty(&self, cx: &App) -> bool {
        match &self.content {
            PaneContent::Terminal(_) => false,
            PaneContent::File(f) => {
                use super::file_view_pane::PaneFileContent;
                !f.view.staged
                    && matches!(f.view.content, PaneFileContent::LoadedRaw)
                    && *f.editor_state.read(cx).text() != f.saved_text
            }
            PaneContent::TaskEditPane(te) => te.is_dirty(cx),
            PaneContent::AgentChat(_) => false,
        }
    }

    /// True when the pane's `save` path is meaningful for the user.
    pub(in crate::workspace) fn can_save(&self, cx: &App) -> bool {
        match &self.content {
            PaneContent::Terminal(_) => false,
            PaneContent::File(f) => {
                use super::file_view_pane::PaneFileContent;
                !f.view.staged
                    && matches!(f.view.content, PaneFileContent::LoadedRaw)
                    && f.view.path.is_absolute()
            }
            PaneContent::TaskEditPane(te) => {
                !matches!(te.branch_validation, BranchValidation::Invalid { .. })
                    && !te.title_input.read(cx).value().trim().is_empty()
            }
            PaneContent::AgentChat(_) => false,
        }
    }

    /// True when the tab strip should paint a small `●` next to the
    /// pane's title to signal unsaved edits (Zed tab indicator).
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
            PaneContent::File(_) | PaneContent::TaskEditPane(_) | PaneContent::AgentChat(_) => {
                false
            }
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
            PaneContent::File(_) | PaneContent::TaskEditPane(_) | PaneContent::AgentChat(_) => true,
        }
    }
}

/// Live-drag coalescing gate (cmux `shouldApplySurfacePixelSizeChange`
/// analog): whether a freshly-computed grid should be forwarded to the PTY
/// and terminal view, given the grid already applied. A live drag emits a
/// bounds notification per pixel, but daruda's grid is cell-quantized, so most
/// notifications recompute the *same* `(cols, rows)`. Forwarding those fires a
/// redundant PTY SIGWINCH — the child app repaints over itself — plus a
/// ghostty reflow on every frame. Skip when the grid is unchanged; the next
/// real cell-boundary crossing differs and is forwarded.
fn grid_resize_needed(current: (u16, u16), computed: (u16, u16)) -> bool {
    current != computed
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
        // Reserve the terminal-pane inset (left+right, top+bottom) so the
        // grid matches the painted content area — the element insets the
        // paint origin by the same `state.inset_*` (single source).
        let (inset_x, inset_y) = self.view.read(cx).inset();
        // ghostty_vt render paths are undefined for a 1-column terminal
        // (mirrors Zed's `cell_width * 2` minimum guard).
        let cols = layout.cols((avail_w - inset_x * 2.0).max(1.0)).max(2);
        let rows = layout.rows((avail_h - pane_header_h - inset_y * 2.0).max(1.0));

        // Live-drag coalescing gate: skip when the recomputed grid equals the
        // grid already applied (sub-cell pixel churn during a drag). Avoids a
        // redundant PTY SIGWINCH + ghostty reflow on every bounds notification.
        // Counts as "measured" — the grid is already correct — so the caller
        // does not mark the resize pending.
        let current_grid = {
            let view = self.view.read(cx);
            (view.session().cols(), view.session().rows())
        };
        if !grid_resize_needed(current_grid, (cols, rows)) {
            return true;
        }

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
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    use crate::platform;
    use crate::surface::{constants::APP_NAME, strings as s};
    use daruda_terminal::{NotificationRequest, TerminalViewEvent};

    // The focused pane is silenced only when daruda itself is the
    // foreground app — backgrounded notifications always surface
    // because the user, by definition, is not looking at the pane.
    let suppressed_by_focus = workspace.notifications.skip_focused_pane
        && platform::attention::is_app_active()
        && workspace.active_runtime().focused_pane_id == pane_id;

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
        TerminalViewEvent::AnnotationDoubleClicked { id } => {
            workspace.open_annotation_dialog_for_edit(pane_id, *id, window, cx);
        }
        TerminalViewEvent::ContextMenuRequested { position, range: _ } => {
            workspace.open_pane_context_menu_at(pane_id, *position, window, cx);
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
/// and `project_root` (both `Option<PathBuf>`); the compiler
/// catches what would otherwise silently pick the wrong tier.
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

/// Pure resolver for a new pane's spawn cwd, keeping the "1 lane = 1 cwd"
/// invariant. Priority:
/// 1) `focused_pane` (when `inherit_cwd`),
/// 2) `active_lane` — pins `Cmd+T` inside the lane even before OSC 7 lands,
/// 3) `project_root` — last resort for non-lane workspaces.
///
/// The `active_lane`-over-`project_root` order matters: swapping it would
/// spawn shells at the repo root from inside a lane, breaking isolation for
/// fresh starts, restored sessions, and `Cmd+T` before OSC 7 reports.
pub(in crate::workspace) fn resolve_default_cwd(
    inherit_cwd: bool,
    candidates: CwdCandidates,
) -> Option<PathBuf> {
    if inherit_cwd && let Some(cwd) = candidates.focused_pane {
        return Some(cwd);
    }
    candidates.active_lane.or(candidates.project_root)
}

/// Which auth domains a pane may resolve a managed account from. Three
/// distinct states, not an `Option<AccountRecipeId>`: "any domain" and "no
/// domain at all" are opposites, and collapsing both onto `None` let a pane
/// with no derivable domain accept an account from every domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) enum AccountDomain {
    /// No agent (a terminal shell, which may run any CLI): every managed
    /// account is usable, and the account's own recipe decides which env var
    /// carries its dir.
    Any,
    /// The pane's agent signs into exactly this domain.
    Exactly(daruda_store::accounts::AccountRecipeId),
    /// The pane's agent has no local auth domain — a remote launch, a JSON
    /// stdio config, or an adapter daruda doesn't recognize — so it can hold
    /// no managed account.
    Unsupported,
}

/// An account-carrying pane as the account layer sees it. File / TaskEdit
/// panes track no account and are never described this way.
pub(in crate::workspace) enum AccountPane {
    Terminal,
    /// Agent chat, carrying the launch of the agent it runs (`None` when that
    /// agent id is no longer in the catalog) and whether *this pane's* lane
    /// is currently remote — a `Raw` launch carries no host of its own, so
    /// the domain gate below can't tell local from remote by the launch
    /// alone (see `AgentLaunch::account_recipe`'s doc).
    AgentChat {
        launch: Option<daruda_config::AgentLaunch>,
        is_remote: bool,
    },
}

impl AccountDomain {
    /// The domain an agent-chat pane resolves in, from its adapter's derived
    /// auth domain. No domain means no managed account, never "any".
    pub(in crate::workspace) fn for_agent(
        recipe: Option<daruda_store::accounts::AccountRecipeId>,
    ) -> Self {
        match recipe {
            Some(r) => Self::Exactly(r),
            None => Self::Unsupported,
        }
    }

    /// The domain a pane resolves in. An agent-chat pane is scoped to *its
    /// own* agent's domain rather than the session's active agent, so a Codex
    /// pane never offers Claude accounts.
    ///
    /// A deprecated `Ssh`/`Docker` launch that `is_remote` reports as
    /// currently resolving `Local` (the lane's Session Host picked Local,
    /// retiring the launch's own embedded host) is special-cased the same
    /// way the connect path does — `account_recipe()` alone can't see past
    /// the launch's own shape, but `is_remote` already reflects the lane's
    /// verified locality here, so the recipe is derived from the bare
    /// adapter command directly (mirrors
    /// `crate::agent::launch_resolve::account_recipe_for_connect`, which the
    /// account-switcher display and the actual connect must agree with —
    /// otherwise the switcher would keep hiding accounts the connect can
    /// now use).
    pub(in crate::workspace) fn for_pane(pane: &AccountPane) -> Self {
        match pane {
            AccountPane::Terminal => Self::Any,
            AccountPane::AgentChat { launch, is_remote } => {
                let recipe = match launch {
                    Some(
                        daruda_config::AgentLaunch::Ssh {
                            adapter_command, ..
                        }
                        | daruda_config::AgentLaunch::Docker {
                            adapter_command, ..
                        },
                    ) if !is_remote => {
                        daruda_config::account_recipe_for_local_command(adapter_command)
                    }
                    Some(l) => l.account_recipe(*is_remote),
                    None => None,
                };
                Self::for_agent(recipe)
            }
        }
    }
}

/// Inject the account's config-dir env var and remove the auth-override vars
/// its recipe strips, so OAuth account selection wins (see
/// `daruda_config::account_env`).
fn apply_account_env(pty: &mut PtyConfig, env: &daruda_config::AccountEnv) {
    pty.env.retain(|(k, _)| !env.strip.contains(&k.as_str()));
    pty.env.extend(env.inject.iter().cloned());
}

/// Resolve a pane's account selection into the config dir + env to spawn
/// with. Pure (no I/O) so the domain gate stays unit-testable; preparing the
/// dir is a separate, explicit step at the call site.
///
/// `domain` is what the pane's process can sign into — see [`AccountDomain`].
/// `SystemDefault` resolves to `None` with no domain-default fallback: that
/// fallback overrode an explicit "System" choice, so the default is seeded at
/// pane creation instead.
pub(in crate::workspace) fn resolve_pane_account(
    state: &daruda_store::accounts::AccountsState,
    data_dir: &Path,
    selection: daruda_store::accounts::AccountSelection,
    domain: AccountDomain,
) -> Option<PreparedAccount> {
    if domain == AccountDomain::Unsupported {
        return None;
    }
    let id = selection.account_id()?;
    let account = state.find(id)?;
    if matches!(domain, AccountDomain::Exactly(r) if account.recipe != r) {
        return None;
    }
    let recipe = daruda_agent::accounts::recipe_for(account.recipe);
    let config_dir = daruda_agent::accounts::account_config_dir(data_dir, id);
    let env = daruda_config::account_env(recipe.config_dir_env(), &config_dir, recipe.strip_env());
    Some(PreparedAccount {
        recipe: account.recipe,
        config_dir,
        env,
    })
}

/// The focused pane's resolved account: the usage-cache key, the auth
/// domain, and the config dir as one value, so a caller can't pair a dir
/// with the wrong domain or cache under a key nothing resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::workspace) enum FocusedAccount {
    /// No managed account — the ambient environment.
    SystemDefault,
    Managed {
        id: daruda_store::accounts::AccountId,
        recipe: daruda_store::accounts::AccountRecipeId,
        config_dir: PathBuf,
    },
}

impl FocusedAccount {
    /// Key the per-account usage caches are stored under.
    pub(in crate::workspace) fn key(&self) -> daruda_store::accounts::AccountSelection {
        match self {
            Self::SystemDefault => daruda_store::accounts::AccountSelection::SystemDefault,
            Self::Managed { id, .. } => daruda_store::accounts::AccountSelection::Managed(*id),
        }
    }

    /// The account's isolated config dir, ready to move into a background
    /// task; `None` for the ambient default.
    pub(in crate::workspace) fn into_config_dir(self) -> Option<PathBuf> {
        match self {
            Self::SystemDefault => None,
            Self::Managed { config_dir, .. } => Some(config_dir),
        }
    }
}

/// Pure core of [`Workspace::focused_account`]. A `Managed` id that no
/// longer resolves collapses to [`FocusedAccount::SystemDefault`] rather
/// than caching under a dangling account.
///
/// Passes `required: None` — this is `&self`-only and a pane's agent id
/// lives in the `AgentChatView` entity, so its auth domain isn't reachable.
fn resolve_focused_account(
    selection: daruda_store::accounts::AccountSelection,
    state: &daruda_store::accounts::AccountsState,
    data_dir: &Path,
) -> FocusedAccount {
    match selection.account_id().zip(resolve_pane_account(
        state,
        data_dir,
        selection,
        AccountDomain::Any,
    )) {
        Some((id, account)) => FocusedAccount::Managed {
            id,
            recipe: account.recipe,
            config_dir: account.config_dir,
        },
        None => FocusedAccount::SystemDefault,
    }
}

impl Workspace {
    /// Default cwd for a new pane. Thin wrapper that gathers the
    /// candidates from `Workspace` state and delegates to
    /// [`resolve_default_cwd`].
    pub(in crate::workspace) fn default_cwd_for_new_pane(&self) -> Option<PathBuf> {
        let candidates = CwdCandidates {
            focused_pane: self
                .active_runtime()
                .panes
                .iter()
                .find(|p| p.id == self.active_runtime().focused_pane_id)
                .and_then(|p| p.cwd().map(Path::to_path_buf)),
            active_lane: self.active_lane().map(|w| w.path.clone()),
            project_root: self.active_project().map(|p| p.root.clone()),
        };
        resolve_default_cwd(self.inherit_cwd, candidates).or_else(home_dir)
    }

    /// Resolve the focused pane's effective account — the usage-cache key,
    /// auth domain and config dir behind it ([`FocusedAccount`]). Gathers
    /// the focused pane's selection ([`AccountSelection::SystemDefault`]
    /// when the pane kind doesn't track an account at all, or there is no
    /// live pane at `focused_pane_id`) and delegates the resolution to
    /// [`resolve_focused_account`].
    pub(in crate::workspace) fn focused_account(&self) -> FocusedAccount {
        let focused_pane_id = self.active_runtime().focused_pane_id;
        let selection = self
            .active_runtime()
            .panes
            .iter()
            .find(|p| p.id == focused_pane_id)
            .and_then(Pane::account_selection)
            .unwrap_or(daruda_store::accounts::AccountSelection::SystemDefault);

        resolve_focused_account(selection, &self.accounts, &self.data_dir)
    }

    /// The focused pane as the account layer sees it. An agent-chat pane's
    /// `agent_id` lives in its view entity, hence the `cx` read; every caller
    /// needs the resulting [`AccountDomain`], so the derivation lives here
    /// rather than being rebuilt per surface.
    pub(in crate::workspace) fn focused_account_pane(&self, cx: &gpui::App) -> AccountPane {
        let focused_pane_id = self.active_runtime().focused_pane_id;
        match self.agent_chat_view(focused_pane_id) {
            Some(view) => {
                let v = view.read(cx);
                let is_remote = matches!(v.cwd, Some(PaneCwd::Remote(_)));
                AccountPane::AgentChat {
                    launch: self.agent_launch_for(&v.agent_id),
                    is_remote,
                }
            }
            None => AccountPane::Terminal,
        }
    }

    /// The account a *freshly created* pane is seeded with: `recipe`'s
    /// configured default, else [`AccountSelection::SystemDefault`]. Seeding
    /// explicitly at creation is what lets resolve-time lookups avoid a
    /// default fallback (see [`resolve_pane_account`]'s doc).
    ///
    /// `recipe: None` is the terminal case — no agent, so no auth domain whose
    /// default could apply; it stays on the ambient environment until the user
    /// picks an account by hand.
    pub(in crate::workspace) fn default_account_selection_for_new_pane(
        &self,
        recipe: Option<daruda_store::accounts::AccountRecipeId>,
    ) -> daruda_store::accounts::AccountSelection {
        recipe
            .and_then(|r| self.accounts.default_account(r))
            .map(|a| daruda_store::accounts::AccountSelection::Managed(a.id))
            .unwrap_or(daruda_store::accounts::AccountSelection::SystemDefault)
    }

    /// Materialize a managed account's config dir before the shell that will
    /// read it exists — Codex symlinks its `CODEX_HOME` in, Claude mirrors the
    /// shared MCP servers.
    ///
    /// Runs inline rather than on the background executor: `spawn_pty` below it
    /// is synchronous, so an off-thread prep would race whatever gets typed (or
    /// programmatically sent) into the fresh pane. The cost is bounded local FS
    /// work on the same order as the PTY spawn it precedes, once per pane
    /// creation — measured at ~8 ms (Claude) / ~0.4 ms (Codex) in a debug build.
    ///
    /// A failure is reported but not fatal, unlike the agent-chat connect: the
    /// config-dir env var is injected either way, so the pane still runs under
    /// the account the user picked — only its mirrored extras are degraded, and
    /// a shell has uses beyond the agent.
    fn prepare_account_dir(&mut self, prepared: &PreparedAccount, cx: &mut Context<Self>) {
        if let Err(e) =
            daruda_agent::accounts::recipe_for(prepared.recipe).prepare_dir(&prepared.config_dir)
        {
            self.report_error(
                daruda_store::observability::error_report::ErrorReport::new(
                    crate::surface::strings::account_prepare_dir_failed(),
                )
                .severity(daruda_store::observability::error_report::ErrorSeverity::Warning)
                .at(file!(), line!())
                .with_context("dir", prepared.config_dir.display().to_string())
                .with_context("error", format!("{e}"))
                .dedup("account.pane.prepare_dir_failed")
                .build(),
                cx,
            );
        }
    }

    pub(in crate::workspace) fn create_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Pane, PaneSpawnError> {
        let cwd = self.default_cwd_for_new_pane();
        let account = self.default_account_selection_for_new_pane(None);
        let prepared =
            resolve_pane_account(&self.accounts, &self.data_dir, account, AccountDomain::Any);
        self.create_pane_with_cwd(cwd, account, prepared.as_ref(), window, cx)
    }

    /// Like `create_pane` but forces a specific initial cwd. Used by
    /// session restore so each restored pane starts at the directory
    /// it last tracked, independent of the focused-pane / project
    /// inheritance rules.
    ///
    /// `account` is the pane's own [`AccountSelection`], cached on the
    /// resulting `TerminalContent` purely for re-serialization; callers that
    /// want a fresh pane to inherit a configured default pass
    /// [`Self::default_account_selection_for_new_pane`]'s result here
    /// explicitly. `prepared` is that selection already resolved by
    /// [`resolve_pane_account`], carrying the env to inject into the spawned
    /// shell; `None` spawns with the ambient environment unchanged.
    pub(in crate::workspace) fn create_pane_with_cwd(
        &mut self,
        cwd: Option<PathBuf>,
        account: daruda_store::accounts::AccountSelection,
        prepared: Option<&PreparedAccount>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Pane, PaneSpawnError> {
        // Propagate the workspace's terminal config so every pane
        // starts with the same font_size / spacing. Zoom actions
        // diverge each view's runtime font_size individually.
        let config = self.terminal_config;
        // `shell_program` is the effective shell from `apply_config`; falls
        // back to `PtyConfig::default()`'s `$SHELL`/`/bin/zsh` when unset.
        let mut pty_config = PtyConfig {
            cwd,
            ..PtyConfig::default()
        };
        if let Some(program) = self.shell_program.as_deref() {
            pty_config.shell = program.to_string();
        }
        if let Some(prepared) = prepared {
            self.prepare_account_dir(prepared, cx);
            apply_account_env(&mut pty_config, &prepared.env);
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
        // Start at the default grid; `resize_terminal` immediately reshapes
        // the session to the pane's measured cols/rows on first layout.
        let session =
            TerminalSession::new(TerminalDims::default(), config).map_err(PaneSpawnError::Vt)?;

        let pane_id = self.alloc_id();

        // Wakes the stdout poll out of its idle backoff the instant
        // bytes head for the PTY, so the echo is drained at the fast
        // interval — typing latency stays at IDLE_POLL even after a
        // long quiet period.
        let (poke_tx, poke_rx) = futures::channel::mpsc::unbounded::<()>();
        let input_poke_tx = poke_tx.clone();

        let font_family = self.font_family.clone();
        let view = cx.new(|cx| {
            let focus_handle = cx.focus_handle();
            let input = TerminalInput::new(move |bytes| {
                let _ = stdin_tx.send(bytes.to_vec());
                let _ = input_poke_tx.unbounded_send(());
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
            poke_rx,
            workspace_entity,
            pane_id,
            window,
            cx,
        );

        // Bridge `TerminalViewEvent`s into Workspace-side platform calls
        // (dock attention, notifications, …), gated by config. `pane_id` is
        // captured so the focus-gate can identify the raising pane; the
        // window-aware subscribe lets `ContextMenuRequested` open the host
        // context menu / annotation dialog (both need `&mut Window`).
        let captured_pane_id = pane_id;
        let view_event_sub = cx.subscribe_in(
            &view,
            window,
            move |this, _view, event: &daruda_terminal::TerminalViewEvent, window, cx| {
                handle_view_event(this, captured_pane_id, event, window, cx);
            },
        );

        // Register the pane's shell PID with the PTY tracker so the
        // sysinfo poller can find `claude` descendants of this pane
        // and bind them back to its session_id. Stub
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
                poke_tx,
                account,
            }),
        })
    }

    #[allow(clippy::too_many_arguments)] // PTY plumbing — splitting wraps tax callers more than it saves.
    fn spawn_stdout_poll(
        view: Entity<TerminalView>,
        stdout_rx: mpsc::Receiver<Vec<u8>>,
        exit_rx: mpsc::Receiver<()>,
        error_rx: mpsc::Receiver<daruda_store::observability::error_report::ErrorReport>,
        mut poke_rx: UnboundedReceiver<()>,
        workspace: Entity<Workspace>,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        window.spawn(cx, async move |cx| {
            let mut streaming_ticks: u32 = 0;
            let mut idle_ticks: u32 = 0;
            // `false` once every poke sender is gone (pane teardown) —
            // plain timer sleeps from then on avoid a busy select loop.
            let mut poke_open = true;
            // Cached redraw cap (`1000 / render.max_fps` ms, mirrored on
            // Workspace; 30 fps default). Refreshed from the mirror only
            // on active ticks so a deep-idle pane performs no entity
            // reads; a config reload is picked up on the next activity.
            let mut cap = cx
                .update(|_, app| workspace.read(app).mirrors.terminal_redraw_interval)
                .unwrap_or(CAP_FALLBACK);
            loop {
                let interval = stdout_poll_interval(cap, streaming_ticks, idle_ticks);
                let mut poked = false;
                if poke_open {
                    let timer = cx.background_executor().timer(interval);
                    match futures::future::select(timer, poke_rx.next()).await {
                        Either::Left(((), _)) => {}
                        Either::Right((Some(()), _)) => {
                            poked = true;
                            // Collapse a poke burst (typed word, pasted
                            // block) into a single fast tick.
                            while poke_rx.try_recv().is_ok() {}
                        }
                        Either::Right((None, _)) => poke_open = false,
                    }
                } else {
                    cx.background_executor().timer(interval).await;
                }

                let mut batch = Vec::new();
                while let Ok(chunk) = stdout_rx.try_recv() {
                    batch.extend_from_slice(&chunk);
                }
                if batch.is_empty() {
                    streaming_ticks = 0;
                    idle_ticks = if poked {
                        0
                    } else {
                        idle_ticks.saturating_add(1)
                    };
                } else {
                    streaming_ticks = streaming_ticks.saturating_add(1);
                    idle_ticks = 0;
                }
                if idle_ticks == 0
                    && let Ok(v) =
                        cx.update(|_, app| workspace.read(app).mirrors.terminal_redraw_interval)
                {
                    cap = v;
                }

                // Drain PTY thread errors (writer/reader death) on
                // every tick so the user sees them within one frame.
                // We enrich each report with pane id + cwd so
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
                                .active_runtime()
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
                            let is_focused = ws.active_runtime().focused_pane_id == pane_id;
                            // Capture the focused pane's cwd before the
                            // update so a change can re-target the MCP
                            // watcher (the Project scope reads the
                            // focused terminal's cwd `.mcp.json`).
                            let prev_cwd = ws
                                .active_runtime()
                                .panes
                                .iter()
                                .find(|p| p.id == pane_id)
                                .and_then(|p| p.cwd().map(std::path::Path::to_path_buf));
                            let updated = ws
                                .active_runtime_mut()
                                .panes
                                .iter_mut()
                                .find(|p| p.id == pane_id)
                                .map(|p| p.update_cached_terminal(title, cwd))
                                .unwrap_or(false);
                            if updated {
                                cx.notify();
                                if is_focused {
                                    let new_cwd = ws
                                        .active_runtime()
                                        .panes
                                        .iter()
                                        .find(|p| p.id == pane_id)
                                        .and_then(|p| p.cwd().map(std::path::Path::to_path_buf));
                                    if new_cwd != prev_cwd {
                                        ws.refresh_mcp_on_cwd_change(cx);
                                    }
                                }
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
        &mut self,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The bottom input is the shared prompt/command surface for every
        // pane, so on each (re-)entry surface its panel and sync the
        // placeholder to the focused pane kind. Done here — the canonical
        // windowed focus path — so it fires on click, keyboard pane nav,
        // and tab switch alike; `set_focused_pane` deliberately keeps only
        // focus-tracking + the draft swap, leaving placeholder sync to this
        // path.
        //
        // `apply_input_placeholder` reads mode state and modifier policy
        // and writes to `terminal_input` using the live `window` — avoids
        // nested `update_window` re-entry that the windowless
        // `refresh_terminal_input_placeholder` path would trigger.
        let is_agent = self.is_agent_chat_pane(pane_id);
        self.activate_bottom_input(cx);
        self.apply_input_placeholder(window, cx);

        if is_agent {
            // Lazy connect: a restored Agent chat pane stays `Idle` (no
            // session) until first focus. This is that trigger — connect
            // only the pane the user actually opens, so cold restore no
            // longer spins up an agent process per pane.
            self.maybe_connect_agent_chat(pane_id, cx);
            // Agent chat panes have no in-pane input; keyboard focus goes
            // to the shared bottom input so the user can type immediately.
            self.terminal_input
                .read(cx)
                .focus_handle(cx)
                .focus(window, cx);
        } else if let Some(pane) = self.active_runtime().panes.iter().find(|p| p.id == pane_id) {
            pane.focus_handle(cx).focus(window, cx);
        }
    }
}

/// Responsive idle poll so first output after a quiet period appears
/// promptly; the cap interval only kicks in once output is sustained.
const IDLE_POLL: Duration = Duration::from_millis(16);
/// Consecutive non-empty drains before backing off to the cap.
const STREAM_ENTER_TICKS: u32 = 3;
/// Consecutive empty drains tolerated at the fast interval before the
/// idle backoff starts (~128 ms of fast polling after the last byte).
const IDLE_GRACE_TICKS: u32 = 8;
/// Ceiling for the idle backoff. Also bounds how late a shell exit or
/// an un-poked first byte (background process output) is noticed.
const IDLE_BACKOFF_MAX: Duration = Duration::from_millis(250);
/// 16 ms × 2⁴ = 256 ms ≥ `IDLE_BACKOFF_MAX` — further doublings are moot.
const IDLE_BACKOFF_MAX_DOUBLINGS: u32 = 4;
/// Redraw-cap fallback when the workspace mirror is unreachable
/// (30 fps default, matching `render.max_fps`).
const CAP_FALLBACK: Duration = Duration::from_millis(33);

/// Poll interval for the stdout drain loop, from the redraw cap and the
/// two drain counters.
///
/// Regimes: sustained output (≥ [`STREAM_ENTER_TICKS`] non-empty drains)
/// polls at the redraw cap; recent activity (output or an input poke
/// within [`IDLE_GRACE_TICKS`] drains) polls fast so a keystroke echo is
/// never held longer than [`IDLE_POLL`]; past the grace window the
/// interval doubles per empty drain up to [`IDLE_BACKOFF_MAX`] so a
/// quiet pane costs ~4 wakes/s instead of ~60.
fn stdout_poll_interval(cap: Duration, streaming_ticks: u32, idle_ticks: u32) -> Duration {
    if streaming_ticks >= STREAM_ENTER_TICKS {
        return cap;
    }
    let fast = cap.min(IDLE_POLL);
    if idle_ticks <= IDLE_GRACE_TICKS {
        return fast;
    }
    let doublings = (idle_ticks - IDLE_GRACE_TICKS).min(IDLE_BACKOFF_MAX_DOUBLINGS);
    (fast * (1u32 << doublings)).min(IDLE_BACKOFF_MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAP_30FPS: Duration = Duration::from_millis(33);

    #[test]
    fn streaming_polls_at_cap() {
        assert_eq!(
            stdout_poll_interval(CAP_30FPS, STREAM_ENTER_TICKS, 0),
            CAP_30FPS
        );
        // Streaming wins regardless of a stale idle counter.
        assert_eq!(
            stdout_poll_interval(CAP_30FPS, STREAM_ENTER_TICKS + 5, 99),
            CAP_30FPS
        );
    }

    #[test]
    fn active_and_grace_window_poll_fast() {
        assert_eq!(stdout_poll_interval(CAP_30FPS, 0, 0), IDLE_POLL);
        assert_eq!(
            stdout_poll_interval(CAP_30FPS, STREAM_ENTER_TICKS - 1, 0),
            IDLE_POLL
        );
        assert_eq!(
            stdout_poll_interval(CAP_30FPS, 0, IDLE_GRACE_TICKS),
            IDLE_POLL
        );
    }

    #[test]
    fn idle_backoff_doubles_up_to_max() {
        let at = |idle| stdout_poll_interval(CAP_30FPS, 0, idle);
        assert_eq!(at(IDLE_GRACE_TICKS + 1), Duration::from_millis(32));
        assert_eq!(at(IDLE_GRACE_TICKS + 2), Duration::from_millis(64));
        assert_eq!(at(IDLE_GRACE_TICKS + 3), Duration::from_millis(128));
        assert_eq!(at(IDLE_GRACE_TICKS + 4), IDLE_BACKOFF_MAX);
        assert_eq!(at(u32::MAX), IDLE_BACKOFF_MAX);
    }

    #[test]
    fn apply_account_env_injects_and_strips() {
        use daruda_terminal::pty::PtyConfig;
        let mut cfg = PtyConfig::default();
        cfg.env.push(("ANTHROPIC_API_KEY".into(), "leak".into()));
        let env = daruda_config::account_env(
            "CLAUDE_CONFIG_DIR",
            std::path::Path::new("/data/acc/alice"),
            &["ANTHROPIC_API_KEY"],
        );
        apply_account_env(&mut cfg, &env);
        assert!(
            cfg.env
                .iter()
                .any(|(k, v)| k == "CLAUDE_CONFIG_DIR" && v == "/data/acc/alice")
        );
        assert!(
            !cfg.env.iter().any(|(k, _)| k == "ANTHROPIC_API_KEY"),
            "auth override stripped"
        );
    }

    #[test]
    fn high_fps_cap_bounds_the_fast_interval() {
        let cap = Duration::from_millis(8); // 120 fps
        assert_eq!(stdout_poll_interval(cap, 0, 0), cap);
        assert_eq!(stdout_poll_interval(cap, STREAM_ENTER_TICKS, 0), cap);
        // Backoff doubles from the bounded fast interval.
        assert_eq!(
            stdout_poll_interval(cap, 0, IDLE_GRACE_TICKS + 1),
            Duration::from_millis(16)
        );
    }

    #[test]
    fn grid_resize_skips_unchanged_grid() {
        // Live-drag coalescing gate: a recomputed grid equal to the applied
        // grid must NOT be forwarded — skips the redundant PTY SIGWINCH +
        // ghostty reflow that sub-cell pixel churn fires on every bounds
        // notification during a drag (Retina: 1pt = 2px).
        assert!(!grid_resize_needed((80, 24), (80, 24)));
        // A real cell-boundary crossing in either dimension is forwarded.
        assert!(grid_resize_needed((80, 24), (81, 24)));
        assert!(grid_resize_needed((80, 24), (80, 25)));
    }

    /// One managed account of `recipe`, registered as that domain's default.
    fn accounts_with(
        recipe: daruda_store::accounts::AccountRecipeId,
    ) -> (
        daruda_store::accounts::AccountId,
        daruda_store::accounts::AccountsState,
    ) {
        use daruda_store::accounts::{AccountId, AccountsState, ManagedAccount};
        let id = AccountId::new();
        let mut st = AccountsState::default();
        st.accounts.push(ManagedAccount {
            id,
            recipe,
            email: None,
            organization: None,
            config_dir: std::path::PathBuf::from("/x"),
            created_at: 0,
            last_authenticated_at: 0,
        });
        st.default_by_recipe.insert(recipe, id);
        (id, st)
    }

    #[test]
    fn terminal_pane_takes_every_domain() {
        assert_eq!(
            AccountDomain::for_pane(&AccountPane::Terminal),
            AccountDomain::Any
        );
    }

    #[test]
    fn agent_chat_pane_scopes_to_its_own_adapter() {
        let claude = AccountPane::AgentChat {
            launch: Some(daruda_config::AgentLaunch::Raw(
                "npx -y @agentclientprotocol/claude-agent-acp@latest".into(),
            )),
            is_remote: false,
        };
        assert_eq!(
            AccountDomain::for_pane(&claude),
            AccountDomain::Exactly(daruda_store::accounts::AccountRecipeId::Claude)
        );

        let codex = AccountPane::AgentChat {
            launch: Some(daruda_config::AgentLaunch::Raw(
                "npx -y @agentclientprotocol/codex-acp@latest".into(),
            )),
            is_remote: false,
        };
        assert_eq!(
            AccountDomain::for_pane(&codex),
            AccountDomain::Exactly(daruda_store::accounts::AccountRecipeId::Codex)
        );
    }

    #[test]
    fn agent_chat_pane_on_a_remote_launch_has_no_domain() {
        let remote = [
            AccountPane::AgentChat {
                launch: Some(daruda_config::AgentLaunch::Ssh {
                    adapter_command: "npx -y @agentclientprotocol/claude-agent-acp@latest".into(),
                    host: "build-box".into(),
                }),
                is_remote: true,
            },
            AccountPane::AgentChat {
                launch: Some(daruda_config::AgentLaunch::Docker {
                    adapter_command: "npx -y @agentclientprotocol/codex-acp@latest".into(),
                    container: "dev".into(),
                }),
                is_remote: true,
            },
            AccountPane::AgentChat {
                launch: Some(daruda_config::AgentLaunch::Raw(
                    "ssh box sh -c 'cd \"{{cwd}}\" && npx -y @agentclientprotocol/claude-agent-acp@latest'"
                        .into(),
                )),
                // The `{{cwd}}` token isn't reflected in `is_remote` at all
                // (see `crate::agent::launch_resolve::account_recipe_for_connect`'s
                // doc on why `Raw` stays on `account_recipe`'s own,
                // `needs_remote_cwd()`-based exclusion) — excluded either way.
                is_remote: false,
            },
            AccountPane::AgentChat {
                launch: None,
                is_remote: false,
            },
        ];
        for pane in &remote {
            assert_eq!(AccountDomain::for_pane(pane), AccountDomain::Unsupported);
        }
    }

    /// Finding: a deprecated `Ssh`/`Docker` launch that `is_remote` reports
    /// as resolving `Local` right now (the lane's Session Host picked
    /// Local) must still offer a managed account, exactly like a `Raw`
    /// launch running the same command would — the account-switcher display
    /// must agree with what the actual connect now does (see
    /// `crate::agent::launch_resolve::account_recipe_for_connect`).
    #[test]
    fn agent_chat_pane_on_a_locally_resolved_legacy_launch_still_has_a_domain() {
        let pane = AccountPane::AgentChat {
            launch: Some(daruda_config::AgentLaunch::Ssh {
                adapter_command: "npx -y @agentclientprotocol/claude-agent-acp@latest".into(),
                host: "old-box".into(),
            }),
            is_remote: false,
        };
        assert_eq!(
            AccountDomain::for_pane(&pane),
            AccountDomain::Exactly(daruda_store::accounts::AccountRecipeId::Claude)
        );
    }

    /// The case `Ssh`/`Docker`/the `{{cwd}}` token can't cover on their own:
    /// a plain `Raw` command carries no host, so a pane whose *lane* is
    /// remote must be excluded via `is_remote`, not the launch shape.
    #[test]
    fn agent_chat_pane_on_a_remote_lane_has_no_domain_even_for_a_bare_raw_launch() {
        let pane = AccountPane::AgentChat {
            launch: Some(daruda_config::AgentLaunch::Raw(
                "npx -y @agentclientprotocol/claude-agent-acp@latest".into(),
            )),
            is_remote: true,
        };
        assert_eq!(AccountDomain::for_pane(&pane), AccountDomain::Unsupported);
    }

    #[test]
    fn resolve_system_default_ignores_the_domain_default_even_when_set() {
        use daruda_store::accounts::{AccountRecipeId, AccountSelection, AccountsState};
        let (id, st) = accounts_with(AccountRecipeId::Claude);
        let data = std::path::Path::new("/data");
        // `SystemDefault` is the explicit "System (~/.claude)" choice — it
        // must NOT fall back to the domain's default, even when one is
        // configured. (Seeding a fresh pane with that default happens
        // once, at creation time — see
        // `Workspace::default_account_selection_for_new_pane`.)
        for domain in [
            AccountDomain::Any,
            AccountDomain::Exactly(AccountRecipeId::Claude),
            AccountDomain::Exactly(AccountRecipeId::Codex),
        ] {
            assert_eq!(
                resolve_pane_account(&st, data, AccountSelection::SystemDefault, domain),
                None
            );
        }
        // An explicit `Managed(id)` still resolves normally.
        let resolved = resolve_pane_account(
            &st,
            data,
            AccountSelection::Managed(id),
            AccountDomain::Exactly(AccountRecipeId::Claude),
        )
        .expect("a Claude account under a Claude-scoped pane resolves");
        assert_eq!(
            resolved.config_dir,
            daruda_agent::accounts::account_config_dir(data, id)
        );
        assert!(
            resolved
                .env
                .inject
                .iter()
                .any(|(k, _)| k == "CLAUDE_CONFIG_DIR")
        );
        // no default set → None either way (uses system ~/.claude)
        let empty = AccountsState::default();
        assert_eq!(
            resolve_pane_account(
                &empty,
                data,
                AccountSelection::SystemDefault,
                AccountDomain::Any
            ),
            None
        );
    }

    #[test]
    fn resolve_pane_account_refuses_every_account_when_no_domain_applies() {
        use daruda_store::accounts::{AccountRecipeId, AccountSelection};
        // A remote / JSON-stdio / unrecognized adapter can hold no managed
        // account. `Unsupported` must refuse, where "no constraint" would
        // have let every domain through.
        for recipe in AccountRecipeId::all() {
            let (id, st) = accounts_with(recipe);
            assert_eq!(
                resolve_pane_account(
                    &st,
                    std::path::Path::new("/data"),
                    AccountSelection::Managed(id),
                    AccountDomain::Unsupported
                ),
                None,
                "{recipe:?} must not reach a pane with no auth domain"
            );
        }
    }

    #[test]
    fn resolve_pane_account_refuses_an_account_from_another_auth_domain() {
        use daruda_store::accounts::{AccountRecipeId, AccountSelection};
        let (id, st) = accounts_with(AccountRecipeId::Claude);
        let data = std::path::Path::new("/data");
        // A codex pane must never receive a Claude account's config dir.
        assert_eq!(
            resolve_pane_account(
                &st,
                data,
                AccountSelection::Managed(id),
                AccountDomain::Exactly(AccountRecipeId::Codex)
            ),
            None
        );
    }

    #[test]
    fn resolve_pane_account_injects_the_accounts_own_env_var() {
        use daruda_store::accounts::{AccountRecipeId, AccountSelection};
        let (id, st) = accounts_with(AccountRecipeId::Codex);
        let data = std::path::Path::new("/data");
        let codex_env = |domain| {
            resolve_pane_account(&st, data, AccountSelection::Managed(id), domain)
                .expect("a Codex account resolves")
                .env
                .inject
        };
        assert!(
            codex_env(AccountDomain::Exactly(AccountRecipeId::Codex))
                .iter()
                .any(|(k, _)| k == "CODEX_HOME")
        );
        // A terminal pane passes no constraint — the account's own recipe
        // still picks the env var.
        assert!(
            codex_env(AccountDomain::Any)
                .iter()
                .any(|(k, _)| k == "CODEX_HOME")
        );
    }

    #[test]
    fn resolve_pane_account_managed_but_deleted_resolves_to_none() {
        use daruda_store::accounts::{AccountId, AccountSelection, AccountsState};
        let empty = AccountsState::default();
        assert_eq!(
            resolve_pane_account(
                &empty,
                std::path::Path::new("/data"),
                AccountSelection::Managed(AccountId::new()),
                AccountDomain::Any
            ),
            None
        );
    }

    #[test]
    fn focused_account_resolves_to_the_selected_account() {
        use daruda_store::accounts::{AccountRecipeId, AccountSelection};
        let (id, st) = accounts_with(AccountRecipeId::Claude);
        let data = std::path::Path::new("/data");

        let focused = resolve_focused_account(AccountSelection::Managed(id), &st, data);
        assert_eq!(focused.key(), AccountSelection::Managed(id));
        assert_eq!(
            focused.into_config_dir(),
            Some(daruda_agent::accounts::account_config_dir(data, id))
        );
    }

    #[test]
    fn focused_account_system_default_ignores_the_domain_default() {
        use daruda_store::accounts::{AccountRecipeId, AccountSelection};
        let (_id, st) = accounts_with(AccountRecipeId::Claude);
        let data = std::path::Path::new("/data");

        // `SystemDefault` on the pane must not resolve to the domain's
        // default even though one is configured, matching
        // `resolve_pane_account`.
        assert_eq!(
            resolve_focused_account(AccountSelection::SystemDefault, &st, data),
            FocusedAccount::SystemDefault
        );
    }

    #[test]
    fn focused_account_managed_but_deleted_collapses_to_system_default() {
        use daruda_store::accounts::{AccountId, AccountSelection, AccountsState};
        // A pane pinned to an account that no longer exists caches its usage
        // under the system slot rather than a dangling account key.
        let empty = AccountsState::default();
        let data = std::path::Path::new("/data");
        let stale = AccountId::new();
        assert_eq!(
            resolve_focused_account(AccountSelection::Managed(stale), &empty, data),
            FocusedAccount::SystemDefault
        );
    }

    #[test]
    fn focused_account_system_default_without_managed_accounts() {
        use daruda_store::accounts::{AccountSelection, AccountsState};
        // Zero managed accounts (today's default state): system-default
        // Keychain fetch, identical to pre-account behavior.
        let empty = AccountsState::default();
        let data = std::path::Path::new("/data");
        assert_eq!(
            resolve_focused_account(AccountSelection::SystemDefault, &empty, data),
            FocusedAccount::SystemDefault
        );
    }
}
