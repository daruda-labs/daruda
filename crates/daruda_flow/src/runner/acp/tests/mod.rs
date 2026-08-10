//! The runner driven against a scripted stdio adapter — a shell script
//! that speaks line-delimited JSON-RPC. Every turn shape the engine has
//! to tell apart is a different reply from that one script.

use super::*;
use crate::model::PermissionPolicy;
use crate::runner::CancelToken;
use daruda_acp::{CostView, UsageView};
use std::path::Path;
use std::time::{Duration, Instant};

/// The catalog id every fixture uses.
const AGENT: &str = "claude";

/// The session id the fake adapter answers `session/new` with.
const SESSION: &str = "s1";

/// A turn that never settles is a suite that never finishes, so every run
/// here is bounded. Far above any timeout a test sets, so it only ever
/// fires when the runner itself failed to return.
const HARNESS_GUARD: Duration = Duration::from_secs(10);
const NEVER_RETURNED: &str = "the runner never returned on its own";

/// A line-delimited JSON-RPC adapter, which is the whole of what ACP
/// needs over stdio — so a shell script can stand in for a real agent.
/// `pre_prompt` emits extra traffic before the turn's reply; `reply` is
/// the `"result"`/`"error"` member of that reply.
///
/// The id is echoed back as a string because the SDK sends uuids.
fn adapter_script(pre_prompt: &str, reply: &str) -> String {
    format!(
        r#"while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  case "$line" in
*'"method":"initialize"'*)
  printf '{{"jsonrpc":"2.0","id":"%s","result":{{"protocolVersion":1,"agentCapabilities":{{}}}}}}\n' "$id" ;;
*'"method":"session/new"'*)
  printf '{{"jsonrpc":"2.0","id":"%s","result":{{"sessionId":"{SESSION}"}}}}\n' "$id" ;;
*'"method":"session/prompt"'*)
  {pre_prompt}
  printf '{{"jsonrpc":"2.0","id":"%s",{reply}}}\n' "$id" ;;
  esac
done
"#
    )
}

/// A turn reply carrying `wire` as its protocol stop reason (snake_case,
/// as the schema serializes it).
fn stops_with(wire: &str) -> String {
    format!(r#""result":{{"stopReason":"{wire}"}}"#)
}

/// The handshake, in two arms: a settings fixture answers `session/new`
/// itself, because that response is where an adapter says what it offers.
const INITIALIZE: &str = r#"    *'"method":"initialize"'*)
  printf '{"jsonrpc":"2.0","id":"%s","result":{"protocolVersion":1,"agentCapabilities":{}}}\n' "$id" ;;"#;
const NEW_SESSION: &str = r#"*'"method":"session/new"'*)
  printf '{"jsonrpc":"2.0","id":"%s","result":{"sessionId":"s1"}}\n' "$id" ;;"#;

/// An adapter that parks the turn: it remembers the prompt's id and
/// answers nothing until told to. `on_cancel` is what it does when
/// `session/cancel` arrives — empty for one that ignores the cancel.
/// Either way it records that the cancel reached the wire.
fn parking_adapter(cancel_seen: &Path, on_cancel: &str) -> String {
    let seen = cancel_seen.display();
    format!(
        r#"while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  case "$line" in
{INITIALIZE}
{NEW_SESSION}
*'"method":"session/prompt"'*)
  prompt_id="$id" ;;
*'"method":"session/cancel"'*)
  : > "{seen}"
  {on_cancel} ;;
  esac
done
"#
    )
}

/// What an adapter that honours a cancel replies with: the protocol's own
/// cancelled stop reason, on the parked prompt's id.
const ANSWERS_THE_CANCEL: &str =
    r#"printf '{"jsonrpc":"2.0","id":"%s","result":{"stopReason":"cancelled"}}\n' "$prompt_id""#;

/// An adapter that asks for permission before finishing its turn. It
/// records the client's answer verbatim, which is the only place the
/// chosen option is observable.
fn permission_adapter(options: &str, answer_file: &Path) -> String {
    let answer = answer_file.display();
    format!(
        r#"while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  case "$line" in
{INITIALIZE}
{NEW_SESSION}
*'"method":"session/prompt"'*)
  prompt_id="$id"
  printf '{{"jsonrpc":"2.0","id":"perm-1","method":"session/request_permission","params":{{"sessionId":"{SESSION}","toolCall":{{"toolCallId":"t1","title":"write a file"}},"options":[{options}]}}}}\n' ;;
*'"id":"perm-1"'*)
  printf '%s\n' "$line" > "{answer}"
  printf '{{"jsonrpc":"2.0","id":"%s","result":{{"stopReason":"end_turn"}}}}\n' "$prompt_id" ;;
  esac
done
"#
    )
}

const ALLOW_ONCE_OPTION: &str = r#"{"optionId":"once","name":"Allow once","kind":"allow_once"}"#;
const ALLOW_ALWAYS_OPTION: &str = r#"{"optionId":"always","name":"Always","kind":"allow_always"}"#;
const REJECT_ONCE_OPTION: &str = r#"{"optionId":"no","name":"Reject","kind":"reject_once"}"#;

/// The same option as a value, for asking `decide` directly.
fn option(id: &'static str, kind: PermissionOptionKind) -> PermissionOption {
    PermissionOption::new(id, id, kind)
}

/// The owned half of a `RunContext`, plus the adapter the runner will
/// launch. The script lives in the temp dir so it dies with the test.
struct Fixture {
    _dir: tempfile::TempDir,
    node: crate::NodeId,
    cwd: PathBuf,
    run_dir: PathBuf,
    log_dir: PathBuf,
    output: PathBuf,
    cancel: CancelToken,
    command: String,
    timeout: Duration,
    permission: PermissionPolicy,
    /// Where a `permission: ask` fixture puts its question. Held by the
    /// fixture because `Permission::Ask` borrows it, so it has to outlive
    /// the `RunContext` built from it.
    ask: Option<crate::runner::AskChannel>,
    grace: Duration,
    settings_budget: Duration,
}

impl Fixture {
    /// A fixture whose adapter runs `script`.
    fn with_script(script: &str) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("adapter.sh");
        std::fs::write(&path, script).expect("write the adapter");
        Self::with_command(dir, format!("/bin/sh {}", path.display()))
    }

    /// A fixture whose adapter is `command` verbatim — the only way to
    /// pose a launch that cannot connect at all.
    fn with_command(dir: tempfile::TempDir, command: String) -> Self {
        let run_dir = dir.path().join("run");
        Self {
            node: "design".to_string(),
            cwd: dir.path().to_path_buf(),
            log_dir: run_dir.join("logs"),
            output: run_dir.join("design.md"),
            run_dir,
            cancel: CancelToken::default(),
            command,
            timeout: Duration::from_secs(60),
            permission: PermissionPolicy::Deny,
            ask: None,
            grace: CANCEL_GRACE,
            settings_budget: SETTINGS_BUDGET,
            _dir: dir,
        }
    }

    fn context(&self) -> RunContext<'_> {
        RunContext {
            node_id: &self.node,
            attempt: 1,
            cwd: &self.cwd,
            run_dir: &self.run_dir,
            log_dir: &self.log_dir,
            output: Some(&self.output),
            evidence_seq: 1,
            timeout: self.timeout,
            // The same promotion `Run::permission_for` makes: a policy
            // becomes a capability, and `Ask` without a port is not one.
            permission: match (self.permission, self.ask.as_ref()) {
                (PermissionPolicy::Deny, _) => crate::runner::Permission::Deny,
                (PermissionPolicy::AllowOnce, _) => crate::runner::Permission::AllowOnce,
                (PermissionPolicy::Ask, Some(channel)) => crate::runner::Permission::Ask(channel),
                (PermissionPolicy::Ask, None) => crate::runner::Permission::Deny,
            },
            cancel: &self.cancel,
        }
    }

    /// The runner this fixture's adapter is registered in.
    fn runner(&self) -> AcpRunner {
        let launch = LaunchSpec {
            command: self.command.clone(),
            strip_env: Vec::new(),
        };
        AcpRunner::new(
            HashMap::from([(AGENT.to_string(), launch)]),
            self.cwd.clone(),
        )
        .with_grace(self.grace)
        .with_settings_budget(self.settings_budget)
    }

    /// One run, bounded: a runner that never returns fails this as a
    /// stated result instead of hanging the suite.
    /// Run a turn with somebody on the other end of the questions.
    ///
    /// `answer` is the person: `Some` is what they said, `None` is them
    /// walking away, which drops the reply channel — the case a host that
    /// closed its window produces. Everything runs on one thread; the
    /// answering loop is just a third arm of the race.
    /// `thinks_for` is how long the person takes, awaited rather than
    /// slept: everything here shares one thread, and a blocking sleep
    /// would freeze the very timer the wait is supposed to be racing —
    /// a test that then passes proves nothing about the clock.
    fn run_answered(
        &mut self,
        agent: &AgentSpec,
        thinks_for: Duration,
        answer: impl Fn(&crate::runner::PendingAsk) -> Option<PermissionDecision>,
    ) -> (RunResult, Vec<(u64, crate::runner::AskRequest)>) {
        let (tx, rx) = smol::channel::unbounded();
        self.permission = PermissionPolicy::Ask;
        self.ask = Some(crate::runner::AskChannel::new(tx));

        let asked = RefCell::new(Vec::new());
        let runner = self.runner();
        let ctx = self.context();
        let result = smol::block_on(smol::future::or(
            runner.run_agent(&ctx, agent, "write it"),
            smol::future::or(
                async {
                    crate::runner::sleep(HARNESS_GUARD).await;
                    failed(NEVER_RETURNED.to_string())
                },
                async {
                    while let Ok(pending) = rx.recv().await {
                        asked
                            .borrow_mut()
                            .push((pending.ask_id, pending.request.clone()));
                        crate::runner::sleep(thinks_for).await;
                        match answer(&pending) {
                            Some(decision) => {
                                let _ = pending.reply.send(decision).await;
                            }
                            // Dropping `pending` drops the only sender.
                            None => drop(pending),
                        }
                    }
                    // The stream is only closed once the run has let go of
                    // it, so there is nothing left for this arm to do but
                    // lose the race.
                    crate::runner::sleep(HARNESS_GUARD).await;
                    failed(NEVER_RETURNED.to_string())
                },
            ),
        ));
        (result, asked.into_inner())
    }

    fn run(&self, agent: &AgentSpec) -> RunResult {
        let runner = self.runner();
        let ctx = self.context();
        smol::block_on(smol::future::or(
            runner.run_agent(&ctx, agent, "write it"),
            async {
                crate::runner::sleep(HARNESS_GUARD).await;
                failed(NEVER_RETURNED.to_string())
            },
        ))
    }
}

/// The plainest agent spec: named, with nothing pinned and nothing
/// allowed. Every group builds on it.
fn spec(id: &str) -> AgentSpec {
    AgentSpec {
        id: id.to_string(),
        model: None,
        effort: None,
        mode: None,
        permission: PermissionPolicy::Deny,
    }
}

/// A scratch directory the fake adapter records into. Separate from the
/// fixture's own, whose path is only known after the script is built.
fn probe(name: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(name);
    (dir, path)
}

mod control;
mod settings;
mod turns;
