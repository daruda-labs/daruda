//! Every type word in both directions, the path a nested problem reports,
//! and the keywords this build refuses to pretend it enforces.

use super::*;

/// One schema per type word, so `holds` is covered in both directions.
fn of(kind: &str) -> SchemaSubset {
    schema(&format!("type: {kind}"))
}

/// A schema written the way a flow writes it, so the tests exercise the
/// wire spelling (`type:`, `enum:`) rather than a hand-built struct.
fn schema(yaml: &str) -> SchemaSubset {
    yaml_serde::from_str(yaml).expect("the fixture is a schema")
}

fn json(text: &str) -> Value {
    serde_json::from_str(text).expect("the fixture is JSON")
}

/// Every type word, accepted and refused, in one table — the six-way match
/// in `holds` is the thing a new type would be forgotten in.
#[test]
fn every_declared_type_accepts_its_own_and_refuses_the_rest() {
    let cases = [
        ("object", "{}", "[]"),
        ("array", "[]", "{}"),
        ("string", "\"s\"", "1"),
        ("number", "1.5", "\"1.5\""),
        ("integer", "7", "\"7\""),
        ("boolean", "true", "\"true\""),
    ];
    for (kind, good, bad) in cases {
        assert_eq!(validate(&json(good), &of(kind)), Ok(()), "{kind} / {good}");
        let Err(problems) = validate(&json(bad), &of(kind)) else {
            panic!("{kind} accepted {bad}");
        };
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].starts_with(&format!("$: expected {kind}, found ")),
            "{problems:?}"
        );
    }
}

/// JSON Schema calls `3.0` an integer and `serde_json::Number` has no such
/// case, so this is stated rather than inherited — a model writing `3.0`
/// would otherwise cost a whole node run.
#[test]
fn an_integer_valued_float_is_an_integer_and_a_fractional_one_is_not() {
    assert_eq!(validate(&json("3.0"), &of("integer")), Ok(()));
    assert!(validate(&json("3.5"), &of("integer")).is_err());
}

#[test]
fn a_required_field_that_is_absent_is_named_by_its_path() {
    let schema = schema(
        "\
type: object
required: [verdict, why]
properties:
  verdict: { type: string }
  why: { type: string }
",
    );
    assert_eq!(
        validate(&json(r#"{"verdict": "pass"}"#), &schema),
        Err(vec!["$.why: required, and absent".to_string()])
    );
}

/// **The forgiving half, asserted rather than assumed.** The schema reaches
/// the agent as prompt text, which gets extra invented fields — failing a
/// node over one nothing reads buys another run for nothing.
#[test]
fn a_property_the_schema_never_mentions_is_allowed() {
    let schema = schema(
        "\
type: object
required: [verdict]
properties:
  verdict: { type: string }
",
    );
    assert_eq!(
        validate(
            &json(r#"{"verdict": "pass", "invented": {"deep": 1}}"#),
            &schema
        ),
        Ok(())
    );
}

/// Nesting, and the path a nested problem reports — the one thing that
/// tells an agent *where* to look.
#[test]
fn a_nested_property_reports_the_path_to_it() {
    let schema = schema(
        "\
type: object
properties:
  a:
    type: object
    properties:
      b:
        type: array
        items: { type: string }
",
    );
    assert_eq!(
        validate(&json(r#"{"a": {"b": ["ok", 2]}}"#), &schema),
        Err(vec!["$.a.b[1]: expected string, found number".to_string()])
    );
}

/// `items` applies to every element, and a schema with no `items` says
/// nothing about them.
#[test]
fn an_array_is_judged_element_by_element_only_when_items_says_how() {
    let listed = schema("type: array\nitems: { type: integer }\n");
    assert_eq!(validate(&json("[1, 2, 3]"), &listed), Ok(()));
    assert!(validate(&json(r#"[1, "two"]"#), &listed).is_err());
    assert_eq!(validate(&json(r#"[1, "two", {}]"#), &of("array")), Ok(()));
}

#[test]
fn an_enum_admits_only_what_it_lists() {
    let schema = schema("type: string\nenum: [pass, fail]\n");
    assert_eq!(validate(&json("\"pass\""), &schema), Ok(()));
    assert_eq!(
        validate(&json("\"maybe\""), &schema),
        Err(vec![
            r#"$: expected one of ["pass","fail"], found "maybe""#.to_string()
        ])
    );
}

/// The list is pasted into a prompt, so it is capped — and the cap is what
/// a wide array of wrong elements would otherwise blow past.
#[test]
fn the_problem_list_is_capped() {
    let schema = schema("type: array\nitems: { type: string }\n");
    let wide = Value::Array((0..50).map(Value::from).collect());
    let Err(problems) = validate(&wide, &schema) else {
        panic!("50 numbers are not 50 strings");
    };
    assert_eq!(problems.len(), MAX_PROBLEMS);
}

/// A wrong type stops the walk under it: one line about the object, not
/// one per field it was supposed to have.
#[test]
fn a_wrong_type_is_reported_once_and_not_walked_into() {
    let schema = schema(
        "\
type: object
required: [a, b, c]
properties:
  a: { type: string }
",
    );
    assert_eq!(
        validate(&json("\"prose, not an object\""), &schema),
        Err(vec!["$: expected object, found string".to_string()])
    );
}

fn kinds(schema: &SchemaSubset) -> Vec<ValidationKind> {
    issues(schema, &NodeId::from("design"))
        .into_iter()
        .map(|i| i.kind)
        .collect()
}

/// The keywords an author reaches for that this build does not enforce.
/// Each is refused by name, and the refusal names the node.
#[test]
fn a_keyword_this_build_does_not_enforce_is_refused_by_name() {
    for keyword in ["$ref", "oneOf", "additionalProperties", "format"] {
        let schema = schema(&format!("type: object\n{keyword}: false\n"));
        let found = issues(&schema, &NodeId::from("design"));
        assert_eq!(
            found.iter().map(|i| i.kind.clone()).collect::<Vec<_>>(),
            vec![ValidationKind::UnsupportedSchemaKeyword {
                keyword: keyword.to_string()
            }]
        );
        assert_eq!(found[0].node.as_ref(), Some(&NodeId::from("design")));
    }
}

/// Nested, too: a keyword three levels down is as unenforced as one at the
/// top, and an author who cannot see which one was ignored fixes nothing.
#[test]
fn an_unenforced_keyword_is_found_wherever_it_sits() {
    let schema = schema(
        "\
type: object
properties:
  a:
    type: array
    items:
      type: object
      $ref: '#/x'
",
    );
    assert_eq!(
        kinds(&schema),
        vec![ValidationKind::UnsupportedSchemaKeyword {
            keyword: "$ref".to_string()
        }]
    );
}

/// `required` alone is the only spelling of "this key must exist, whatever
/// its type": `properties: { verdict: {} }` has no `type` and so is a
/// *parse* error, which costs the file's whole graph. The check it gets is
/// presence-only, which is exactly what it asked for.
#[test]
fn required_without_properties_asks_only_for_presence_and_is_accepted() {
    let presence_only = schema("type: object\nrequired: [verdict]\n");
    assert!(kinds(&presence_only).is_empty());
    assert_eq!(validate(&json(r#"{"verdict": 1}"#), &presence_only), Ok(()));
    assert!(validate(&json("{}"), &presence_only).is_err());

    assert!(
        yaml_serde::from_str::<SchemaSubset>("type: object\nproperties: { verdict: {} }\n")
            .is_err(),
        "the alternative spelling has to be the parse error this rule ignored"
    );
}

/// `enum: [1, 2]` parses — a `Vec<String>` there would have made it a
/// parse error and taken the file's whole graph away — and is then refused
/// here, because a numeric one is not what `validate` enforces.
#[test]
fn an_enum_on_anything_but_a_string_is_refused_at_load() {
    assert_eq!(
        kinds(&schema("type: integer\nenum: [1, 2]\n")),
        vec![ValidationKind::SchemaKeywordOnWrongKind {
            keyword: "enum",
            kind: "integer",
        }]
    );
    assert!(kinds(&schema("type: string\nenum: [pass]\n")).is_empty());
}

/// The gap the guard above only half closed: `required` and `properties` are
/// read under a `type: string`, accepted at load, and then never asked —
/// `check` reaches them only under an object. A flow author who mistyped the
/// type got a clean bill of health for a schema enforcing nothing, and paid
/// an agent turn to find out.
#[test]
fn structural_keywords_under_the_wrong_type_are_refused_at_load() {
    const MISTYPED: &str = "\
type: string
required: [verdict]
properties:
  verdict: { type: string }
";
    let kinds = kinds(&schema(MISTYPED));
    assert_eq!(
        kinds,
        vec![
            ValidationKind::SchemaKeywordOnWrongKind {
                keyword: "required",
                kind: "string",
            },
            ValidationKind::SchemaKeywordOnWrongKind {
                keyword: "properties",
                kind: "string",
            },
        ],
        "{kinds:?}"
    );
    assert_eq!(
        self::kinds(&schema("type: string\nitems: { type: string }\n")),
        vec![ValidationKind::SchemaKeywordOnWrongKind {
            keyword: "items",
            kind: "string",
        }]
    );
}

/// The shapes that were always right stay right — the new rule must not
/// refuse an ordinary object or array schema.
#[test]
fn keywords_under_the_type_that_enforces_them_are_accepted() {
    assert!(
        kinds(&schema(
            "type: object\nrequired: [a]\nproperties:\n  a: { type: string }\n"
        ))
        .is_empty()
    );
    assert!(kinds(&schema("type: array\nitems: { type: string }\n")).is_empty());
}
