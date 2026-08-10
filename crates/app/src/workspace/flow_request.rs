//! Assembling a [`RunRequest`] — every question `daruda_flow` refuses to
//! answer for itself.
//!
//! The engine is handed finished values: absolute paths, a resolved agent
//! catalog, a budget with no `None` it did not choose. `examples/run_flow.rs`
//! is the same shape written for a terminal; this is it in the app's
//! vocabulary.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use daruda_acp::LaunchSpec;
use daruda_flow::event::FlowEvent;
use daruda_flow::request::{Budget, CostLimit, RunRequest};
use daruda_store::project::PaneCwd;
use gpui::Context;

use super::Workspace;
use super::command::flow_picker::FlowPurpose;
use crate::agent::launch_resolve::{account_recipe_for_connect, resolve_launch};
use crate::workspace::main_area::pane::{AccountDomain, resolve_pane_account};

/// Why a flow could not be submitted. These are refusals, not failures —
/// nothing has been taken or written when one is returned.
#[derive(Debug)]
pub(in crate::workspace) enum FlowSubmitError {
    /// No active lane, so no working directory. The welcome window.
    NoLane,
    /// The lane runs its agents on another machine. See the module note
    /// below on why this is refused rather than resolved.
    RemoteLane {
        agent: String,
    },
    Read {
        path: PathBuf,
        message: String,
    },
    Load(daruda_flow::FlowError),
    /// A live process — not this one — already holds the lane.
    LockHeld {
        pid: u32,
    },
    /// The rules that need the request's own context — a node naming an
    /// agent the catalog lacks, a `prompt_file` that is not there.
    Invalid(Vec<daruda_flow::error::ValidationIssue>),
}

/// A request and the stream that narrates it. The receiver is handed back
/// separately because the sender lives inside the request, and the two are
/// created together exactly once.
pub(in crate::workspace) struct FlowSubmission {
    pub request: RunRequest,
    pub node_install_dir: PathBuf,
    pub events: smol::channel::Receiver<FlowEvent>,
}

/// Every env var to unset before a command node runs: the union of what
/// each agent's account strips.
///
/// Design §9 lets a command node inherit the environment but not the ACP
/// account credentials, and computing that list is the host's job — the
/// runner only unsets what it is given. Passing an empty vec here (which
/// is what the terminal example does, because its `LaunchSpec` strips
/// nothing) would leak an account's credentials into every shell line a
/// committed flow file names.
pub(in crate::workspace) fn union_strip_env(agents: &HashMap<String, LaunchSpec>) -> Vec<String> {
    let mut names: Vec<String> = agents
        .values()
        .flat_map(|spec| spec.strip_env.iter().cloned())
        .collect();
    names.sort();
    names.dedup();
    names
}

/// A run id that sorts in the order runs were made.
///
/// Retention sweeps run directories by name and only that ordering makes
/// the sweep chronological, so this property is load-bearing rather than
/// cosmetic. Millisecond, pid and counter are each fixed-width hex: the
/// clock orders across runs, the pid separates two apps that started in
/// the same millisecond, and the counter separates two runs from one app.
pub(in crate::workspace) fn run_id(millis: u128, pid: u32, counter: u32) -> String {
    format!("{millis:016x}-{pid:08x}-{counter:04x}")
}

impl Workspace {
    /// Assemble everything one run needs and refuse anything the static
    /// rules reject. `execute` checks the same rules again, and should:
    /// the engine cannot trust a host to have done it. But it reports by
    /// *returning*, and a run that never started emits no event — so a
    /// host that only watched the stream would show the user nothing.
    pub(in crate::workspace) fn build_flow_request(
        &mut self,
        flow_path: &Path,
        cx: &mut Context<Self>,
    ) -> Result<FlowSubmission, FlowSubmitError> {
        let submission = self.assemble_flow_request(flow_path, FlowPurpose::Run, cx)?;
        let issues = daruda_flow::request::validate_request(&submission.request);
        if issues.is_empty() {
            Ok(submission)
        } else {
            Err(FlowSubmitError::Invalid(issues))
        }
    }

    /// Every static problem this flow has, found without running it.
    ///
    /// `load` alone is not the whole of stage 1: a missing `prompt_file`,
    /// a prompt file reaching for a non-ancestor's output, and an agent id
    /// the catalog lacks are only knowable with the request's own context
    /// (`validate_request`). Checking less here than `Run` checks would
    /// mean "no problems found" followed by a refusal — the exact split
    /// design §12 exists to prevent.
    ///
    /// Takes no lock and creates no run directory: assembling a request is
    /// pure path arithmetic, and `FlowPurpose::Validate` keeps it that way
    /// by skipping the account-directory preparation a real run does.
    pub(in crate::workspace) fn check_flow(
        &mut self,
        flow_path: &Path,
        cx: &mut Context<Self>,
    ) -> Result<Vec<daruda_flow::error::ValidationIssue>, FlowSubmitError> {
        let submission = self.assemble_flow_request(flow_path, FlowPurpose::Validate, cx)?;
        Ok(daruda_flow::request::validate_request(&submission.request))
    }

    fn assemble_flow_request(
        &mut self,
        flow_path: &Path,
        purpose: FlowPurpose,
        cx: &mut Context<Self>,
    ) -> Result<FlowSubmission, FlowSubmitError> {
        let Some(cwd) = self.active_lane_root() else {
            return Err(FlowSubmitError::NoLane);
        };
        let text = std::fs::read_to_string(flow_path).map_err(|e| FlowSubmitError::Read {
            path: flow_path.to_path_buf(),
            message: e.to_string(),
        })?;
        let loaded = daruda_flow::load(&text).map_err(FlowSubmitError::Load)?;

        let agents = self.flow_agent_catalog(&cwd, purpose, cx)?;
        let node_install_dir = daruda_store::persistence::node_install_dir();
        let (tx, rx) = smol::channel::unbounded();

        let request = RunRequest {
            loaded,
            run_dir: super::flow_paths::runs_dir(&cwd).join(self.next_run_id()),
            flow_dir: flow_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| cwd.clone()),
            cwd: cwd.clone(),
            agents,
            node_install_dir: node_install_dir.clone(),
            budget: budget_from(&self.config_flow()),
            is_alive: Box::new(process_is_alive),
            git_status: Some(Box::new(move || git_status(&cwd))),
            events: Some(tx),
        };
        Ok(FlowSubmission {
            request,
            node_install_dir,
            events: rx,
        })
    }

    /// Every configured agent, resolved for this lane and this run's
    /// account. The whole catalog rather than only what the flow names, so
    /// a node naming an id that is not configured is reported by
    /// `validate_request` as `UnknownAgent` instead of silently resolving
    /// to something else.
    fn flow_agent_catalog(
        &mut self,
        cwd: &Path,
        purpose: FlowPurpose,
        cx: &mut Context<Self>,
    ) -> Result<HashMap<String, LaunchSpec>, FlowSubmitError> {
        let pane_cwd = PaneCwd::Local(cwd.to_path_buf());
        let definitions: Vec<(String, daruda_config::AgentLaunch)> = self
            .agents
            .iter()
            .map(|a| (a.id.clone(), a.launch.clone()))
            .collect();

        let mut catalog = HashMap::new();
        for (id, launch) in definitions {
            let lane = self.active_lane();
            let resolved_host = lane.map(|lane| {
                lane.effective_session_host(
                    &launch,
                    &self.session_hosts,
                    &self.session_host_tombstones,
                )
            });
            // A flow's `cwd` means two things at once — the directory
            // `ProcessRunner` runs a gate in, and the one the ACP session
            // opens on. On a remote lane those are different machines, and
            // the engine's contract has only one field for both. Refusing
            // is honest; running the gates locally and the agents remotely
            // would be a split the user has no way to see.
            if resolved_host
                .as_ref()
                .is_some_and(daruda_store::project::LaneSessionHost::is_remote)
            {
                return Err(FlowSubmitError::RemoteLane { agent: id });
            }
            let recipe = account_recipe_for_connect(&launch, false);
            let selection = self.default_account_selection_for_new_pane(recipe);
            let prepared = resolve_pane_account(
                &self.accounts,
                &self.data_dir,
                selection,
                AccountDomain::for_agent(recipe),
            );
            // The same step a freshly created pane takes before its process
            // starts: Codex symlinks its `CODEX_HOME` in, Claude mirrors the
            // shared MCP servers. Skipping it would make only the first run
            // under a new account behave differently, which is the worst
            // kind of difference — but it writes to disk, and checking a
            // flow is supposed to cost nothing and change nothing.
            if let (FlowPurpose::Run, Some(prepared)) = (purpose, prepared.as_ref()) {
                self.prepare_account_dir(prepared, cx);
            }
            let lane = self.active_lane();
            let cached_host = lane.and_then(|lane| lane.session_host.clone());
            let resolved = resolve_launch(
                &launch,
                lane,
                &pane_cwd,
                prepared.as_ref(),
                cached_host.as_ref(),
                resolved_host.as_ref(),
                &self.session_hosts,
                &self.session_host_tombstones,
            );
            // A launch shape that cannot produce a command is left out of
            // the catalog: a flow that names it then fails validation with
            // `UnknownAgent`, and one that does not is unaffected.
            if let Ok(spec) = resolved.spec {
                catalog.insert(id, spec);
            }
        }
        Ok(catalog)
    }

    fn config_flow(&self) -> daruda_config::flow::FlowConfig {
        self.flow_config.clone()
    }

    fn next_run_id(&mut self) -> String {
        self.flow_run_counter = self.flow_run_counter.wrapping_add(1);
        run_id(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or_default(),
            std::process::id(),
            self.flow_run_counter,
        )
    }
}

/// The deadline is an `Instant` because the engine only ever compares; the
/// host is the one that knows when the run started.
fn budget_from(config: &daruda_config::flow::FlowConfig) -> Budget {
    Budget {
        deadline: config.timeout().map(|d| {
            std::time::Instant::now()
                .checked_add(d)
                .unwrap_or_else(std::time::Instant::now)
        }),
        max_node_runs: config.max_node_runs(),
        max_cost: config
            .cost_limit()
            .map(|(amount, currency)| CostLimit { amount, currency }),
    }
}

/// The engine never asks the OS whether a pid is alive; the host answers.
/// The picker asks the same question of the lock holder.
pub(in crate::workspace) fn process_is_alive(pid: u32) -> bool {
    let mut system = sysinfo::System::new();
    let pid = sysinfo::Pid::from_u32(pid);
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&[pid]),
        true,
        sysinfo::ProcessRefreshKind::new(),
    );
    system.process(pid).is_some()
}

fn git_status(cwd: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(cwd)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(strip: &[&str]) -> LaunchSpec {
        LaunchSpec {
            command: "adapter".to_string(),
            strip_env: strip.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    /// The security property design §9 names: a command node inherits the
    /// environment minus the account credentials. An empty list here is
    /// what a committed flow file would need to read them — and a name two
    /// agents share is one name, not two.
    #[test]
    fn every_accounts_credentials_are_stripped_from_a_command_node_exactly_once() {
        let agents = HashMap::from([
            ("claude".to_string(), spec(&["ANTHROPIC_API_KEY"])),
            ("personal".to_string(), spec(&["ANTHROPIC_API_KEY"])),
            ("codex".to_string(), spec(&["OPENAI_API_KEY"])),
        ]);
        assert_eq!(
            union_strip_env(&agents),
            vec!["ANTHROPIC_API_KEY", "OPENAI_API_KEY"]
        );
    }

    /// Retention deletes the oldest runs by sorting directory names, so a
    /// later run must sort after an earlier one — including across a clock
    /// that did not move between them.
    #[test]
    fn run_ids_sort_in_the_order_they_were_made() {
        let ids = [
            run_id(1_700_000_000_000, 42, 1),
            run_id(1_700_000_000_000, 42, 2),
            run_id(1_700_000_000_001, 42, 1),
            run_id(1_700_000_001_000, 7, 1),
        ];
        let mut sorted = ids.to_vec();
        sorted.sort();
        assert_eq!(sorted, ids);
    }

    /// Two apps starting a run in the same millisecond must not land on the
    /// same directory — the counter is per-process and would collide.
    #[test]
    fn two_processes_in_one_millisecond_get_different_ids() {
        assert_ne!(
            run_id(1_700_000_000_000, 42, 1),
            run_id(1_700_000_000_000, 43, 1)
        );
    }

    /// A user who configured nothing still runs under both enforceable
    /// ceilings — `Budget::unlimited()` reaching the engine by accident is
    /// the failure this guards.
    #[test]
    fn an_unconfigured_budget_is_not_an_unlimited_one() {
        let budget = budget_from(&daruda_config::flow::FlowConfig::default());
        assert!(budget.deadline.is_some());
        assert!(budget.max_node_runs.is_some());
        assert!(budget.max_cost.is_none(), "a cost limit needs a currency");
    }
}
