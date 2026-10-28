//! Working-directory containment must still hold after earlier nodes run.

use super::*;

struct CwdPlanter {
    inner: FakeRunner,
    target: PathBuf,
}

impl NodeRunner for CwdPlanter {
    fn run_agent<'a>(
        &'a self,
        ctx: &'a RunContext<'a>,
        agent: &'a crate::model::AgentSpec,
        prompt: &'a str,
    ) -> Pin<Box<dyn Future<Output = RunResult> + 'a>> {
        self.inner.run_agent(ctx, agent, prompt)
    }

    fn run_command<'a>(
        &'a self,
        ctx: &'a RunContext<'a>,
        run: &'a str,
    ) -> Pin<Box<dyn Future<Output = RunResult> + 'a>> {
        if ctx.node_id.as_str() == "setup" {
            let directory = ctx.cwd.join("working");
            std::fs::remove_dir(&directory).expect("remove original directory");
            daruda_core::path::symlink(&self.target, &directory).expect("replace with link");
        }
        self.inner.run_command(ctx, run)
    }
}

#[test]
fn a_cwd_redirected_outside_after_submission_never_reaches_either_runner() {
    for body in [
        "kind: command\n    run: 'true'",
        "kind: agent\n    prompt: write\n    output: out.md",
    ] {
        let dir = tempfile::tempdir().expect("worktree");
        let outside = tempfile::tempdir().expect("outside");
        std::fs::create_dir(dir.path().join("working")).expect("original directory");
        let text = format!(
            "version: 1\ndefaults: {{ agent: {{ id: claude }} }}\nnodes:\n  - id: setup\n    kind: command\n    run: 'true'\n  - id: next\n    deps: [setup]\n    cwd: working\n    {body}\n"
        );
        let runner = CwdPlanter {
            inner: FakeRunner::new(),
            target: outside.path().to_path_buf(),
        };
        let request = request_for(&text, dir.path());
        assert!(crate::request::validate_request(&request).is_empty());
        let report = execute(&request, &runner, &CancelToken::default());
        assert!(
            matches!(report.outcome, RunOutcome::Io(_)),
            "{:?}",
            report.outcome
        );
        assert_eq!(runner.inner.calls().len(), 1, "the redirected node ran");
        assert!(!request.run_dir.join("out.md").exists());
    }
}

#[test]
fn a_cwd_redirected_inside_the_worktree_can_still_run() {
    let dir = tempfile::tempdir().expect("worktree");
    std::fs::create_dir(dir.path().join("working")).expect("original directory");
    std::fs::create_dir(dir.path().join("actual 한글 ' $ `")).expect("target directory");
    let runner = CwdPlanter {
        inner: FakeRunner::new(),
        target: dir.path().join("actual 한글 ' $ `"),
    };
    let request = request_for(
        "version: 1\nnodes:\n  - id: setup\n    kind: command\n    run: 'true'\n  - id: next\n    deps: [setup]\n    cwd: working\n    kind: command\n    run: 'true'\n",
        dir.path(),
    );
    let report = execute(&request, &runner, &CancelToken::default());
    assert!(
        matches!(report.outcome, RunOutcome::Done),
        "{:?}",
        report.outcome
    );
    assert_eq!(runner.inner.calls().len(), 2);
}
