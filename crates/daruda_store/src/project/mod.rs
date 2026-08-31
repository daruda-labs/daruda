//! Project management and state persistence for daruda.
//!
//! A **project** is a root directory plus saved workspace state. This
//! crate is GPUI-free so persistence logic stays unit-testable.
//!
//! Schema-entry-point types ([`WorkspaceState`], [`ProjectState`]) and
//! per-file persistence live in [`types`] and [`persistence`]. Low-level
//! building blocks ([`SerializedLane`], [`SerializedGroup`],
//! [`SerializedProject`], [`LaneRef`], dock / window state, etc.)
//! stay at this level so views and other consumers can import them
//! without pulling in the persistence layer.

pub mod lane;
pub mod persistence;
pub mod session_host_id;
pub mod types;

#[cfg(test)]
mod tests;

pub use lane::{
    LaneId, LaneKind, LaneSessionHost, LaneStatus, LeftDockView, RightDockView, SerializedLane,
};
pub use persistence::{
    RECENT_MAX, delete_project_state_in, delete_workspace_state_in, for_each_project_state_in,
    for_each_workspace_state_in, is_uuid_filename_stem, load_project_state_in, load_recent_in,
    load_workspace_state_in, projects_dir_in, recent_path_in, save_project_state_in,
    save_recent_in, save_workspace_state_in, touch_recent_in, workspaces_dir_in,
};
pub use session_host_id::SessionHostId;
pub use types::{
    PaneId, PaneLayout, ProjectOverride, ProjectState, ProjectUuid, RecentEntry,
    WORKSPACE_SCHEMA_VERSION, WorkspaceState, WorkspaceUuid,
};

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Stable identifier for a project within a workspace. Monotonic per
/// workspace — deleted IDs are never reused so stale references fail
/// the active-pointer fallback ladder (in
/// `Workspace::restore_from_disk::resolve_active`) instead of silently
/// targeting a different project.
pub type ProjectId = u64;

/// Stable identifier for a group within a workspace. Same monotonic
/// rule as [`ProjectId`].
pub type GroupId = u64;

/// A project = a root directory.
#[derive(Clone, Debug)]
pub struct Project {
    pub root: PathBuf,
    pub name: String,
}

impl Project {
    /// Create a project from a directory path. Name = last path component.
    pub fn from_path(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let name = derive_name_from_path(&root);
        Self { root, name }
    }
}

/// Compute a display name from a filesystem path — last path component,
/// or `"untitled"` for root / empty paths. Shared by [`Project::from_path`]
/// and the on-disk-state hydration path so all sources produce identical
/// names.
pub fn derive_name_from_path(root: &Path) -> String {
    root.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("untitled")
        .to_string()
}

// ============================================================================
// Multi-project / Group shape (the on-disk schema).
// ============================================================================

/// Active-tab pointer in the multi-project model. A workspace always
/// points at exactly one (project, lane) pair; an invalid pair is
/// repaired by the runtime restore path
/// (`Workspace::restore_from_disk::resolve_active`) at load time so
/// downstream code never has to handle dangling refs.
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaneRef {
    pub project: ProjectId,
    #[serde(rename = "worktree", alias = "lane")]
    pub lane: LaneId,
}

/// User policy for the "Open Project…" affordance. Persists across
/// launches so the user can opt out of the modal by ticking "Don't
/// ask again" once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowOpenPolicy {
    /// Show the chooser modal (default).
    #[default]
    Ask,
    /// Always add the new project to the current window.
    AddHere,
    /// Always open the new project in a fresh window.
    NewWindow,
}

/// User-defined group of projects in the left dock. Groups carry only
/// visual metadata (name, optional color, collapsed state, tab order);
/// projects reference their group via [`SerializedProject::group_id`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SerializedGroup {
    pub id: GroupId,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default)]
    pub tab_order: u32,
    #[serde(default)]
    pub is_collapsed: bool,
}

/// Serializable per-project payload bundled inside a workspace
/// snapshot. Each project owns its own lanes and tracks which
/// lane was last active so clicking the project header in the
/// left dock can snap to that lane.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SerializedProject {
    pub id: ProjectId,
    pub root: PathBuf,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default)]
    pub tab_order: u32,
    /// `None` = ungrouped (rendered at top level alongside groups in
    /// the same `tab_order` pool).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_id: Option<GroupId>,
    #[serde(default, rename = "worktrees", alias = "lanes")]
    pub lanes: Vec<SerializedLane>,
    /// Last lane the user activated inside this project. Used as a
    /// snap hint when the project becomes active without a specific
    /// lane pick.
    #[serde(
        default,
        rename = "last_active_worktree_id",
        alias = "last_active_lane_id"
    )]
    pub last_active_lane_id: LaneId,
    /// True when the project header is rendered in the left dock with
    /// its lane list hidden. Click on the chevron toggles.
    /// `#[serde(default)]` so older state files load as expanded.
    #[serde(default)]
    pub is_collapsed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SerializedTab {
    pub layout: SerializedLayout,
    pub last_focused_pane: u64,
    /// User-set tab title (Window > Edit Tab Title…). When present
    /// it overrides the auto-derived title (cwd basename / PTY title)
    /// in the tab strip. Old state files without the field decode as
    /// `None` via `#[serde(default)]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_label: Option<String>,
}

/// `large_enum_variant` is allowed for the same reason as its runtime
/// counterpart `PaneContent`: one leaf per pane, built only on save and read
/// only on restore. Boxing a variant's payload would buy stack bytes nobody
/// spends and cost an allocation per leaf on both paths.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "RawLayout", into = "RawLayout")]
pub enum SerializedLayout {
    Leaf {
        pane_id: u64,
        content: SerializedPaneContent,
    },
    Split {
        direction: SplitDirectionSerde,
        children: Vec<SerializedLayout>,
        ratios: Vec<f32>,
    },
}

/// What a leaf restores as — one variant per [`PaneContent`] kind that
/// persists.
///
/// An enum rather than a field per kind: the file format spells them as four
/// optional keys, and "at most one is set" is not something optional keys can
/// say. Three of them had accumulated, each documented as mutually exclusive
/// with the others, which is a rule a reader has to keep rather than one the
/// type keeps for them.
///
/// TaskEdit panes are absent on purpose: they hold an unsaved form, and
/// restoring one would put a half-typed task back on screen as if it had been
/// kept.
///
/// Deliberately not `Serialize`/`Deserialize`: the file's shape is
/// [`RawLayout`]'s, and deriving them here would let this type reach a state
/// file the moment somebody dropped the conversion. Without them, that is a
/// compile error rather than a format change nobody notices.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum SerializedPaneContent {
    /// A shell. `cwd` is the last value reported via OSC 7; `account_id` is
    /// the managed account it runs under, `None` being the system default.
    Terminal {
        cwd: Option<PathBuf>,
        account_id: Option<crate::accounts::AccountId>,
    },
    /// A file viewer. Its cwd is `path.parent()` at runtime, so none is kept.
    File(SerializedFileContent),
    /// An agent chat. Carries its own cwd and account.
    AgentChat(SerializedAgentChatContent),
    /// A flow graph, which is the file's path and nothing else.
    FlowGraph(SerializedFlowGraphContent),
}

/// The file's own shape: a leaf is a `pane_id` plus four optional keys, at
/// most one of the last three set.
///
/// Kept exactly as it was written so no state file needs migrating, and so a
/// build without [`SerializedPaneContent`] still reads what this one writes.
/// The conversion below is the only place the two shapes meet.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
enum RawLayout {
    Leaf {
        pane_id: u64,
        cwd: Option<PathBuf>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        file: Option<SerializedFileContent>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent_chat: Option<SerializedAgentChatContent>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        flow_graph: Option<SerializedFlowGraphContent>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account_id: Option<crate::accounts::AccountId>,
    },
    Split {
        direction: SplitDirectionSerde,
        children: Vec<SerializedLayout>,
        ratios: Vec<f32>,
    },
}

impl From<RawLayout> for SerializedLayout {
    fn from(raw: RawLayout) -> Self {
        match raw {
            RawLayout::Leaf {
                pane_id,
                cwd,
                file,
                agent_chat,
                flow_graph,
                account_id,
            } => {
                // First one wins, and the order is the one restore already
                // read them in. A file holding two is not something this app
                // writes; picking silently beats refusing to open a window.
                let content = if let Some(file) = file {
                    SerializedPaneContent::File(file)
                } else if let Some(chat) = agent_chat {
                    SerializedPaneContent::AgentChat(chat)
                } else if let Some(graph) = flow_graph {
                    SerializedPaneContent::FlowGraph(graph)
                } else {
                    SerializedPaneContent::Terminal { cwd, account_id }
                };
                SerializedLayout::Leaf { pane_id, content }
            }
            RawLayout::Split {
                direction,
                children,
                ratios,
            } => SerializedLayout::Split {
                direction,
                children,
                ratios,
            },
        }
    }
}

impl From<SerializedLayout> for RawLayout {
    fn from(layout: SerializedLayout) -> Self {
        match layout {
            SerializedLayout::Leaf { pane_id, content } => {
                let (cwd, account_id) = match &content {
                    SerializedPaneContent::Terminal { cwd, account_id } => {
                        (cwd.clone(), *account_id)
                    }
                    _ => (None, None),
                };
                RawLayout::Leaf {
                    pane_id,
                    cwd,
                    account_id,
                    file: match &content {
                        SerializedPaneContent::File(fc) => Some(fc.clone()),
                        _ => None,
                    },
                    agent_chat: match &content {
                        SerializedPaneContent::AgentChat(ac) => Some(ac.clone()),
                        _ => None,
                    },
                    flow_graph: match content {
                        SerializedPaneContent::FlowGraph(fg) => Some(fg),
                        _ => None,
                    },
                }
            }
            SerializedLayout::Split {
                direction,
                children,
                ratios,
            } => RawLayout::Split {
                direction,
                children,
                ratios,
            },
        }
    }
}

/// Persisted state for a `PaneContent::File` leaf — enough to
/// reconstruct the file viewer on the next launch. `file_status`
/// (the git badge) is intentionally omitted: it depends on live git
/// state and re-derives when the lane's git status refreshes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SerializedFileContent {
    #[serde(rename = "worktree_id", alias = "lane_id")]
    pub lane_id: LaneId,
    pub path: PathBuf,
    #[serde(default)]
    pub staged: bool,
    pub view_mode: SerializedFileViewMode,
}

/// Persisted state for a `PaneContent::FlowGraph` leaf. The flow file's path
/// is the whole of it: the graph — nodes, edges, placement — is derived from
/// that file on every open, so persisting any of it would let a layout
/// outlive the YAML it was read from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SerializedFlowGraphContent {
    pub path: PathBuf,
}

/// Private wire representation for [`PaneCwd`]. **Not** the internally
/// tagged `#[serde(tag = "type")]` shape used elsewhere in this crate for a
/// similarly-shaped local/remote split ([`LaneKind`]): that shape requires
/// serde to merge the tag key into the variant's serialized form, which
/// works for struct variants but not for a newtype variant whose payload is
/// itself a bare scalar — both `PathBuf` and `String` serialize as a plain
/// JSON string, and serde errors ("cannot serialize tagged newtype variant
/// ... containing a string") rather than emitting one. Untagged dispatch
/// instead: `Remote` is tried first (it only matches an object with a
/// `remote` key) and falls through to `Local` (any bare string) otherwise —
/// which is also exactly the pre-existing on-disk shape of the
/// `Option<PathBuf>` this field replaced, so an old saved `cwd` string loads
/// unchanged as `Local`.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum PaneCwdRepr {
    Remote { remote: String },
    Local(PathBuf),
}

impl From<PaneCwd> for PaneCwdRepr {
    fn from(v: PaneCwd) -> Self {
        match v {
            PaneCwd::Local(p) => PaneCwdRepr::Local(p),
            PaneCwd::Remote(s) => PaneCwdRepr::Remote { remote: s },
        }
    }
}

impl From<PaneCwdRepr> for PaneCwd {
    fn from(v: PaneCwdRepr) -> Self {
        match v {
            PaneCwdRepr::Local(p) => PaneCwd::Local(p),
            PaneCwdRepr::Remote { remote } => PaneCwd::Remote(remote),
        }
    }
}

/// The working directory an agent-chat pane's session is rooted at: either
/// a path on this machine, or an opaque remote-side identifier this machine
/// cannot resolve, canonicalize, or validate directly (e.g. an
/// SSH-reachable host path) — mirroring the local/remote split
/// [`SerializedLane::remote_cwd`] makes for lanes.
///
/// Every consumer that spawns a local process, derives a `Cmd+T` inherited
/// cwd, or matches a local [`crate::tasks::Task`]'s `worktree_path` is
/// local-only by construction, so it must go through [`PaneCwd::as_local`] /
/// [`PaneCwd::into_local`] rather than assuming the value is a `PathBuf` —
/// the gate that keeps a `Remote` value from leaking into a call that
/// expects a real filesystem path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "PaneCwdRepr", into = "PaneCwdRepr")]
pub enum PaneCwd {
    Local(PathBuf),
    Remote(String),
}

impl PaneCwd {
    /// Borrow the local path, or `None` for a [`PaneCwd::Remote`] value.
    pub fn as_local(&self) -> Option<&Path> {
        match self {
            PaneCwd::Local(p) => Some(p),
            PaneCwd::Remote(_) => None,
        }
    }

    /// Consume into the local path, or `None` for a [`PaneCwd::Remote`]
    /// value.
    pub fn into_local(self) -> Option<PathBuf> {
        match self {
            PaneCwd::Local(p) => Some(p),
            PaneCwd::Remote(_) => None,
        }
    }
}

/// Persisted state for a `PaneContent::AgentChat` leaf. The lane working
/// directory anchors the pane to the right lane on the next launch; the
/// ACP `session_id` (when present) lets the pane resume the prior
/// conversation via `session/load` on first focus rather than starting a
/// fresh session. The conversation itself is not stored — the adapter
/// replays it from the resumed session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SerializedAgentChatContent {
    /// Lane working directory the agent session is rooted at. `None`
    /// when the pane was opened without a resolvable lane cwd.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PaneCwd>,
    /// Persisted ACP session id. `Some` once a live session has been
    /// established; on the next launch the pane resumes it via
    /// `session/load` (replaying the prior conversation) instead of
    /// starting a fresh session. `None` for a pane that never connected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Agent-provided session title, cached so a restored dormant pane
    /// shows its label before the session loads. `None` = fallback label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Agent this chat runs under (id from the config `[[agents]]` catalog).
    /// `None` = a pre-feature save, treated as the built-in Claude agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// Managed account this pane runs under; `None` = the system default
    /// (ambient environment, no config-dir override).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<crate::accounts::AccountId>,
    /// ACP session-mode id the host last saw this session in (e.g.
    /// `"acceptEdits"`), persisted so a resumed session (`session/load`) can
    /// have it reapplied via `session/set_mode`. `None` when the agent has no
    /// modes, or none was ever observed.
    ///
    /// WORKAROUND: `session/load`'s response can in principle report the
    /// resumed session's real mode, but at least one shipped adapter
    /// (`claude-agent-acp`) recomputes it from static settings on every
    /// process launch instead of the session's actual last mode — so the
    /// host tracks and reapplies it itself. See `daruda_acp::session`'s
    /// `restore_mode` parameter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode_id: Option<String>,
    /// Model id this pane's user explicitly picked. Persisted so the next
    /// connection can request it during the handshake; an agent catalog's
    /// `default_model` is deliberately not recorded here, because it must stay
    /// editable independently of a pane preference. `None` when no user pick
    /// has been made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    /// Per-pane AgentChat content width mode. Missing in pre-feature state
    /// files, defaulting to `Full` so existing panes keep using the whole pane.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "SerializedChatContentWidth::is_full"
    )]
    pub content_width: SerializedChatContentWidth,
    /// Explicit pane tail choice; `None` continues following config.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub tail_window: Option<SerializedChatTailWindow>,
    /// Superseded by [`Self::visible_kinds`]; read, never written.
    ///
    /// It listed the same visible set, but wrote an empty list to mean
    /// "unfiltered" — the one value the current reading gives the opposite
    /// meaning (a pane the user unchecked entirely). A separate field keeps the
    /// two apart without a version marker, and `skip_serializing_if` drops this
    /// one on the next save, so a file heals itself once.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub display_filter: Option<Vec<String>>,
    /// Kinds of work this pane shows, named by facet token. `None` means the
    /// pane never chose and shows everything; `Some(vec![])` is a real value —
    /// the pane whose every box the user unchecked.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub visible_kinds: Option<Vec<String>>,
    /// Explicit pane fold-mode tokens; `None` continues following config.
    #[serde(
        default,
        deserialize_with = "lenient",
        skip_serializing_if = "Option::is_none"
    )]
    pub fold_mode: Option<Vec<String>>,
}

/// Default an unknown preference value instead of rejecting the project state.
/// JSON buffering is required because a failed deserializer cannot be rewound.
fn lenient<'de, D, T>(de: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    let value = serde_json::Value::deserialize(de)?;
    Ok(T::deserialize(value).unwrap_or_default())
}

/// Serializable mirror of the app-side AgentChat content-width mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SerializedChatContentWidth {
    #[default]
    Full,
    Reading,
}

impl SerializedChatContentWidth {
    fn is_full(&self) -> bool {
        matches!(self, Self::Full)
    }
}

/// Serializable mirror of the AgentChat tail window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SerializedChatTailWindow {
    #[default]
    All,
    Last(u8),
}

/// Serializable mirror of `daruda::workspace::pane_file_view::FileViewMode`.
/// Lives in `daruda_project` so the persistence layer stays free of
/// app-side types; the conversion is one-to-one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SerializedFileViewMode {
    Raw,
    Preview,
    Changes,
}

/// Serializable split direction — validated on deserialization.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitDirectionSerde {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DockStates {
    pub left_open: bool,
    pub left_size: f32,
    pub bottom_open: bool,
    pub bottom_size: f32,
    pub right_open: bool,
    pub right_size: f32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl WindowState {
    /// True when the state has usable values (not all-zero default).
    pub fn is_valid(&self) -> bool {
        self.width > 0.0 && self.height > 0.0
    }
}
