//! Save side of workspace persistence: the disk snapshot, window-bounds
//! capture, and `persist_state` with its recent-list bookkeeping.

use std::collections::BTreeMap;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::observability::system_info::redact_home;
use daruda_store::project::{
    DockStates, ProjectOverride, ProjectState, ProjectUuid, SerializedTab,
    WORKSPACE_SCHEMA_VERSION, WorkspaceState,
};
use gpui::{App, Window};

use super::serialize_layout;
use crate::workspace::Workspace;

impl Workspace {
    /// Snapshot the workspace into the UUID-keyed disk shape
    /// (`(WorkspaceState, Vec<ProjectState>)`). Each project's intrinsic
    /// data lives in its own [`ProjectState`]; per-workspace decoration
    /// (color / tab order / group / collapsed) lives in
    /// [`WorkspaceState::project_overrides`]. Active focus projects from
    /// `self.active` onto persisted `(ProjectUuid, LaneId)` — runtime
    /// `ProjectId` is per-session and not persisted.
    ///
    /// A workspace with no projects snapshots like any other: it is the
    /// Landing state, which survives a restart rather than ending the
    /// window. Its payload carries the window geometry, dock sizes and
    /// open policy and an empty project list.
    ///
    /// Drives [`Workspace::persist_state`]. Infallible: every workspace has
    /// a snapshot, the empty one included.
    pub(in crate::workspace) fn snapshot_for_disk(
        &self,
        cx: &App,
    ) -> (WorkspaceState, Vec<ProjectState>) {
        let mut project_states = Vec::with_capacity(self.projects.len());
        let mut project_ids = Vec::with_capacity(self.projects.len());
        let mut project_overrides = BTreeMap::new();
        let mut project_tabs: BTreeMap<ProjectUuid, Vec<SerializedTab>> = BTreeMap::new();
        let mut focused_pane_id = self.active_runtime().focused_pane_id;

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
                    // Every lane's runtime — active or parked — lives in
                    // the single `runtimes` map. A lane with no registered
                    // runtime (never activated) keeps its serialized form.
                    let Some(rt) = self.main_area.runtimes.get(&wt_ref) else {
                        return s;
                    };
                    let (tabs_src, panes_src, active_idx) =
                        (&rt.tabs, &rt.panes, rt.active_tab_index);
                    s.tabs = tabs_src
                        .iter()
                        .filter(|tab| !self.is_orchestrator_tab(tab))
                        .map(|tab| daruda_store::project::SerializedTab {
                            layout: serialize_layout(&tab.layout, panes_src, wt.id, cx),
                            last_focused_pane: tab.last_focused_pane,
                            user_label: tab.user_label.as_ref().map(|s| s.to_string()),
                        })
                        .collect();
                    s.active_tab_index = tabs_src
                        .iter()
                        .take(active_idx)
                        .filter(|tab| !self.is_orchestrator_tab(tab))
                        .count()
                        .min(s.tabs.len().saturating_sub(1));
                    if wt_ref == self.active && self.is_orchestrator_pane(focused_pane_id) {
                        focused_pane_id = s
                            .tabs
                            .get(s.active_tab_index)
                            .map_or(0, |tab| tab.last_focused_pane);
                    }
                    s
                })
                .collect();

            // `next_lane_id`: smallest id strictly greater than every lane
            // registered for this project (live lanes + runtime keys).
            // Upholds the "monotonic — never reused" invariant from
            // `allocate_lane_id`.
            let max_live = project.lanes.iter().map(|w| w.id).max();
            let max_runtime = self
                .main_area
                .runtimes
                .keys()
                .filter(|r| r.project == project.id)
                .map(|r| r.lane)
                .max();
            let next_lane_id = match (max_live, max_runtime) {
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
                    display_alias: project.display_alias.clone(),
                    rail_grouped_lanes: project
                        .rail_grouped_lanes
                        .iter()
                        .copied()
                        .filter(|lane| project.lane(*lane).is_some())
                        .collect(),
                    color: project.color.clone(),
                    tab_order: project.tab_order as usize,
                    group_id: project.group_id,
                    is_collapsed: project.is_collapsed,
                },
            );
        }

        // Project the runtime `LaneRef` onto the persisted UUID. If the
        // active project has been closed, both fields fall to `None`.
        let active_project = self.projects.get(self.active.project).map(|p| p.uuid);
        let active_lane = active_project.map(|_| self.active.lane);

        let workspace = WorkspaceState {
            schema_version: WORKSPACE_SCHEMA_VERSION,
            uuid: self.uuid,
            project_ids,
            project_overrides,
            groups: self.groups.clone(),
            session_titles: self.agent_usage.usage_by_account.session_titles.clone(),
            active_project,
            active_lane,
            docks: DockStates {
                left_open: self.docks.left.read(cx).is_open,
                left_size: self.docks.left.read(cx).size,
                bottom_open: self.docks.bottom.read(cx).is_open,
                bottom_size: self.docks.bottom.read(cx).size,
                right_open: self.docks.right.read(cx).is_open,
                right_size: self.docks.right.read(cx).size,
            },
            window: self
                .window_runtime
                .cached_bounds
                .clone()
                .unwrap_or_default(),
            font_size: self.mirrors.terminal_config.font_size,
            vertical_spacing: self.mirrors.terminal_config.vertical_spacing,
            horizontal_spacing: self.mirrors.terminal_config.horizontal_spacing,
            focused_pane_id,
            active_dock_view: self.docks.left_view,
            active_right_panel_view: self.docks.right_view,
            active_page: self
                .active_page()
                .map(crate::workspace::pages::Page::stored),
            task_detail: self.task_detail_target(cx),
            flow_detail: self.flow_detail_target(cx),
            window_open_policy: self.window_open_policy,
            next_group_id: self.next_group_id,
            project_tabs,
        };

        (workspace, project_states)
    }

    /// Sample the window's geometry into `WindowRuntime::cached_bounds`: its rect,
    /// the display it is on, and whether it is maximized. Fullscreen is
    /// skipped; relaunching into it would take over a Space unasked, so the
    /// last windowed geometry stays cached instead.
    pub(in crate::workspace) fn capture_window_bounds(&mut self, window: &Window, cx: &App) {
        let bounds = match window.window_bounds() {
            gpui::WindowBounds::Windowed(b) | gpui::WindowBounds::Maximized(b) => b,
            gpui::WindowBounds::Fullscreen(_) => return,
        };
        let reported = daruda_store::project::WindowState {
            x: f32::from(bounds.origin.x),
            y: f32::from(bounds.origin.y),
            width: f32::from(bounds.size.width),
            height: f32::from(bounds.size.height),
            display: window
                .display(cx)
                .and_then(|d| d.uuid().ok())
                .map(|uuid| uuid.to_string()),
            maximized: false,
        };
        let new = crate::window_placement::captured_geometry(
            reported,
            window.is_maximized(),
            self.window_runtime.cached_bounds.as_ref(),
        );
        if !new.is_valid() {
            return;
        }
        self.window_runtime.cached_bounds = Some(new);
    }

    /// Keep a capture's fixtures off disk: the layout stops saving, and the
    /// task log moves to a scratch copy.
    #[cfg(any(test, feature = "screenshot"))]
    pub(in crate::workspace) fn suspend_persistence_for_shot(&mut self, cx: &mut App) {
        self.persistence_suspended = true;
        crate::runs::RunsHost::suspend_persistence(cx);
    }

    /// Persist state to disk: one workspace file
    /// (`workspaces/<uuid>.json`) referencing each project, plus one
    /// project file (`projects/<uuid>.json`) per owned project. Each
    /// project's intrinsic data lives in exactly one file, so workspaces
    /// sharing a project see updates via it (last-writer-wins on the lane
    /// list). Also touches `recent-workspaces.json` with the display name.
    pub fn persist_state(&self, cx: &App) {
        if self.persistence_suspended {
            return;
        }
        let (workspace, projects) = self.snapshot_for_disk(cx);

        // Settle the recent list first, because for an empty workspace its
        // answer decides whether there is anything worth saving.
        //
        // An empty workspace refreshes an existing row but never earns a new
        // one (see `refresh_recent_if_present_in`). The recent list is also
        // the only index of workspaces there is, so an empty one with no row
        // can never be reached again — a window opened empty, or the
        // constructor's save on the restore path, before the saved uuid is
        // adopted. Writing its file would leak one per launch. Delete
        // instead, which also sweeps a row that has aged out past RECENT_MAX.
        if !self.update_recent_entry(&workspace) {
            self.discard_unreachable_state(&workspace);
            return;
        }

        for project in &projects {
            if let Err(e) = self.workspace_store.save_project(project) {
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
                // Do not publish a workspace referencing an unsaved project.
                return;
            }
        }

        if let Err(e) = self.workspace_store.save_workspace(&workspace) {
            LogWriter::log(
                ErrorReport::new("Failed to persist workspace state")
                    .severity(ErrorSeverity::Error)
                    .from_error(&e)
                    .at(file!(), line!())
                    .with_context("workspace_uuid", workspace.uuid.as_inner().to_string())
                    .dedup("workspace.save")
                    .build(),
            );
            return;
        }
        let snapshots: BTreeMap<_, _> = self
            .main_area
            .runtimes
            .iter()
            .filter(|(lane, _)| self.restore_terminal_output_for(lane.project, cx))
            .flat_map(|(_, rt)| &rt.panes)
            .filter_map(|pane| {
                let text = pane
                    .terminal_view()?
                    .read(cx)
                    .session()
                    .dump_viewport()
                    .ok()?;
                (text.len() <= 64 * 1024).then_some((pane.id, text))
            })
            .collect();
        if !snapshots.is_empty() {
            if let Err(error) = self
                .workspace_store
                .save_terminal_snapshots(workspace.uuid, &snapshots)
            {
                LogWriter::log(
                    ErrorReport::new("Failed to save terminal output snapshots")
                        .from_error(&error)
                        .at(file!(), line!())
                        .dedup("workspace.terminal_snapshots.save")
                        .build(),
                );
            }
        } else if let Err(error) = self
            .workspace_store
            .delete_terminal_snapshots(workspace.uuid)
        {
            LogWriter::log(
                ErrorReport::new("Failed to clear disabled terminal output snapshots")
                    .from_error(&error)
                    .at(file!(), line!())
                    .dedup("workspace.terminal_snapshots.clear")
                    .build(),
            );
        }
    }

    /// Update this workspace's recent-list row and answer whether it is
    /// reachable afterwards. A workspace holding projects always is — it
    /// takes the top slot. An empty one is reachable only if it already had
    /// a row, which is refreshed in place.
    fn update_recent_entry(&self, workspace: &WorkspaceState) -> bool {
        let display_name = self.recent_display_name();
        let outcome = if self.projects.is_empty() {
            self.workspace_store
                .refresh_recent_if_present(workspace.uuid, display_name)
        } else {
            self.workspace_store
                .touch_recent(workspace.uuid, display_name)
                .map(|()| true)
        };
        match outcome {
            Ok(reachable) => reachable,
            Err(e) => {
                LogWriter::log(
                    ErrorReport::new("Failed to update recent list")
                        .severity(ErrorSeverity::Warning)
                        .from_error(&e)
                        .at(file!(), line!())
                        .dedup("recent.touch")
                        .build(),
                );
                // The list could not be read or written, so reachability is
                // unknown. Keep the state file: a stale file is recoverable,
                // a deleted workspace is not.
                true
            }
        }
    }

    /// Remove the state file of a workspace nothing can reach, so an empty
    /// window does not leave one behind on every launch.
    fn discard_unreachable_state(&self, workspace: &WorkspaceState) {
        if let Err(e) = self.workspace_store.delete_workspace(workspace.uuid) {
            LogWriter::log(
                ErrorReport::new("Failed to discard unreachable workspace state")
                    .severity(ErrorSeverity::Warning)
                    .from_error(&e)
                    .at(file!(), line!())
                    .with_context("workspace_uuid", workspace.uuid.as_inner().to_string())
                    .dedup("workspace.discard")
                    .build(),
            );
        }
    }
}
