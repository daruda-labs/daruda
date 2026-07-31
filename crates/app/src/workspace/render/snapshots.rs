//! Pure-data builders for the three dock snapshots consumed by
//! `impl Render for Workspace`. Lifting them out of the render body
//! makes the high-level flow visible: build all three snapshots,
//! publish all three, then read dock geometry, then build the
//! element tree.

use gpui::Context;

use crate::workspace::Workspace;
use crate::workspace::layout::snap::{BottomDockSnapshot, LeftDockSnapshot, RightDockSnapshot};

impl Workspace {
    pub(in crate::workspace) fn prepare_left_dock_snapshot(
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
        // Agent chat (ACP) statuses across every lane — see
        // `agent_chat_statuses`. Reading each view here (during render)
        // registers the dependency that makes a view's `cx.notify()` dirty
        // this snapshot, the same pattern the bottom-dock snapshot uses for
        // `is_busy()`.
        let acp_statuses = self.agent_chat_statuses(cx);
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
                .get(&self.active_runtime().focused_pane_id)
                .map(|b| b.session_id.clone())
                .or_else(|| {
                    // If the focused pane is an agent chat with a live session,
                    // use its synthetic ACP id so the sub-row badge highlights.
                    let focused = self.active_runtime().focused_pane_id;
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

    pub(in crate::workspace) fn prepare_bottom_dock_snapshot(
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
        // When the focused pane is an Agent chat pane that is busy (a turn in
        // flight OR a background subagent still running), the bottom-input
        // button toggles to "Stop" and cancels that pane's activity instead of
        // sending. `None` in every other state.
        //
        // Reading the `AgentChatView` entity here registers it in the
        // Workspace window's `tracked_entities`, so the bottom-dock snapshot
        // (and the Stop/Send button) refreshes when `is_busy()` flips.
        // This does NOT widen the scroll-repaint cost: the embedded view is an
        // element-tree descendant of the Workspace, so its `cx.notify()`
        // already marks the Workspace dirty via ancestor propagation
        // (`mark_view_dirty`) regardless of this read — and that propagation
        // walks ancestors only, so sibling docks / terminals stay cached.
        let focused_id = self.active_runtime().focused_pane_id;
        let agent_stop_pane = self
            .active_runtime()
            .panes
            .iter()
            .find(|p| p.id == focused_id)
            .and_then(|p| p.agent_chat_view())
            .filter(|view| view.read(cx).is_busy())
            .map(|_| focused_id);
        // The mode chip shows in the bottom input only when the focused pane is
        // an Agent chat pane that advertises modes — independent of the pane's
        // busy state, so the mode can be set before the first prompt. A
        // terminal-pane focus yields `None` (the input is shared across pane
        // kinds, so the chip must stay agent-only).
        let agent_mode = self
            .active_runtime()
            .panes
            .iter()
            .find(|p| p.id == focused_id)
            .and_then(|p| p.agent_chat_view())
            .and_then(|view| view.read(cx).session_config.mode_for_chip().cloned())
            .map(|m| (focused_id, m));
        // Config-option chips: same focused-agent-pane gate as the mode chip,
        // sourced from `config_options` (which never carries mode — `daruda_acp`
        // folds that into `agent_mode` above). This is deliberately not an
        // allowlist of known categories: whatever a given agent advertises
        // (Model, ThoughtLevel, ModelConfig, a future protocol category, …)
        // rides the same generic `config_chip` UI without daruda needing to
        // special-case it per agent — the only trim is the user's own
        // description-matched hide list (see `config_options_for_chips`).
        // `None` when the focused pane is not an agent pane or no option
        // survives the trim.
        let agent_config_options = self
            .active_runtime()
            .panes
            .iter()
            .find(|p| p.id == focused_id)
            .and_then(|p| p.agent_chat_view())
            .map(|view| {
                view.read(cx)
                    .session_config
                    .config_options_for_chips(&self.mirrors.hidden_config_option_descriptions)
            })
            .filter(|opts| !opts.is_empty())
            .map(|opts| (focused_id, opts));
        // Queued prompts of the focused agent pane, projected for the
        // queued-prompt strip. Same focused-agent-pane gate as the mode chip;
        // `None` when not an agent pane or the queue is empty (strip hidden).
        let queued_prompts = self
            .active_runtime()
            .panes
            .iter()
            .find(|p| p.id == focused_id)
            .and_then(|p| p.agent_chat_view())
            .map(|view| {
                let view = view.read(cx);
                let editing = view.queue.editing_prompt;
                // Parked prompts (kept by a Stop) sort ahead of the live queue —
                // they were submitted before anything queued after the Stop.
                let mut out: Vec<crate::workspace::layout::QueuedPromptView> = Vec::new();
                for (prompts, paused) in [
                    (&view.queue.paused_prompts, true),
                    (&view.queue.pending_prompts, false),
                ] {
                    for q in prompts {
                        out.push(crate::workspace::layout::QueuedPromptView {
                            id: q.id,
                            text: q.text.clone(),
                            editing: editing == Some(q.id),
                            paused,
                        });
                    }
                }
                out
            })
            .filter(|q| !q.is_empty())
            .map(|q| (focused_id, q));
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
            agent_config_options,
            queued_prompts,
            shell,
            workspace: self.bottom_dock.read(cx).workspace.clone(),
        }
    }

    pub(in crate::workspace) fn prepare_right_dock_snapshot(
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
        // each row's session-id badge without dipping into
        // the workspace.
        let claude_status_per_session: std::collections::HashMap<
            String,
            daruda_agent::SessionStatus,
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
        // The Usage tab stacks a section per auth domain, so both providers'
        // limits can be compared without switching panes. Each reads the
        // sticky account for its domain (`usage_account`) rather than the
        // instantaneous focus, so an unrelated pane gaining focus (a
        // terminal, or another domain's agent) can't snap a domain's section
        // back to its ambient login. This is the single write site for the
        // sticky map (`observe_focus`) — the pump and manual refresh only
        // read what it leaves behind.
        let focused = self.focused_account();
        let pane_domain = crate::workspace::main_area::pane::AccountDomain::for_pane(
            &self.focused_account_pane(cx),
        );
        crate::workspace::sync::limits::observe_focus(
            &mut self.claude.sticky_focus_by_recipe,
            focused,
            pane_domain,
        );
        let usage_sections: Vec<crate::workspace::layout::snap::UsageSectionSnapshot> =
            daruda_store::accounts::AccountRecipeId::all()
                .filter_map(|recipe| {
                    let account = crate::workspace::sync::limits::usage_account(
                        recipe,
                        &self.claude.sticky_focus_by_recipe,
                    );
                    let outcome = self
                        .claude
                        .usage_by_account
                        .usage(crate::workspace::claude_session_ops::UsageKey { recipe, account });
                    if outcome.is_signed_out() {
                        return None;
                    }
                    Some(crate::workspace::layout::snap::UsageSectionSnapshot {
                        recipe,
                        account_label: usage_account_label(
                            account
                                .account_id()
                                .and_then(|id| self.accounts.find(id))
                                .and_then(|a| a.email.as_deref()),
                            recipe,
                        )
                        .into(),
                        outcome,
                        service_status: self.claude.service_status.get(&recipe).cloned(),
                    })
                })
                .collect();
        // One activity entry per section shown above — a domain with no
        // visible usage section has nowhere in the UI to attach its
        // activity chart either, so it's not worth looking up.
        let activity: Vec<(
            daruda_store::accounts::AccountRecipeId,
            daruda_agent::ActivityStats,
        )> = usage_sections
            .iter()
            .filter_map(|section| {
                let account = crate::workspace::sync::limits::usage_account(
                    section.recipe,
                    &self.claude.sticky_focus_by_recipe,
                );
                self.claude
                    .usage_by_account
                    .activity(crate::workspace::claude_session_ops::UsageKey {
                        recipe: section.recipe,
                        account,
                    })
                    .filter(|stats| !stats.daily.is_empty())
                    .cloned()
                    .map(|stats| (section.recipe, stats))
            })
            .collect();
        // The Usage tab is rendered in the current workspace context, and
        // recent-session rows do not carry a lane label. Keep the list scoped
        // to the active Lane so Restore does not jump to another lane from an
        // unlabelled row; switching lanes naturally swaps the list.
        let active_lane_ref = self.active_ref();
        let active_lane_cwd = self.active_lane().map(|l| l.path.clone());
        // Session ids already open as a live pane, in any lane (not just
        // the active one) — such a session isn't "past" any more, and
        // `restore_session` would just focus the existing pane anyway, so
        // it doesn't belong in a list of things to restore.
        let open_session_ids: std::collections::HashSet<String> = self
            .main_area
            .runtimes
            .values()
            .flat_map(|rt| rt.panes.iter())
            .filter_map(|p| p.agent_chat_content()?.view.read(cx).session_id.clone())
            .collect();
        // Up to 10 most-recent sessions per section, restricted to the active
        // Lane. A session belonging to another Lane is restorable after the
        // user switches there; a session outside this window's Lanes needs a
        // Project-open detour this feature doesn't attempt.
        let recent_sessions: Vec<(
            daruda_store::accounts::AccountRecipeId,
            Vec<crate::workspace::layout::snap::RestorableSession>,
        )> = usage_sections
            .iter()
            .filter_map(|section| {
                let account = crate::workspace::sync::limits::usage_account(
                    section.recipe,
                    &self.claude.sticky_focus_by_recipe,
                );
                let raw = self.claude.usage_by_account.activity(
                    crate::workspace::claude_session_ops::UsageKey {
                        recipe: section.recipe,
                        account,
                    },
                )?;
                // Restore always launches under the domain's configured
                // default agent — the original session's exact agent
                // variant isn't tracked. A domain with no default agent
                // configured has nothing to restore into. Catalog-wide scan,
                // no lane in scope — same `is_remote: false` reasoning as
                // `resolve_login_command`.
                let agent_id = self
                    .agents
                    .iter()
                    .find(|a| a.launch.account_recipe(false) == Some(section.recipe))?
                    .id
                    .clone();
                let mut matched: Vec<crate::workspace::layout::snap::RestorableSession> = raw
                    .recent_sessions
                    .iter()
                    .filter_map(|s| {
                        if open_session_ids.contains(&s.session_id) {
                            return None;
                        }
                        if active_lane_cwd.as_ref() != Some(&s.cwd) {
                            return None;
                        }
                        Some(crate::workspace::layout::snap::RestorableSession {
                            session_id: s.session_id.clone(),
                            agent_id: agent_id.clone(),
                            account,
                            lane_ref: active_lane_ref,
                            title: s.title.clone().map(Into::into),
                            prompt_preview: s.prompt_preview.clone().map(Into::into),
                            git_branch: s.git_branch.clone().map(Into::into),
                            cwd: s.cwd.clone(),
                            last_active: s.last_active,
                        })
                    })
                    .collect();
                matched.sort_by_key(|s| std::cmp::Reverse(s.last_active));
                matched.truncate(10);
                (!matched.is_empty()).then_some((section.recipe, matched))
            })
            .collect();
        RightDockSnapshot {
            right_dock_view: self.right_dock_view,
            workspace: self.right_dock.read(cx).workspace.clone(),
            usage: usage_sections,
            focused_agent_domain: pane_domain,
            usage_domain_override: self.claude.usage_domain_override,
            activity,
            recent_sessions,
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

/// Display-only identity for the Usage tab header: `email` when the focused
/// account resolved one, else a "System" fallback naming `recipe`'s ambient
/// home. Reuses the status bar's formatter with `plan=None` — this header
/// wants identity only, not the "(plan)" suffix the dropdown slot shows.
fn usage_account_label(
    email: Option<&str>,
    recipe: daruda_store::accounts::AccountRecipeId,
) -> String {
    crate::workspace::status_bar::account_label(email, None).unwrap_or_else(|| {
        crate::surface::strings::status_bar_account_system(
            daruda_agent::accounts::recipe_for(recipe).system_home_hint(),
        )
    })
}

#[cfg(test)]
mod tests {
    // Snapshot builders are pure projections over Workspace state.
    // Behavioral coverage is exercised indirectly by every UI test
    // that renders the workspace (lifecycle, files, splits, etc.).
    use super::usage_account_label;
    use daruda_store::accounts::AccountRecipeId;

    #[test]
    fn the_usage_header_fallback_names_the_focused_domains_home() {
        let claude = usage_account_label(None, AccountRecipeId::Claude);
        let codex = usage_account_label(None, AccountRecipeId::Codex);
        assert!(claude.contains("~/.claude"), "{claude}");
        assert!(codex.contains("~/.codex"), "{codex}");
    }

    #[test]
    fn a_resolved_email_wins_over_the_domain_fallback() {
        assert_eq!(
            usage_account_label(Some("alice@x.com"), AccountRecipeId::Codex),
            "alice@x.com"
        );
    }
}
