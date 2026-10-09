//! Pane models and view coordination. Accounts, PTY startup, and output
//! draining have separate lifecycle modules behind this public boundary.

mod accounts;
mod output;
mod terminal;

use accounts::apply_account_env;
#[cfg(test)]
pub(in crate::workspace) use accounts::resolve_pane_account;
pub(in crate::workspace) use accounts::{AccountDomain, FocusedAccount};
#[cfg(test)]
use accounts::{AccountPane, resolve_focused_account};
#[cfg(test)]
use output::*;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use daruda_store::project::PaneCwd;
use daruda_store::tasks::ExecutionRef;
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
            PaneSpawnError::Pty(e) => e.fmt(f),
            PaneSpawnError::Vt(e) => f.write_str(
                &crate::surface::strings::error::pane_vt_init_failed(e.to_string()),
            ),
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
    AgentChat(AgentChatContent),
}

/// PTY-backed terminal content. Owns the `TerminalView` entity, the
/// PTY master (resize / drop), the stdout-poll task, and the cached
/// title / cwd that OSC 0/2 + OSC 7 scanners feed back to the workspace
/// for tab strip + status bar rendering.
pub(in crate::workspace) struct TerminalContent {
    /// The task run this terminal was opened for: the first `claude` bound
    /// here is that run's session.
    pub(in crate::workspace) task_run: Option<ExecutionRef>,
    pub(in crate::workspace) view: Entity<TerminalView>,
    /// `None` when the pane was created via stub (test builds).
    pub(super) master: Option<Arc<dyn MasterPty + Send>>,
    /// The shell's pid, which also names its process group.
    pub(super) shell_pid: Option<u32>,
    /// The shell exited and the pane stayed open (`close_pane_on_exit` off).
    pub(super) exited: bool,
    pub(super) cached_title: SharedString,
    /// Cached cwd (OSC 7) — `None` until the shell first reports it.
    pub(super) cached_cwd: Option<PathBuf>,
    pub(super) _stdout_task: Task<()>,
    /// Listens for `TerminalViewEvent`s emitted by the view (e.g. OSC
    /// 1337 attention requests) and dispatches them to platform APIs
    /// gated by `[notifications]` config. Dropped with the pane.
    pub(super) _view_event_subscription: Subscription,
    /// Outgoing channel into the PTY's writer thread, cloned from `stdin_tx`
    /// so `Workspace::send_to_pane` (skills, macros) can write into the same
    /// channel as the user's keystrokes. `None` for stub panes.
    pub(super) pty_input_tx: Option<mpsc::Sender<Vec<u8>>>,
    /// Wakes the stdout poll out of its idle backoff (see
    /// `stdout_poll_interval`) so output following a PTY write is
    /// drained at the fast interval. Poked by the keyboard path
    /// (`TerminalInput` closure) and by `Pane::send_input`.
    pub(super) poke_tx: UnboundedSender<()>,
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
    pub(super) focus_handle: FocusHandle,
    /// Keeps the per-pane `InputEvent` subscription alive; dropped
    /// with the pane.
    pub(super) _search_subscription: Subscription,
    /// Tab title — file basename. Set at construction.
    pub(in crate::workspace) cached_title: SharedString,
    /// Code-editor state for raw file editing.
    pub(in crate::workspace) editor_state: Entity<gpui_component::input::InputState>,
    /// Text that was last saved to disk — for dirty comparison.
    pub(in crate::workspace) saved_text: String,
    /// GPU images for the Markdown preview, indexed by the slot the load's
    /// resolve pass stamped into the blocks. Seeded empty at construction;
    /// after that, `install_content` / `release_images`
    /// (`file_view_pane/images.rs`) are by convention the only writers — the
    /// visibility below narrows who *could* write it, it does not enforce that.
    pub(in crate::workspace::main_area) images: super::file_view_pane::images::MdImages,
}

/// Pane-level handle to an Agent chat pane. A thin wrapper over the
/// self-owned [`AgentChatView`] entity (which holds the model + UI state and
/// renders itself), mirroring how [`TerminalContent`] wraps `TerminalView`.
///
/// Unlike `TerminalContent` / `FileContent`, there is
/// no cached title here: the view's `session_title` and `items` (first-prompt
/// fallback) can change from several independent internal paths (event pump,
/// prompt echo, session reset), so a write-once cache drifts stale — `/clear`
/// in an existing pane is one repro. `Pane::title()` reads the entity live
/// via `cx` instead, the same way `Pane::is_dirty` / `Pane::can_save` already
/// read `f.editor_state` / `te.title_input` live for File / TaskEdit content.
pub(in crate::workspace) struct AgentChatContent {
    /// Only the pane that dispatched this execution can finish its task.
    pub(in crate::workspace) task_run: Option<ExecutionRef>,
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
    /// The catalog agent the view runs, cached like `cwd` so the status bar
    /// can name the pane's auth domain without reading the view — a read from
    /// render registers it as displayed even behind Settings (Pitfall 10).
    /// Re-pointed only through `update_agent_chat_agent_id`.
    pub(in crate::workspace) agent_id: String,
}

pub(in crate::workspace) struct Pane {
    pub(in crate::workspace) id: PaneId,
    pub(in crate::workspace) content: PaneContent,
}

/// Cache key capturing all view settings that affect `cell_dimensions()`.
/// Adding a new font attribute here forces the cache lookup to account for it.
#[derive(Hash, PartialEq, Eq)]
pub(super) struct FontMetricsKey {
    font_size_bits: u32,
    v_spacing_bits: u32,
    h_spacing_bits: u32,
    font_family_hash: u64,
}

impl FontMetricsKey {
    pub(super) fn from_view(v: &TerminalView) -> Self {
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
            PaneContent::AgentChat(ac) => {
                let v = ac.view.read(cx);
                super::agent_chat_pane::agent_chat_helpers::activity_bar_title(
                    v.session_title(),
                    v.items(),
                )
                .map(SharedString::from)
                .unwrap_or_else(|| {
                    crate::surface::strings::common::new_agent_chat_named(v.agent_name()).into()
                })
            }
        }
    }

    /// Refresh pane-title caches whose fallback text follows the active
    /// locale. Content-derived titles are left untouched.
    pub(in crate::workspace) fn refresh_locale_dependent_title(&mut self, cx: &App) {
        match &mut self.content {
            PaneContent::Terminal(t) => {
                let title = t.view.read(cx).terminal_title().into_owned();
                if t.cached_title.as_ref() != title {
                    t.cached_title = title.into();
                }
            }
            PaneContent::File(_) | PaneContent::AgentChat(_) => {}
        }
    }

    /// Filesystem cwd if the content tracks one (Terminal: from OSC 7;
    /// File: the file's parent directory). The Files-view "show parent
    /// of focused file" affordance reuses this.
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
            PaneContent::AgentChat(ac) => ac.cwd.as_ref().and_then(PaneCwd::as_local),
        }
    }

    /// Last path component of `cwd`, for compact display in
    /// tab / header / status bar.
    pub(in crate::workspace) fn display_cwd(&self) -> Option<SharedString> {
        cwd_basename(self.cwd())
    }

    pub(super) fn is_terminal(&self) -> bool {
        matches!(self.content, PaneContent::Terminal(_))
    }

    pub(in crate::workspace) fn is_file(&self) -> bool {
        matches!(self.content, PaneContent::File(_))
    }

    pub(super) fn is_agent_chat(&self) -> bool {
        matches!(self.content, PaneContent::AgentChat(_))
    }

    pub(in crate::workspace) fn file_path(&self) -> Option<PathBuf> {
        self.file_content().map(|file| file.view.path.clone())
    }

    /// Focus handle the pane gives to the window when activated.
    /// File panes return their pane-level handle; the find-panel input
    /// has its own handle that takes priority while the panel is open
    /// (search uses `track_focus(&search_input.focus_handle())`).
    pub(in crate::workspace) fn focus_handle(&self, cx: &App) -> FocusHandle {
        match &self.content {
            PaneContent::Terminal(t) => t.view.read(cx).focus_handle().clone(),
            PaneContent::File(f) => f.focus_handle.clone(),
            PaneContent::AgentChat(ac) => ac.view.read(cx).focus_handle(cx),
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
            PaneContent::File(_) | PaneContent::AgentChat(_) => None,
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
            PaneContent::File(_) | PaneContent::AgentChat(_) => None,
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
            PaneContent::File(_) => None,
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
            PaneContent::File(_) | PaneContent::AgentChat(_) => false,
        }
    }

    /// Immutable accessor for the file-viewer state.
    pub(in crate::workspace) fn file_content(&self) -> Option<&FileContent> {
        match &self.content {
            PaneContent::File(f) => Some(f),
            PaneContent::Terminal(_) | PaneContent::AgentChat(_) => None,
        }
    }

    /// Mutable accessor for the file-viewer state. Used by action
    /// handlers that want to update the focused pane's file viewer
    /// (search, scroll, mode toggle).
    pub(in crate::workspace) fn file_content_mut(&mut self) -> Option<&mut FileContent> {
        match &mut self.content {
            PaneContent::File(f) => Some(f),
            PaneContent::Terminal(_) | PaneContent::AgentChat(_) => None,
        }
    }

    /// Immutable accessor for the AgentChat pane wrapper. Used by the layout
    /// serializer to persist the anchored lane cwd (cx-free).
    pub(in crate::workspace) fn agent_chat_content(&self) -> Option<&AgentChatContent> {
        match &self.content {
            PaneContent::AgentChat(ac) => Some(ac),
            PaneContent::Terminal(_) | PaneContent::File(_) => None,
        }
    }

    /// Mutable counterpart to `agent_chat_content`. Used by session restore
    /// (`Workspace::rebuild_layout`) to patch the freshly-built pane's
    /// `account_id` from the persisted `SerializedAgentChatContent` — the
    /// constructor path itself has no override to seed with.
    pub(in crate::workspace) fn agent_chat_content_mut(&mut self) -> Option<&mut AgentChatContent> {
        match &mut self.content {
            PaneContent::AgentChat(ac) => Some(ac),
            PaneContent::Terminal(_) | PaneContent::File(_) => None,
        }
    }

    /// The AgentChat pane's view entity. Used by the Workspace ops + pump (via
    /// `Workspace::agent_chat_view`) to drive the session, and by the snapshot
    /// builder to read `turn.is_in_flight()`. Mutation goes through `view.update`,
    /// which notifies the view's own cached subtree.
    pub(in crate::workspace) fn agent_chat_view(&self) -> Option<&Entity<AgentChatView>> {
        match &self.content {
            PaneContent::AgentChat(ac) => Some(&ac.view),
            PaneContent::Terminal(_) | PaneContent::File(_) => None,
        }
    }

    /// True when the pane holds unsaved user edits. A File pane in Raw mode
    /// diffs its editor against the text it loaded; Terminal and AgentChat
    /// have no editable buffer.
    ///
    /// Also what takes a tab out of the left dock's replaceable scratch slot —
    /// see [`crate::workspace::Workspace::preview_tab_index`].
    pub(in crate::workspace) fn is_dirty(&self, cx: &App) -> bool {
        match &self.content {
            PaneContent::Terminal(_) => false,
            PaneContent::File(f) => {
                f.view.holds_editable_buffer() && *f.editor_state.read(cx).text() != f.saved_text
            }
            PaneContent::AgentChat(_) => false,
        }
    }

    /// True when closing the pane would stop work in progress: a job in a
    /// terminal's foreground, or an agent turn.
    pub(in crate::workspace) fn runs_work(&self, cx: &App) -> bool {
        match &self.content {
            PaneContent::Terminal(t) => {
                !t.exited
                    && (t.view.read(cx).session().command_is_running()
                        || t.master.as_deref().is_some_and(|master| {
                            daruda_terminal::pty::runs_foreground_job(master, t.shell_pid)
                        }))
            }
            PaneContent::AgentChat(ac) => ac.view.read(cx).is_busy(),
            PaneContent::File(_) => false,
        }
    }

    /// True when the pane's `save` path is meaningful for the user.
    pub(super) fn can_save(&self) -> bool {
        match &self.content {
            PaneContent::Terminal(_) => false,
            PaneContent::File(f) => f.view.holds_editable_buffer() && f.view.path.is_absolute(),
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
    pub(super) fn update_cached_terminal(
        &mut self,
        new_title: String,
        new_cwd: Option<PathBuf>,
    ) -> bool {
        match &mut self.content {
            PaneContent::Terminal(t) => t.update_cached(new_title, new_cwd),
            PaneContent::File(_) | PaneContent::AgentChat(_) => false,
        }
    }

    /// Compute grid dimensions and propagate to the underlying
    /// transport. Per-content dispatch — Terminal resizes the PTY +
    /// view; File content has no grid to resize and returns `true`
    /// (counts as "measured") so workspace doesn't keep retrying.
    pub(super) fn resize(
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
            PaneContent::File(_) | PaneContent::AgentChat(_) => true,
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
    pub(in crate::workspace) fn has_exited(&self) -> bool {
        self.exited
    }

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

    // A pane in view is silenced only when daruda itself is the
    // foreground app — backgrounded notifications always surface
    // because the user, by definition, is not looking at the pane.
    let suppressed_by_focus = workspace.mirrors.notifications.skip_focused_pane
        && platform::attention::is_app_active()
        && workspace.pane_on_screen(pane_id);

    match event {
        TerminalViewEvent::AttentionRequested(kind) => {
            // macOS auto-suppresses dock bounce when the app is
            // foreground, so we don't apply the focused-pane rule
            // here — the kernel of attention is "tell the user
            // something happened in the background".
            if workspace.mirrors.notifications.attention_enabled {
                platform::attention::apply(*kind);
            }
        }
        TerminalViewEvent::NotificationRequested(req) => {
            if suppressed_by_focus {
                return;
            }
            match req {
                NotificationRequest::Osc9 { body } => {
                    if workspace.mirrors.notifications.osc9_enabled {
                        platform::notifications::show(APP_NAME, body);
                    }
                }
                NotificationRequest::Osc777 { title, body } => {
                    if workspace.mirrors.notifications.osc777_enabled {
                        platform::notifications::show(title, body);
                    }
                }
            }
        }
        TerminalViewEvent::CommandFinishedAfter { elapsed } => {
            if !workspace.mirrors.notifications.long_running_enabled {
                return;
            }
            let threshold = std::time::Duration::from_secs(
                workspace.mirrors.notifications.long_running_threshold_secs,
            );
            if *elapsed < threshold {
                return;
            }
            if suppressed_by_focus {
                return;
            }
            let body = s::notification::format_duration_compact(*elapsed);
            let title = s::notification::long_running_title();
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
pub(super) fn cwd_basename(cwd: Option<&std::path::Path>) -> Option<SharedString> {
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
    /// The pane the user currently has focus on — a new pane reuses its
    /// directory (iTerm2 "Reuse previous session's directory").
    pub focused_pane: Option<PathBuf>,
    /// The active lane's filesystem path. The "always preserve
    /// 1 lane = 1 cwd" tier — wins whenever the focused-pane
    /// path is unavailable.
    pub active_lane: Option<PathBuf>,
    /// Umbrella project root. Last-resort fallback for legacy /
    /// non-lane workspaces; in the steady state it is shadowed
    /// by `active_lane` because every Workspace bootstraps at
    /// least one lane.
    pub project_root: Option<PathBuf>,
}

/// The home directory as a last-resort cwd when no workspace-level path is
/// available. `None` only when the OS reports none, or it is not an
/// accessible directory (unusual but possible in sandboxed environments).
fn home_dir() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    home.is_accessible_dir().then_some(home)
}

/// Pure resolver for a new pane's spawn cwd, keeping the "1 lane = 1 cwd"
/// invariant. Priority:
/// 1) `focused_pane`,
/// 2) `active_lane` — pins `Cmd+T` inside the lane even before OSC 7 lands,
/// 3) `project_root` — last resort for non-lane workspaces.
///
/// The `active_lane`-over-`project_root` order matters: swapping it would
/// spawn shells at the repo root from inside a lane, breaking isolation for
/// fresh starts, restored sessions, and `Cmd+T` before OSC 7 reports.
pub(in crate::workspace) fn resolve_default_cwd(candidates: CwdCandidates) -> Option<PathBuf> {
    candidates
        .focused_pane
        .or(candidates.active_lane)
        .or(candidates.project_root)
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
        resolve_default_cwd(candidates).or_else(home_dir)
    }

    pub(in crate::workspace) fn focus_pane(
        &mut self,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_page(cx);
        // The bottom input is the shared prompt/command surface for every
        // pane, so on each (re-)entry surface its panel and sync the
        // placeholder to the focused pane kind. Done here — the canonical
        // windowed focus path — so it fires on click, keyboard pane nav,
        // and tab switch alike; `set_focused_pane` deliberately keeps only
        // focus-tracking + the draft swap, leaving placeholder sync to this
        // path.
        //
        // `apply_input_placeholder` reads mode state and modifier policy
        // and writes to `InputDock::input` using the live `window` — avoids
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
            self.input_dock
                .input
                .read(cx)
                .focus_handle(cx)
                .focus(window, cx);
        } else if let Some(pane) = self.active_runtime().panes.iter().find(|p| p.id == pane_id) {
            pane.focus_handle(cx).focus(window, cx);
        }
    }
}

#[cfg(test)]
mod tests;
