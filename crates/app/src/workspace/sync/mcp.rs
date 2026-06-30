//! MCP watcher → GPUI bridge.
//!
//! Mirrors `skills_pump`: every 100 ms the pump task drains every
//! [`McpEvent`] queued by the watcher and dispatches a per-file reload
//! through [`Workspace::apply_mcp_event`]. Dropping the returned
//! `Task<()>` (held as `_mcp_event_pump` on `Workspace`) cancels the
//! loop, the receiver disconnects, and the watcher threads exit.
//!
//! Mutations land in the app-wide `McpState` Global; other Workspaces
//! re-render via their `observe_global::<McpState>` subscription
//! registered in `Workspace::new_with_project`.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::{BorrowAppContext, Context, Task};

use crate::agent::mcp::{McpState, claude_json_path, project_mcp_path};
use crate::hooks::mcp_watcher::{self, McpEvent};
use crate::workspace::Workspace;

const POLL_INTERVAL: Duration = Duration::from_millis(100);

pub(in crate::workspace) fn spawn(
    events: Receiver<McpEvent>,
    cx: &mut Context<Workspace>,
) -> Task<()> {
    cx.spawn(async move |this, cx| {
        'outer: loop {
            cx.background_executor().timer(POLL_INTERVAL).await;
            loop {
                let ev = match events.try_recv() {
                    Ok(e) => e,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => break 'outer,
                };
                if this
                    .update(cx, |ws, cx| ws.apply_mcp_event(ev, cx))
                    .is_err()
                {
                    break 'outer;
                }
            }
        }
    })
}

impl Workspace {
    /// Apply one debounced [`McpEvent`] from the watcher.
    ///
    /// The event carries only the changed file; the lane path is taken
    /// from *this* workspace's active lane at fire time so the pump
    /// never reloads against a stale project root. The result lands in
    /// the `McpState` Global, where every other open Workspace picks it
    /// up through `observe_global`.
    ///
    /// Notify (and thus re-render) only fires when the reload actually
    /// changed an MCP server. `~/.claude.json` can be multi-megabyte and
    /// Claude Code rewrites it on nearly every interaction; without this
    /// gate every unrelated write would repaint the workspace.
    pub(in crate::workspace) fn apply_mcp_event(
        &mut self,
        event: McpEvent,
        cx: &mut Context<Self>,
    ) {
        let lane = self.active_lane_root();
        let project_dirs = self.mcp_project_dirs.clone();
        let label = match event {
            McpEvent::ClaudeJsonReloaded => "user/local",
            McpEvent::ProjectReloaded => "project",
        };
        let result = cx.update_global::<McpState, _>(|state, _| match event {
            McpEvent::ClaudeJsonReloaded => state.reload_claude_json(lane.as_deref()),
            McpEvent::ProjectReloaded => {
                // The Project scope merges every `.mcp.json` from the
                // lane root up to the git repo root (+ the focused cwd
                // chain) — reload them all.
                let mut changed = false;
                for d in &project_dirs {
                    changed |= state.reload_project(Some(d))?;
                }
                Ok::<bool, _>(changed)
            }
        });
        match result {
            Ok(true) => cx.notify(),
            Ok(false) => {}
            Err(e) => {
                let report = ErrorReport::new(format!("MCP reload failed ({label})"))
                    .severity(ErrorSeverity::Warning)
                    .from_error(&e)
                    .at(file!(), line!())
                    .with_context("source", label)
                    .dedup(format!("mcp.reload.{label}"))
                    .build();
                self.report_error(report, cx);
            }
        }
    }

    /// (Re)spawn the MCP watcher with the current active lane's
    /// `.mcp.json` path. Call from initial construction, lane
    /// create / remove, and lane activation.
    ///
    /// Dismisses any open MCP modal first — `AddMcpInitial` /
    /// `EditMcpInitial` snapshots are taken at open time and become
    /// stale when the active lane changes (the project scope path
    /// underneath them moves to a different file). Delete-confirm uses
    /// the shared `ConfirmModal` and isn't type-checkable here, so
    /// users who confirmed a delete during a lane swap may see the
    /// post-confirm error banner — acceptable, since we'd otherwise
    /// have to dismiss every confirm modal in the workspace.
    pub fn refresh_mcp_watcher(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        // Close any open dialog — `AddMcpInitial` / `EditMcpInitial`
        // snapshots are taken at open time and become stale when the
        // active lane changes. We can't downcast a Dialog to a
        // specific modal type, so close indiscriminately. In practice
        // lane activation is a major user action and stale dialogs
        // (incl. delete-confirm) are reasonable to dismiss.
        //
        // Guard the root-walk for the case where this method runs
        // during initial Workspace construction — before
        // `windows.rs` wraps us in `gpui_component::Root`, calling
        // `Root::read` panics.
        if window.root::<gpui_component::Root>().flatten().is_some() {
            use crate::ui::WindowExt as _;
            if window.has_active_dialog(cx) {
                window.close_dialog(cx);
            }
        }
        self.respawn_mcp_watcher(cx);
    }

    /// The focused terminal pane's live working directory (OSC 7).
    /// `None` when no focused pane reports a cwd.
    pub(in crate::workspace) fn active_mcp_cwd(&self) -> Option<PathBuf> {
        self.active_runtime()
            .panes
            .iter()
            .find(|p| p.id == self.active_runtime().focused_pane_id)
            .and_then(|p| p.cwd().map(Path::to_path_buf))
    }

    /// Compute the directories whose `.mcp.json` form the Project scope,
    /// in precedence order (nearest first). Mirrors Claude Code
    /// searching upward from its cwd: the lane root and the focused
    /// terminal cwd each contribute their own directory plus every
    /// ancestor up to the enclosing git repo root. This is why a
    /// repo-root `.mcp.json` shows for a subdirectory lane without the
    /// user having to `cd`. The lane root is always first (write target
    /// / Local key).
    ///
    /// Stat-walks the filesystem, so it is called only from
    /// [`Workspace::respawn_mcp_watcher`] and the result cached in
    /// `mcp_project_dirs`; the per-frame render snapshot reads the cache.
    fn compute_mcp_project_dirs(&self) -> Vec<PathBuf> {
        let mut dirs: Vec<PathBuf> = Vec::new();
        for start in [self.active_lane_root(), self.active_mcp_cwd()]
            .into_iter()
            .flatten()
        {
            for d in mcp_dirs_up_to_git_root(&start) {
                if !dirs.contains(&d) {
                    dirs.push(d);
                }
            }
        }
        dirs
    }

    /// Respawn the MCP watcher on a focused-terminal cwd change, but
    /// only when the change actually alters the Project dir set —
    /// `cd`-ing between two leaves with the same git-root chain (or OSC 7
    /// noise) shouldn't tear down and re-spawn watcher threads.
    pub(in crate::workspace) fn refresh_mcp_on_cwd_change(&mut self, cx: &mut Context<Self>) {
        if self.compute_mcp_project_dirs() != self.mcp_project_dirs {
            self.respawn_mcp_watcher(cx);
        }
    }

    /// Window-free core of [`Workspace::refresh_mcp_watcher`]: reloads
    /// every scope synchronously and (re)spawns the filesystem watcher
    /// for `~/.claude.json` plus every Project `.mcp.json` directory
    /// ([`Workspace::mcp_project_dirs`]). Also called when the focused
    /// terminal's cwd changes so a new cwd's chain is picked up live.
    pub(in crate::workspace) fn respawn_mcp_watcher(&mut self, cx: &mut Context<Self>) {
        self._mcp_watcher = None;
        self._mcp_event_pump = None;

        // Recompute + cache the Project dirs here (the only site that
        // stat-walks); everything else reads the cached field.
        self.mcp_project_dirs = self.compute_mcp_project_dirs();
        let lane_root = self.active_lane_root();
        let project_dirs = self.mcp_project_dirs.clone();
        let claude_json = claude_json_path();

        // Reload synchronously so the panel reflects whatever is on
        // disk for the new root before the watcher's first event lands.
        let claude_json_result = cx.update_global::<McpState, _>(|state, _| {
            state.reload_claude_json(lane_root.as_deref())
        });
        if let Err(e) = claude_json_result {
            let report = ErrorReport::new("MCP reload failed (user/local)")
                .severity(ErrorSeverity::Warning)
                .from_error(&e)
                .at(file!(), line!())
                .with_context("source", "user/local")
                .dedup("mcp.reload.user_local")
                .build();
            self.report_error(report, cx);
        }
        let dirs_for_reload = project_dirs.clone();
        let project_result = cx.update_global::<McpState, _>(|state, _| {
            let mut last = Ok(false);
            for d in &dirs_for_reload {
                let r = state.reload_project(Some(d));
                if r.is_err() {
                    last = r;
                }
            }
            last
        });
        if let Err(e) = project_result {
            let report = ErrorReport::new("MCP reload failed (project)")
                .severity(ErrorSeverity::Warning)
                .from_error(&e)
                .at(file!(), line!())
                .with_context("source", "project")
                .dedup("mcp.reload.project")
                .build();
            self.report_error(report, cx);
        }
        cx.notify();

        let project_paths: Vec<PathBuf> =
            project_dirs.iter().map(|d| project_mcp_path(d)).collect();

        let (events, handle) = mcp_watcher::spawn(project_paths, claude_json);
        let pump = spawn(events, cx);
        self._mcp_watcher = Some(handle);
        self._mcp_event_pump = Some(pump);
    }
}

/// Walk up from `start` collecting each directory until (and including)
/// the enclosing git repo root (the directory holding `.git`). When no
/// git root is found within [`MAX_ANCESTOR_WALK`] levels, only `start`
/// is returned so unrelated ancestor `.mcp.json` files aren't pulled in.
fn mcp_dirs_up_to_git_root(start: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let mut cur = start.to_path_buf();
    for _ in 0..MAX_ANCESTOR_WALK {
        out.push(cur.clone());
        if cur.join(".git").exists() {
            return out;
        }
        match cur.parent() {
            Some(p) if p != cur => cur = p.to_path_buf(),
            _ => break,
        }
    }
    vec![start.to_path_buf()]
}

/// Upper bound on the ancestor walk so a non-repo path deep in the
/// filesystem can't fan the search out unboundedly.
const MAX_ANCESTOR_WALK: usize = 64;

#[cfg(test)]
mod tests {
    use super::mcp_dirs_up_to_git_root;

    #[test]
    fn walk_collects_up_to_and_including_git_root() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        let sub = repo.join("pkg").join("inner");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::create_dir_all(repo.join(".git")).unwrap();

        let dirs = mcp_dirs_up_to_git_root(&sub);
        assert_eq!(dirs, vec![sub.clone(), repo.join("pkg"), repo.clone()]);
    }

    #[test]
    fn walk_without_git_root_returns_only_start() {
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("a").join("b");
        std::fs::create_dir_all(&sub).unwrap();
        // No `.git` anywhere up the chain → only the start dir, so
        // unrelated ancestor `.mcp.json` files aren't pulled in.
        assert_eq!(mcp_dirs_up_to_git_root(&sub), vec![sub]);
    }
}
