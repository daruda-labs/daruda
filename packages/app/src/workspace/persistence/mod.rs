//! Workspace state persistence — save / restore project state plus the
//! helpers (`serialize_layout`, `normalize_ratios`, `rebuild_layout`)
//! translating between the in-memory pane tree and the on-disk JSON form.
//! Owns `LaneRuntime`, the per-lane runtime in the single `runtimes` map.

use crate::workspace::main_area::file_view_pane::FileOrigin;

use gpui::App;

use super::Workspace;
use crate::workspace::main_area::pane::{self, TabEntry};
use crate::workspace::main_area::pane_tree::{self as pane_tree};
use daruda_transcript::fold_mode::FoldMode;

mod rebuild;
mod restore;
mod save;

/// Runtime tab/pane state of a single lane. One of these lives in
/// `MainAreaContext::runtimes` per lane; the active lane is just the
/// entry under `Workspace::active`. Switching lanes re-points that key
/// rather than moving any fields.
#[derive(Default)]
pub(in crate::workspace) struct LaneRuntime {
    pub tabs: Vec<pane::TabEntry>,
    pub panes: Vec<pane::Pane>,
    pub active_tab_index: usize,
    /// Tab navigation history (most-recent-last). Not serialized — a
    /// session-only convenience; starts empty on every app launch.
    pub tab_history: Vec<usize>,
    pub focused_pane_id: pane_tree::PaneId,
    /// Id of the tab a left-dock preview opened, if one is still open. The
    /// *only* tab a later preview may replace — anything the user committed
    /// to by entering it (Enter, a flow, an agent link) is not this one, and
    /// so survives skimming. One `Option` rather than a flag per tab, like
    /// zed's `Pane::preview_item_id`: two preview tabs is not a state that
    /// needs representing.
    ///
    /// Not serialized, for the reason `tab_history` is not: previewing is a
    /// thing the user is doing right now, and a restored session is not in
    /// the middle of it. A restored tab is simply committed, so the first
    /// skim after a restart opens one new tab and reuses it from then on.
    pub preview_tab_id: Option<u64>,
}

impl Workspace {
    fn restore_terminal_output_for(
        &self,
        project_id: daruda_store::project::ProjectId,
        cx: &App,
    ) -> bool {
        let project = self
            .projects
            .iter()
            .find(|project| project.id == project_id);
        let user = crate::settings_store::SettingsStore::global(cx).user();
        // Persistence resolves the owning project, independent of the active UI layer.
        project
            .and_then(|project| {
                daruda_config::ProjectConfig::load_in(&self.data_dir, &project.root).shell
            })
            .map_or(user.shell.restore_output, |shell| shell.restore_output)
    }
    /// The active lane's live runtime, resolved from `runtimes` by
    /// `self.active`. Invariant: the entry is seeded at construction, on
    /// every `activate_lane`, and on `reset_to_empty_workspace`, so the
    /// `expect` never fires in correct code.
    /// Point the window at `target`, seeding its runtime first so
    /// [`Self::active_runtime`] holds from the next line on.
    pub(in crate::workspace) fn set_active(&mut self, target: daruda_store::project::LaneRef) {
        self.main_area.runtimes.entry(target).or_default();
        self.active = target;
    }

    pub(in crate::workspace) fn active_runtime(&self) -> &LaneRuntime {
        self.main_area
            .runtimes
            .get(&self.active)
            .expect("active runtime always present")
    }

    pub(in crate::workspace) fn active_runtime_mut(&mut self) -> &mut LaneRuntime {
        let active = self.active;
        self.main_area
            .runtimes
            .get_mut(&active)
            .expect("active runtime always present")
    }

    /// `get_mut` of the active lane's currently-active tab. Hoists the index
    /// read out of the mutable borrow so callers don't have to.
    pub(in crate::workspace) fn active_tab_mut(&mut self) -> Option<&mut TabEntry> {
        let idx = self.active_runtime().active_tab_index;
        self.active_runtime_mut().tabs.get_mut(idx)
    }
}

/// The account a restored agent-chat pane keeps: its persisted pin only when
/// that account belongs to `recipe`, the pane's agent's auth domain.
///
/// A cross-domain pin (or one whose account row is gone, or a pane whose agent
/// supports no managed account) can no longer resolve, so keeping it would only
/// re-serialize an account the pane can never use.
pub(in crate::workspace) fn restored_agent_account(
    persisted: Option<daruda_store::accounts::AccountId>,
    recipe: Option<daruda_store::accounts::AccountRecipeId>,
    accounts: &daruda_store::accounts::AccountsState,
) -> daruda_store::accounts::AccountSelection {
    use daruda_store::accounts::AccountSelection;
    let Some(id) = persisted else {
        return AccountSelection::SystemDefault;
    };
    match (accounts.find(id), recipe) {
        (Some(account), Some(required)) if account.recipe == required => {
            AccountSelection::Managed(id)
        }
        _ => AccountSelection::SystemDefault,
    }
}

/// The account a restored terminal pane keeps: its persisted pin only while
/// that account still exists. A terminal has no agent, so no domain narrows
/// it further — the account's own recipe decides the env.
///
/// The same rule `Workspace::reconcile_account_pins` holds for a live pane,
/// applied where a lane first loads: a lane not visited when the account was
/// deleted still carries the pin on disk.
pub(in crate::workspace) fn restored_terminal_account(
    persisted: Option<daruda_store::accounts::AccountId>,
    accounts: &daruda_store::accounts::AccountsState,
) -> daruda_store::accounts::AccountSelection {
    use daruda_store::accounts::AccountSelection;
    match persisted {
        Some(id) if accounts.find(id).is_some() => AccountSelection::Managed(id),
        _ => AccountSelection::SystemDefault,
    }
}

/// Clamp a serialized ratios vector back into a valid distribution:
/// length must match the child count; the sum must normalize to 1.0.
/// Falls back to equal splits if either invariant is violated.
pub(in crate::workspace) fn normalize_ratios(ratios: &[f32], expected_len: usize) -> Vec<f32> {
    let fallback = || vec![1.0 / expected_len as f32; expected_len];
    if ratios.len() != expected_len {
        return fallback();
    }
    let sum: f32 = ratios.iter().sum();
    if sum <= f32::EPSILON || !sum.is_finite() {
        return fallback();
    }
    ratios.iter().map(|r| r / sum).collect()
}

/// Map the in-memory `FileViewMode` to its persistence mirror.
/// Kept as a free function so the conversion lives next to the only
/// site that needs it (serialization round-trips).
fn serialize_view_mode(
    mode: crate::workspace::main_area::file_view_pane::FileViewMode,
) -> daruda_store::project::SerializedFileViewMode {
    use crate::workspace::main_area::file_view_pane::FileViewMode;
    match mode {
        FileViewMode::Raw => daruda_store::project::SerializedFileViewMode::Raw,
        FileViewMode::Preview => daruda_store::project::SerializedFileViewMode::Preview,
        FileViewMode::Changes => daruda_store::project::SerializedFileViewMode::Changes,
    }
}

/// Inverse of [`serialize_view_mode`].
pub(in crate::workspace) fn deserialize_view_mode(
    mode: daruda_store::project::SerializedFileViewMode,
) -> crate::workspace::main_area::file_view_pane::FileViewMode {
    use crate::workspace::main_area::file_view_pane::FileViewMode;
    match mode {
        daruda_store::project::SerializedFileViewMode::Raw => FileViewMode::Raw,
        daruda_store::project::SerializedFileViewMode::Preview => FileViewMode::Preview,
        daruda_store::project::SerializedFileViewMode::Changes => FileViewMode::Changes,
    }
}

fn serialize_chat_content_width(
    mode: daruda_chat::view::ChatContentWidth,
) -> daruda_store::project::SerializedChatContentWidth {
    use daruda_chat::view::ChatContentWidth;
    match mode {
        ChatContentWidth::Full => daruda_store::project::SerializedChatContentWidth::Full,
        ChatContentWidth::Reading => daruda_store::project::SerializedChatContentWidth::Reading,
    }
}

fn serialize_chat_tail_window(
    tail: daruda_chat::rows::tail::TailWindow,
) -> daruda_store::project::SerializedChatTailWindow {
    use daruda_chat::rows::tail::TailWindow;
    match tail {
        TailWindow::All => daruda_store::project::SerializedChatTailWindow::All,
        last @ TailWindow::Last(_) => {
            daruda_store::project::SerializedChatTailWindow::Last(last.size())
        }
    }
}

fn deserialize_chat_tail_window(
    tail: daruda_store::project::SerializedChatTailWindow,
) -> daruda_chat::rows::tail::TailWindow {
    use daruda_chat::rows::tail::TailWindow;
    match tail {
        daruda_store::project::SerializedChatTailWindow::All => TailWindow::All,
        daruda_store::project::SerializedChatTailWindow::Last(n) => TailWindow::last(n),
    }
}

fn deserialize_chat_content_width(
    mode: daruda_store::project::SerializedChatContentWidth,
) -> daruda_chat::view::ChatContentWidth {
    use daruda_chat::view::ChatContentWidth;
    match mode {
        daruda_store::project::SerializedChatContentWidth::Full => ChatContentWidth::Full,
        daruda_store::project::SerializedChatContentWidth::Reading => ChatContentWidth::Reading,
    }
}

/// Select the cwd to use when spawning a restored terminal pane.
///
/// `saved` is the path serialized at the time of the last save (reported
/// via OSC 7 from the shell). `worktree_root` is the physical checkout
/// directory of the owning lane.
///
/// **Invariant**: the returned path is always a descendant of
/// `worktree_root` (or `worktree_root` itself). A saved path that
/// escapes the lane — e.g. a stale cwd from a different checkout —
/// is discarded in favour of `worktree_root`. This prevents a pane from
/// opening in an unrelated directory or the parent process's cwd (HOME
/// when daruda is launched from Finder/Dock).
fn effective_cwd(
    saved: Option<std::path::PathBuf>,
    worktree_root: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    saved
        // A shell's `$PWD` keeps the symlinked or typed-case spelling the
        // lane root may not; either spelling is inside the lane.
        .filter(|s| worktree_root.is_none_or(|root| daruda_core::path::is_within(s, root)))
        .or_else(|| worktree_root.map(|p| p.to_path_buf()))
}

/// Re-anchor each lane's `path` to the project subdirectory when
/// the serialized path is a git root that *contains* the project root.
///
/// **Before the bootstrap fix**: `Lane::bootstrap_from_project`
/// stored the git root (`/repo`) as the lane path even when the
/// user opened a subdirectory (`/repo/subdir`). After the fix, the
/// path is anchored to `project_root`. This function updates surviving
/// old-format entries so restored cwd anchoring (`effective_cwd`)
/// rejects out-of-lane paths correctly.
///
/// **Prefix relationship**:
/// ```text
/// wt.path  = /repo          (git root — the old value)
/// canonical = /repo/subdir  (project root — the new anchor)
///
/// canonical.starts_with(wt.path)  →  true   (subdir of the lane)
/// canonical != wt.path            →  true   (not identical — needs update)
/// ```
/// Only the first matching lane is updated because each project has
/// exactly one primary checkout that maps to the project root.
fn anchor_lane_paths_to_project_root(
    lanes: &mut [crate::lane::Lane],
    project_root: &std::path::Path,
) {
    if let Ok(canonical) = daruda_core::path::canonicalize(project_root) {
        for wt in lanes.iter_mut() {
            if canonical.starts_with(&wt.path) && canonical != wt.path {
                wt.path = canonical;
                break;
            }
        }
    }
}

/// What this pane restores as. The kinds are exclusive, and this is where
/// that is decided — a leaf can no longer be handed two of them.
fn serialize_pane_content(
    pane: &pane::Pane,
    lane: daruda_store::project::LaneId,
    cx: &App,
) -> daruda_store::project::SerializedPaneContent {
    use daruda_store::project::SerializedPaneContent as Content;
    if let Some(fv) = pane.file_view() {
        let (staged, range) = fv.source.to_serialized();
        return Content::File(daruda_store::project::SerializedFileContent {
            lane_id: lane,
            path: fv.path.clone(),
            staged,
            range,
            view_mode: serialize_view_mode(fv.view_mode),
            reference: fv.origin == FileOrigin::Reference,
        });
    }
    if let Some(ac) = pane.agent_chat_content() {
        let v = ac.view.read(cx);
        // Only explicit choices are written, so an untouched pane keeps
        // following config.
        let choices = v.pane_choices();
        return Content::AgentChat(daruda_store::project::SerializedAgentChatContent {
            access: v.access(),
            cwd: ac.cwd.clone(),
            session_id: v.session_id().map(str::to_owned),
            title: v.session_title().map(str::to_owned),
            agent_id: Some(v.agent_id().to_owned()),
            account_id: ac.account.to_persisted(),
            picked_mode_id: choices.mode_id,
            model_id: choices.model_id,
            content_width: choices.content_width.map(serialize_chat_content_width),
            tail_window: choices.tail_steps.map(serialize_chat_tail_window),
            tail_window_calls: choices.tail_calls.map(serialize_chat_tail_window),
            // Superseded field: never written, so a file carrying it sheds it
            // on the first save after the split.
            display_filter: None,
            visible_kinds: choices
                .display_filter
                .map(|f| f.tokens().into_iter().map(str::to_owned).collect()),
            fold_mode: choices.fold_mode.map(FoldMode::tokens),
        });
    }
    Content::Terminal {
        cwd: pane.cwd().map(std::path::Path::to_path_buf),
        account_id: pane.terminal_account_id(),
    }
}

fn serialize_layout(
    layout: &pane_tree::PaneLayout,
    panes: &[pane::Pane],
    lane: daruda_store::project::LaneId,
    cx: &App,
) -> daruda_store::project::SerializedLayout {
    match layout {
        pane_tree::PaneLayout::Pane(id) => {
            let pane = panes.iter().find(|p| p.id == *id);
            // One kind, chosen once. What each kind keeps is its own: a File
            // pane's viewer state (its cwd is `path.parent()` at runtime), an
            // AgentChat's cwd plus the ACP session id and title so a later
            // launch resumes the conversation via `session/load`, a FlowGraph's
            // file path and nothing else — the graph is re-derived from that
            // file on every open — and a Terminal's cwd and account.
            let content = pane.map_or(
                daruda_store::project::SerializedPaneContent::Terminal {
                    cwd: None,
                    account_id: None,
                },
                |p| serialize_pane_content(p, lane, cx),
            );
            daruda_store::project::SerializedLayout::Leaf {
                pane_id: *id,
                content,
            }
        }
        pane_tree::PaneLayout::Split {
            direction,
            children,
            ratios,
        } => {
            let dir = match direction {
                pane_tree::SplitDirection::Horizontal => {
                    daruda_store::project::SplitDirectionSerde::Horizontal
                }
                pane_tree::SplitDirection::Vertical => {
                    daruda_store::project::SplitDirectionSerde::Vertical
                }
            };
            daruda_store::project::SerializedLayout::Split {
                direction: dir,
                children: children
                    .iter()
                    .map(|c| serialize_layout(c, panes, lane, cx))
                    .collect(),
                ratios: ratios.clone(),
            }
        }
    }
}
