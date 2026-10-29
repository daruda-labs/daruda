//! Resolve and apply the launch inputs for one ACP connection plan.

use super::super::agent_chat_ops::agent_name_for;
use super::AgentChatConnectionPlan;
use crate::agent::launch_resolve::{
    AgentLaunchSpec, ConnectCommandError, account_recipe_for_connect, resolve_launch,
};
use crate::surface::strings as s;
use crate::workspace::{Workspace, main_area::pane_tree::PaneId};
use daruda_acp::NodeProgress;
use daruda_chat::transcript_defaults::TranscriptDefaults;
use daruda_chat::view::RuntimePrepPhase;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::project::{LaneSessionHost, PaneCwd};
use gpui::Context;

/// The banner phase for a runtime-provisioning milestone, or `None` for
/// milestones that shouldn't surface a banner (system node found, or a cache
/// probe — both instant, so the plain "Connecting…" banner already fits).
pub(super) fn runtime_prep_phase(progress: NodeProgress) -> Option<RuntimePrepPhase> {
    match progress {
        NodeProgress::UsingSystemNode | NodeProgress::CheckingCache => None,
        NodeProgress::Downloading => Some(RuntimePrepPhase::Downloading),
        NodeProgress::Verifying => Some(RuntimePrepPhase::Verifying),
        NodeProgress::Extracting => Some(RuntimePrepPhase::Extracting),
    }
}

impl Workspace {
    /// Resolve the launch spec for `pane_id`'s agent, reconciling the view when
    /// its `agent_id` is stale (a live config reload removed/renamed it) by
    /// rewriting to the session-sticky default. `None` only when the pane is gone.
    fn resolve_pane_launch(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) -> Option<AgentLaunchSpec> {
        let agent_id = self
            .agent_chat_view(pane_id)?
            .read(cx)
            .agent_id()
            .to_owned();
        // Happy path: the view's agent is still in the catalog.
        if let Some(spec) = self.agent_launch_for(&agent_id) {
            return Some(spec);
        }
        // Stale id — reconcile so the chip / persisted state stop lying, then
        // launch the effective agent (a catalog entry, or the Claude id when the
        // catalog is somehow empty).
        let effective_id = self.open_agent_id(None);
        let defaults = TranscriptDefaults::resolve(
            self.mirrors.agents.iter().find(|a| a.id == effective_id),
            self.mirrors.agent_reader_defaults,
        );
        if let Some(view) = self.agent_chat_view(pane_id).cloned() {
            let id = effective_id.clone();
            let name = agent_name_for(&self.mirrors.agents, &effective_id);
            view.update(cx, |v, cx| {
                v.switch_agent(
                    super::super::agent_chat_ops::agent_identity(id, name),
                    &defaults,
                    cx,
                )
            });
            self.update_agent_chat_agent_id(pane_id, effective_id.clone());
            self.mutate_durable(cx, |_, _| {});
        }
        Some(self.agent_launch_for(&effective_id).unwrap_or_else(|| {
            AgentLaunchSpec::of(&daruda_config::AgentDefinition::claude_default())
        }))
    }

    #[cfg(test)]
    pub(in crate::workspace) fn resolve_pane_launch_for_test(
        &mut self,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) -> Option<AgentLaunchSpec> {
        self.resolve_pane_launch(pane_id, cx)
    }

    pub(super) fn prepare_agent_chat_connection(
        &mut self,
        pane_id: PaneId,
        cwd: &PaneCwd,
        cx: &mut Context<Self>,
    ) -> Option<AgentChatConnectionPlan> {
        // Resolve the pane's agent_id -> launch spec, reconciling a stale
        // agent_id. `None` only when the pane is gone.
        let AgentLaunchSpec {
            launch,
            env: agent_env,
        } = self.resolve_pane_launch(pane_id, cx)?;
        // Read back after `resolve_pane_launch` so any stale-id reconcile it did
        // is picked up: the preferences below resolve against that agent, and
        // the id keys the dev-build wire-tap file name.
        let view = self.agent_chat_view(pane_id)?.read(cx);
        let agent_id = view.agent_id().to_owned();
        // Requested on every connect, a resume (`session/load`) included: a
        // value the adapter reports for itself never outranks these.
        let daruda_config::SessionPreferences {
            model: initial_model,
            modes: initial_modes,
        } = view.session_preferences(&self.mirrors.agents);
        let vocabulary_source = crate::lane::session_host::adapter_command(&launch)
            .trim()
            .to_string();
        if let Some(view) = self.agent_chat_view(pane_id).cloned() {
            view.update(cx, |view, _| {
                view.set_agent_vocabulary_source(vocabulary_source)
            });
        }

        // Resolve the pane's owning lane so remote-ness is decided by the
        // lane's session host, not by whatever host (if any) `launch` itself
        // names.
        let owning_lane_ref = self.lane_ref_for_pane(pane_id);
        let owning_lane = owning_lane_ref.and_then(|lane_ref| self.lane_for(lane_ref));
        let resolved_host = match owning_lane.map(|lane| {
            lane.effective_session_host(
                &launch,
                &self.mirrors.session_hosts,
                &self.mirrors.session_host_tombstones,
            )
        }) {
            None => None,
            Some(Ok(host)) => Some(host),
            // The same surface a launch that cannot produce a command gets
            // below: the pane says why, and nothing connects.
            Some(Err(unusable)) => {
                if let Some(view) = self.agent_chat_view(pane_id).cloned() {
                    let message = s::agent_chat::session_host_unusable(
                        daruda_settings::labels::host_error(unusable.reason),
                    );
                    view.update(cx, |v, cx| {
                        v.set_error(message, daruda_acp::Remedy::Configure, cx);
                    });
                    self.notify_status_docks(cx);
                }
                return None;
            }
        };
        let cached_host = owning_lane.and_then(|lane| lane.session_host.clone());
        let is_remote = resolved_host
            .as_ref()
            .is_some_and(LaneSessionHost::is_remote);
        if is_remote
            && self
                .main_area
                .pane(pane_id)
                .and_then(|pane| pane.agent_chat_content())
                .is_some_and(|chat| chat.restore.requires_exact())
        {
            if let Some(view) = self.agent_chat_view(pane_id).cloned() {
                view.update(cx, |view, cx| {
                    view.set_error(
                        s::flow::session_identity_missing(),
                        daruda_acp::Remedy::Configure,
                        cx,
                    )
                });
            }
            self.notify_status_docks(cx);
            return None;
        }
        let selection = self.agent_chat_account_selection(pane_id);
        let domain = crate::workspace::main_area::pane::AccountDomain::for_agent(
            account_recipe_for_connect(&launch, is_remote),
        );
        let prepared = self.resolve_account(selection, domain);
        if selection.account_id().is_some() && prepared.is_none() {
            // Not an error: the pane falls back to the system account. Log
            // only so a surprised user has ground truth for why.
            let report = ErrorReport::new(
                "Agent chat: pane account not usable for this agent; using system",
            )
            .severity(ErrorSeverity::Info)
            .with_context("agent_id", agent_id.clone())
            .at(file!(), line!())
            .dedup("agent_chat.account.unusable")
            .build();
            daruda_store::observability::log_writer::LogWriter::log(report);
        }
        let resolved = resolve_launch(
            &launch,
            &agent_env,
            owning_lane,
            cwd,
            prepared.as_ref(),
            cached_host.as_ref(),
            resolved_host.as_ref(),
            &self.mirrors.session_hosts,
            &self.mirrors.session_host_tombstones,
        );
        if let Some(lane_ref) = owning_lane_ref
            && let Some(corrected) = resolved.host_write_back
        {
            self.set_lane_session_host(lane_ref, corrected, cx);
        }
        let cwd_changed = self
            .agent_chat_view(pane_id)
            .is_some_and(|view| view.read(cx).cwd() != Some(&resolved.resolved_cwd));
        if cwd_changed {
            if let Some(view) = self.agent_chat_view(pane_id).cloned() {
                view.update(cx, |v, _| v.set_cwd(resolved.resolved_cwd.clone()));
            }
            self.update_agent_chat_cwd(pane_id, resolved.resolved_cwd.clone());
            self.mutate_durable(cx, |_, _| {});
        }

        let connect_cwd = resolved.wire_cwd;
        let launch_spec = match resolved.spec {
            Ok(spec) => spec,
            Err(err) => {
                let message = match err {
                    ConnectCommandError::NoRemotePath => s::agent_chat::no_remote_cwd(),
                    ConnectCommandError::JsonStdioRemote => {
                        s::agent_chat::json_stdio_remote_unsupported()
                    }
                    ConnectCommandError::JsonStdioEnv => {
                        s::agent_chat::json_stdio_env_unsupported()
                    }
                };
                if let Some(view) = self.agent_chat_view(pane_id).cloned() {
                    view.update(cx, |v, cx| {
                        v.set_error(message, daruda_acp::Remedy::Configure, cx);
                    });
                    // Connecting -> Error clears the badge; dirty the cached
                    // docks so it doesn't linger stale.
                    self.notify_status_docks(cx);
                }
                return None;
            }
        };

        self.capture_library_launch(
            pane_id,
            &launch,
            &agent_env,
            prepared.as_ref(),
            &resolved.resolved_cwd,
        );

        let mcp_servers = if self.offers_control_tools(pane_id, cx) {
            crate::orchestrator::session_tools(cx).into_iter().collect()
        } else {
            Vec::new()
        };
        Some(AgentChatConnectionPlan {
            agent_id,
            launch_spec,
            connect_cwd,
            initial_model,
            initial_modes,
            prepared,
            mcp_servers,
        })
    }

    /// Whether this pane's session may reach the control MCP server.
    ///
    /// Exactly one pane may: the orchestrator's. Decided here, at the
    /// single place a session is planned, rather than by whoever opens a pane
    /// — a new pane path must not be able to hand out tools by omission.
    fn offers_control_tools(&self, pane_id: PaneId, cx: &Context<Self>) -> bool {
        let Some((_, weak)) = crate::window_registry::WindowRegistry::orchestrator(cx) else {
            return false;
        };
        // Identity by entity id, never by reading the registry's handle back:
        // this runs inside `prepare_agent_chat_connection`, which is already
        // inside `Workspace::update`, and the orchestrator seeding its own
        // pane *is* that workspace — so `weak.upgrade().read(cx)` would be a
        // second lease on the entity we are in (AGENTS.md pitfall 5).
        weak.entity_id() == cx.entity_id()
            && self.orchestrator_chat_pane().map(|p| p.pane) == Some(pane_id)
    }

    /// The servers a session planned for `pane_id` would be offered, for the
    /// test that pins the threat model: exactly one pane in the process may be
    /// offered the server.
    #[cfg(test)]
    pub(crate) fn mcp_servers_for_pane_for_test(
        &self,
        pane_id: PaneId,
        cx: &Context<Self>,
    ) -> Vec<daruda_acp::McpServer> {
        if !self.offers_control_tools(pane_id, cx) {
            return Vec::new();
        }
        crate::orchestrator::mcp_server(cx).into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::runtime_prep_phase;

    #[test]
    fn instant_runtime_phases_do_not_replace_the_connecting_banner() {
        assert!(runtime_prep_phase(daruda_acp::NodeProgress::UsingSystemNode).is_none());
        assert!(runtime_prep_phase(daruda_acp::NodeProgress::CheckingCache).is_none());
    }
}
