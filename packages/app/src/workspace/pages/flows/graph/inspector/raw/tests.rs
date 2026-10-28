use super::*;

#[test]
fn inactive_fields_do_not_change_effective_values() {
    let file = daruda_flow::parse::parse_flow_file(
        "version: 1\nnodes:\n  - id: build\n    kind: command\n    run: echo ok\n",
    )
    .unwrap();
    let fields = fields_of(&file.nodes[0]);
    let mut raw = RawInputs::from_fields(&fields);
    let initial = raw.effective();
    raw.prompt_inline = "retained prose".into();
    raw.agent_model = "retained model".into();
    raw.max_turns = "not a number".into();
    assert_eq!(raw.effective(), initial);
    raw.kind = KindChoice::Agent;
    assert_eq!(
        raw.effective().into_fields().agent.model.as_deref(),
        Some("retained model")
    );
}

#[test]
fn effective_fields_preserve_the_existing_file_mutation_for_each_kind_and_policy() {
    let cases = [
        "version: 1\nnodes:\n  - id: build\n    kind: command\n    run: echo ok\n",
        "version: 1\nnodes:\n  - id: build\n    kind: command\n    run: echo ok\n    timeout: 3m\n    cwd: src\n    on_fail:\n      repair:\n        fix: diagnose\n        rerun: [build]\n        max_attempts: 2\n        wait: 1s\n",
        "version: 1\nnodes:\n  - id: build\n    kind: agent\n    agent:\n      id: codex\n      model: custom\n      mode: auto\n      effort: high\n      permission: ask\n    prompt: write prose\n    output: out.md\n    max_turns: 3\n",
        "version: 1\nnodes:\n  - id: build\n    kind: agent\n    prompt_file: prompt.md\n    output: out.md\n    on_fail:\n      retry:\n        hint_file: retry.md\n        max_attempts: 4\n        wait: 1m\n",
        "version: 1\nnodes:\n  - id: build\n    kind: agent\n    prompt: write prose\n    output: out.md\n    on_fail:\n      retry:\n        hint: retry prose\n        max_attempts: 4\n",
    ];
    for text in cases {
        let file = daruda_flow::parse::parse_flow_file(text).unwrap();
        let fields = fields_of(&file.nodes[0]);
        let effective = RawInputs::from_fields(&fields).effective().into_fields();
        let mut before = file.clone();
        let mut after = file;
        let node = daruda_flow::NodeId::from("build");
        crate::workspace::pages::flows::graph::form::apply::node_fields(
            &mut before,
            &node,
            &fields,
        );
        crate::workspace::pages::flows::graph::form::apply::node_fields(
            &mut after, &node, &effective,
        );
        assert_eq!(before, after, "{text}");
    }
}
