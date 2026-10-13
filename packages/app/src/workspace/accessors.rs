//! Read-side lookups on `Workspace`: the active project and lane, ref
//! resolution, per-lane scoped state, and the branch labels built from them.

use super::{BranchStatus, Workspace, lane_scoped};

impl Workspace {
    pub(in crate::workspace) fn lane_git_worktree(
        &self,
        target: daruda_store::project::LaneRef,
    ) -> Option<&crate::lane::git::GitWorktreeStatus> {
        self.lane_scoped.get(&target)?.git.worktree.as_ref()
    }

    pub(in crate::workspace) fn lane_scoped_mut(
        &mut self,
        target: daruda_store::project::LaneRef,
    ) -> &mut lane_scoped::LaneScoped {
        self.lane_scoped.entry(target).or_default()
    }

    pub(in crate::workspace) fn lane_file_tree(
        &self,
        target: daruda_store::project::LaneRef,
    ) -> Option<&crate::files::tree::FileTree> {
        self.lane_scoped.get(&target)?.files.tree.as_ref()
    }

    pub(in crate::workspace) fn lane_file_tree_mut(
        &mut self,
        target: daruda_store::project::LaneRef,
    ) -> Option<&mut crate::files::tree::FileTree> {
        self.lane_scoped.get_mut(&target)?.files.tree.as_mut()
    }

    pub(in crate::workspace) fn lane_file_tree_refs(
        &self,
    ) -> impl Iterator<Item = daruda_store::project::LaneRef> + '_ {
        self.lane_scoped
            .iter()
            .filter_map(|(target, state)| state.files.tree.as_ref().map(|_| *target))
    }

    /// Human-readable name for this workspace in the recent list.
    /// Uses the active project's name when available, otherwise the
    /// first project's directory name, appending " +N more" when
    /// the workspace holds multiple projects.
    pub(in crate::workspace) fn recent_display_name(&self) -> String {
        let primary = self
            .projects
            .get(self.active.project)
            .or_else(|| self.projects.first());

        let label = match primary {
            Some(p) if !p.name.is_empty() => p.name.clone(),
            Some(p) => p
                .root
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "?".into()),
            // The workspace emptied out. Its recent row is kept so the next
            // launch finds it, so the label has to stop naming a project it
            // no longer holds.
            //
            // The departed project's name is deliberately not carried along
            // (as "Empty workspace — was <name>"): a recent row addresses a
            // workspace, not a folder, and reopening the folder is the job
            // of Open Folder. Keeping the name would make the row advertise
            // something it cannot deliver.
            None => crate::surface::strings::menu::recent_empty_workspace(),
        };

        let rest = self.projects.len().saturating_sub(1);
        if rest == 0 {
            label
        } else {
            format!("{label} +{rest} more")
        }
    }

    /// Borrow the currently active project. `None` when the workspace
    /// has no projects (Landing state).
    pub(in crate::workspace) fn active_project(&self) -> Option<&crate::project::Project> {
        self.projects.get(self.active.project)
    }

    /// Mutably borrow the currently active project.
    pub(in crate::workspace) fn active_project_mut(
        &mut self,
    ) -> Option<&mut crate::project::Project> {
        self.projects.get_mut(self.active.project)
    }

    /// Borrow the currently active lane (active project's active
    /// lane). `None` when no project is loaded.
    pub(in crate::workspace) fn active_lane(&self) -> Option<&crate::lane::Lane> {
        let id = self.active.lane;
        self.active_project()?.lane(id)
    }

    /// Mutably borrow the active lane.
    #[allow(dead_code)]
    pub(in crate::workspace) fn active_lane_mut(&mut self) -> Option<&mut crate::lane::Lane> {
        let id = self.active.lane;
        self.active_project_mut()?.lane_mut(id)
    }

    /// `true` when the active lane exists *and* its `LaneAvailability` is
    /// not `Present` (i.e. `Missing` or `AccessDenied`). Such a lane renders
    /// the empty-state and must reject pane-spawning actions (new tab /
    /// split) that would root a PTY at the dead path. `false` when there is
    /// no active lane at all (no project) — that
    /// legitimately allows tabs.
    pub(in crate::workspace) fn active_lane_is_inaccessible(&self) -> bool {
        self.active_lane()
            .is_some_and(|l| l.availability != crate::lane::availability::LaneAvailability::Present)
    }

    /// `true` when `focused_pane_id` points at a real pane in the active
    /// lane's runtime. Tests pane membership (not equality to a sentinel):
    /// the first real pane gets id 0, the same value as the default, so a
    /// paneless lane (the inaccessible empty-state) is detected only because
    /// `panes` is empty — not by comparing against a magic id.
    pub(in crate::workspace) fn has_focused_pane(&self) -> bool {
        let focused = self.active_runtime().focused_pane_id;
        self.active_runtime().panes.iter().any(|p| p.id == focused)
    }

    /// Resolve a `ProjectId` to its runtime project.
    pub(in crate::workspace) fn project_for(
        &self,
        id: daruda_store::project::ProjectId,
    ) -> Option<&crate::project::Project> {
        self.projects.get(id)
    }

    /// The project open in this window under `uuid`, the durable name a task
    /// and a persisted state file use for it.
    pub(in crate::workspace) fn project_by_uuid(
        &self,
        uuid: daruda_store::project::ProjectUuid,
    ) -> Option<&crate::project::Project> {
        self.projects.by_uuid(uuid)
    }

    pub(in crate::workspace) fn project_for_mut(
        &mut self,
        id: daruda_store::project::ProjectId,
    ) -> Option<&mut crate::project::Project> {
        self.projects.get_mut(id)
    }

    /// Resolve a `LaneRef` to its runtime lane.
    pub(in crate::workspace) fn lane_for(
        &self,
        target: daruda_store::project::LaneRef,
    ) -> Option<&crate::lane::Lane> {
        self.projects.lane(target)
    }

    /// Resolve a `LaneRef` to its runtime lane, mutably. Mirror of
    /// [`Self::lane_for`] for the write paths (branch reconcile, kind
    /// updates) that mutate the lane in place.
    pub(in crate::workspace) fn lane_for_mut(
        &mut self,
        target: daruda_store::project::LaneRef,
    ) -> Option<&mut crate::lane::Lane> {
        self.projects.lane_mut(target)
    }

    /// Borrow the active project's lane list. Empty when the
    /// workspace has no projects (Landing state).
    pub(in crate::workspace) fn active_lanes(&self) -> &[crate::lane::Lane] {
        self.active_project()
            .map(|p| p.lanes.as_slice())
            .unwrap_or(&[])
    }

    /// `LaneRef` pointing at the workspace's currently active
    /// lane. Convenience for call sites that already know the
    /// active id but want the `LaneRef` form for HashMap lookups.
    pub(in crate::workspace) fn active_ref(&self) -> daruda_store::project::LaneRef {
        self.active
    }

    /// `true` when at least one lane has a Claude session (PTY-bound or
    /// ACP agent chat) in an animating status (anything but `Idle`). Gates
    /// the status-pulse pump so the shared `StatusPulseClock` only
    /// repaints windows that actually show motion — idle windows stay at
    /// zero redraws.
    pub(crate) fn has_animating_claude_status(&self, cx: &gpui::App) -> bool {
        if !self.mirrors.claude_status_enabled {
            return false;
        }
        // ACP pane statuses across every lane (see `agent_chat_statuses`),
        // so a parked lane with a live agent turn still keeps the pulse
        // running.
        let acp_statuses = self.agent_chat_statuses(cx);
        if self.claude_sessions.pty_bindings.is_empty() && acp_statuses.is_empty() {
            return false;
        }
        // Per-session, not per-lane-aggregate: the aggregate's
        // max-priority collapse would hide a `Connecting` session
        // (priority 0) behind an `Idle` sibling (priority 1) and stop
        // the pulse while that Connecting badge still animates in the
        // sub-row. Only a pane set where every bound session is `Idle`
        // is fully at rest.
        let index = self.pane_lane_index();
        crate::workspace::claude_status_aggregate::any_pane_session_animating(
            &index,
            &self.claude_sessions.pty_bindings,
            &self.claude_sessions.status,
            &acp_statuses,
        )
    }

    /// Display name of the currently active project. `None` when the
    /// workspace has no projects.
    pub(crate) fn active_project_name(&self) -> Option<String> {
        self.active_project().map(|p| p.name.clone())
    }

    /// Active lane's branch status, split into the three cases
    /// the status bar / window title care about: attached git branch,
    /// detached HEAD, or non-git default kind. Returned as a small
    /// enum so call-site renderers can decide whether to draw an
    /// inline "detached" chip in addition to the textual label.
    pub(in crate::workspace) fn active_branch_status(&self) -> BranchStatus {
        let Some(wt) = self.active_lane() else {
            return BranchStatus::NotGit;
        };
        match &wt.kind {
            daruda_store::project::LaneKind::Git {
                branch: Some(b), ..
            } => BranchStatus::Branch(b.clone()),
            daruda_store::project::LaneKind::Git { branch: None, .. } => BranchStatus::Detached,
            _ => BranchStatus::NotGit,
        }
    }

    /// Text portion of the status-bar project-branch slot. Returns
    /// `<project>/<branch>` for attached branches and `<project>`
    /// otherwise; the detached state is signalled separately by an
    /// inline chip ([`StatusBarData::is_detached`]), not folded into
    /// this string.
    pub(in crate::workspace) fn active_project_branch_label(&self) -> Option<String> {
        self.project_branch_label_with("/", false)
    }

    /// Window title fallback (`WindowRuntime::user_label` overrides this).
    /// Folds the detached marker into the text — macOS window titles
    /// are plain strings, so the chip cannot ride along the way it
    /// does in the status bar.
    pub(in crate::workspace) fn window_title_label(&self) -> Option<String> {
        self.project_branch_label_with(" · ", true)
    }

    fn project_branch_label_with(&self, sep: &str, mark_detached_inline: bool) -> Option<String> {
        let project = self.active_project()?;
        Some(match self.active_branch_status() {
            BranchStatus::Branch(b) => format!("{}{sep}{}", project.name, b),
            BranchStatus::Detached if mark_detached_inline => {
                format!("{} (detached)", project.name)
            }
            BranchStatus::Detached | BranchStatus::NotGit => project.name.clone(),
        })
    }

    /// True when this workspace holds no projects — the state the
    /// Landing view paints. The open-folder path reads it to skip the
    /// `WindowOpenPolicy` question entirely: "add here or open a new
    /// window?" has no meaning for a window with nothing in it.
    pub(crate) fn has_no_projects(&self) -> bool {
        self.projects.is_empty()
    }

    /// True when any project in this workspace already has `root` as
    /// its checkout. Used by the `AddHere` policy path so opening the
    /// same folder twice in the same window focuses the existing
    /// window instead of registering a duplicate runtime project.
    /// Scoped to this window only — Policy B explicitly permits the
    /// same root across separate windows. Compares both as-given and
    /// canonicalized so the symlinked `/tmp` vs `/private/tmp`
    /// flavours on macOS still match.
    pub(crate) fn has_project_root(&self, root: &std::path::Path) -> bool {
        self.projects
            .iter()
            .any(|p| daruda_core::path::same_path(&p.root, root))
    }

    /// Read-only slice of every runtime project currently in this
    /// workspace. Used by menu / palette code that needs to list project
    /// metadata without going through `WorkspaceState`.
    #[allow(dead_code)]
    pub(crate) fn projects(&self) -> &[crate::project::Project] {
        &self.projects
    }
}
