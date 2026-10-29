//! Recorded identity validation and attachment to the existing chat host.

use super::*;

impl Workspace {
    pub(super) fn resolve_flow_session_identity(
        &self,
        lane_ref: daruda_store::project::LaneRef,
        binding: &daruda_flow::session::SessionBinding,
    ) -> Result<AccountSelection, String> {
        let recorded = binding
            .account
            .as_ref()
            .ok_or_else(s::flow::session_legacy)?;
        let account = match &recorded.managed {
            None => AccountSelection::SystemDefault,
            Some(id) => AccountSelection::Managed(AccountId(
                uuid::Uuid::parse_str(id).map_err(|_| s::flow::session_identity_missing())?,
            )),
        };
        if !self.task_chat_identity_available(&binding.agent, account.to_persisted()) {
            return Err(s::flow::session_identity_missing());
        }
        let local = self
            .lane_for(lane_ref)
            .zip(self.mirrors.agents.iter().find(|a| a.id == binding.agent))
            .is_some_and(|(lane, agent)| {
                lane.effective_session_host(
                    &agent.launch,
                    &self.mirrors.session_hosts,
                    &self.mirrors.session_host_tombstones,
                )
                .is_ok_and(|host| !host.is_remote())
            });
        if !local {
            return Err(s::flow::session_identity_missing());
        }
        Ok(account)
    }

    pub(super) fn attach_flow_session(
        &mut self,
        origin: RunOrigin,
        binding: daruda_flow::session::SessionBinding,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let account = match self.resolve_flow_session_identity(origin.lane, &binding) {
            Ok(account) => account,
            Err(reason) => {
                self.flow_session_error(reason, cx);
                return;
            }
        };
        if self
            .lane_for(origin.lane)
            .is_none_or(|lane| {
                !lane.path.is_dir() || !matches!(
                    (daruda_core::path::canonicalize(&binding.cwd), daruda_core::path::canonicalize(&lane.path)),
                    (Ok(cwd), Ok(root)) if daruda_core::path::is_within(&cwd, &root)
                )
            })
            || !binding.cwd.is_dir()
        {
            self.flow_session_error(s::flow::session_worktree_missing(), cx);
            return;
        }
        let session = RestorableSession {
            session_id: binding.session_id,
            agent_id: binding.agent,
            account,
            lane_ref: origin.lane,
            title: Some(format!("{} · {}", origin.node, origin.evidence).into()),
            prompt_preview: None,
            git_branch: None,
            cwd: binding.cwd,
            last_active: std::time::SystemTime::now(),
        };
        self.close_page(cx);
        self.restore_session_with_flow_origin(session, Some(origin), window, cx);
    }
}

#[cfg(test)]
mod tests {}
