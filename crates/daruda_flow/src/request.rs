//! What the host hands the engine to start a run, and the checks that need
//! that context. The engine never reads the agent catalog or the config
//! itself — it is given a finished map and a directory.

use crate::NodeId;
use crate::error::{ValidationIssue, ValidationKind};
use crate::model::{AgentFail, NodeKind, PermissionPolicy, Prompt};
use daruda_acp::LaunchSpec;
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

/// Everything one run needs. The host resolves lanes, agents and paths
/// before this point; the engine only ever sees finished values.
pub struct RunRequest {
    /// The flow and its graph, which only mean anything as a pair.
    pub loaded: crate::load::LoadedFlow,
    /// The working directory every node shares.
    pub cwd: PathBuf,
    /// `<cwd>/.daruda/flow-runs/<run-id>/`.
    pub run_dir: PathBuf,
    /// Where the flow file lives — `prompt_file` and `hint_file` resolve
    /// against this, not against `cwd`.
    pub flow_dir: PathBuf,
    /// Catalog id to launch spec, already resolved for this run's host and
    /// account.
    pub agents: HashMap<String, LaunchSpec>,
    pub node_install_dir: PathBuf,
    /// The three runaway defences, already resolved from config.
    pub budget: Budget,
    /// Whether a pid is still running. The engine never asks the OS; the
    /// host already tracks processes and answers. Owned rather than
    /// `lock::IsAlive`, which is a borrow and would put a lifetime on every
    /// type that carries a request.
    pub is_alive: Box<dyn Fn(u32) -> bool + Send>,
    /// `git status --porcelain` for `cwd`, asked once per attempt. Owned for
    /// the same reason as `is_alive`, and `None` for a host that has nothing
    /// to say — the record then carries no note, and the run is unaffected.
    pub git_status: Option<Box<dyn Fn() -> Option<String> + Send>>,
    /// Where the run narrates itself, for a host that would otherwise poll
    /// the run directory. Unbounded and never awaited, so a subscriber that
    /// stops reading cannot slow or stop the run; `None` is a host that does
    /// not watch, which is most of them.
    pub events: Option<smol::channel::Sender<crate::event::FlowEvent>>,
    /// Where a `permission: ask` node puts its question. Only a host that
    /// built somewhere to answer wires this, so **its presence is the
    /// capability declaration** — which is why `validate_request` reads it
    /// rather than `events`.
    ///
    /// The distinction is not academic: `examples/run_flow.rs` passes an
    /// `events` channel and only prints what arrives, so keying on that
    /// would let an `ask` flow start there and park forever.
    pub ask: Option<smol::channel::Sender<crate::runner::PendingAsk>>,
}

/// A cost ceiling is only meaningful in a currency, and two currencies do
/// not add up — so the currency travels with the amount.
#[derive(Debug, Clone, PartialEq)]
pub struct CostLimit {
    pub amount: f64,
    pub currency: String,
}

/// The three runaway defences. `None` is unlimited, which the host may
/// only choose explicitly (the config default has all three on).
#[derive(Debug, Clone, Default)]
pub struct Budget {
    /// When the run must stop. The host computes it from configured
    /// duration and start time; the engine only compares.
    pub deadline: Option<std::time::Instant>,
    /// Every runner call counts, including reruns and fix sessions.
    pub max_node_runs: Option<u32>,
    /// Only enforceable while the agent reports a cost, which is why the
    /// run's report carries a warning when it never does.
    pub max_cost: Option<CostLimit>,
}

impl Budget {
    /// Every defence off. Spelled out at each call site so a test that
    /// wants one limit does not silently get the other two.
    pub fn unlimited() -> Self {
        Self::default()
    }
}

/// The two static rules that need context the flow file does not carry.
/// Collected, not short-circuited.
pub fn validate_request(request: &RunRequest) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();
    check_absolute(request, &mut issues);
    let mut checked_agents = HashSet::new();

    for node in &request.loaded.flow().nodes {
        let NodeKind::Agent {
            agent,
            prompt,
            on_fail,
            ..
        } = &node.kind
        else {
            continue;
        };

        check_agent_id(
            &agent.id,
            Some(&node.id),
            &request.agents,
            &mut checked_agents,
            &mut issues,
        );
        // A file-backed prompt hides its `{{node.x.output}}` references from
        // `crate::validate`, which never opens the file; this is the only
        // stage that knows `flow_dir`, so the ancestor rule is applied here.
        let ancestors: HashSet<NodeId> = request
            .loaded
            .graph()
            .ancestors(&node.id)
            .into_iter()
            .collect();
        check_file(
            &request.flow_dir,
            prompt,
            "prompt_file",
            &node.id,
            &ancestors,
            &mut issues,
        );
        if let AgentFail::Retry { hint, .. } = on_fail {
            check_file(
                &request.flow_dir,
                hint,
                "hint_file",
                &node.id,
                &ancestors,
                &mut issues,
            );
        }
    }

    if let Some(agent) = &request.loaded.flow().default_agent {
        check_agent_id(
            &agent.id,
            None,
            &request.agents,
            &mut checked_agents,
            &mut issues,
        );
    }

    check_someone_can_answer(request, &mut issues);
    issues
}

/// Refuse a run that could put a question to a person this host cannot
/// reach. Without it the run parks and only a Stop ends it.
///
/// The default agent counts even when no node names `ask`: a repair's
/// `fix` session runs as `flow.default_agent` and inherits its policy, so
/// a flow of nothing but `deny` nodes can still ask.
fn check_someone_can_answer(request: &RunRequest, issues: &mut Vec<ValidationIssue>) {
    if request.ask.is_some() {
        return;
    }
    let flow = request.loaded.flow();
    let node_asks = flow.nodes.iter().find_map(|node| match &node.kind {
        NodeKind::Agent { agent, .. } if agent.permission == PermissionPolicy::Ask => {
            Some(node.id.clone())
        }
        _ => None,
    });
    let repair_asks = flow
        .default_agent
        .as_ref()
        .is_some_and(|agent| agent.permission == PermissionPolicy::Ask);
    if node_asks.is_none() && !repair_asks {
        return;
    }
    issues.push(ValidationIssue {
        node: node_asks,
        kind: ValidationKind::NobodyToAsk,
        message: "this run asks a person for permission but was given no way to ask".to_string(),
    });
}

fn check_agent_id(
    id: &str,
    node: Option<&NodeId>,
    agents: &HashMap<String, LaunchSpec>,
    checked: &mut HashSet<String>,
    issues: &mut Vec<ValidationIssue>,
) {
    if !checked.insert(id.to_string()) || agents.contains_key(id) {
        return;
    }
    issues.push(ValidationIssue {
        node: node.cloned(),
        kind: ValidationKind::UnknownAgent { id: id.to_string() },
        message: format!("`{id}` is not in this run's agent catalog"),
    });
}

fn check_file(
    flow_dir: &Path,
    prompt: &Prompt,
    field: &'static str,
    node: &NodeId,
    ancestors: &HashSet<NodeId>,
    issues: &mut Vec<ValidationIssue>,
) {
    let Prompt::File(relative) = prompt else {
        return;
    };
    if relative.is_absolute()
        || relative.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        issues.push(ValidationIssue {
            node: Some(node.clone()),
            kind: ValidationKind::PromptFileOutsideFlowDir {
                field,
                path: relative.clone(),
            },
            message: format!(
                "`{field}` must stay under the flow directory: `{}`",
                relative.display()
            ),
        });
        return;
    }
    let path = flow_dir.join(relative);
    if !path.is_file() {
        issues.push(ValidationIssue {
            node: Some(node.clone()),
            kind: ValidationKind::MissingPromptFile {
                field,
                path: path.clone(),
            },
            message: format!(
                "`{field}` points at `{}`, which does not exist",
                path.display()
            ),
        });
        return;
    }

    let root = flow_dir
        .canonicalize()
        .unwrap_or_else(|_| flow_dir.to_path_buf());
    let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
    if !canonical.starts_with(&root) {
        issues.push(ValidationIssue {
            node: Some(node.clone()),
            kind: ValidationKind::PromptFileOutsideFlowDir {
                field,
                path: path.clone(),
            },
            message: format!(
                "`{field}` resolves outside `{}`: `{}`",
                root.display(),
                canonical.display()
            ),
        });
        return;
    }

    match std::fs::read_to_string(&path) {
        Ok(text) => crate::validate::check_output_refs(&text, node, ancestors, issues),
        // Unreadable is the same fact as missing to a caller — a distinct
        // kind would only add a branch nothing renders differently.
        Err(e) => issues.push(ValidationIssue {
            node: Some(node.clone()),
            kind: ValidationKind::MissingPromptFile {
                field,
                path: path.clone(),
            },
            message: format!("`{field}` at `{}` cannot be read: {e}", path.display()),
        }),
    }
}

/// Every path here was resolved by the host. A relative one is resolved a
/// second time — against whatever directory the process happens to be in —
/// so the run would take its lock, write its record and build its outputs
/// somewhere that is neither the lane nor the run, and only notice when an
/// adapter refused the `cwd` much later.
fn check_absolute(request: &RunRequest, issues: &mut Vec<ValidationIssue>) {
    for (field, path) in [
        ("cwd", &request.cwd),
        ("run_dir", &request.run_dir),
        ("flow_dir", &request.flow_dir),
        ("node_install_dir", &request.node_install_dir),
    ] {
        if !path.is_absolute() {
            issues.push(ValidationIssue {
                node: None,
                kind: ValidationKind::RelativeRequestPath { field },
                message: format!("`{field}` must be absolute: `{}`", path.display()),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `execute` blocks, so a host runs it on a thread of its own and the
    /// request has to cross. The two callbacks are the only thing that could
    /// stop it, which is why they are declared `+ Send`.
    #[test]
    fn a_request_and_its_report_can_cross_a_thread() {
        fn assert_send<T: Send>() {}
        assert_send::<RunRequest>();
        assert_send::<crate::schedule::RunReport>();
    }

    use crate::error::ValidationKind;
    use crate::load::load;

    fn spec(command: &str) -> daruda_acp::LaunchSpec {
        daruda_acp::LaunchSpec {
            command: command.to_string(),
            strip_env: Vec::new(),
        }
    }

    fn request_for(text: &str, agents: &[&str], dir: &std::path::Path) -> RunRequest {
        let loaded = load(text).expect("valid flow");
        RunRequest {
            loaded,
            cwd: dir.to_path_buf(),
            run_dir: dir.join("run"),
            flow_dir: dir.to_path_buf(),
            agents: agents.iter().map(|a| (a.to_string(), spec("x"))).collect(),
            node_install_dir: dir.to_path_buf(),
            budget: Budget::unlimited(),
            is_alive: Box::new(|_| true),
            git_status: None,
            events: None,
            ask: None,
        }
    }

    const AGENT_FLOW: &str = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt: write
";

    #[test]
    fn an_agent_id_absent_from_the_catalog_is_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let req = request_for(AGENT_FLOW, &["codex"], dir.path());
        let issues = validate_request(&req);
        assert!(
            issues.iter().any(|i| matches!(&i.kind,
                ValidationKind::UnknownAgent { id } if id == "claude")),
            "{issues:?}"
        );
    }

    #[test]
    fn a_known_agent_id_passes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let req = request_for(AGENT_FLOW, &["claude"], dir.path());
        assert_eq!(validate_request(&req), Vec::new());
    }

    /// `prompt_file` is relative to the flow file, not to the working
    /// directory — a flow kept in `.daruda/flows/` names its prompts
    /// beside itself.
    #[test]
    fn a_missing_prompt_file_is_rejected_and_a_present_one_is_not() {
        let dir = tempfile::tempdir().expect("tempdir");
        let flow = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt_file: ./prompts/a.md
";
        let req = request_for(flow, &["claude"], dir.path());
        assert!(
            validate_request(&req)
                .iter()
                .any(|i| matches!(i.kind, ValidationKind::MissingPromptFile { .. })),
            "the file does not exist yet"
        );

        std::fs::create_dir_all(dir.path().join("prompts")).expect("mkdir");
        std::fs::write(dir.path().join("prompts/a.md"), "hi").expect("write");
        assert_eq!(validate_request(&req), Vec::new());
    }

    /// File-backed prompts are relative to the flow file and must stay
    /// under that directory. An absolute path that happens to exist is
    /// still not a valid flow-local prompt.
    #[test]
    fn an_absolute_prompt_file_is_rejected_even_if_it_exists() {
        let dir = tempfile::tempdir().expect("tempdir");
        let outside_dir = tempfile::tempdir().expect("outside tempdir");
        let outside = outside_dir.path().join("outside.md");
        std::fs::write(&outside, "hi").expect("write outside");
        let flow = format!(
            "\
version: 1
defaults: {{ agent: {{ id: claude }} }}
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt_file: {}
",
            outside.display()
        );
        let req = request_for(&flow, &["claude"], dir.path());
        assert!(
            validate_request(&req)
                .iter()
                .any(|i| matches!(i.kind, ValidationKind::PromptFileOutsideFlowDir { .. })),
            "absolute paths must not be accepted just because they exist"
        );

        let parent_flow = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt_file: ../outside.md
";
        let req = request_for(parent_flow, &["claude"], dir.path());
        assert!(
            validate_request(&req)
                .iter()
                .any(|i| matches!(i.kind, ValidationKind::PromptFileOutsideFlowDir { .. })),
            "`..` must be rejected before existence is checked"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_prompt_file_symlink_escaping_the_flow_dir_is_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let outside_dir = tempfile::tempdir().expect("outside tempdir");
        let outside = outside_dir.path().join("outside.md");
        std::fs::write(&outside, "hi").expect("write outside");
        std::fs::create_dir_all(dir.path().join("prompts")).expect("mkdir");
        std::os::unix::fs::symlink(&outside, dir.path().join("prompts/link.md")).expect("symlink");

        let flow = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt_file: ./prompts/link.md
";
        let req = request_for(flow, &["claude"], dir.path());
        assert!(
            validate_request(&req)
                .iter()
                .any(|i| matches!(i.kind, ValidationKind::PromptFileOutsideFlowDir { .. })),
            "canonical containment must be checked after existence"
        );
    }

    /// A repair fix runs as `flow.default_agent`. A command-only flow can
    /// therefore need an agent even though no node has kind `agent`.
    #[test]
    fn a_repair_default_agent_absent_from_the_catalog_is_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let flow = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: gate
    kind: command
    run: \"true\"
    on_fail:
      repair:
        fix: fix it from {{attempts}}
        max_attempts: 2
";
        let req = request_for(flow, &["codex"], dir.path());
        assert!(
            validate_request(&req).iter().any(|i| matches!(&i.kind,
                ValidationKind::UnknownAgent { id } if id == "claude")),
            "the repair session's default agent must be checked"
        );
    }

    /// The inline form of this is rejected by `validate`; the file form
    /// reached the runner untouched and rendered to an empty string, so the
    /// flow silently ran with a hole in its prompt.
    #[test]
    fn an_unreachable_output_ref_inside_a_prompt_file_is_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("design.md"),
            "continue from {{node.review.output}}",
        )
        .expect("write");
        let flow = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt_file: design.md
  - id: review
    kind: agent
    deps: [design]
    output: review.md
    prompt: read {{node.design.output}}
";
        let req = request_for(flow, &["claude"], dir.path());
        assert!(
            validate_request(&req).iter().any(|i| matches!(&i.kind,
                ValidationKind::UnreachableOutputRef { referenced } if referenced == "review")),
            "`review` is downstream of `design`, so its output cannot exist yet"
        );
    }

    /// A retry's `hint_file` is the same kind of reference and must be
    /// checked the same way — it is read only on a failure path, which is
    /// exactly when a missing file is most expensive to discover.
    #[test]
    fn a_missing_hint_file_is_rejected_too() {
        let dir = tempfile::tempdir().expect("tempdir");
        let flow = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt: write
    on_fail:
      retry:
        max_attempts: 2
        hint_file: ./missing-hint.md
";
        let req = request_for(flow, &["claude"], dir.path());
        assert!(
            validate_request(&req)
                .iter()
                .any(|i| matches!(i.kind, ValidationKind::MissingPromptFile { .. }))
        );
    }

    const ASKS: &str = "\
version: 1
defaults: { agent: { id: claude } }
nodes:
  - id: a
    kind: agent
    agent: { id: claude, mode: default, permission: ask }
    output: a.md
    prompt: write
";

    /// Only the *repair* can ask: no node names `ask`, but the fix session
    /// runs as `defaults.agent` and inherits its policy. Checking nodes
    /// alone lets this flow start and park with nobody to release it.
    const REPAIR_ASKS: &str = "\
version: 1
defaults: { agent: { id: claude, mode: default, permission: ask } }
nodes:
  - id: gate
    kind: command
    run: \"true\"
    on_fail:
      repair:
        fix: fix it, see {{attempts}}
        max_attempts: 2
        wait: 0s
";

    /// **A channel is not a capability.** `examples/run_flow.rs` hands over
    /// an `events` sender and only *prints* what arrives — so keying this
    /// check on `events` would let an `ask` flow start there and park until
    /// somebody noticed. The port a host wires only to answer questions is
    /// the one thing that means it can.
    #[test]
    fn an_ask_flow_is_refused_when_only_the_narration_channel_is_wired() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut req = request_for(ASKS, &["claude"], dir.path());
        let (tx, _rx) = smol::channel::unbounded();
        req.events = Some(tx);

        let kinds: Vec<_> = validate_request(&req).into_iter().map(|i| i.kind).collect();
        assert!(
            kinds.contains(&ValidationKind::NobodyToAsk),
            "an ask flow was accepted with nowhere to ask: {kinds:?}"
        );
    }

    /// The same refusal for a flow only a repair can make ask.
    #[test]
    fn a_flow_whose_only_asker_is_its_repair_is_refused_too() {
        let dir = tempfile::tempdir().expect("tempdir");
        let req = request_for(REPAIR_ASKS, &["claude"], dir.path());
        let kinds: Vec<_> = validate_request(&req).into_iter().map(|i| i.kind).collect();
        assert!(kinds.contains(&ValidationKind::NobodyToAsk), "{kinds:?}");
    }

    /// And a host that did wire the port runs.
    #[test]
    fn an_ask_flow_with_somewhere_to_ask_is_accepted() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut req = request_for(ASKS, &["claude"], dir.path());
        let (tx, _rx) = smol::channel::unbounded();
        req.ask = Some(tx);

        let kinds: Vec<_> = validate_request(&req).into_iter().map(|i| i.kind).collect();
        assert!(!kinds.contains(&ValidationKind::NobodyToAsk), "{kinds:?}");
    }

    /// A flow nobody would ever ask about is unaffected — the check must
    /// not become a reason every host needs an answering surface.
    #[test]
    fn a_flow_that_never_asks_needs_no_answering_surface() {
        let dir = tempfile::tempdir().expect("tempdir");
        let req = request_for(AGENT_FLOW, &["claude"], dir.path());
        let kinds: Vec<_> = validate_request(&req).into_iter().map(|i| i.kind).collect();
        assert!(!kinds.contains(&ValidationKind::NobodyToAsk), "{kinds:?}");
    }
}
