//! Workspace state persistence — save / restore project state plus
//! the helpers (`serialize_layout`, `normalize_ratios`,
//! `rebuild_layout`) that translate between the in-memory pane tree
//! and the on-disk JSON form.
//!
//! Owns `LaneRuntime`, the frozen-fields struct used by the
//! tab-swap path; both `restore_from_disk` and `activate_lane`
//! live across persistence + lane_ops, but the *type* belongs here
//! so the inactive map and the save format share a single definition.

use std::collections::{BTreeMap, HashMap};

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::observability::system_info::redact_home;
use daruda_store::project::{
    DockStates, ProjectOverride, ProjectState, ProjectUuid, SerializedTab,
    WORKSPACE_SCHEMA_VERSION, WorkspaceState,
};
use gpui::{App, Context, Window};

use super::Workspace;
use crate::lane::availability::LaneAvailability;
use crate::workspace::main_area::pane::{self, PaneSpawnError, TabEntry};
use crate::workspace::main_area::pane_tree::{self as pane_tree, PaneLayout, SplitDirection};

/// Frozen runtime state of a non-active lane. `activate_lane`
/// swaps this with the live `Workspace` fields (`tabs`, `panes`, etc.).
/// Kept in its own struct so the swap is a single `std::mem::take` +
/// assignment pair per direction, rather than a tangle of field moves.
#[derive(Default)]
pub(in crate::workspace) struct LaneRuntime {
    pub tabs: Vec<pane::TabEntry>,
    pub panes: Vec<pane::Pane>,
    pub active_tab_index: usize,
    /// Tab navigation history (most-recent-last). Not serialized —
    /// history is a session-only convenience; cold-restoring it would
    /// be confusing because the user's intent from a previous session
    /// is unknown. Starts empty on every app launch.
    pub tab_history: Vec<usize>,
    pub focused_pane_id: pane_tree::PaneId,
}

impl Workspace {
    /// Snapshot the workspace into the UUID-keyed disk shape
    /// (`(WorkspaceState, Vec<ProjectState>)`).
    ///
    /// Each project's intrinsic data (root / name / lanes / last-
    /// active hint) lives in its own [`ProjectState`]; per-workspace
    /// decoration (color / tab order / group / collapsed flag) lives
    /// in [`WorkspaceState::project_overrides`]. The active focus is
    /// projected from `self.active` (runtime `LaneRef`) into
    /// `(active_project: ProjectUuid, active_lane: LaneId)` —
    /// runtime `ProjectId` is per-session and therefore not persisted.
    ///
    /// Returns `None` when `self.projects` is empty (Welcome state) —
    /// the caller has nothing to write.
    ///
    /// Drives [`Workspace::persist_state`].
    pub(in crate::workspace) fn snapshot_for_disk(
        &self,
        cx: &App,
    ) -> Option<(WorkspaceState, Vec<ProjectState>)> {
        if self.projects.is_empty() {
            return None;
        }

        let mut project_states = Vec::with_capacity(self.projects.len());
        let mut project_ids = Vec::with_capacity(self.projects.len());
        let mut project_overrides = BTreeMap::new();
        let mut project_tabs: BTreeMap<ProjectUuid, Vec<SerializedTab>> = BTreeMap::new();

        for project in &self.projects {
            // Per-lane serialized payload — captures the active
            // lane's live `main_area` tab tree, plus any inactive
            // lane's stashed runtime.
            let lanes: Vec<daruda_store::project::SerializedLane> = project
                .lanes
                .iter()
                .map(|wt| {
                    let mut s = wt.to_serialized();
                    let wt_ref = daruda_store::project::LaneRef {
                        project: project.id,
                        lane: wt.id,
                    };
                    let (tabs_src, panes_src, active_idx) = if self.active == wt_ref {
                        (
                            &self.main_area.tabs,
                            &self.main_area.panes,
                            self.main_area.active_tab_index,
                        )
                    } else if let Some(rt) = self.main_area.inactive_lane_runtimes.get(&wt_ref) {
                        (&rt.tabs, &rt.panes, rt.active_tab_index)
                    } else {
                        return s;
                    };
                    s.tabs = tabs_src
                        .iter()
                        .map(|tab| daruda_store::project::SerializedTab {
                            layout: serialize_layout(&tab.layout, panes_src),
                            last_focused_pane: tab.last_focused_pane,
                            user_label: tab.user_label.as_ref().map(|s| s.to_string()),
                        })
                        .collect();
                    s.active_tab_index = active_idx;
                    s
                })
                .collect();

            // `next_lane_id`: smallest id strictly greater than every
            // lane currently registered for this project (both live
            // lanes and any stashed inactive runtimes for the same
            // project). Matches the "monotonic — never reused" invariant
            // enforced by `allocate_lane_id`.
            let max_live = project.lanes.iter().map(|w| w.id).max();
            let max_inactive = self
                .main_area
                .inactive_lane_runtimes
                .keys()
                .filter(|r| r.project == project.id)
                .map(|r| r.lane)
                .max();
            let next_lane_id = match (max_live, max_inactive) {
                (Some(a), Some(b)) => a.max(b) + 1,
                (Some(a), None) => a + 1,
                (None, Some(b)) => b + 1,
                (None, None) => 0,
            };

            // Flatten this project's per-lane tabs into the
            // workspace envelope's per-project bucket. Lane order
            // matches `project.lanes`.
            let mut flat_tabs: Vec<SerializedTab> = Vec::new();
            for w in &lanes {
                flat_tabs.extend(w.tabs.iter().cloned());
            }
            project_tabs.insert(project.uuid, flat_tabs);

            project_states.push(ProjectState {
                schema_version: WORKSPACE_SCHEMA_VERSION,
                uuid: project.uuid,
                root: project.root.clone(),
                name: Some(project.name.clone()),
                lanes,
                last_active_lane_id: project.last_active_lane_id,
                next_lane_id,
                default_branch: project.default_branch.clone(),
                base_branch: project.base_branch.clone(),
            });

            project_ids.push(project.uuid);
            project_overrides.insert(
                project.uuid,
                ProjectOverride {
                    color: project.color.clone(),
                    tab_order: project.tab_order as usize,
                    group_id: project.group_id,
                    is_collapsed: project.is_collapsed,
                },
            );
        }

        // Project the runtime `LaneRef` (per-session `ProjectId`)
        // onto the persisted UUID. If the active project has been
        // closed, both fields fall to `None`.
        let active_project = self
            .projects
            .iter()
            .find(|p| p.id == self.active.project)
            .map(|p| p.uuid);
        let active_lane = active_project.map(|_| self.active.lane);

        let workspace = WorkspaceState {
            schema_version: WORKSPACE_SCHEMA_VERSION,
            uuid: self.uuid,
            project_ids,
            project_overrides,
            groups: self.groups.clone(),
            active_project,
            active_lane,
            docks: DockStates {
                left_open: self.left_dock.read(cx).is_open,
                left_size: self.left_dock.read(cx).size,
                bottom_open: self.bottom_dock.read(cx).is_open,
                bottom_size: self.bottom_dock.read(cx).size,
                right_open: self.right_dock.read(cx).is_open,
                right_size: self.right_dock.read(cx).size,
            },
            window: self.cached_window_bounds.clone().unwrap_or_default(),
            font_size: self.terminal_config.font_size,
            vertical_spacing: self.terminal_config.vertical_spacing,
            horizontal_spacing: self.terminal_config.horizontal_spacing,
            focused_pane_id: self.main_area.focused_pane_id,
            active_dock_view: self.left_dock_view,
            active_right_panel_view: self.right_dock_view,
            window_open_policy: self.window_open_policy,
            next_group_id: self.next_group_id,
            project_tabs,
        };

        Some((workspace, project_states))
    }

    /// Sample the current window's windowed bounds (position + size)
    /// and store them in `cached_window_bounds`. Skips fullscreen and
    /// maximized states — persisting those would relaunch the app in
    /// that mode next time even if the user exited it before quitting.
    /// The last known windowed geometry (from before entering those
    /// states) remains cached.
    pub(in crate::workspace) fn capture_window_bounds(&mut self, window: &Window) {
        let bounds = match window.window_bounds() {
            gpui::WindowBounds::Windowed(b) => b,
            gpui::WindowBounds::Maximized(_) | gpui::WindowBounds::Fullscreen(_) => return,
        };
        let new = daruda_store::project::WindowState {
            x: f32::from(bounds.origin.x),
            y: f32::from(bounds.origin.y),
            width: f32::from(bounds.size.width),
            height: f32::from(bounds.size.height),
        };
        if !new.is_valid() {
            return;
        }
        self.cached_window_bounds = Some(new);
    }

    /// Persist state to disk via the new UUID-keyed layout.
    ///
    /// Writes one workspace file (`workspaces/<uuid>.json`) referencing
    /// each project, plus one project file (`projects/<uuid>.json`) per
    /// owned project. No fan-out — each project's intrinsic data lives
    /// in exactly one file, and only this workspace's file references
    /// them. Other workspaces holding the same project see updates via
    /// the shared project file (last-writer-wins on intrinsic lane
    /// list).
    ///
    /// Updates `recent-workspaces.json` with this workspace's display
    /// name.
    pub fn persist_state(&self, cx: &App) {
        let Some((workspace, projects)) = self.snapshot_for_disk(cx) else {
            return;
        };

        for project in &projects {
            if let Err(e) = daruda_store::project::save_project_state_in(&self.data_dir, project) {
                LogWriter::log(
                    ErrorReport::new("Failed to persist project state")
                        .severity(ErrorSeverity::Error)
                        .from_error(&e)
                        .at(file!(), line!())
                        .with_context("project_uuid", project.uuid.as_inner().to_string())
                        .with_context("project_root", redact_home(&project.root))
                        .dedup("project.save")
                        .build(),
                );
            }
        }

        if let Err(e) = daruda_store::project::save_workspace_state_in(&self.data_dir, &workspace) {
            LogWriter::log(
                ErrorReport::new("Failed to persist workspace state")
                    .severity(ErrorSeverity::Error)
                    .from_error(&e)
                    .at(file!(), line!())
                    .with_context("workspace_uuid", workspace.uuid.as_inner().to_string())
                    .dedup("workspace.save")
                    .build(),
            );
        }

        let display_name = self.recent_display_name();
        if let Err(e) =
            daruda_store::project::touch_recent_in(&self.data_dir, workspace.uuid, display_name)
        {
            LogWriter::log(
                ErrorReport::new("Failed to update recent list")
                    .severity(ErrorSeverity::Warning)
                    .from_error(&e)
                    .at(file!(), line!())
                    .dedup("recent.touch")
                    .build(),
            );
        }
    }

    /// Restore from the UUID-keyed disk shape
    /// (`(WorkspaceState, &[ProjectState])` from
    /// [`Workspace::snapshot_for_disk`]). Rebuilds every project's
    /// lanes, dock open/size, font settings, and the full tab /
    /// split-tree layout from the persisted state. Each restored leaf
    /// starts at its serialized cwd so the shell opens where it was
    /// last tracked. On any pane spawn failure, falls back to a single
    /// fresh tab and surfaces the error in the status bar.
    ///
    /// Each runtime [`crate::project::Project`] is hydrated from its
    /// `ProjectState` plus the per-workspace `ProjectOverride`; runtime
    /// `ProjectId`s are minted dense `0..N` so the canonical ordering
    /// is `project_states` iteration order.
    pub(crate) fn restore_from_disk(
        &mut self,
        workspace: &WorkspaceState,
        project_states: &[ProjectState],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Restore dock states.
        let left_open = workspace.docks.left_open;
        let left_size = workspace.docks.left_size;
        self.left_dock.update(cx, |d, _| {
            d.is_open = left_open;
            if left_size > 0.0 {
                d.size = left_size;
            }
        });
        self.left_dock_view = workspace.active_dock_view;
        self.right_dock_view = workspace.active_right_panel_view;
        let bottom_open = workspace.docks.bottom_open;
        let bottom_size = workspace.docks.bottom_size;
        self.bottom_dock.update(cx, |d, _| {
            d.is_open = bottom_open;
            if bottom_size > 0.0 {
                d.size = bottom_size;
            }
        });
        let right_open = workspace.docks.right_open;
        let right_size = workspace.docks.right_size;
        self.right_dock.update(cx, |d, _| {
            d.is_open = right_open;
            if right_size > 0.0 {
                d.size = right_size;
            }
        });

        // Restore font settings.
        self.terminal_config.font_size = workspace.font_size;
        self.terminal_config.vertical_spacing = workspace.vertical_spacing;
        self.terminal_config.horizontal_spacing = workspace.horizontal_spacing;
        self.terminal_config.clamp_font_settings();

        // Adopt the persisted workspace UUID so subsequent saves land
        // on the same on-disk record.
        self.uuid = workspace.uuid;

        // Restore workspace-level multi-project metadata.
        self.window_open_policy = workspace.window_open_policy;
        self.next_group_id = workspace.next_group_id;
        self.groups = workspace.groups.clone();
        // Cache window bounds so the next save round-trips the same
        // geometry even if `observe_window_bounds` hasn't fired yet.
        self.cached_window_bounds = Some(workspace.window.clone());

        // Empty workspace — keep whatever `new_with_project` bootstrapped.
        if project_states.is_empty() {
            self.main_area.pending_resize = true;
            return;
        }

        // Mint dense runtime ids `0..N` for the hydrated projects. The
        // new schema has no `next_project_id`, so the counter advances
        // past the highest minted id — same monotonic invariant
        // `add_project` enforces.
        self.projects = project_states
            .iter()
            .enumerate()
            .map(|(idx, ps)| {
                let runtime_id = idx as daruda_store::project::ProjectId;
                let ov = workspace
                    .project_overrides
                    .get(&ps.uuid)
                    .cloned()
                    .unwrap_or_default();
                let mut p = crate::project::Project::from_disk(runtime_id, ps, &ov);
                let root = p.root.clone();
                anchor_lane_paths_to_project_root(&mut p.lanes, &root);
                p
            })
            .collect();
        self.next_project_id = self.projects.len() as daruda_store::project::ProjectId;

        // Classify every project / lane root against the live filesystem
        // before the pane-rebuild loop runs. The persisted set may
        // reference directories that no longer exist (lane deleted, repo
        // moved); stamping the runtime `availability` flag here lets the
        // skip below and the read side (file-tree scan, watcher, PTY
        // spawn) short-circuit instead of spamming directory-read errors.
        self.recompute_availability();

        // Re-detect each git project's `default_branch` from git and
        // update the runtime project if it drifted. Backfills legacy
        // state files (where `default_branch` is `None`) and absorbs
        // external drift. Runs async on the background executor; the
        // persisted value stands until detection completes. Placed
        // before the pane-rebuild loop so it fires even when a later
        // restore step early-returns.
        self.reconcile_project_default_branches(cx);
        // Heal any project persisted while its lanes were still the
        // construction placeholder (single `Default` lane for a git
        // root — e.g. the app died before the async discovery
        // returned). Same async upgrade pass as fresh-open.
        self.reconcile_bootstrapped_lanes(cx);

        // Project the persisted (active_project: ProjectUuid,
        // active_lane: LaneId) onto the runtime `LaneRef`
        // by looking up the UUID → runtime `ProjectId` mapping.
        let requested = match (workspace.active_project, workspace.active_lane) {
            (Some(p_uuid), Some(wt_id)) => self
                .projects
                .iter()
                .find(|p| p.uuid == p_uuid)
                .map(|p| daruda_store::project::LaneRef {
                    project: p.id,
                    lane: wt_id,
                })
                .unwrap_or_default(),
            _ => daruda_store::project::LaneRef::default(),
        };
        self.active = self.resolve_active(requested);

        // Drop bootstrapped pane/tab; rebuild every lane's
        // runtime from the persisted SerializedTab lists carried on
        // each `SerializedLane`.
        self.main_area.tabs.clear();
        self.main_area.panes.clear();
        self.main_area.activity_counter.clear();
        self.main_area.inactive_lane_runtimes.clear();

        let mut active_focus: Option<pane_tree::PaneId> = None;
        let mut early_exit = false;
        for (idx, ps) in project_states.iter().enumerate() {
            if early_exit {
                break;
            }
            let runtime_project_id = idx as daruda_store::project::ProjectId;
            for swt in &ps.lanes {
                if early_exit {
                    break;
                }
                let wt_ref = daruda_store::project::LaneRef {
                    project: runtime_project_id,
                    lane: swt.id,
                };
                // `recompute_availability` above already classified this
                // lane's root. A non-`Present` lane cannot host a usable
                // PTY/file-tree, so skip its pane/tab rebuild entirely —
                // including the active lane, so a missing active root does
                // not spawn a shell into a dead cwd. The Warning toast
                // still fires (it carries the path the user lost).
                let unavailable = self
                    .lane_for(wt_ref)
                    .map(|l| l.availability != LaneAvailability::Present)
                    .unwrap_or(false);
                if unavailable {
                    let report = ErrorReport::new("Lane path not found")
                        .severity(ErrorSeverity::Warning)
                        .at(file!(), line!())
                        .with_context("path", redact_home(&swt.path))
                        .dedup("lane.restore.missing_path")
                        .build();
                    self.report_error(report, cx);
                    continue;
                }

                let panes_start = self.main_area.panes.len();
                let mut id_map: HashMap<u64, pane_tree::PaneId> = HashMap::new();
                let mut tabs: Vec<TabEntry> = Vec::new();
                let mut failed = false;

                for stab in &swt.tabs {
                    let panes_before = self.main_area.panes.len();
                    match self.rebuild_layout(
                        &stab.layout,
                        Some(&swt.path),
                        &mut id_map,
                        window,
                        cx,
                    ) {
                        Ok(layout) => {
                            let last_focus = id_map
                                .get(&stab.last_focused_pane)
                                .copied()
                                .unwrap_or_else(|| layout.first_leaf());
                            let tab_id = self.alloc_id();
                            tabs.push(TabEntry {
                                id: tab_id,
                                layout,
                                last_focused_pane: last_focus,
                                user_label: stab
                                    .user_label
                                    .as_ref()
                                    .map(|s| gpui::SharedString::from(s.clone())),
                            });
                        }
                        Err(e) => {
                            self.main_area.panes.truncate(panes_before);
                            self.report_pane_error("restore", e, cx);
                            failed = true;
                            break;
                        }
                    }
                }
                if failed {
                    early_exit = true;
                    break;
                }

                let wt_panes = self.main_area.panes.split_off(panes_start);
                let wt_active_tab = swt.active_tab_index.min(tabs.len().saturating_sub(1));
                let focused = if wt_ref == self.active {
                    let focused_tab = tabs.get(wt_active_tab);
                    let leaves = focused_tab.map(|t| t.layout.pane_ids());
                    id_map
                        .get(&workspace.focused_pane_id)
                        .copied()
                        .filter(|id| leaves.as_ref().is_some_and(|l| l.contains(id)))
                        .unwrap_or_else(|| {
                            focused_tab.map(|t| t.last_focused_pane).unwrap_or_default()
                        })
                } else {
                    tabs.get(wt_active_tab)
                        .map(|t| t.last_focused_pane)
                        .unwrap_or_default()
                };

                let runtime = LaneRuntime {
                    tabs,
                    panes: wt_panes,
                    active_tab_index: wt_active_tab,
                    tab_history: Vec::new(),
                    focused_pane_id: focused,
                };

                if wt_ref == self.active {
                    self.main_area.tabs = runtime.tabs;
                    self.main_area.panes = runtime.panes;
                    self.main_area.active_tab_index = runtime.active_tab_index;
                    self.main_area.focused_pane_id = runtime.focused_pane_id;
                    self.main_area.tab_history = runtime.tab_history;
                    active_focus = Some(runtime.focused_pane_id);
                } else {
                    self.main_area
                        .inactive_lane_runtimes
                        .insert(wt_ref, runtime);
                }
            }
        }

        if self.main_area.tabs.is_empty() {
            // Seed a fresh tab only for a legitimately-empty *Present*
            // active lane. When the active lane is inaccessible the
            // restore loop above deliberately skipped its pane rebuild
            // (Task 1), so seeding a tab here would spawn a stray $HOME
            // terminal that contradicts the inaccessible empty-state the
            // main area renders. Leave `tabs` empty so `render` shows the
            // empty-state instead.
            let active_present = self
                .active_lane()
                .map(|l| l.availability == LaneAvailability::Present)
                .unwrap_or(true);
            if active_present {
                self.add_tab(window, cx);
            }
            return;
        }

        if let Some(focus) = active_focus {
            self.bump_activity(focus);
            self.focus_pane(focus, window, cx);
        }
        self.main_area.pending_resize = true;

        if self.left_dock_view == daruda_store::project::LeftDockView::GitChanges {
            let target = self.active;
            self.refresh_git_status(target, cx);
        }

        self.load_pending_file_panes(cx);

        cx.notify();
    }

    /// Pick the right (project, lane) when restoring. Falls back
    /// through: requested pair → project's `last_active_lane_id` →
    /// project's first lane → workspace's first project's first
    /// lane → default `LaneRef`.
    fn resolve_active(
        &self,
        requested: daruda_store::project::LaneRef,
    ) -> daruda_store::project::LaneRef {
        if let Some(project) = self.projects.iter().find(|p| p.id == requested.project) {
            if project.lanes.iter().any(|w| w.id == requested.lane) {
                return requested;
            }
            if project
                .lanes
                .iter()
                .any(|w| w.id == project.last_active_lane_id)
            {
                return daruda_store::project::LaneRef {
                    project: project.id,
                    lane: project.last_active_lane_id,
                };
            }
            if let Some(first) = project.lanes.first() {
                return daruda_store::project::LaneRef {
                    project: project.id,
                    lane: first.id,
                };
            }
        }
        if let Some(first_project) = self.projects.first()
            && let Some(first_lane) = first_project.lanes.first()
        {
            return daruda_store::project::LaneRef {
                project: first_project.id,
                lane: first_lane.id,
            };
        }
        daruda_store::project::LaneRef::default()
    }

    /// Recursively rebuild a `PaneLayout` from its serialized form.
    /// Panes are spawned eagerly; `id_map` records the new PaneId for
    /// each serialized pane_id so cross-references (focus, last
    /// focused per tab) can be rewritten.
    /// Recursively materialize a serialized layout into live panes.
    /// `fallback_cwd` is the cwd to use when a leaf's serialized cwd
    /// is missing — typically the owning lane's path so restore
    /// keeps each pane inside its lane even for sessions that
    /// were saved before OSC 7 first reported the live cwd.
    fn rebuild_layout(
        &mut self,
        slayout: &daruda_store::project::SerializedLayout,
        fallback_cwd: Option<&std::path::Path>,
        id_map: &mut HashMap<u64, pane_tree::PaneId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<PaneLayout, PaneSpawnError> {
        match slayout {
            daruda_store::project::SerializedLayout::Leaf {
                pane_id,
                cwd,
                file,
                agent_chat,
            } => {
                let pane = if let Some(fc) = file {
                    // File pane — `file_status` is not persisted; the
                    // git badge re-derives when the lane's git
                    // status next refreshes. Content stays at
                    // `Loading` until the owning lane becomes
                    // active and `load_pending_file_panes` fires.
                    self.create_file_pane(
                        fc.lane_id,
                        fc.path.clone(),
                        fc.staged,
                        None,
                        deserialize_view_mode(fc.view_mode),
                        window,
                        cx,
                    )
                } else if let Some(ac) = agent_chat {
                    // AgentChat pane — the ACP session itself is not
                    // persisted; restore re-opens at the saved lane cwd in
                    // the dormant `Idle` state. The live session is *not*
                    // started here — `focus_pane` connects it lazily on
                    // first focus, so cold restore doesn't spin up an agent
                    // process per restored pane. Items begin empty.
                    self.create_agent_chat_pane(ac.cwd.clone(), window, cx)
                } else {
                    let effective = effective_cwd(cwd.clone(), fallback_cwd);
                    self.create_pane_with_cwd(effective, window, cx)?
                };
                let new_id = pane.id;
                id_map.insert(*pane_id, new_id);
                self.main_area.panes.push(pane);
                Ok(PaneLayout::Pane(new_id))
            }
            daruda_store::project::SerializedLayout::Split {
                direction,
                children,
                ratios,
            } => {
                let dir = match direction {
                    daruda_store::project::SplitDirectionSerde::Horizontal => {
                        SplitDirection::Horizontal
                    }
                    daruda_store::project::SplitDirectionSerde::Vertical => {
                        SplitDirection::Vertical
                    }
                };
                let mut rebuilt = Vec::with_capacity(children.len());
                for c in children {
                    rebuilt.push(self.rebuild_layout(c, fallback_cwd, id_map, window, cx)?);
                }
                let n = rebuilt.len();
                if n == 0 {
                    // Degenerate serialization — materialize a fresh leaf
                    // so we never surface an empty Split to the renderer.
                    let pane = self.create_pane_with_cwd(
                        fallback_cwd.map(|p| p.to_path_buf()),
                        window,
                        cx,
                    )?;
                    let id = pane.id;
                    self.main_area.panes.push(pane);
                    return Ok(PaneLayout::Pane(id));
                }
                if n == 1 {
                    // Invariant: n == 1, rebuilt has exactly one element.
                    return Ok(rebuilt
                        .into_iter()
                        .next()
                        .expect("n == 1: rebuilt has one element"));
                }
                let normalized = normalize_ratios(ratios, n);
                Ok(PaneLayout::Split {
                    direction: dir,
                    children: rebuilt,
                    ratios: normalized,
                })
            }
        }
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
        .filter(|s| worktree_root.is_none_or(|root| s.starts_with(root)))
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
    if let Ok(canonical) = std::fs::canonicalize(project_root) {
        for wt in lanes.iter_mut() {
            if canonical.starts_with(&wt.path) && canonical != wt.path {
                wt.path = canonical;
                break;
            }
        }
    }
}

fn serialize_layout(
    layout: &pane_tree::PaneLayout,
    panes: &[pane::Pane],
) -> daruda_store::project::SerializedLayout {
    match layout {
        pane_tree::PaneLayout::Pane(id) => {
            let pane = panes.iter().find(|p| p.id == *id);
            // File panes serialize their viewer state; Terminal panes
            // serialize their cwd. The two are mutually exclusive —
            // a File leaf carries `cwd: None` because it derives its
            // cwd from `path.parent()` at runtime.
            let file = pane.and_then(|p| p.file_view()).map(|fv| {
                daruda_store::project::SerializedFileContent {
                    lane_id: fv.lane_id,
                    path: fv.path.clone(),
                    staged: fv.staged,
                    view_mode: serialize_view_mode(fv.view_mode),
                }
            });
            // AgentChat panes serialize their anchored lane cwd (the
            // session + conversation are intentionally not persisted).
            // Mutually exclusive with `file`.
            let agent_chat = pane.and_then(|p| p.agent_chat_content()).map(|ac| {
                daruda_store::project::SerializedAgentChatContent {
                    cwd: ac.cwd.clone(),
                }
            });
            let cwd = if file.is_some() || agent_chat.is_some() {
                None
            } else {
                pane.and_then(|p| p.cwd().map(std::path::Path::to_path_buf))
            };
            daruda_store::project::SerializedLayout::Leaf {
                pane_id: *id,
                cwd,
                file,
                agent_chat,
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
                    .map(|c| serialize_layout(c, panes))
                    .collect(),
                ratios: ratios.clone(),
            }
        }
    }
}
