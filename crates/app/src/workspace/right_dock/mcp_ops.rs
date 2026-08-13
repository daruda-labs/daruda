//! Workspace methods for MCP server CRUD.
//!
//! Mutations run inside `cx.update_global::<McpState, _>` and stage raw JSON
//! before atomic persist, so failed writes leave memory and disk untouched.
//! Persist helpers patch only `mcpServers[name]`, preserving unrelated
//! `.claude.json` keys.
//!
//! Disk I/O stays synchronous because files are tiny and writes are user-driven.
//! Self-write events are allowed to loop back; reload parses the same content
//! and the change gate converges without visible flicker.

use std::path::{Path, PathBuf};

use gpui::{BorrowAppContext, Context};
use serde_json::Value;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};

use crate::agent::mcp::{
    McpLocation, McpPersistError, McpScope, McpServer, McpServerDraft, McpState, ProjectMcp,
    delete_server, parse, set_disabled, update_server, write_server,
};

use crate::workspace::Workspace;

/// Materialize a draft for the in-memory list without re-parsing disk.
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

/// Resolve `(write path, JSON location)` for `scope`. `None` when a
/// lane root is required (`Project` / `Local`) but absent. Free helper
/// so callers can compute the target without holding a `&McpState`.
fn target_meta(scope: McpScope, lane: Option<&Path>) -> Option<(PathBuf, McpLocation)> {
    match scope {
        McpScope::User => Some((parse::claude_json_path(), McpLocation::TopLevel)),
        McpScope::Local => lane.map(|w| (parse::claude_json_path(), McpLocation::project(w))),
        McpScope::Project => lane.map(|w| (parse::project_mcp_path(w), McpLocation::TopLevel)),
    }
}

/// Mutable handle to one scope's `(servers, raw)` inside the Global.
/// Returns `None` for `Project` / `Local` when `lane` is absent — no
/// scoped entry to mutate. For `User`, always returns `Some`.
///
/// User and Local both write into the shared `claude_json_raw` tree;
/// only the in-memory server vector differs (top-level vs per-lane).
fn scope_slot_mut<'a>(
    state: &'a mut McpState,
    scope: McpScope,
    lane: Option<&Path>,
) -> Option<(&'a mut Vec<McpServer>, &'a mut Value)> {
    match scope {
        McpScope::User => Some((&mut state.user, &mut state.claude_json_raw)),
        McpScope::Local => {
            let w = lane?;
            // Disjoint fields — borrow the shared raw tree and the
            // per-lane Local vector simultaneously.
            let raw = &mut state.claude_json_raw;
            let servers = state.local.entry(w.to_path_buf()).or_default();
            Some((servers, raw))
        }
        McpScope::Project => {
            let w = lane?;
            let entry = state.project.entry(w.to_path_buf()).or_default();
            let ProjectMcp { servers, raw } = entry;
            Some((servers, raw))
        }
    }
}

/// Re-read `~/.claude.json` from disk into the Global before patching a
/// User / Local server. `~/.claude.json` holds the user's entire Claude
/// Code state and Claude Code rewrites it constantly; refreshing right
/// before our whole-file atomic rewrite shrinks the lost-update window
/// against a concurrent write. No-op for the Project scope, whose
/// `.mcp.json` is daruda-owned and low-collision.
fn refresh_before_write(
    state: &mut McpState,
    scope: McpScope,
    lane: Option<&Path>,
) -> Result<(), McpPersistError> {
    if scope.in_claude_json() {
        state.reload_claude_json(lane)?;
    }
    Ok(())
}

impl Workspace {
    /// Flip the `disabled` flag on the named server in `scope`.
    pub(in crate::workspace) fn toggle_mcp_server(
        &mut self,
        scope: McpScope,
        name: &str,
        cx: &mut Context<Self>,
    ) {
        let lane = self.active_lane_root();
        let Some((path, location)) = target_meta(scope, lane.as_deref()) else {
            return;
        };
        let result = cx.update_global::<McpState, _>(|state, _| {
            refresh_before_write(state, scope, lane.as_deref())?;
            let Some((servers, raw)) = scope_slot_mut(state, scope, lane.as_deref()) else {
                return Ok::<_, McpPersistError>(false);
            };
            let Some(current) = servers.iter().find(|s| s.name == name).map(|s| s.disabled) else {
                return Ok(false);
            };
            let new_disabled = !current;
            set_disabled(raw, &path, scope, &location, name, new_disabled)?;
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
            let report = ErrorReport::new(crate::surface::strings::error_mcp_toggle_failed(
                &crate::surface::strings::mcp_scope_display(scope),
            ))
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
        let lane = self.active_lane_root();
        let (path, location) =
            target_meta(scope, lane.as_deref()).ok_or(McpPersistError::NoProjectRoot)?;
        let result = cx.update_global::<McpState, _>(|state, _| {
            refresh_before_write(state, scope, lane.as_deref())?;
            let (servers, raw) = scope_slot_mut(state, scope, lane.as_deref())
                .ok_or(McpPersistError::NoProjectRoot)?;
            write_server(raw, &path, scope, &location, &draft)?;
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
        let lane = self.active_lane_root();
        let (path, location) =
            target_meta(scope, lane.as_deref()).ok_or(McpPersistError::NoProjectRoot)?;
        let result = cx.update_global::<McpState, _>(|state, _| {
            refresh_before_write(state, scope, lane.as_deref())?;
            let (servers, raw) = scope_slot_mut(state, scope, lane.as_deref())
                .ok_or(McpPersistError::NoProjectRoot)?;
            update_server(raw, &path, scope, &location, &draft)?;
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
        let lane = self.active_lane_root();
        let (path, location) =
            target_meta(scope, lane.as_deref()).ok_or(McpPersistError::NoProjectRoot)?;
        let result = cx.update_global::<McpState, _>(|state, _| {
            refresh_before_write(state, scope, lane.as_deref())?;
            let (servers, raw) = scope_slot_mut(state, scope, lane.as_deref())
                .ok_or(McpPersistError::NoProjectRoot)?;
            delete_server(raw, &path, scope, &location, name)?;
            servers.retain(|s| s.name != name);
            Ok(())
        });
        cx.notify();
        result
    }
}
