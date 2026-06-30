//! Pure-data builders for the three dock snapshots consumed by
//! `impl Render for Workspace`. Lifting them out of the render body
//! makes the high-level flow visible: build all three snapshots,
//! publish all three, then read dock geometry, then build the
//! element tree.

use gpui::Context;

use crate::workspace::Workspace;
use crate::workspace::layout::snap::{BottomDockSnapshot, LeftDockSnapshot, RightDockSnapshot};

impl Workspace {
    pub(super) fn prepare_left_dock_snapshot(
        &mut self,
        cx: &mut Context<Self>,
    ) -> LeftDockSnapshot {
        // Aggregate Claude status over every pane once per frame, keyed by
        // the lane that owns each pane. PTY-bound sessions are surfaced
        // once the PtyTracker has bound a live PID to a pane daruda owns.
        // Agent chat (ACP) panes contribute their own session status
        // directly, so the lane indicator reflects both PTY and ACP
        // sessions.
        let pane_lane = self.pane_lane_index();
        // Collect agent chat pane statuses for the aggregation. Reading
        // each view registers a GPUI entity dependency so `cx.notify()`
        // on the view dirties this snapshot — the same pattern the
        // bottom-dock snapshot uses for `turn_in_flight`.
        let acp_statuses: Vec<(crate::workspace::main_area::pane_tree::PaneId, daruda_claude::SessionStatus)> = self
            .main_area
            .panes
            .iter()
            .filter_map(|p| {
                let view = p.agent_chat_view()?;
                let status = view.read(cx).to_session_status()?;
                Some((p.id, status))
            })
            .collect();
        let (agent_status_per_lane, agent_per_session_per_lane) =
            crate::workspace::claude_status_aggregate::aggregate_over_panes(
                &pane_lane,
                &self.claude.pty_claude_bindings,
                &self.claude.claude_status,
                &acp_statuses,
            );
        LeftDockSnapshot {
            left_dock_view: self.left_dock_view,
            active_project_name: self
                .active_project()
                .map(|p| gpui::SharedString::from(p.name.clone())),
            lanes: self.active_lanes().to_vec(),
            projects: {
                let mut projects: Vec<crate::workspace::layout::snap::ProjectSnapshot> = self
                    .projects
                    .iter()
                    .map(|p| crate::workspace::layout::snap::ProjectSnapshot {
                        id: p.id,
                        name: gpui::SharedString::from(p.name.clone()),
                        group_id: p.group_id,
                        color: p
                            .color
                            .as_ref()
                            .map(|c| gpui::SharedString::from(c.clone())),
                        tab_order: p.tab_order,
                        default_branch: p
                            .default_branch
                            .as_ref()
                            .map(|b| gpui::SharedString::from(b.clone())),
                        lanes: p.lanes.clone(),
                        last_active_lane_id: p.last_active_lane_id,
                        is_collapsed: p.is_collapsed,
                        availability: p.availability,
                    })
                    .collect();
                projects.sort_by_key(|p| p.tab_order);
                projects
            },
            groups: {
                let mut groups: Vec<crate::workspace::layout::snap::GroupSnapshot> = self
                    .groups
                    .iter()
                    .map(|g| crate::workspace::layout::snap::GroupSnapshot {
                        id: g.id,
                        name: gpui::SharedString::from(g.name.clone()),
                        color: g
                            .color
                            .as_ref()
                            .map(|c| gpui::SharedString::from(c.clone())),
                        tab_order: g.tab_order,
                        is_collapsed: g.is_collapsed,
                    })
                    .collect();
                groups.sort_by_key(|g| g.tab_order);
                groups
            },
            active: self.active,
            git_status_cache: self.git_status_cache.clone(),
            git_stage_in_flight: self.git_stage_in_flight,
            git_op_in_flight: self.git_op_in_flight,
            git_collapsed_dirs: self
                .git_collapsed_dirs
                .get(&self.active)
                .cloned()
                .unwrap_or_default(),
            git_changes_cursor: self.git_changes_cursor.get(&self.active).cloned(),
            git_changes_panel_focus: self.git_changes_panel_focus.clone(),
            focused_file_selection: self
                .focused_file_view()
                .map(|fv| (fv.lane_id, fv.path.clone(), fv.staged)),
            git_changes_scroll_handle: self.git_changes_scroll_handle.clone(),
            git_commit_input: self.git_commit_input.clone(),
            files_panel_focus: self.file_tree.files_panel_focus.clone(),
            files_scroll_handle: self.file_tree.files_scroll_handle.clone(),
            files_icon_color_mode: self.mirrors.files_icon_color_mode.clone(),
            cached_visible: self.cached_or_rebuild_visible(self.active_ref()),
            root_kind: self
                .file_tree
                .file_trees
                .get(&self.active_ref())
                .and_then(|t| t.entry(t.root_id))
                .map(|e| e.kind),
            agent_status_per_lane,
            agent_per_session_per_lane,
            agent_active_session_id: self
                .claude
                .pty_claude_bindings
                .get(&self.main_area.focused_pane_id)
                .map(|b| b.session_id.clone())
                .or_else(|| {
                    // If the focused pane is an agent chat with a live session,
                    // use its synthetic ACP id so the sub-row badge highlights.
                    let focused = self.main_area.focused_pane_id;
                    acp_statuses
                        .iter()
                        .find(|(pid, _)| *pid == focused)
                        .map(|(pid, _)| format!("acp:{pid}"))
                }),
            agent_install_banner_visible: self.claude.claude_status_enabled
                && !self.claude.claude_hooks_installed,
            workspace: self.left_dock.read(cx).workspace.clone(),
        }
    }

    pub(super) fn prepare_bottom_dock_snapshot(
        &mut self,
        cx: &mut Context<Self>,
    ) -> BottomDockSnapshot {
        let active_tab_id = self.panels.active_tab_id.clone();
        let tab_summaries: Vec<(daruda_store::panels::TabId, String, usize)> = {
            let mut v: Vec<_> = self
                .panels
                .tabs
                .iter()
                .map(|t| (t.order, t.id.clone(), t.name.clone(), t.widgets.len()))
                .collect();
            v.sort_by_key(|(order, _, _, _)| *order);
            v.into_iter()
                .map(|(_, id, name, count)| (id, name, count))
                .collect()
        };
        let active_tab_widgets = self
            .panels
            .active_tab_id
            .as_ref()
            .and_then(|id| self.panels.tabs.iter().find(|t| &t.id == id))
            .map(|t| t.widgets.clone())
            .unwrap_or_default();
        // `shell_program` is None when neither the user nor project
        // config sets `[shell] program` — drag-drop quoting falls back
        // to Posix, which is right for daruda's macOS target where the
        // PTY inherits `$SHELL` (zsh / bash / sh).
        let shell = self
            .shell_program
            .as_deref()
            .map(crate::shell_quote::Shell::detect_from_program)
            .unwrap_or_default();
        let bottom_dock_size = self.bottom_dock.read(cx).size;
        // When the focused pane is an Agent chat pane mid-turn, the
        // bottom-input button toggles to "Stop" and cancels that pane's
        // turn instead of sending. `None` in every other state.
        //
        // Reading the `AgentChatView` entity here registers it in the
        // Workspace window's `tracked_entities`, so the bottom-dock snapshot
        // (and the Stop/Send button) refreshes when `turn_in_flight` flips.
        // This does NOT widen the scroll-repaint cost: the embedded view is an
        // element-tree descendant of the Workspace, so its `cx.notify()`
        // already marks the Workspace dirty via ancestor propagation
        // (`mark_view_dirty`) regardless of this read — and that propagation
        // walks ancestors only, so sibling docks / terminals stay cached.
        let focused_id = self.main_area.focused_pane_id;
        let agent_stop_pane = self
            .main_area
            .panes
            .iter()
            .find(|p| p.id == focused_id)
            .and_then(|p| p.agent_chat_view())
            .filter(|view| view.read(cx).turn_in_flight)
            .map(|_| focused_id);
        // The mode chip shows in the bottom input only when the focused pane is
        // an Agent chat pane that advertises modes — independent of
        // `turn_in_flight`, so the mode can be set before the first prompt. A
        // terminal-pane focus yields `None` (the input is shared across pane
        // kinds, so the chip must stay agent-only).
        let agent_mode = self
            .main_area
            .panes
            .iter()
            .find(|p| p.id == focused_id)
            .and_then(|p| p.agent_chat_view())
            .and_then(|view| view.read(cx).modes.clone())
            .filter(|m| !m.available.is_empty())
            .map(|m| (focused_id, m));
        BottomDockSnapshot {
            terminal_input_visible: self.terminal_input_visible,
            active_tab_id,
            tab_summaries,
            active_tab_widgets,
            grid_columns: self.mirrors.panels_grid_columns,
            bottom_dock_size,
            terminal_input: self.terminal_input.clone(),
            agent_stop_pane,
            agent_mode,
            shell,
            workspace: self.bottom_dock.read(cx).workspace.clone(),
        }
    }

    pub(super) fn prepare_right_dock_snapshot(
        &mut self,
        cx: &mut Context<Self>,
    ) -> RightDockSnapshot {
        // Deliberate attribution seam: Tasks rows anchor to the task's
        // recorded `worktree_path`, so this aggregate stays cwd-keyed
        // while the left dock attributes by pane ownership
        // (`aggregate_over_panes`). A session `cd`'d away from its lane
        // path can therefore appear under different anchors in the two
        // docks.
        let claude_status_per_path = cx
            .global::<crate::agent::tasks_global::GlobalTasks>()
            .tasks
            .iter()
            .filter_map(|t| t.state.worktree_path().cloned())
            .filter_map(|p| {
                self.claude
                    .claude_status
                    .aggregate_for_cwd(&p)
                    .map(|s| (p, s))
            })
            .collect();
        // Per-session status keyed by the `session_id` so the Tasks
        // tab's row renderer can paint a `⟳ / ● / ⚠` glyph next to
        // each row's session-id badge (R-23) without dipping into
        // the workspace.
        let claude_status_per_session: std::collections::HashMap<
            String,
            daruda_claude::SessionStatus,
        > = self
            .claude
            .claude_status
            .iter()
            .map(|(sid, file)| (sid.to_string(), file.status))
            .collect();
        // Mirror the per-session failure counters too — only entries
        // that have actually accumulated failures travel into the
        // snap so an idle session with 0 failures stays absent from
        // the map (cheaper hash, no false-positive lookups).
        let tool_use_failure_counts: std::collections::HashMap<String, u32> = self
            .claude
            .tool_use_failure_counts
            .iter()
            .filter(|&(_, &n)| n > 0)
            .map(|(sid, &n)| (sid.clone(), n))
            .collect();
        RightDockSnapshot {
            right_dock_view: self.right_dock_view,
            workspace: self.right_dock.read(cx).workspace.clone(),
            plan_limits: self.claude.plan_limits.clone(),
            service_status: self.claude.service_status.clone(),
            activity: self.claude.activity.clone(),
            usage_refresh_in_flight: self.claude.usage_refresh_in_flight,
            skills: cx
                .global::<crate::agent::skills::SkillsState>()
                .snapshot_for(self.active_lane_root().as_deref()),
            skill_search_input: self.skill_search_input.clone(),
            skill_search_query: self.skill_search_input.read(cx).value().to_string(),
            skill_plugin_expanded: self.skill_plugin_expanded.clone(),
            tasks: cx
                .global::<crate::agent::tasks_global::GlobalTasks>()
                .0
                .clone(),
            task_search_input: self.task_search_input.clone(),
            task_search_query: self.task_search_input.read(cx).value().to_string(),
            task_filter: self.task_filter,
            claude_status_per_path,
            claude_status_per_session,
            tool_use_failure_counts,
            now: chrono::Utc::now(),
            right_panel_scroll_handle: self.right_panel_scroll_handle.clone(),
            mcp: cx
                .global::<crate::agent::mcp::McpState>()
                .snapshot_for(self.active_lane_root().as_deref(), &self.mcp_project_dirs),
        }
    }
}

#[cfg(test)]
mod tests {
    // Snapshot builders are pure projections over Workspace state.
    // Behavioral coverage is exercised indirectly by every UI test
    // that renders the workspace (lifecycle, files, splits, etc.).
}
