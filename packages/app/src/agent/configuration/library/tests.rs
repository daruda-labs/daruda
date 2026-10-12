use super::*;

fn configuration(home: &Path, account: &Path, cwd: &Path) -> Configuration {
    Configuration::resolve(
        Provider::Claude,
        Some(home.into()),
        Some(account.into()),
        Some(cwd.into()),
    )
    .unwrap()
}
fn draft(name: &str) -> McpServerDraft {
    let mut draft = mcp::import::parse("npx example").unwrap().remove(0);
    draft.name = name.into();
    draft
}

#[test]
fn accounts_and_local_worktrees_have_independent_sources() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let a = configuration(home, &home.join("a"), &home.join("repo/worktree-a"));
    let b = configuration(home, &home.join("b"), &home.join("repo/worktree-b"));
    for config in [&a, &b] {
        std::fs::create_dir_all(&config.root).unwrap();
    }
    let entry = draft("account-a");
    Target::new(&a, McpScope::User, None)
        .unwrap()
        .mutate(&entry.name, None, Some(&entry))
        .unwrap();
    let local = Target::new(&a, McpScope::Local, None).unwrap();
    local.mutate("account-a", None, Some(&entry)).unwrap();
    let inventory = discover(&a).unwrap();
    assert_eq!(inventory.mcp.user.len(), 1);
    assert_eq!(inventory.mcp.local.len(), 1);
    assert!(local.label().contains("repo/worktree-a"));
    assert!(discover(&b).unwrap().mcp.user.is_empty());
    assert!(!home.join(".claude.json").exists());
    assert!(!b.user_file.exists());
}

#[test]
fn toggle_only_changes_the_captured_projects_disable_list() {
    let temp = tempfile::tempdir().unwrap();
    let config = configuration(temp.path(), temp.path(), &temp.path().join("repo"));
    let source = serde_json::json!({"mcpServers":{"docs":{"command":"npx","disabled":true}},"permissions":{"keep":true}});
    std::fs::write(&config.user_file, source.to_string()).unwrap();
    let before = discover(&config).unwrap();
    assert!(
        !before.mcp.user[0].disabled,
        "legacy entry.disabled is not runtime evidence"
    );
    let original = McpServerDraft::from_server(&before.mcp.user[0]);
    let target = Target::new(&config, McpScope::User, None).unwrap();
    target.toggle(&original).unwrap();
    let raw = mcp::read_json_or_empty(&config.user_file).unwrap();
    assert_eq!(raw["mcpServers"], source["mcpServers"]);
    assert_eq!(raw["permissions"], source["permissions"]);
    let key = config.cwd.as_ref().unwrap().to_string_lossy();
    assert_eq!(
        raw["projects"][key.as_ref()]["disabledMcpServers"],
        serde_json::json!(["docs"])
    );
    assert!(
        target.toggle(&original).is_err(),
        "stale switch must not invert a newer choice"
    );
    let loaded = discover(&config).unwrap();
    let expected = McpServerDraft::from_server(&loaded.mcp.user[0]);
    let mut changed = expected.clone();
    changed.args.push("edited".into());
    target
        .mutate("docs", Some(&expected), Some(&changed))
        .unwrap();
    let after = mcp::read_json_or_empty(&config.user_file).unwrap();
    assert_eq!(after["mcpServers"]["docs"]["disabled"], true);
    assert_eq!(after["projects"], raw["projects"]);
}

#[test]
fn deletion_refuses_a_replacement_record_and_preserves_unrelated_edits() {
    let temp = tempfile::tempdir().unwrap();
    let config = configuration(temp.path(), temp.path(), temp.path());
    let target = Target::new(&config, McpScope::User, None).unwrap();
    let entry = draft("docs");
    target.mutate("docs", None, Some(&entry)).unwrap();
    let mut raw = mcp::read_json_or_empty(&config.user_file).unwrap();
    raw["mcpServers"]["docs"]["command"] = "replacement".into();
    raw["unrelated"] = true.into();
    std::fs::write(&config.user_file, raw.to_string()).unwrap();
    assert!(target.mutate("docs", Some(&entry), None).is_err());
    assert_eq!(mcp::read_json_or_empty(&config.user_file).unwrap(), raw);
}

#[cfg(unix)]
#[test]
fn local_keys_preserve_a_single_alias_and_reject_ambiguity() {
    let temp = tempfile::tempdir().unwrap();
    let real = temp.path().join("repo");
    std::fs::create_dir(&real).unwrap();
    let alias = temp.path().join("alias");
    daruda_core::path::symlink(&real, &alias).unwrap();
    let alternate = temp.path().join("alternate");
    daruda_core::path::symlink(&real, &alternate).unwrap();
    let mut raw = serde_json::json!({"projects":{}});
    raw["projects"][alias.to_string_lossy().as_ref()] = serde_json::json!({});
    assert_eq!(
        local_key(&raw, &real.to_string_lossy()).unwrap(),
        alias.to_string_lossy()
    );
    raw["projects"][alternate.to_string_lossy().as_ref()] = serde_json::json!({});
    assert!(local_key(&raw, &real.to_string_lossy()).is_err());
    assert_eq!(
        local_key(&raw, &alias.to_string_lossy()).unwrap(),
        alias.to_string_lossy()
    );
}

#[test]
fn invalid_known_fields_cannot_be_silently_dropped_by_an_edit() {
    let temp = tempfile::tempdir().unwrap();
    let config = configuration(temp.path(), temp.path(), temp.path());
    let raw = serde_json::json!({"mcpServers":{"docs":{"command":"npx","env":{"TOKEN":123}}}});
    std::fs::write(&config.user_file, raw.to_string()).unwrap();
    let inventory = discover(&config).unwrap();
    let expected = McpServerDraft::from_server(&inventory.mcp.user[0]);
    let mut changed = expected.clone();
    changed.args.push("edited".into());
    assert!(
        Target::new(&config, McpScope::User, None)
            .unwrap()
            .mutate("docs", Some(&expected), Some(&changed))
            .is_err()
    );
    assert_eq!(mcp::read_json_or_empty(&config.user_file).unwrap(), raw);
}
