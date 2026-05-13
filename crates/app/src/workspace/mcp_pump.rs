//! MCP watcher → GPUI bridge.
//!
//! Mirrors `skills_pump`: every 100 ms the pump task drains every
//! [`McpEvent`] queued by the watcher and dispatches a per-scope reload
//! through [`Workspace::apply_mcp_event`]. Dropping the returned
//! `Task<()>` (held as `_mcp_event_pump` on `Workspace`) cancels the
//! loop, the receiver disconnects, and the watcher threads exit.
//!
//! Mutations land in the app-wide `McpState` Global; other Workspaces
//! re-render via their `observe_global::<McpState>` subscription
//! registered in `Workspace::new_with_project`.

use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::{BorrowAppContext, Context, Task};

use super::Workspace;
use crate::agent::mcp::{McpScope, McpState, personal_settings_path, project_mcp_path};
use crate::hooks::mcp_watcher::{self, McpEvent};

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
    /// The event carries only the scope; the worktree path is taken
    /// from *this* workspace's active worktree at fire time so the
    /// pump never reloads against a stale project root. The result
    /// lands in the `McpState` Global, where every other open
    /// Workspace picks it up through `observe_global`.
    pub(in crate::workspace) fn apply_mcp_event(
        &mut self,
        event: McpEvent,
        cx: &mut Context<Self>,
    ) {
        let McpEvent::Reloaded(scope) = event;
        let worktree = self.active_worktree_root();
        let result = cx.update_global::<McpState, _>(|state, _| {
            state.reload_scope(scope, worktree.as_deref())
        });
        if let Err(e) = result {
            let report = ErrorReport::new(format!("MCP reload failed ({})", scope.slug()))
                .severity(ErrorSeverity::Warning)
                .from_error(&e)
                .at(file!(), line!())
                .with_context("scope", scope.slug())
                .dedup(format!("mcp.reload.{}", scope.slug()))
                .build();
            self.report_error(report, cx);
            return;
        }
        cx.notify();
    }

    /// (Re)spawn the MCP watcher with the current active worktree's
    /// `.mcp.json` path. Call from initial construction, worktree
    /// create / remove, and worktree activation.
    ///
    /// Dismisses any open MCP modal first — `AddMcpInitial` /
    /// `EditMcpInitial` snapshots are taken at open time and become
    /// stale when the active worktree changes (the project scope path
    /// underneath them moves to a different file). Delete-confirm uses
    /// the shared `ConfirmModal` and isn't type-checkable here, so
    /// users who confirmed a delete during a worktree swap may see the
    /// post-confirm error banner — acceptable, since we'd otherwise
    /// have to dismiss every confirm modal in the workspace.
    pub fn refresh_mcp_watcher(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) {
        // Close any open dialog — `AddMcpInitial` / `EditMcpInitial`
        // snapshots are taken at open time and become stale when the
        // active worktree changes. We can't downcast a Dialog to a
        // specific modal type, so close indiscriminately. In practice
        // worktree activation is a major user action and stale dialogs
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
        self._mcp_watcher = None;
        self._mcp_event_pump = None;

        let project_root = self.active_worktree_root();
        let project_path = project_root.as_deref().map(project_mcp_path);
        let personal_path = personal_settings_path();

        // Reload synchronously so the panel reflects whatever is on
        // disk for the new root before the watcher's first event lands.
        let personal_result = cx
            .update_global::<McpState, _>(|state, _| state.reload_scope(McpScope::Personal, None));
        if let Err(e) = personal_result {
            let report = ErrorReport::new("MCP reload failed (personal)")
                .severity(ErrorSeverity::Warning)
                .from_error(&e)
                .at(file!(), line!())
                .with_context("scope", McpScope::Personal.slug())
                .dedup("mcp.reload.personal")
                .build();
            self.report_error(report, cx);
        }
        let project_result = cx.update_global::<McpState, _>(|state, _| {
            state.reload_scope(McpScope::Project, project_root.as_deref())
        });
        if let Err(e) = project_result {
            let report = ErrorReport::new("MCP reload failed (project)")
                .severity(ErrorSeverity::Warning)
                .from_error(&e)
                .at(file!(), line!())
                .with_context("scope", McpScope::Project.slug())
                .dedup("mcp.reload.project")
                .build();
            self.report_error(report, cx);
        }
        cx.notify();

        let (events, handle) = mcp_watcher::spawn(project_path, personal_path);
        let pump = spawn(events, cx);
        self._mcp_watcher = Some(handle);
        self._mcp_event_pump = Some(pump);
    }
}
