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
    /// The run directory cannot be picked up — it ended on purpose, it is
    /// still going, or what it left behind is not enough to continue from.
    Resume(daruda_flow::resume::ResumeError),
}

/// A request and the stream that narrates it. The receiver is handed back
/// separately because the sender lives inside the request, and the two are
/// created together exactly once.
pub(in crate::workspace) struct FlowSubmission {
    pub request: RunRequest,
    pub node_install_dir: PathBuf,
    pub events: smol::channel::Receiver<FlowEvent>,
    /// Where the run's permission questions arrive. A second pump rather
    /// than a `FlowEvent` variant: wiring this is what declares the app can
    /// answer, and the engine's validation reads exactly that field.
    pub asks: smol::channel::Receiver<daruda_flow::runner::PendingAsk>,
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

/// Every agent id this flow can actually launch: the ones its nodes name,
/// plus the default the repair's fix session runs as — which may be no
/// node's agent at all.
///
/// Used only to decide what the *lane* has to support. The credential
/// strip list below stays wider on purpose: a command node should not
/// inherit any configured account's credentials, whether or not this
/// particular flow launches that agent.
pub(in crate::workspace) fn referenced_agents(
    loaded: &daruda_flow::LoadedFlow,
) -> std::collections::HashSet<String> {
    let flow = loaded.flow();
    flow.nodes
        .iter()
        .filter_map(|node| match &node.kind {
            daruda_flow::model::NodeKind::Agent { agent, .. } => Some(agent.id.clone()),
            daruda_flow::model::NodeKind::Command { .. } => None,
        })
        .chain(flow.default_agent.iter().map(|a| a.id.clone()))
        .collect()
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

/// When a run started, read back out of its id.
///
/// Deliberately here rather than beside the reader: the id's layout is the
/// host's, and `daruda_flow` says so — its retention sweep notes that "a
/// host using a different id scheme would silently have this delete the
/// wrong ones". One place builds the format and reads it, so the two
/// cannot drift.
///
/// `None` for any name this host did not make (a hand-created directory,
/// or a run from a future scheme), which the caller shows without a time
/// rather than guessing one from the directory's mtime.
pub(in crate::workspace) fn run_started_at(run_id: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let (millis, rest) = run_id.split_once('-')?;
    // Both remaining fields must be there: a bare 16-hex name is not this
    // scheme, and accepting it would date arbitrary directories.
    if millis.len() != 16 || !rest.contains('-') {
        return None;
    }
    chrono::DateTime::from_timestamp_millis(i64::from_str_radix(millis, 16).ok()?)
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
        profile: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Result<FlowSubmission, FlowSubmitError> {
        let submission = self.assemble_flow_request(flow_path, profile, FlowPurpose::Run, cx)?;
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
        profile: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Result<Vec<daruda_flow::error::ValidationIssue>, FlowSubmitError> {
        let submission =
            self.assemble_flow_request(flow_path, profile, FlowPurpose::Validate, cx)?;
        Ok(daruda_flow::request::validate_request(&submission.request))
    }

    /// Assemble a request that **continues** the run in `run_dir`, rather
    /// than starting a new one.
    ///
    /// The predicate that decides whether the earlier process is gone is
    /// made **once, here**, and handed to both the question and the lock
    /// that question implies. Two of them can disagree, and then the panel
    /// offers a resume that the lock immediately refuses as held.
    pub(in crate::workspace) fn build_resume_request(
        &mut self,
        run_dir: &Path,
        cx: &mut Context<Self>,
    ) -> Result<FlowSubmission, FlowSubmitError> {
        let Some(cwd) = self.active_lane_root() else {
            return Err(FlowSubmitError::NoLane);
        };
        let is_alive: fn(u32) -> bool = process_is_alive;
        let resumed =
            daruda_flow::resume::prepare(run_dir, &is_alive).map_err(FlowSubmitError::Resume)?;

        let agents = self.flow_agent_catalog(
            &cwd,
            FlowPurpose::Run,
            &referenced_agents(&resumed.loaded),
            cx,
        )?;
        let node_install_dir = daruda_store::persistence::node_install_dir();
        let (tx, rx) = smol::channel::unbounded();
        let (ask_tx, ask_rx) = smol::channel::unbounded();

        let request = RunRequest {
            loaded: resumed.loaded,
            run_dir: run_dir.to_path_buf(),
            // The run's own directory: `run.yaml` inlined every file-backed
            // prompt and hint when it was written, so a resumed run resolves
            // nothing against the flow file's directory — which may not even
            // hold that flow any more.
            flow_dir: run_dir.to_path_buf(),
            cwd: cwd.clone(),
            agents,
            node_install_dir: node_install_dir.clone(),
            // A fresh ceiling, measured from now. What the earlier half
            // spent is carried by the journal, not by this.
            budget: budget_from(&self.config_flow()),
            is_alive: Box::new(is_alive),
            git_status: Some(Box::new(move || git_status(&cwd))),
            events: Some(tx),
            ask: Some(ask_tx),
            resume: Some(resumed.replay),
        };
        Ok(FlowSubmission {
            request,
            node_install_dir,
            events: rx,
            asks: ask_rx,
        })
    }

    fn assemble_flow_request(
        &mut self,
        flow_path: &Path,
        profile: Option<&str>,
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
        let loaded = daruda_flow::load(&text, profile).map_err(FlowSubmitError::Load)?;

        let agents = self.flow_agent_catalog(&cwd, purpose, &referenced_agents(&loaded), cx)?;
        let node_install_dir = daruda_store::persistence::node_install_dir();
        let (tx, rx) = smol::channel::unbounded();
        let (ask_tx, ask_rx) = smol::channel::unbounded();

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
            ask: Some(ask_tx),
            // This assembles a run that is starting. Picking one up reads
            // the directory it left instead, and builds its own request.
            resume: None,
        };
        Ok(FlowSubmission {
            request,
            node_install_dir,
            events: rx,
            asks: ask_rx,
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
        referenced: &std::collections::HashSet<String>,
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
            // Only for an agent this flow actually names. A catalog entry
            // that happens to be remote says nothing about a command-only
            // flow, or one that runs entirely on a local agent — refusing
            // those would be refusing on a fact the flow does not depend on.
            if referenced.contains(&id)
                && resolved_host
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

    /// What the lane has to support is what the flow can launch, and a
    /// command-only flow launches nothing. Keyed off the whole configured
    /// catalog instead, a single remote agent in the user's settings
    /// refused every flow in that lane — on a fact none of them depend on.
    #[test]
    fn only_the_agents_a_flow_can_launch_are_its_own() {
        let command_only = daruda_flow::load(
            "version: 1\nnodes:\n  - id: g\n    kind: command\n    run: \"true\"\n",
            None,
        )
        .expect("loads");
        assert!(referenced_agents(&command_only).is_empty());

        // The repair's fix runs as `defaults.agent`, which may be no
        // node's agent — so the default counts even when nothing names it.
        let repairing = daruda_flow::load(
            "\
version: 1
defaults:
  agent: { id: claude, mode: bypassPermissions }
nodes:
  - id: g
    kind: command
    run: \"true\"
    on_fail:
      repair:
        fix: fix it from {{attempts}}
        max_attempts: 2
",
            None,
        )
        .expect("loads");
        assert_eq!(
            referenced_agents(&repairing),
            std::collections::HashSet::from(["claude".to_string()])
        );
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

    /// The panel dates a past run by decoding its directory name, so the
    /// two halves of that format have to stay one thing. Change the layout
    /// in `run_id` alone and every past run loses its time.
    #[test]
    fn a_run_id_carries_back_the_time_it_was_made() {
        let millis = 1_786_000_000_000u128;
        let decoded = run_started_at(&run_id(millis, 42, 1)).expect("decodes");
        assert_eq!(decoded.timestamp_millis(), millis as i64);
    }

    /// Anything this host did not name gets no time rather than a wrong
    /// one — a user's own directory under `flow-runs/` must not be dated
    /// from whatever its name happens to parse as.
    #[test]
    fn a_name_that_is_not_a_run_id_decodes_to_nothing() {
        for name in [
            "scratch",
            "",
            // Right shape, not hex.
            "zzzzzzzzzzzzzzzz-0000002a-0001",
            // Hex, but not the full fixed width.
            "1786-0000002a-0001",
            // The clock alone is not the scheme.
            "0000019fbe1f8a00",
        ] {
            assert!(run_started_at(name).is_none(), "{name} was dated");
        }
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
