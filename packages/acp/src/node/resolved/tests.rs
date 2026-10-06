use super::*;

#[cfg(unix)]
fn npm_fixture(bin: &Path) -> PathBuf {
    let script = bin.join("npm-cli.js");
    std::fs::write(&script, "").unwrap();
    std::os::unix::fs::symlink(&script, bin.join("npm")).unwrap();
    script.canonicalize().unwrap()
}

#[cfg(unix)]
#[test]
fn finds_the_selected_installations_npm_without_requiring_npx() {
    let dir = tempfile::tempdir().unwrap();
    let npm = npm_fixture(dir.path());
    assert_eq!(npm_entry(dir.path()).unwrap(), npm);
    assert!(!dir.path().join("npx").exists());
}

#[cfg(unix)]
#[test]
fn js_adapter_requirements_do_not_change_the_legacy_runtime_floor() {
    let dir = tempfile::tempdir().unwrap();
    npm_fixture(dir.path());
    let (_, arch) = node_platform().unwrap();
    let identity = |version| {
        serde_json::json!({
            "path": dir.path().join("node"), "version": version, "arch": arch,
        })
    };
    assert!(from_identity(identity("20.0.0")).is_err());
    assert!(from_identity(identity("22.0.0")).is_ok());
    assert_eq!(
        super::super::MIN_NODE_VERSION,
        semver::Version::new(20, 0, 0)
    );
}

#[cfg(unix)]
#[test]
fn configured_runtime_path_and_probe_environment_precede_the_host_environment() {
    use std::os::unix::fs::PermissionsExt as _;
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("custom bin");
    std::fs::create_dir(&bin).unwrap();
    let node = bin.join("node");
    let npm = npm_fixture(&bin);
    let (_, arch) = node_platform().unwrap();
    let identity = serde_json::json!({"path": node, "version": "22.0.0", "arch": arch});
    let script = format!(
        "#!/bin/sh\n[ \"$TEST_RUNTIME_ENV\" = configured ] || exit 1\n\
         [ -z \"${{TEST_RUNTIME_STRIPPED+x}}\" ] || exit 1\nprintf '%s' {}\n",
        shell_words::quote(&identity.to_string())
    );
    std::fs::write(&node, script).unwrap();
    std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o755)).unwrap();
    let env = BTreeMap::from([
        ("PATH".into(), bin.to_string_lossy().into_owned()),
        ("TEST_RUNTIME_ENV".into(), "configured".into()),
        (
            "TEST_RUNTIME_STRIPPED".into(),
            "must not reach probe".into(),
        ),
    ]);
    let install_root = root.path().join("managed");
    let mut progress = Vec::new();
    let resolved = resolve(
        &install_root,
        &env,
        &["TEST_RUNTIME_STRIPPED".into()],
        &mut |phase| progress.push(phase),
        &PreparationContext::default(),
    )
    .unwrap();
    assert_eq!(resolved.node, node);
    assert_eq!(resolved.npm, npm);
    assert!(matches!(
        progress.as_slice(),
        [NodeProgress::UsingSystemNode]
    ));
    assert!(
        !install_root.exists(),
        "a configured runtime needs no download"
    );
    assert_eq!(system_node(&env, &["PATH".into()]), None);
}

#[test]
fn absent_path_override_uses_the_host_path_but_an_empty_override_does_not() {
    assert_eq!(
        system_node(&BTreeMap::new(), &[]),
        which::which("node").ok()
    );
    let env = BTreeMap::from([("PATH".into(), String::new())]);
    assert_eq!(system_node(&env, &[]), None);
}
