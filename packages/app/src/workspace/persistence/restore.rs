//! Restore side of workspace persistence: `restore_from_disk` and the
//! active-ref fallback it resolves.

use std::collections::HashMap;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::observability::system_info::redact_home;
use daruda_store::project::{ProjectState, WorkspaceState};
use gpui::{Context, Window};

use super::{LaneRuntime, anchor_lane_paths_to_project_root};
use crate::lane::availability::LaneAvailability;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane::{self, TabEntry};
use crate::workspace::main_area::pane_tree::{self as pane_tree};

impl Workspace {
    /// Restore from the UUID-keyed disk shape (from
    /// [`Workspace::snapshot_for_disk`]). Rebuilds every project's lanes,
    /// dock open/size, font settings, and the full tab / split-tree
    /// layout; each restored leaf starts at its serialized cwd. On any
    /// pane spawn failure, aborts and surfaces the error. Each runtime
    /// [`crate::project::Project`] is hydrated from its `ProjectState` +
    /// `ProjectOverride`; runtime `ProjectId`s are minted dense `0..N` in
    /// `project_states` iteration order.
    pub(crate) fn restore_from_disk(
        &mut self,
        workspace: &WorkspaceState,
        project_states: &[ProjectState],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Restore dock states.
        self.agent_usage.usage_by_account.session_titles = workspace.session_titles.clone();
        let terminal_snapshots = self
            .workspace_store
            .load_terminal_snapshots(workspace.uuid)
            .unwrap_or_else(|error| {
                LogWriter::log(
                    ErrorReport::new("Failed to load terminal output snapshots")
                        .from_error(&error)
                        .at(file!(), line!())
                        .build(),
                );
                Default::default()
            });
        let left_open = workspace.docks.left_open;
        let left_size = workspace.docks.left_size;
        self.docks.left.update(cx, |d, _| {
            d.is_open = left_open;
            if left_size > 0.0 {
                d.size = left_size;
            }
        });
        self.docks.left_view = workspace.active_dock_view;
        self.docks.right_view = workspace.active_right_panel_view;
        self.workspace_page = workspace
            .active_page
            .map(|page| self.page_state(crate::workspace::pages::Page::from_stored(page)));
        let bottom_open = workspace.docks.bottom_open;
        let bottom_size = workspace.docks.bottom_size;
        self.docks.bottom.update(cx, |d, _| {
            d.is_open = bottom_open;
            if bottom_size > 0.0 {
                d.size = bottom_size;
            }
        });
        let right_open = workspace.docks.right_open;
        let right_size = workspace.docks.right_size;
        self.docks.right.update(cx, |d, _| {
            d.is_open = right_open;
            if right_size > 0.0 {
                d.size = right_size;
            }
        });

        // Adopt the persisted workspace UUID so subsequent saves land
        // on the same on-disk record.
        self.uuid = workspace.uuid;

        // Restore workspace-level multi-project metadata.
        self.window_open_policy = workspace.window_open_policy;
        self.next_group_id = workspace.next_group_id;
        self.groups = workspace.groups.clone();
        // Cache window bounds so the next save round-trips the same
        // geometry even if `observe_window_bounds` hasn't fired yet.
        self.window_runtime.cached_bounds = Some(workspace.window.clone());

        // Empty workspace — the Landing state. Everything above (geometry,
        // docks, uuid, open policy) has already been adopted, which is what
        // makes it worth persisting; there is simply nothing to hydrate, so
        // keep whatever `new_with_project` bootstrapped.
        if project_states.is_empty() {
            self.main_area.pending_resize = true;
            return;
        }

        // Mint dense runtime ids `0..N` for the hydrated projects; the
        // counter advances past the highest minted id — same monotonic
        // invariant `add_project` enforces.
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

        // Classify every project / lane root against the filesystem before
        // the pane-rebuild loop. The persisted set may reference gone
        // directories; stamping `availability` here lets the skip below and
        // the read side (file-tree scan, watcher, PTY spawn) short-circuit
        // instead of spamming directory-read errors.
        self.recompute_availability();

        // Re-detect each git project's `default_branch` and update if it
        // drifted; backfills legacy `None` state. Async on the background
        // executor (persisted value stands until it completes). Placed
        // before the pane-rebuild loop so it fires even if a later step
        // early-returns.
        self.reconcile_project_default_branches(cx);
        // Heal any project persisted while its lanes were still the
        // construction placeholder (app died before async discovery
        // returned). Same async upgrade pass as fresh-open.
        self.reconcile_bootstrapped_lanes(cx);

        // Project persisted (ProjectUuid, LaneId) onto the runtime
        // `LaneRef` via the UUID → runtime `ProjectId` mapping.
        let requested = match (workspace.active_project, workspace.active_lane) {
            (Some(p_uuid), Some(wt_id)) => self
                .projects
                .by_uuid(p_uuid)
                .map(|p| daruda_store::project::LaneRef {
                    project: p.id,
                    lane: wt_id,
                })
                .unwrap_or_default(),
            _ => daruda_store::project::LaneRef::default(),
        };
        self.active = self.resolve_active(requested);

        // Drop every bootstrapped runtime; rebuild each lane's runtime
        // from its `SerializedLane` tab lists. The active entry is
        // re-seeded after the loop so the "active runtime always present"
        // invariant holds even if the active lane was skipped.
        self.main_area.runtimes.clear();
        self.main_area.activity_counter.clear();

        let mut active_focus: Option<pane_tree::PaneId> = None;
        for (idx, ps) in project_states.iter().enumerate() {
            let runtime_project_id = idx as daruda_store::project::ProjectId;
            for swt in &ps.lanes {
                let wt_ref = daruda_store::project::LaneRef {
                    project: runtime_project_id,
                    lane: swt.id,
                };
                // A non-`Present` lane cannot host a usable PTY/file-tree,
                // so skip its pane/tab rebuild — including the active lane,
                // so a missing active root never spawns a shell into a dead
                // cwd. The Warning toast still fires (carries the lost path).
                let unavailable = self
                    .lane_for(wt_ref)
                    .map(|l| l.availability != LaneAvailability::Present)
                    .unwrap_or(false);
                if unavailable {
                    let report =
                        ErrorReport::new(crate::surface::strings::error::lane_path_not_found())
                            .severity(ErrorSeverity::Warning)
                            .at(file!(), line!())
                            .with_context("path", redact_home(&swt.path))
                            .dedup("lane.restore.missing_path")
                            .build();
                    self.report_error(report, cx);
                    continue;
                }

                // Build this lane's panes into a local scratch buffer so
                // the recursion never touches `self`'s runtimes — the
                // finished `Vec` becomes `LaneRuntime.panes` in one
                // `insert`, uniform for active and parked alike.
                let mut scratch: Vec<pane::Pane> = Vec::new();
                let mut id_map: HashMap<u64, pane_tree::PaneId> = HashMap::new();
                let mut tabs: Vec<TabEntry> = Vec::new();
                // The saved active index counts the saved tabs; the ones that
                // did not come back must not shift it onto a neighbour.
                let mut kept_before_active = 0;

                for (ix, stab) in swt.tabs.iter().enumerate() {
                    // A tab that fails part-way leaves the panes it did
                    // build in `scratch`; they go with it.
                    let built_before = scratch.len();
                    match self.rebuild_layout(
                        &stab.layout,
                        Some(&swt.path),
                        &mut id_map,
                        &mut scratch,
                        window,
                        cx,
                    ) {
                        // Every leaf of it dropped out: nothing to restore.
                        Ok(None) => {}
                        Ok(Some(layout)) => {
                            let last_focus = id_map
                                .get(&stab.last_focused_pane)
                                .copied()
                                .unwrap_or_else(|| layout.first_leaf());
                            let tab_id = self.alloc_id();
                            if ix < swt.active_tab_index {
                                kept_before_active += 1;
                            }
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
                            // Only this tab is lost; the lane's other tabs
                            // and every lane after it still restore.
                            self.report_pane_error(
                                &crate::surface::strings::error::pane_context_restore(),
                                e,
                                cx,
                            );
                            scratch.truncate(built_before);
                        }
                    }
                }

                let wt_active_tab = kept_before_active.min(tabs.len().saturating_sub(1));
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

                let restore_output = self.restore_terminal_output_for(runtime_project_id, cx);
                for (old_id, new_id) in &id_map {
                    if !restore_output {
                        break;
                    }
                    if let Some(text) = terminal_snapshots.get(old_id)
                        && let Some(view) = scratch
                            .iter()
                            .find(|pane| pane.id == *new_id)
                            .and_then(|pane| pane.terminal_view())
                    {
                        view.update(cx, |view, cx| {
                            view.restore_text_snapshot(
                                text,
                                &crate::surface::strings::terminal::restored_output(),
                                cx,
                            )
                        });
                    }
                }
                let runtime = LaneRuntime {
                    preview_tab_id: None,
                    tabs,
                    panes: scratch,
                    active_tab_index: wt_active_tab,
                    tab_history: Vec::new(),
                    focused_pane_id: focused,
                };
                if wt_ref == self.active {
                    active_focus = Some(runtime.focused_pane_id);
                }
                self.main_area.runtimes.insert(wt_ref, runtime);
            }
        }

        // After the runtimes, so the editor's id is minted past every pane's.
        if let Some(target) = &workspace.task_detail {
            self.restore_task_detail(target, window, cx);
        }
        if let Some(target) = &workspace.flow_detail {
            self.restore_flow_detail(target, window, cx);
        }

        // Invariant: the active lane's runtime must exist before any
        // `active_runtime()` read below. The active lane may have been
        // skipped (unavailable) or absent from disk — seed an empty entry
        // so the tail never panics.
        self.main_area.runtimes.entry(self.active).or_default();

        if self.active_runtime().tabs.is_empty() {
            // An empty active lane is a first-class state: restore never
            // auto-seeds a shell. The main area renders the empty-state and
            // the user opens content from there, so an emptied lane
            // round-trips across restart instead of springing a fresh tab.
            return;
        }

        if let Some(focus) = active_focus {
            self.bump_activity(focus);
            if self.workspace_page.is_none() {
                self.focus_pane(focus, window, cx);
            } else {
                self.window_runtime.focus_handle.focus(window, cx);
            }
            // Seed the input-draft owner so text typed into the bottom
            // input before the first focus change is attributed to the
            // restored pane. `set_focused_pane` is avoided here: it would
            // notify a not-yet-cached view (see its doc), and
            // `focused_pane_id` is already restored from the runtime.
            if self.pane_consumes_bottom_input(focus) {
                self.input_dock.owner = Some(focus);
            }
        }
        self.main_area.pending_resize = true;

        if self.docks.left_view == daruda_store::project::LeftDockView::GitChanges {
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
        if let Some(project) = self.projects.get(requested.project) {
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
}
