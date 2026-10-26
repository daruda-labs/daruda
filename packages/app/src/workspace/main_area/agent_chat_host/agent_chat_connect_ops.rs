//! ACP connection entry points: lazy connect, retry and session reset.
//! Launch planning, advisories, login recovery and the event pump have
//! separate child modules; pane construction stays in [`super::agent_chat_ops`].

mod login;
mod notices;
mod prepare;
mod pump;

use prepare::runtime_prep_phase;

use std::path::PathBuf;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::Context;

use crate::agent::account::PreparedAccount;
use crate::surface::strings as s;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;
use daruda_chat::view::AgentSessionStatus;

struct AgentChatConnectionPlan {
    agent_id: String,
    launch_spec: daruda_acp::LaunchSpec,
    connect_cwd: PathBuf,
    initial_model: Option<String>,
    initial_modes: Vec<String>,
    prepared: Option<PreparedAccount>,
    /// MCP servers this session may reach. Non-empty for exactly one pane —
    /// the orchestrator's — because a lane agent holding daruda's tools could
    /// drive the app, which is the threat model this whole surface is shaped
    /// around.
    mcp_servers: Vec<daruda_acp::McpServer>,
}

#[cfg(test)]
struct TestAgentConnections(std::rc::Rc<std::cell::RefCell<Vec<daruda_acp::HandleProbe>>>);

#[cfg(test)]
impl gpui::Global for TestAgentConnections {}

#[cfg(test)]
impl Workspace {
    pub(crate) fn fake_agent_connections_for_test(
        cx: &mut gpui::App,
    ) -> std::rc::Rc<std::cell::RefCell<Vec<daruda_acp::HandleProbe>>> {
        let probes = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        cx.set_global(TestAgentConnections(probes.clone()));
        probes
    }
}

/// Abort a connect whose selected account's config dir could not be
/// prepared: park the pane in `Error` and toast. Never falls back to the
/// system account — the user picked this one, and connecting as another
/// silently is the failure this gate exists to prevent.
fn fail_connect_account_prepare(
    this: &gpui::WeakEntity<Workspace>,
    pane_id: PaneId,
    detail: String,
    cx: &mut gpui::AsyncApp,
) {
    let report = ErrorReport::new(s::agent_chat::account_prepare_failed())
        .message(detail)
        .severity(ErrorSeverity::Error)
        .at(file!(), line!())
        .dedup("agent_chat.account.prepare_failed")
        .build();
    let unreported = report.clone();
    match this.update(cx, |ws, cx| {
        if let Some(view) = ws.agent_chat_view(pane_id).cloned() {
            view.update(cx, |v, cx| {
                v.set_error(
                    s::agent_chat::account_prepare_failed(),
                    daruda_acp::Remedy::Retry,
                    cx,
                )
            });
            // Connecting → Error clears the badge; dirty the cached docks so
            // it doesn't linger stale.
            ws.notify_status_docks(cx);
        }
        ws.report_error(report, cx);
        ws.fail_agent_chat_start(
            pane_id,
            daruda_store::tasks::machine::StartFailure::AccountUnavailable,
            cx,
        );
    }) {
        Ok(()) => {}
        // Window gone before the toast could land — keep the log record.
        Err(_) => daruda_store::observability::log_writer::LogWriter::log(unreported),
    }
}

impl Workspace {
    /// Lazy-connect entry point: start the ACP session for `pane_id` iff still
    /// parked in [`AgentSessionStatus::Idle`] with a cwd. Called from
    /// `focus_pane` so the session attaches on first focus and never twice.
    pub(in crate::workspace) fn maybe_connect_agent_chat(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        if self.agent_chat_view(pane_id).is_some_and(|view| {
            let v = view.read(cx);
            matches!(v.status(), AgentSessionStatus::Idle) && v.is_read_only()
        }) {
            self.refresh_cli_chat(pane_id, cx);
            return;
        }
        let (cwd, resume) = {
            let Some(view) = self.agent_chat_view(pane_id) else {
                return;
            };
            let v = view.read(cx);
            if !matches!(v.status(), AgentSessionStatus::Idle) {
                return;
            }
            // Both `Local` and `Remote` panes are connectable; a cwd-less pane
            // was parked in `Error` at construction and never reaches `Idle`.
            let Some(cwd) = v.cwd().cloned() else {
                return;
            };
            // A persisted session id resumes via `session/load`; `None` is fresh.
            (cwd, v.session_id().map(str::to_owned))
        };
        // Flip to `Connecting` before spawning so a second focus during the
        // handshake doesn't start a duplicate session. `begin_connect` takes the
        // same `resume` the spawn below gets, so the replay gate it opens can
        // never name a different session than the one actually loaded.
        if let Some(view) = self.agent_chat_view(pane_id).cloned() {
            let resume = resume.clone();
            view.update(cx, |v, cx| v.begin_connect(resume, cx));
            // Idle → Connecting is a dock-badge status change; dirty the cached
            // docks explicitly (see `notify_status_docks`).
            self.notify_status_docks(cx);
        }
        self.connect_agent_chat(pane_id, cwd, resume, cx);
    }

    /// Manual retry for the Error banner: connect `pane_id` iff parked in
    /// [`AgentSessionStatus::Error`]. Like [`Self::maybe_connect_agent_chat`]
    /// but keeps `session_id` so it resumes via `session/load`.
    pub(super) fn retry_agent_chat_connect(&mut self, pane_id: PaneId, cx: &mut Context<Self>) {
        if self
            .agent_chat_view(pane_id)
            .is_some_and(|view| view.read(cx).is_read_only())
        {
            self.refresh_cli_chat(pane_id, cx);
            return;
        }
        let (cwd, resume) = {
            let Some(view) = self.agent_chat_view(pane_id) else {
                return;
            };
            let v = view.read(cx);
            if !matches!(v.status(), AgentSessionStatus::Error { .. }) {
                return;
            }
            let Some(cwd) = v.cwd().cloned() else {
                return;
            };
            (cwd, v.session_id().map(str::to_owned))
        };
        if let Some(view) = self.agent_chat_view(pane_id).cloned() {
            view.update(cx, |v, cx| v.retry_for_reconnect(resume.clone(), cx));
            // Error → Connecting is a dock-badge status change; dirty the cached
            // docks explicitly (see `notify_status_docks`).
            self.notify_status_docks(cx);
        }
        self.connect_agent_chat(pane_id, cwd, resume, cx);
    }

    /// Whether a resume of `pane_id` must load its exact session: a failed
    /// `session/load` then surfaces as an error instead of a fresh session.
    pub(in crate::workspace) fn requires_exact_resume(
        &self,
        pane_id: PaneId,
        cx: &gpui::App,
    ) -> bool {
        self.agent_chat_view(pane_id)
            .is_some_and(|view| view.read(cx).is_read_only())
            || self.is_task_chat_restore(pane_id, cx)
    }

    /// Full local reset for `/clear`: wipe the conversation, tear down the ACP
    /// session, clear the persisted session id, start a fresh `session/new`.
    /// No-op when `pane_id` is gone or has no lane cwd.
    pub(in crate::workspace) fn reset_agent_chat_session(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return;
        };
        if view.read(cx).is_read_only() {
            return;
        }
        // Same gate as `maybe_connect_agent_chat`: both `Local` and `Remote`
        // reconnect through `connect_agent_chat` here; only a genuinely
        // cwd-less pane (never had a session) no-ops.
        let Some(cwd) = view.read(cx).cwd().cloned() else {
            return; // no cwd → never had a session
        };
        crate::runs::RunsHost::release_execution(
            self.run_pane(pane_id),
            daruda_store::tasks::machine::UndeliveredReason::SessionTornDown,
            cx,
        );
        view.update(cx, |v, cx| v.reset_for_new_session(cx));
        self.mutate_durable(cx, |_, _| {});
        self.notify_status_docks(cx);
        self.connect_agent_chat(pane_id, cwd, None, cx);
    }
}

/// What a connect asks of the adapter. A pane that must show one exact
/// session — a task's conversation or a CLI snapshot — gets `Required`, which
/// never falls back to `session/new` on the wire.
fn session_resume(resume: Option<String>, exact: bool) -> daruda_acp::SessionResume {
    match resume.map(daruda_acp::SessionId::new) {
        Some(id) if exact => daruda_acp::SessionResume::Required(id),
        other => other.into(),
    }
}

/// Whether an error on the event stream retries as a fresh session: only a
/// best-effort resume that failed before `Connected`, and never a closed
/// transport, which says nothing about the saved session.
fn retries_fresh(
    was_resume: bool,
    exact: bool,
    connected_seen: bool,
    event: &daruda_acp::AcpEvent,
) -> bool {
    was_resume
        && !exact
        && !connected_seen
        && matches!(event, daruda_acp::AcpEvent::Error(failure)
            if !matches!(failure, daruda_acp::AcpFailure::TransportClosed { .. }))
}

#[cfg(test)]
mod tests;
