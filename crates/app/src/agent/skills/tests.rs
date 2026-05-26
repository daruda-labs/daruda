use std::fs;

use super::frontmatter::{
    SkillFrontmatter, parse_frontmatter, render_skill_md, serialize_frontmatter, split_frontmatter,
};
use super::persist::{SkillDraft, delete_skill, rename_skill, write_skill};
use super::{
    NameError, SkillInvocation, SkillScope, SkillsState, body_preview, scan_scope, validate_name,
};

fn frontmatter_with(name: &str, description: &str) -> SkillFrontmatter {
    let mut fm = SkillFrontmatter::empty();
    fm.name = Some(name.into());
    fm.description = Some(description.into());
    fm
}

#[test]
fn split_frontmatter_extracts_yaml_block_and_body() {
    let src = "---\nname: foo\ndescription: bar\n---\n\nbody text\n";
    let (yaml, body) = split_frontmatter(src);
    assert_eq!(yaml.unwrap().trim(), "name: foo\ndescription: bar");
    assert_eq!(body, "\nbody text\n");
}

#[test]
fn split_frontmatter_handles_missing_delimiter_gracefully() {
    let src = "no frontmatter here\nbody starts immediately";
    let (yaml, body) = split_frontmatter(src);
    assert!(yaml.is_none());
    assert_eq!(body, src);
}

#[test]
fn parse_frontmatter_round_trips_known_keys() {
    let yaml = "name: pr-review\n\
                description: Review pull requests\n\
                argument-hint: <pr-number>\n\
                allowed-tools: \"Bash(git diff *)\"\n\
                disable-model-invocation: true\n\
                user-invocable: false\n";
    let fm = parse_frontmatter(yaml).unwrap();
    assert_eq!(fm.name.as_deref(), Some("pr-review"));
    assert_eq!(fm.description.as_deref(), Some("Review pull requests"));
    assert_eq!(fm.argument_hint.as_deref(), Some("<pr-number>"));
    assert_eq!(fm.allowed_tools.as_deref(), Some("Bash(git diff *)"));
    assert!(fm.disable_model_invocation);
    assert!(!fm.user_invocable);

    let serialized = serialize_frontmatter(&fm);
    let reparsed =
        parse_frontmatter(split_frontmatter(&serialized).0.expect("frontmatter block")).unwrap();
    assert_eq!(reparsed, fm);
}

#[test]
fn parse_frontmatter_preserves_unknown_keys_lossless() {
    let yaml = "name: example\n\
                hooks:\n  PreToolUse: ./scripts/pre.sh\n  PostToolUse: ./scripts/post.sh\n\
                custom_tag: keep-me\n";
    let fm = parse_frontmatter(yaml).unwrap();
    assert!(fm.extra.contains_key("hooks"));
    assert!(fm.extra.contains_key("custom_tag"));

    let rendered = serialize_frontmatter(&fm);
    let again = parse_frontmatter(split_frontmatter(&rendered).0.unwrap()).unwrap();
    assert_eq!(again.extra, fm.extra);
}

#[test]
fn parse_frontmatter_arguments_string_to_vec() {
    let yaml = "name: x\narguments:\n  - foo\n  - bar\n";
    let fm = parse_frontmatter(yaml).unwrap();
    assert_eq!(fm.arguments, vec!["foo".to_string(), "bar".to_string()]);
}

#[test]
fn parse_frontmatter_default_user_invocable_is_true() {
    let fm = parse_frontmatter("name: defaults\n").unwrap();
    assert!(fm.user_invocable);
    assert!(!fm.disable_model_invocation);
}

#[test]
fn round_trip_preserves_yaml_reserved_scalar_strings() {
    // YAML 1.2 core schema parses these as bool/null/int/float, so
    // an unquoted emit would round-trip them as a non-string and the
    // typed-string `value_to_string` would drop them to `None`.
    // Each entry here exercises one reserved-scalar variant —
    // `serialize_frontmatter` must quote them so the next parse
    // keeps the `String` shape.
    let cases = [
        "true", "false", "null", "~", "yes", "no", "on", "off", "0", "42", "-7", "3.14", ".inf",
        ".nan",
    ];
    for case in cases {
        let mut fm = SkillFrontmatter::empty();
        fm.description = Some(case.to_string());
        fm.model = Some(case.to_string());
        let rendered = serialize_frontmatter(&fm);
        let yaml = split_frontmatter(&rendered)
            .0
            .unwrap_or_else(|| panic!("{case}: missing frontmatter block in {rendered:?}"));
        let parsed = parse_frontmatter(yaml)
            .unwrap_or_else(|e| panic!("{case}: parse error {e:?} on {rendered:?}"));
        assert_eq!(
            parsed.description.as_deref(),
            Some(case),
            "description round-trip lost value for {case:?}: rendered={rendered:?}"
        );
        assert_eq!(
            parsed.model.as_deref(),
            Some(case),
            "model round-trip lost value for {case:?}: rendered={rendered:?}"
        );
    }
}

#[test]
fn round_trip_preserves_extra_keys_with_reserved_scalar_values() {
    // `extra` keys go through serde_yaml directly, so YAML's core
    // schema handles them — but verify a representative sample so we
    // notice if we ever change the extra-key path.
    let yaml = "name: x\ncustom: \"true\"\nother_count: \"42\"\n";
    let fm = parse_frontmatter(yaml).unwrap();
    let rendered = serialize_frontmatter(&fm);
    let reparsed = parse_frontmatter(split_frontmatter(&rendered).0.unwrap()).unwrap();
    assert_eq!(reparsed.extra, fm.extra);
}

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
fn validate_name_accepts_canonical_names() {
    for ok in ["pr-review", "standup", "a", "skill_with_underscore", "x9"] {
        assert!(validate_name(ok).is_ok(), "{ok} should validate");
    }
}

#[test]
fn skill_invocation_4_state_table() {
    assert_eq!(
        SkillInvocation::from_flags(true, false),
        SkillInvocation::Both
    );
    assert_eq!(
        SkillInvocation::from_flags(true, true),
        SkillInvocation::UserOnly
    );
    assert_eq!(
        SkillInvocation::from_flags(false, false),
        SkillInvocation::ModelOnly
    );
    assert_eq!(
        SkillInvocation::from_flags(false, true),
        SkillInvocation::Disabled
    );
}

#[test]
fn body_preview_extracts_first_paragraph() {
    let body =
        "First paragraph spans\nseveral lines but stops at the blank.\n\nSecond paragraph here.";
    let preview = body_preview(body);
    assert_eq!(
        preview,
        "First paragraph spans several lines but stops at the blank."
    );
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
fn scan_scope_picks_up_skills_and_skips_dirs_without_skill_md() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    fs::create_dir(root.join("alpha")).unwrap();
    fs::write(
        root.join("alpha").join("SKILL.md"),
        "---\nname: alpha\ndescription: hello\n---\n\nFirst paragraph.\n",
    )
    .unwrap();

    fs::create_dir(root.join("incomplete")).unwrap();
    fs::write(root.join("incomplete").join("notes.md"), "ignored").unwrap();

    fs::create_dir(root.join("beta")).unwrap();
    fs::write(
        root.join("beta").join("SKILL.md"),
        "---\nname: beta\n---\nbody.\n",
    )
    .unwrap();
    fs::write(root.join("beta").join("scripts.sh"), "echo hi").unwrap();

    let mut skills = scan_scope(root, SkillScope::Project);
    skills.sort_by(|a, b| a.name.cmp(&b.name));

    assert_eq!(skills.len(), 2);
    assert_eq!(skills[0].name, "alpha");
    assert_eq!(skills[1].name, "beta");
    assert_eq!(skills[0].aux_file_count, 0);
    assert_eq!(skills[1].aux_file_count, 1);
    assert_eq!(
        skills[0].body_preview, "First paragraph.",
        "body preview must capture first non-frontmatter paragraph"
    );
}

#[test]
fn write_skill_round_trips_through_disk() {
    let tmp = tempfile::tempdir().unwrap();
    let project_root = tmp.path();
    fs::create_dir_all(project_root.join(".claude").join("skills")).unwrap();

    let draft = SkillDraft {
        name: "round-trip".into(),
        scope: SkillScope::Project,
        frontmatter: frontmatter_with("round-trip", "tested"),
        body: "Body content.\n".into(),
    };
    let path = write_skill(&draft, Some(project_root), false).unwrap();
    assert!(path.exists());

    let raw = fs::read_to_string(&path).unwrap();
    let (yaml, body) = split_frontmatter(&raw);
    let fm = parse_frontmatter(yaml.unwrap()).unwrap();
    assert_eq!(fm.name.as_deref(), Some("round-trip"));
    assert_eq!(fm.description.as_deref(), Some("tested"));
    assert!(body.contains("Body content."));
}

#[test]
fn write_skill_refuses_overwrite_without_flag() {
    let tmp = tempfile::tempdir().unwrap();
    let project_root = tmp.path();

    let draft = SkillDraft {
        name: "exists".into(),
        scope: SkillScope::Project,
        frontmatter: frontmatter_with("exists", "first"),
        body: "first body\n".into(),
    };
    write_skill(&draft, Some(project_root), false).unwrap();

    let err = write_skill(&draft, Some(project_root), false).unwrap_err();
    assert!(matches!(err, super::PersistError::AlreadyExists(_)));

    // Overwrite=true succeeds and replaces content.
    let mut draft2 = draft.clone();
    draft2.body = "replaced body\n".into();
    let path = write_skill(&draft2, Some(project_root), true).unwrap();
    let raw = fs::read_to_string(&path).unwrap();
    assert!(raw.contains("replaced body"));
}

#[test]
fn rename_skill_moves_directory_and_refuses_collisions() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("orig")).unwrap();
    fs::write(
        root.join("orig").join("SKILL.md"),
        "---\nname: orig\n---\nx\n",
    )
    .unwrap();

    let renamed = rename_skill(&root.join("orig"), "renamed").unwrap();
    assert!(renamed.exists());
    assert!(!root.join("orig").exists());

    fs::create_dir_all(root.join("collide")).unwrap();
    fs::write(root.join("collide").join("SKILL.md"), "x").unwrap();
    let err = rename_skill(&renamed, "collide").unwrap_err();
    assert!(matches!(err, super::PersistError::RenameTargetExists(_)));
}

#[test]
fn delete_skill_removes_directory_and_aux_files() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("kill").join("scripts")).unwrap();
    fs::write(root.join("kill").join("SKILL.md"), "---\nname: kill\n---\n").unwrap();
    fs::write(root.join("kill").join("scripts").join("a.sh"), "echo").unwrap();

    delete_skill(&root.join("kill")).unwrap();
    assert!(!root.join("kill").exists());
}

#[test]
fn skills_state_load_and_overrides_detection() {
    let tmp_project = tempfile::tempdir().unwrap();
    let tmp_personal = tempfile::tempdir().unwrap();
    let project_root = tmp_project.path();
    let personal_root = tmp_personal.path();

    fs::create_dir_all(project_root.join(".claude").join("skills").join("shared")).unwrap();
    fs::write(
        project_root
            .join(".claude")
            .join("skills")
            .join("shared")
            .join("SKILL.md"),
        "---\nname: shared\ndescription: project-side\n---\n",
    )
    .unwrap();
    fs::create_dir_all(personal_root.join("shared")).unwrap();
    fs::write(
        personal_root.join("shared").join("SKILL.md"),
        "---\nname: shared\ndescription: personal-side\n---\n",
    )
    .unwrap();

    let mut state = SkillsState::default();
    state.reload_scope(SkillScope::Personal, None, personal_root);
    state.reload_scope(SkillScope::Project, Some(project_root), personal_root);
    let snap = state.snapshot_for(Some(project_root));
    assert_eq!(snap.project.len(), 1);
    assert_eq!(snap.personal.len(), 1);
    assert!(snap.project_overrides_personal("shared"));
    assert!(state.name_exists(SkillScope::Project, "shared", Some(project_root)));
    assert!(!state.name_exists(SkillScope::Project, "missing", Some(project_root)));
}

#[test]
fn render_skill_md_composes_frontmatter_and_body() {
    let mut fm = SkillFrontmatter::empty();
    fm.name = Some("alpha".into());
    fm.description = Some("hi".into());
    let rendered = render_skill_md(&fm, "Body goes here.\n");
    assert!(rendered.starts_with("---\n"));
    assert!(rendered.contains("name: alpha"));
    assert!(rendered.contains("Body goes here."));
}
