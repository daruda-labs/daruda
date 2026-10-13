use std::collections::HashSet;
use std::fs;

use super::plugins::PluginAvailability;
use super::scan::scan_skills_dir_plugins;
use super::{NameError, SkillScope, body_preview, validate_name};

#[test]
fn validate_name_rejects_uppercase_and_specials() {
    assert!(matches!(validate_name(""), Err(NameError::Empty)));
    assert!(matches!(
        validate_name("Pr-review"),
        Err(NameError::InvalidChar { ch: 'P', .. })
    ));
    assert!(matches!(
        validate_name("hello world"),
        Err(NameError::InvalidChar { ch: ' ', .. })
    ));
    assert!(matches!(
        validate_name("-leading-dash"),
        Err(NameError::InvalidLeading { ch: '-' })
    ));
    let too_long = "a".repeat(super::MAX_NAME_LEN + 1);
    assert!(matches!(
        validate_name(&too_long),
        Err(NameError::TooLong { .. })
    ));
}

#[test]
fn body_preview_clamps_to_max_chars() {
    let long = "a".repeat(super::scan::PREVIEW_MAX_CHARS + 50);
    let preview = body_preview(&long);
    let len = preview.chars().count();
    assert_eq!(len, super::scan::PREVIEW_MAX_CHARS + 1);
    assert!(preview.ends_with('…'));
}

#[test]
fn skills_dir_plugin_namespaced_under_plugin_scope() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // Create the plugin manifest directory.
    fs::create_dir_all(root.join("daruda").join(".claude-plugin")).unwrap();
    fs::write(
        root.join("daruda")
            .join(".claude-plugin")
            .join("plugin.json"),
        "{}",
    )
    .unwrap();

    // Create a skill inside the plugin.
    fs::create_dir_all(root.join("daruda").join("skills").join("sd")).unwrap();
    fs::write(
        root.join("daruda")
            .join("skills")
            .join("sd")
            .join("SKILL.md"),
        "---\nname: sd\ndescription: x\n---\nbody\n",
    )
    .unwrap();

    let mut out = Vec::new();
    scan_skills_dir_plugins(root, &mut HashSet::new(), &mut out);

    assert_eq!(out.len(), 1, "expected exactly one skill");
    let skill = &out[0];
    assert_eq!(skill.name, "daruda:sd");
    assert_eq!(skill.scope, SkillScope::Plugin);
    assert_eq!(skill.plugin_id.as_deref(), Some("daruda@skills-dir"));
    assert_eq!(
        skill.plugin_availability,
        Some(PluginAvailability::Installed)
    );
}

#[test]
fn skills_dir_requires_plugin_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // No `.claude-plugin/plugin.json` — directory is a plain folder.
    fs::create_dir_all(root.join("plain").join("skills").join("x")).unwrap();
    fs::write(
        root.join("plain").join("skills").join("x").join("SKILL.md"),
        "---\nname: x\n---\nbody\n",
    )
    .unwrap();

    let mut out = Vec::new();
    scan_skills_dir_plugins(root, &mut HashSet::new(), &mut out);

    assert!(out.is_empty(), "expected no skills without plugin manifest");
}
