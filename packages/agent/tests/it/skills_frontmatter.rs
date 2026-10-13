//! Tests for the public `daruda_agent::skills::frontmatter` API.

use daruda_agent::skills::frontmatter::*;
use yaml_serde::Value;

#[test]
fn modal_safety_editing_a_description_preserves_all_other_yaml_values() {
    let source = "---\nname: sample\ndescription: old\npaths: [src/**, tests/**]\nallowed-tools: [Read, Bash]\ncontext: fork\nagent: reviewer\neffort: high\nshell: bash\nfuture: {enabled: true}\n---\n\nKeep this body.\n";
    let (yaml, body) = split_frontmatter(source);
    let before = parse_frontmatter(yaml.unwrap()).unwrap();
    let mut after = before.clone();
    after.description = Some("new".into());
    let edited = render_edited_skill_md(source, &before, &after, body).unwrap();
    let (new_yaml, new_body) = split_frontmatter(&edited);
    let mut old: yaml_serde::Mapping = yaml_serde::from_str(yaml.unwrap()).unwrap();
    old.insert(
        Value::String("description".into()),
        Value::String("new".into()),
    );
    assert_eq!(
        yaml_serde::from_str::<yaml_serde::Mapping>(new_yaml.unwrap()).unwrap(),
        old
    );
    assert_eq!(new_body, body);
}

#[test]
fn modal_safety_an_unchanged_edit_is_byte_for_byte_identical() {
    let source = "---\n# Retain comments too.\nname: sample\nuser-invocable: true\n---\n\n\nBody without final newline";
    let (yaml, body) = split_frontmatter(source);
    let fm = parse_frontmatter(yaml.unwrap()).unwrap();
    assert_eq!(
        render_edited_skill_md(source, &fm, &fm, body).unwrap(),
        source
    );
}
