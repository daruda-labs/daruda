//! The YAML a flow file literally contains. Every override is an `Option`
//! so a node can name one axis without restating the rest; merging them
//! against `defaults` is `crate::resolve`'s job, not this module's.
//!
//! `Serialize` belongs here rather than on the resolved model: `kind:`,
//! `prompt:` and `on_fail:` are spelled by these serde attributes alone, so
//! `run.yaml` is written by mapping back to this shape.

use crate::NodeId;
use crate::error::FlowError;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;

/// One flow file, exactly as written.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FlowFile {
    /// Schema version. Required so a future change to the execution rules
    /// can coexist with files written against today's.
    pub version: u32,
    #[serde(default)]
    pub defaults: Defaults,
    pub nodes: Vec<NodeFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    #[serde(
        default,
        with = "humantime_serde",
        skip_serializing_if = "Option::is_none"
    )]
    pub timeout: Option<Duration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentOverride>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct NodeFile {
    pub id: NodeId,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deps: Vec<NodeId>,
    #[serde(
        default,
        with = "humantime_serde",
        skip_serializing_if = "Option::is_none"
    )]
    pub timeout: Option<Duration>,
    /// `kind` and the fields it selects sit at the same level as `id` in
    /// the file, so the tag is flattened into this struct rather than
    /// nested under a key of its own.
    #[serde(flatten)]
    pub kind: NodeKindFile,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NodeKindFile {
    Agent {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent: Option<AgentOverride>,
        #[serde(flatten)]
        prompt: PromptSource,
        output: PathBuf,
        #[serde(default, with = "yaml_serde::with::singleton_map")]
        on_fail: AgentFailFile,
    },
    Command {
        run: String,
        #[serde(default, with = "yaml_serde::with::singleton_map")]
        on_fail: GateFailFile,
    },
}

/// Every field optional: a node names only the axes it overrides, and
/// `crate::resolve` fills the rest from `defaults`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission: Option<PermissionPolicyFile>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionPolicyFile {
    Deny,
    AllowOnce,
}

/// A node's prompt: inline prose or a sibling file, under keys `prompt:` /
/// `prompt_file:`. If a node names both, `prompt` silently wins and the
/// flattened shape leaves no trace of the other — not enforceable here.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptSource {
    Prompt(String),
    PromptFile(PathBuf),
}

/// A retry's hint. A separate enum from [`PromptSource`] on purpose:
/// flattening erases the field name, so reusing `PromptSource` here would
/// make the retry block's key `prompt:` instead of `hint:`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HintSource {
    Hint(String),
    HintFile(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentFailFile {
    #[default]
    Halt,
    Retry {
        #[serde(flatten)]
        hint: HintSource,
        max_attempts: u32,
        #[serde(
            default,
            with = "humantime_serde",
            skip_serializing_if = "Option::is_none"
        )]
        wait: Option<Duration>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GateFailFile {
    #[default]
    Halt,
    Repair {
        fix: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        rerun: Vec<NodeId>,
        max_attempts: u32,
        #[serde(
            default,
            with = "humantime_serde",
            skip_serializing_if = "Option::is_none"
        )]
        wait: Option<Duration>,
    },
}

/// Parse one flow file's text. Shape errors only — cross-node rules are
/// `crate::validate`'s job.
pub fn parse_flow_file(text: &str) -> Result<FlowFile, FlowError> {
    yaml_serde::from_str(text).map_err(|e| FlowError::Parse(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = "\
version: 1
defaults:
  timeout: 10m
nodes:
  - id: test
    kind: command
    run: cargo test
";

    #[test]
    fn parses_a_minimal_command_only_flow() {
        let file = parse_flow_file(MINIMAL).expect("minimal flow parses");
        assert_eq!(file.version, 1);
        assert_eq!(
            file.defaults.timeout,
            Some(std::time::Duration::from_secs(600))
        );
        assert_eq!(file.nodes.len(), 1);
        assert_eq!(file.nodes[0].id, "test");
        assert!(file.nodes[0].deps.is_empty());
        match &file.nodes[0].kind {
            NodeKindFile::Command { run, on_fail } => {
                assert_eq!(run, "cargo test");
                assert!(matches!(on_fail, GateFailFile::Halt));
            }
            other => panic!("expected a command node, got {other:?}"),
        }
    }

    const FULL: &str = "\
version: 1
defaults:
  timeout: 10m
  agent:
    id: claude
    mode: bypassPermissions
    permission: deny
nodes:
  - id: design
    kind: agent
    agent: { effort: high }
    output: design.md
    prompt: |
      Read DESIGN.md and write the design to {{output}}.
    on_fail:
      retry:
        max_attempts: 2
        hint: |
          The previous attempt failed: {{failure}}
  - id: implement
    kind: agent
    deps: [design]
    prompt_file: ./prompts/implement.md
    output: implement.md
  - id: test
    kind: command
    deps: [implement]
    run: cargo test
    on_fail:
      repair:
        fix: \"{{failure}}. Read {{attempts}} and fix the cause.\"
        max_attempts: 2
";

    #[test]
    fn parses_defaults_partial_override_both_prompt_sources_and_both_fail_shapes() {
        let file = parse_flow_file(FULL).expect("flow parses");

        let defaults_agent = file
            .defaults
            .agent
            .as_ref()
            .expect("defaults name an agent");
        assert_eq!(defaults_agent.id.as_deref(), Some("claude"));
        assert_eq!(defaults_agent.permission, Some(PermissionPolicyFile::Deny));

        match &file.nodes[0].kind {
            NodeKindFile::Agent {
                agent,
                prompt,
                output,
                on_fail,
            } => {
                let agent = agent.as_ref().expect("node overrides the agent axis");
                assert_eq!(agent.effort.as_deref(), Some("high"));
                assert_eq!(
                    agent.id, None,
                    "an unnamed axis stays None for resolve to fill"
                );
                assert!(matches!(prompt, PromptSource::Prompt(t) if t.contains("{{output}}")));
                assert_eq!(output, &PathBuf::from("design.md"));
                // The retry's hint key is `hint:`, not `prompt:` — that is
                // what `HintSource` exists for.
                match on_fail {
                    AgentFailFile::Retry {
                        hint,
                        max_attempts,
                        wait,
                    } => {
                        assert!(matches!(hint, HintSource::Hint(t) if t.contains("{{failure}}")));
                        assert_eq!(*max_attempts, 2);
                        assert_eq!(*wait, None);
                    }
                    AgentFailFile::Halt => panic!("expected retry"),
                }
            }
            other => panic!("expected an agent node, got {other:?}"),
        }

        match &file.nodes[1].kind {
            NodeKindFile::Agent { prompt, .. } => assert_eq!(
                prompt,
                &PromptSource::PromptFile(PathBuf::from("./prompts/implement.md"))
            ),
            other => panic!("expected an agent node, got {other:?}"),
        }

        match &file.nodes[2].kind {
            NodeKindFile::Command { on_fail, .. } => match on_fail {
                GateFailFile::Repair {
                    fix,
                    rerun,
                    max_attempts,
                    wait,
                } => {
                    assert!(fix.contains("{{attempts}}"));
                    assert!(rerun.is_empty());
                    assert_eq!(*max_attempts, 2);
                    assert_eq!(*wait, None);
                }
                GateFailFile::Halt => panic!("expected repair"),
            },
            other => panic!("expected a command node, got {other:?}"),
        }
    }

    #[test]
    fn a_missing_version_is_a_parse_error_not_a_default() {
        let err = parse_flow_file("nodes: []").expect_err("version is required");
        assert!(
            matches!(err, FlowError::Parse(msg) if msg.contains("version")),
            "the error should name the missing field"
        );
    }

    /// A plain YAML scalar may not contain `": "`, which a `grep` for
    /// `VERDICT: PASS` does. Quoting is the flow author's job, and the
    /// parser must say so clearly rather than silently mis-read the node.
    /// A typo in an override key must not silently disarm it — `moed`
    /// parsing as `Ok` with `mode: None` would be indistinguishable from an
    /// intentional omission.
    #[test]
    fn an_unknown_field_in_an_agent_override_is_a_parse_error() {
        let err = parse_flow_file(
            "\
version: 1
nodes:
  - id: a
    kind: agent
    agent: { moed: bypassPermissions }
    output: a.md
    prompt: write
",
        )
        .expect_err("moed is not a known AgentOverride field");
        assert!(matches!(err, FlowError::Parse(_)));
    }

    /// `on_fail: halt` is a bare string, unlike `retry`/`repair`'s mapping
    /// shape — deliberately asymmetric, so it needs its own coverage.
    #[test]
    fn on_fail_halt_parses_on_both_node_kinds() {
        let file = parse_flow_file(
            "\
version: 1
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt: write
    on_fail: halt
  - id: b
    kind: command
    run: \"true\"
    on_fail: halt
",
        )
        .expect("halt parses on both kinds");

        match &file.nodes[0].kind {
            NodeKindFile::Agent { on_fail, .. } => {
                assert!(matches!(on_fail, AgentFailFile::Halt));
            }
            other => panic!("expected an agent node, got {other:?}"),
        }
        match &file.nodes[1].kind {
            NodeKindFile::Command { on_fail, .. } => {
                assert!(matches!(on_fail, GateFailFile::Halt));
            }
            other => panic!("expected a command node, got {other:?}"),
        }
    }

    #[test]
    fn an_unquoted_colon_in_a_command_is_a_parse_error() {
        let err = parse_flow_file(
            "\
version: 1
nodes:
  - id: gate
    kind: command
    run: grep -q '^VERDICT: PASS' out.md
",
        )
        .expect_err("a plain scalar cannot hold \": \"");
        assert!(matches!(err, FlowError::Parse(_)));
    }
}
