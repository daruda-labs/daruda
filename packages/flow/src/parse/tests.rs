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
            output_schema,
            continue_until,
            max_turns,
            on_fail,
        } => {
            assert_eq!(*output_schema, None, "no declared shape, none owed");
            // A flow written before these fields existed reads with both
            // absent — which is what makes `DEFAULT_MAX_TURNS` the
            // behaviour that already held rather than a new one.
            assert_eq!(*continue_until, None);
            assert_eq!(*max_turns, None);
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

/// Issues by kind. `FlowError::Validate`'s `Display` deliberately
/// carries only a count — the per-issue wording is the host's — so a
/// test asserts on what a consumer matches, not on a string.
fn kinds_for(text: &str) -> Vec<ValidationKind> {
    match crate::load(text, None) {
        Err(FlowError::Validate(issues)) => issues.into_iter().map(|i| i.kind).collect(),
        Err(other) => panic!("expected validation issues, got {other}"),
        Ok(_) => panic!("expected the flow to be refused"),
    }
}

/// The field most worth mistyping. `dep:` leaves the node with no
/// dependency at all, so the DAG runs in an order the file does not
/// describe — and nothing downstream can tell, because an empty `deps`
/// is perfectly legal.
#[test]
fn a_mistyped_ordering_field_is_refused_rather_than_defaulted() {
    let kinds = kinds_for(
        "\
version: 1
nodes:
  - id: a
    kind: command
    run: \"true\"
  - id: b
    kind: command
    dep: [a]
    run: \"true\"
",
    );
    assert_eq!(
        kinds,
        vec![ValidationKind::UnknownField {
            field: "dep".to_string()
        }]
    );
}

/// Naming both halves of an either-or pair: serde picks one and drops
/// the other without trace, and the dropped one is usually the one
/// being edited. Checked at both levels it occurs.
#[test]
fn naming_both_halves_of_a_prompt_or_hint_is_refused() {
    assert_eq!(
        kinds_for(
            "\
version: 1
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt: inline
    prompt_file: ./a.md
"
        ),
        vec![ValidationKind::ConflictingField {
            field: "prompt_file".to_string(),
            wins: "prompt",
        }]
    );

    assert_eq!(
        kinds_for(
            "\
version: 1
nodes:
  - id: a
    kind: agent
    output: a.md
    prompt: w
    on_fail:
      retry:
        hint: from {{failure}}
        hint_file: ./h.md
        max_attempts: 2
"
        ),
        vec![ValidationKind::ConflictingField {
            field: "hint_file".to_string(),
            wins: "hint",
        }]
    );
}

/// Collected, not short-circuited — the same promise every other check
/// in this crate makes. Three typos should cost one round trip, not
/// three. A node with no `on_fail` must not end the scan either: the
/// first cut used `?` on that key and let every later node through.
///
/// All three are *optional* fields, which is the set this check has to
/// cover: mistype a required one and serde rejects the file first,
/// with a line and column this check cannot give.
#[test]
fn every_mistyped_field_is_reported_in_one_pass() {
    let kinds = kinds_for(
        "\
version: 1
nodes:
  - id: a
    kind: command
    run: \"true\"
  - id: b
    kind: command
    run: \"true\"
    timeoutt: 1s
    dep: [a]
    on_fail:
      repair:
        fix: fix from {{failure}}
        max_attempts: 2
        waitt: 1s
",
    );
    assert_eq!(kinds.len(), 3, "{kinds:?}");
    assert!(
        kinds
            .iter()
            .all(|k| matches!(k, ValidationKind::UnknownField { .. }))
    );
}

/// The key allowlist is hand-maintained, so a field added to the wire
/// type and forgotten here deserializes fine and is then refused as an
/// unknown one — which takes every flow using the feature with it. Both
/// halves asserted together: the key is known, and a typo of it is not.
#[test]
fn an_output_schema_is_a_known_key_and_a_typo_of_it_is_not() {
    let flow = |key: &str| {
        format!(
            "\
version: 1
nodes:
  - id: a
    kind: agent
    agent: {{ id: claude, mode: bypassPermissions }}
    output: a.json
    prompt: write
    {key}:
      type: object
      required: [verdict]
      properties:
        verdict: {{ type: string }}
"
        )
    };
    crate::load(&flow("output_schema"), None).expect("output_schema is a known key");
    assert_eq!(
        kinds_for(&flow("output_schemas")),
        vec![ValidationKind::UnknownField {
            field: "output_schemas".to_string()
        }]
    );
}

/// Every keyword this build does not enforce has to *parse*, landing in
/// `rest` for a node-named refusal: a parse error here would take the whole
/// file's graph and inspector away instead. Which is also why `enum` holds
/// `Value`s — `enum: [1, 2]` is legal JSON Schema (refused in
/// `contract::schema`, not by serde).
#[test]
fn an_unenforced_keyword_lands_in_rest_rather_than_failing_the_parse() {
    let file = parse_flow_file(
        "\
version: 1
nodes:
  - id: a
    kind: agent
    output: a.json
    prompt: write
    output_schema:
      type: object
      properties:
        n: { type: integer, additionalProperties: false, enum: [1, 2] }
",
    )
    .expect("an unenforced keyword still parses");
    let NodeKindFile::Agent {
        output_schema: Some(schema),
        ..
    } = &file.nodes[0].kind
    else {
        panic!("expected an agent node with a schema");
    };
    assert_eq!(
        schema.properties["n"].rest.get("additionalProperties"),
        Some(&serde_json::Value::Bool(false)),
        "the keyword has to survive at the level it was written"
    );
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
    let err = crate::load(
        "\
version: 1
nodes:
  - id: a
    kind: agent
    agent: { moed: bypassPermissions }
    output: a.md
    prompt: write
",
        None,
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
    let err = crate::load(
        "\
version: 1
nodes:
  - id: gate
    kind: command
    run: grep -q '^VERDICT: PASS' out.md
",
        None,
    )
    .expect_err("a plain scalar cannot hold \": \"");
    assert!(matches!(err, FlowError::Parse(_)));
}
