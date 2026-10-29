//! Deterministic execution fixtures through the real scheduler, without provider processes.

use daruda_flow::{
    model::AgentSpec,
    request::{Budget, RunRequest},
    runner::{CancelToken, NodeFailure, NodeRunner, RunContext, RunResult},
};
use std::path::{Path, PathBuf};
use std::{collections::HashMap, future::Future, pin::Pin};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExecutionShot {
    Completed,
    Report,
    Output,
    Metadata,
    Retry,
    Active,
    Pending,
    Docked,
    Legacy,
    Unavailable,
}

pub(in crate::workspace) fn seed(root: &Path, shot: ExecutionShot) -> PathBuf {
    let dir = root.join("runs/000001a1817fab35-0000e459-0001");
    std::fs::create_dir_all(&dir).unwrap();
    if shot == ExecutionShot::Legacy {
        std::fs::write(dir.join(daruda_flow::record::RUN_REPORT_FILE), "# Run — release\n\nThis older run retains its human report. No session binding was recorded.\n").unwrap();
        return dir;
    }
    let text = "version: 1\nname: Release preparation\ndefaults:\n  agent:\n    id: claude\nnodes:\n  - id: prepare\n    kind: agent\n    output: prepare.md\n    prompt: Prepare the release notes.\n    on_fail:\n      retry:\n        max_attempts: 2\n        hint: Correct the reported failure {{failure}}.\n  - id: review\n    kind: agent\n    deps: [prepare]\n    output: review.md\n    prompt: Review the release notes.\n";
    let request = RunRequest {
        history: daruda_flow::session::HistoryContext {
            source: Some(".daruda/flows/release.yaml".into()),
            accounts: HashMap::new(),
        },
        started_by: Some(daruda_flow::request::StartedBy::Desktop),
        loaded: daruda_flow::load(text, None).unwrap(),
        cwd: root.to_path_buf(),
        run_dir: dir.clone(),
        lock_dir: root.join("locks"),
        flow_dir: root.to_path_buf(),
        agents: HashMap::from([(
            "claude".into(),
            daruda_acp::LaunchSpec {
                command: "unused-fixture-launcher".into(),
                strip_env: Vec::new(),
            },
        )]),
        node_install_dir: root.join("node"),
        budget: Budget::unlimited(),
        is_alive: Box::new(|_| true),
        git_status: None,
        events: None,
        ask: None,
        until: None,
        pinned: Vec::new(),
        resume: None,
    };
    let report =
        daruda_flow::schedule::execute(&request, &FixtureRunner(shot), &CancelToken::default());
    assert!(
        matches!(
            report.outcome,
            daruda_flow::schedule::RunOutcome::Done | daruda_flow::schedule::RunOutcome::ShutDown
        ),
        "{report:?}"
    );
    dir
}

struct FixtureRunner(ExecutionShot);
impl NodeRunner for FixtureRunner {
    fn run_agent<'a>(
        &'a self,
        ctx: &'a RunContext<'a>,
        agent: &'a AgentSpec,
        _prompt: &'a str,
    ) -> Pin<Box<dyn Future<Output = RunResult> + 'a>> {
        Box::pin(async move {
            let account = daruda_flow::session::SessionAccount {
                managed: (self.0 == ExecutionShot::Unavailable)
                    .then(|| uuid::Uuid::nil().to_string()),
            };
            let outcome = match &ctx.session {
                Some(port) => {
                    port.record(
                        &agent.id,
                        Some(account),
                        ctx.cwd,
                        format!("fixture-session-{}", ctx.evidence_seq),
                    )
                    .await
                }
                None => Ok(()),
            };
            let artifact = ctx.log_dir.join(format!(
                "{}.attempt-{}.evidence-{}.transcript.md",
                ctx.node_id, ctx.attempt, ctx.evidence_seq
            ));
            std::fs::create_dir_all(ctx.log_dir).unwrap();
            std::fs::write(&artifact, "# Saved transcript\n\n## Human\nPrepare the release notes.\n\n## Agent\nI checked the release inputs and wrote the requested output.\n\n`Read release inputs`\n\n`Write release notes`\n\nThe output is ready for review.\n").unwrap();
            if let Some(output) = ctx.output {
                std::fs::write(
                    output,
                    if self.0 == ExecutionShot::Output {
                        "## Verified change\n\nThe release inputs were checked and the requested output is ready for review.\n\n".repeat(40)
                    } else {
                        "# Release notes\n\nThe requested output is ready.\n".into()
                    },
                )
                .unwrap();
            }
            if matches!(self.0, ExecutionShot::Active | ExecutionShot::Pending) {
                ctx.cancel.shut_down();
            }
            let outcome = outcome.and_then(|()| {
                if self.0 == ExecutionShot::Retry
                    && ctx.node_id.as_str() == "prepare"
                    && ctx.attempt == 1
                {
                    Err(NodeFailure::SessionError(
                        "fixture validation failed".into(),
                    ))
                } else {
                    Ok(())
                }
            });
            RunResult {
                outcome,
                artifacts: vec![artifact],
                usage: None,
                waiting: Default::default(),
                turns: 1,
                tools: Vec::new(),
                stopped_the_run: false,
            }
        })
    }
    fn run_command<'a>(
        &'a self,
        ctx: &'a RunContext<'a>,
        _run: &'a str,
    ) -> Pin<Box<dyn Future<Output = RunResult> + 'a>> {
        self.run_agent(ctx, fixture_agent(), "")
    }
}

fn fixture_agent() -> &'static AgentSpec {
    static AGENT: std::sync::OnceLock<AgentSpec> = std::sync::OnceLock::new();
    AGENT.get_or_init(|| AgentSpec {
        id: "claude".into(),
        model: None,
        effort: None,
        mode: None,
        permission: daruda_flow::model::PermissionPolicy::Deny,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fixtures_are_readable_through_the_production_reader() {
        for shot in [
            ExecutionShot::Completed,
            ExecutionShot::Report,
            ExecutionShot::Output,
            ExecutionShot::Metadata,
            ExecutionShot::Retry,
            ExecutionShot::Active,
            ExecutionShot::Pending,
            ExecutionShot::Docked,
            ExecutionShot::Legacy,
            ExecutionShot::Unavailable,
        ] {
            let root = tempfile::tempdir().unwrap();
            let dir = seed(root.path(), shot);
            let history = daruda_flow::history::RunHistory::read(&dir).unwrap();
            assert_eq!(
                history.can_continue(),
                matches!(
                    shot,
                    ExecutionShot::Completed
                        | ExecutionShot::Report
                        | ExecutionShot::Docked
                        | ExecutionShot::Output
                        | ExecutionShot::Metadata
                        | ExecutionShot::Retry
                        | ExecutionShot::Unavailable
                )
            );
        }
    }
}
