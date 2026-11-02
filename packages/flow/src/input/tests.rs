use super::*;

const FLOW: &str = "\
version: 1
inputs:
  pr:
    type: number
    description: The pull request
    required: true
  focus:
    type: choice
    choices: [security, speed, clarity]
    default: clarity
  fix:
    type: boolean
    default: false
  notes:
    multiline: true
nodes:
  - id: fetch
    kind: command
    run: \"true\"
";

fn decls() -> Vec<InputDecl> {
    crate::parse::parse_flow_file(FLOW).expect("parses").inputs
}

fn given(pairs: &[(&str, InputValue)]) -> InputValues {
    pairs
        .iter()
        .map(|(name, value)| (name.to_string(), value.clone()))
        .collect()
}

fn text(value: &str) -> InputValue {
    InputValue::Text(value.into())
}

fn kinds(result: Result<InputValues, Vec<ValidationIssue>>) -> Vec<ValidationKind> {
    result
        .expect_err("refused")
        .into_iter()
        .map(|issue| issue.kind)
        .collect()
}

/// The form offers inputs in the order the author wrote them, and the file
/// written back keeps that order.
#[test]
fn inputs_keep_the_order_they_are_declared_in() {
    let decls = decls();
    let names: Vec<_> = decls.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["pr", "focus", "fix", "notes"]);
    assert_eq!(decls[0].kind, InputType::Number);
    assert!(decls[0].required);
    assert_eq!(
        decls[3].kind,
        InputType::String,
        "string is the default type"
    );
    assert!(decls[3].multiline);

    let file = crate::parse::parse_flow_file(FLOW).unwrap();
    let written = yaml_serde::to_string(&file).unwrap();
    let again = crate::parse::parse_flow_file(&written).unwrap();
    assert_eq!(again.inputs, decls);
}

#[test]
fn an_input_declared_twice_is_a_parse_error() {
    let twice = "version: 1\ninputs:\n  a: {}\n  a: {}\nnodes: []\n";
    assert!(crate::parse::parse_flow_file(twice).is_err());
}

/// A run's values are checked against their types, defaults fill the gaps,
/// and an optional input with no default renders empty.
#[test]
fn values_are_checked_and_defaults_fill_the_rest() {
    let values = resolve(
        &decls(),
        &given(&[("pr", text(" 1234 ")), ("fix", text("true"))]),
    )
    .expect("valid");
    assert_eq!(values["pr"].as_text(), "1234", "text read as a number");
    assert_eq!(values["focus"].as_text(), "clarity", "from its default");
    assert_eq!(values["fix"], InputValue::Bool(true));
    assert_eq!(values["notes"].as_text(), "", "optional and unset");
}

#[test]
fn every_problem_with_the_values_is_reported() {
    assert_eq!(
        kinds(resolve(&decls(), &given(&[]))),
        [ValidationKind::InputMissing { name: "pr".into() }]
    );
    assert_eq!(
        kinds(resolve(&decls(), &given(&[("pr", text("  "))]))),
        [ValidationKind::InputMissing { name: "pr".into() }],
        "a blank field is not an answer"
    );
    let mut problems = kinds(resolve(
        &decls(),
        &given(&[
            ("pr", text("abc")),
            ("focus", text("taste")),
            ("fix", text("maybe")),
            ("branch", text("main")),
        ]),
    ));
    problems.sort_by_key(|kind| format!("{kind:?}"));
    assert_eq!(
        problems,
        [
            ValidationKind::InputNotDeclared {
                name: "branch".into()
            },
            ValidationKind::InputValueInvalid { name: "fix".into() },
            ValidationKind::InputValueInvalid {
                name: "focus".into()
            },
            ValidationKind::InputValueInvalid { name: "pr".into() },
        ]
    );
}

#[test]
fn declarations_are_checked_at_load() {
    let file = "\
version: 1
inputs:
  pick:
    type: choice
  count:
    type: number
    default: many
  bad name:
    default: x
nodes: []
";
    let decls = crate::parse::parse_flow_file(file).unwrap().inputs;
    let kinds: Vec<_> = check_decls(&decls).into_iter().map(|i| i.kind).collect();
    assert_eq!(
        kinds,
        [
            ValidationKind::InputChoicesMissing {
                name: "pick".into()
            },
            ValidationKind::InputDefaultInvalid {
                name: "count".into()
            },
            ValidationKind::InvalidInputName {
                name: "bad name".into()
            },
        ]
    );
}

/// Values render as the run recorded them, never localized.
#[test]
fn values_render_as_plain_text() {
    assert_eq!(InputValue::Bool(false).as_text(), "false");
    let number: InputValue = serde_json::from_str("1234").unwrap();
    assert_eq!(number.as_text(), "1234");
    assert_eq!(text("a b").as_text(), "a b");
}

#[test]
fn only_undeclared_input_tokens_are_reported() {
    let decls = decls();
    let text = "{{input.pr}} {{input.branch}} {{.Names}} {{node.fetch.output}} {{input.focus}}";
    assert_eq!(undeclared_refs(text, &decls), ["branch"]);
}

/// Emptying a pre-filled string runs without its default; a blank number,
/// boolean or choice is no value at all, so its default stands.
#[test]
fn a_cleared_string_stays_empty_and_other_blanks_take_the_default() {
    let decls = crate::parse::parse_flow_file(
        "version: 1\ninputs:\n  flags: { default: --verbose }\n  n: { type: number, default: 3 }\n  name: { required: true }\nnodes: []\n",
    )
    .unwrap()
    .inputs;
    let values = resolve(
        &decls,
        &given(&[("flags", text("")), ("n", text(" ")), ("name", text("x"))]),
    )
    .unwrap();
    assert_eq!(values["flags"].as_text(), "");
    assert_eq!(values["n"].as_text(), "3");
    assert_eq!(
        kinds(resolve(&decls, &given(&[("name", text("  "))]))),
        [ValidationKind::InputMissing {
            name: "name".into()
        }],
        "a blank required string is still unanswered"
    );
}

#[test]
fn a_number_is_read_the_way_a_person_types_it() {
    let decl = &decls()[0];
    let read = |typed: &str| text(typed).coerce(decl).map(|v| v.as_text());
    assert_eq!(read("1234").as_deref(), Some("1234"));
    assert_eq!(read("1.50").as_deref(), Some("1.5"));
    assert_eq!(read("+5").as_deref(), Some("5"));
    assert_eq!(read("007").as_deref(), Some("7"));
    assert_eq!(read("abc"), None);
    assert_eq!(read("inf"), None);
}
