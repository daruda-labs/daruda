use super::*;

fn adapter() -> NpmAdapter {
    NpmAdapter::parse("npx -y @agentclientprotocol/claude-agent-acp@1.2.3").unwrap()
}

fn fixture(root: &Path, version: &str, bin: &str) -> PathBuf {
    let package = root.join(NODE_MODULES).join(adapter().name);
    fs::create_dir_all(package.join("dist")).unwrap();
    fs::write(
        package.join(MANIFEST),
        serde_json::to_vec(&serde_json::json!({
            "name": adapter().name, "version": version, "bin": {"claude-agent-acp": bin}
        }))
        .unwrap(),
    )
    .unwrap();
    let entry = package.join("dist/index.js");
    fs::write(&entry, "console.log('ready')").unwrap();
    entry
}

#[test]
fn publishes_once_and_reuses_a_readable_non_executable_entry() {
    let root = tempfile::tempdir().unwrap();
    let dest = root.path().join("version");
    let entry = install_at(&adapter(), "1.2.3", &dest, |staging| {
        let entry = fixture(staging, "1.2.3", "dist/index.js");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(entry, fs::Permissions::from_mode(0o644)).unwrap();
        }
        assert!(!dest.exists());
        Ok(())
    })
    .unwrap();
    assert!(entry.is_file());
    assert_eq!(
        install_at(&adapter(), "1.2.3", &dest, |_| panic!(
            "warm installation must not run npm"
        ))
        .unwrap(),
        entry
    );
}

#[test]
fn failed_install_never_publishes_a_partial_tree() {
    let root = tempfile::tempdir().unwrap();
    let dest = root.path().join("version");
    assert!(
        install_at(&adapter(), "1.2.3", &dest, |staging| {
            fixture(staging, "1.2.3", "dist/missing.js");
            Ok(())
        })
        .is_err()
    );
    assert!(!dest.exists());
}

#[test]
fn damaged_cache_is_reinstalled_without_deleting_the_old_tree() {
    let root = tempfile::tempdir().unwrap();
    let dest = root.path().join("version");
    fixture(&dest, "wrong", "dist/index.js");
    install_at(&adapter(), "1.2.3", &dest, |staging| {
        fixture(staging, "1.2.3", "dist/index.js");
        Ok(())
    })
    .unwrap();
    assert!(fs::read_dir(root.path()).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".invalid-")
    }));
}

#[test]
fn rejects_wrong_identity_non_js_and_escaping_entries() {
    let root = tempfile::tempdir().unwrap();
    fixture(root.path(), "1.2.4", "dist/index.js");
    assert!(validate_entry(&adapter(), "1.2.3", root.path()).is_err());
    let entry = fixture(root.path(), "1.2.3", "dist/native");
    fs::rename(&entry, entry.with_file_name("native")).unwrap();
    assert!(validate_entry(&adapter(), "1.2.3", root.path()).is_err());
    #[cfg(unix)]
    {
        let entry = fixture(root.path(), "1.2.3", "dist/escape.js");
        std::os::unix::fs::symlink("/etc/passwd", entry.with_file_name("escape.js")).unwrap();
        assert!(validate_entry(&adapter(), "1.2.3", root.path()).is_err());
    }
}

#[test]
fn resolves_tags_and_ranges_to_concrete_versions() {
    assert_eq!(resolved_version("\"1.2.3\"").unwrap(), "1.2.3");
    assert_eq!(
        resolved_version("[\"1.9.0\",\"1.10.0\"]").unwrap(),
        "1.10.0"
    );
    assert!(resolved_version("{}").is_err());
}

#[test]
fn network_failure_reuses_only_a_valid_recorded_installation() {
    let root = tempfile::tempdir().unwrap();
    let receipt = root.path().join("selector");
    let destination = |version: &str| root.path().join(version);
    fixture(&destination("1.2.3"), "1.2.3", "dist/index.js");
    cache::write_receipt(&receipt, "1.2.3").unwrap();
    let notices = std::cell::RefCell::new(Vec::new());
    let notice = |text: &str| notices.borrow_mut().push(text.to_owned());
    let context = PreparationContext::new(&|| false, &notice);
    let installed = prepare_or_cached(&adapter(), &receipt, &destination, &context, || {
        Err(PreparationError::new(
            PreparationKind::Network,
            "registry offline",
        ))
    })
    .unwrap();
    assert!(installed.entry.is_file());
    assert!(notices.borrow()[0].contains("1.2.3"));
    assert!(
        cache::exclusive_lease(&destination("1.2.3"))
            .unwrap()
            .is_none()
    );
    drop(installed);
    fs::remove_file(
        destination("1.2.3")
            .join(NODE_MODULES)
            .join(adapter().name)
            .join("dist/index.js"),
    )
    .unwrap();
    assert!(
        prepare_or_cached(&adapter(), &receipt, &destination, &context, || Err(
            PreparationError::new(PreparationKind::Network, "offline")
        ))
        .is_err()
    );
}

#[test]
fn cancellation_integrity_and_invalid_versions_never_fall_back() {
    let root = tempfile::tempdir().unwrap();
    let receipt = root.path().join("selector");
    let destination = |version: &str| root.path().join(version);
    fixture(&destination("1.2.3"), "1.2.3", "dist/index.js");
    cache::write_receipt(&receipt, "1.2.3").unwrap();
    for kind in [
        PreparationKind::Canceled,
        PreparationKind::Integrity,
        PreparationKind::Configuration,
        PreparationKind::InvalidPackage,
    ] {
        let result = prepare_or_cached(
            &adapter(),
            &receipt,
            &destination,
            &PreparationContext::default(),
            || Err(PreparationError::new(kind, "failed")),
        );
        assert!(matches!(result, Err(error) if error.kind == kind));
    }
    assert_eq!(fs::read_to_string(receipt).unwrap(), "1.2.3");
}

#[test]
fn refuses_to_replace_a_damaged_installation_that_is_still_in_use() {
    let root = tempfile::tempdir().unwrap();
    let dest = root.path().join("version");
    fixture(&dest, "wrong", "dist/index.js");
    let _lease = InstallationLease::acquire(&dest).unwrap();
    let error = install_at(&adapter(), "1.2.3", &dest, |staging| {
        fixture(staging, "1.2.3", "dist/index.js");
        Ok(())
    })
    .unwrap_err();
    assert!(error.to_string().contains("still in use"));
    assert!(dest.is_dir());
}
