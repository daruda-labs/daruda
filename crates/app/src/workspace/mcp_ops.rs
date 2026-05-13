//! Workspace methods for MCP server CRUD.
//!
//! Each method opens a `cx.update_global::<McpState, _>` closure so the
//! persist machinery operates on the app-wide Global. Inside the
//! closure we mirror the previous pattern: stage a copy of the raw
//! JSON tree, write it atomically via `tempfile::NamedTempFile::persist`,
//! and only swap the in-memory tree on success. A failed write leaves
//! both sides untouched. Non-`mcpServers` keys (permissions, hooks,
//! etc.) survive every save by definition because
//! `agent::mcp::persist::*` only ever patches `mcpServers[name]`.
//!
//! Disk I/O is **synchronous on the main thread**. The files involved
//! are tiny (a JSON object with a handful of keys) and the writes are
//! infrequent (only on user CRUD action), so the cost is well below
//! perceptible. A future revision could move the rename onto a
//! background task if a slow filesystem ever surfaces.
//!
//! There is no self-write suppression: every save we perform fires a
//! filesystem event that loops back as `McpEvent::Reloaded`. That's
//! deliberate — the reload re-parses the same disk content we just
//! wrote, so the in-memory state converges to itself with no visible
//! flicker. Keeping the path simple is worth the negligible duplicate
//! work.

use std::path::{Path, PathBuf};

use gpui::{BorrowAppContext, Context};
use serde_json::Value;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};

use crate::agent::mcp::{
    McpPersistError, McpScope, McpServer, McpServerDraft, McpState, ProjectMcp, delete_server,
    parse, set_disabled, update_server, write_server,
};

use super::Workspace;

/// Materialise a draft as a freshly-built `McpServer` for the in-memory
/// list. The on-disk JSON tree was already updated by the persist
/// helpers; this keeps the renderer's snapshot in sync without
/// re-parsing.
fn make_server_from_draft(scope: McpScope, draft: &McpServerDraft) -> McpServer {
    McpServer {
        name: draft.name.clone(),
        scope,
        transport: draft.transport,
        command: draft.command.clone(),
        args: draft.args.clone(),
        url: draft.url.clone(),
        env: draft.env.clone(),
        headers: draft.headers.clone(),
        disabled: draft.disabled,
        extra: draft.extra.clone(),
    }
}

/// Resolve the on-disk path for `scope`. `None` for `Project` when
/// `worktree` is absent. Free helper so callers can compute the path
/// without holding a `&McpState`.
fn path_for(scope: McpScope, worktree: Option<&Path>) -> Option<PathBuf> {
    match scope {
        McpScope::Project => worktree.map(parse::project_mcp_path),
        McpScope::Personal => Some(parse::personal_settings_path()),
    }
}

/// Mutable handle to one scope's `(servers, raw)` inside the Global.
/// Returns `None` for `Project` when `worktree` is absent — no scoped
/// entry to mutate. For `Personal`, always returns `Some`.
fn scope_slot_mut<'a>(
    state: &'a mut McpState,
    scope: McpScope,
    worktree: Option<&Path>,
) -> Option<(&'a mut Vec<McpServer>, &'a mut Value)> {
    match scope {
        McpScope::Project => {
            let w = worktree?;
            let entry = state.project.entry(w.to_path_buf()).or_default();
            let ProjectMcp { servers, raw } = entry;
            Some((servers, raw))
        }
        McpScope::Personal => Some((&mut state.personal, &mut state.personal_raw)),
    }
}

impl Workspace {
    /// Flip the `disabled` flag on the named server in `scope`.
    pub(in crate::workspace) fn toggle_mcp_server(
        &mut self,
        scope: McpScope,
        name: &str,
        cx: &mut Context<Self>,
    ) {
        let worktree = self.active_worktree_root();
        let Some(path) = path_for(scope, worktree.as_deref()) else {
            return;
        };
        let result = cx.update_global::<McpState, _>(|state, _| {
            let Some((servers, raw)) = scope_slot_mut(state, scope, worktree.as_deref()) else {
                return Ok::<_, McpPersistError>(false);
            };
            let Some(current) = servers.iter().find(|s| s.name == name).map(|s| s.disabled) else {
                return Ok(false);
            };
            let new_disabled = !current;
            set_disabled(raw, &path, scope, name, new_disabled)?;
            if let Some(s) = servers.iter_mut().find(|s| s.name == name) {
                s.disabled = new_disabled;
            }
            Ok(true)
        });
        if let Err(e) = result {
            // Route through the toast/log pipeline like the other MCP
            // mutators (add / update / delete) — earlier revisions only
            // poked `self.last_error`, which is now reserved for inline
            // form-validation banners.
            let report = ErrorReport::new(format!("MCP toggle failed ({})", scope.slug()))
                .severity(ErrorSeverity::Warning)
                .from_error(&e)
                .at(file!(), line!())
                .with_context("scope", scope.slug())
                .with_context("server", name)
                .dedup(format!("mcp.toggle.{}", scope.slug()))
                .build();
            self.report_error(report, cx);
        }
        cx.notify();
    }

    /// Add a new server to `scope`.
    pub(in crate::workspace) fn add_mcp_server(
        &mut self,
        scope: McpScope,
        draft: McpServerDraft,
        cx: &mut Context<Self>,
    ) -> Result<(), McpPersistError> {
        let worktree = self.active_worktree_root();
        let path = path_for(scope, worktree.as_deref()).ok_or(McpPersistError::NoProjectRoot)?;
        let result = cx.update_global::<McpState, _>(|state, _| {
            let (servers, raw) = scope_slot_mut(state, scope, worktree.as_deref())
                .ok_or(McpPersistError::NoProjectRoot)?;
            write_server(raw, &path, scope, &draft)?;
            let server = make_server_from_draft(scope, &draft);
            servers.push(server);
            servers.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(())
        });
        cx.notify();
        result
    }

    /// Replace the entry at `(scope, draft.name)`.
    pub(in crate::workspace) fn update_mcp_server(
        &mut self,
        scope: McpScope,
        draft: McpServerDraft,
        cx: &mut Context<Self>,
    ) -> Result<(), McpPersistError> {
        let worktree = self.active_worktree_root();
        let path = path_for(scope, worktree.as_deref()).ok_or(McpPersistError::NoProjectRoot)?;
        let result = cx.update_global::<McpState, _>(|state, _| {
            let (servers, raw) = scope_slot_mut(state, scope, worktree.as_deref())
                .ok_or(McpPersistError::NoProjectRoot)?;
            update_server(raw, &path, scope, &draft)?;
            let new_server = make_server_from_draft(scope, &draft);
            if let Some(slot) = servers.iter_mut().find(|s| s.name == draft.name) {
                *slot = new_server;
            } else {
                servers.push(new_server);
                servers.sort_by(|a, b| a.name.cmp(&b.name));
            }
            Ok(())
        });
        cx.notify();
        result
    }

    /// Remove the named server from `scope`. Caller is the
    /// delete-confirm modal, which already gated on user confirmation.
    pub(in crate::workspace) fn delete_mcp_server_internal(
        &mut self,
        scope: McpScope,
        name: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), McpPersistError> {
        let worktree = self.active_worktree_root();
        let path = path_for(scope, worktree.as_deref()).ok_or(McpPersistError::NoProjectRoot)?;
        let result = cx.update_global::<McpState, _>(|state, _| {
            let (servers, raw) = scope_slot_mut(state, scope, worktree.as_deref())
                .ok_or(McpPersistError::NoProjectRoot)?;
            delete_server(raw, &path, scope, name)?;
            servers.retain(|s| s.name != name);
            Ok(())
        });
        cx.notify();
        result
    }

    // -------- Modal opener shims (called from render.rs hover actions) --------

    /// Opens the AddMcpServerModal. Public surface for command palette
    /// + Tools tab `[+ Add Server]` button.
    pub fn open_add_mcp_server_modal(
        &mut self,
        prefill_scope: Option<McpScope>,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        super::right_panel::tools::open_add_mcp_server_modal(self, prefill_scope, window, cx);
    }

    /// Opens the EditMcpServerModal for the given `(scope, name)`.
    pub fn open_edit_mcp_server_modal(
        &mut self,
        scope: McpScope,
        name: String,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        super::right_panel::tools::open_edit_mcp_server_modal(self, scope, name, window, cx);
    }

    /// Opens the delete-confirm modal for the given `(scope, name)`.
    pub fn open_delete_mcp_server_confirm(
        &mut self,
        scope: McpScope,
        name: String,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        super::right_panel::tools::open_delete_mcp_server_confirm(self, scope, name, window, cx);
    }
}
