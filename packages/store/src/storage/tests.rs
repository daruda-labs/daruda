use super::*;

fn layout(profile: &str, root: Option<PathBuf>) -> StorageLayout {
    StorageLayout::resolve(
        Path::new("config"),
        Some(PathBuf::from("native-logs").join(application_name(profile))),
        Some(PathBuf::from("native-state").join(application_name(profile))),
        Some(Path::new("home")),
        root,
        profile,
    )
}

#[test]
fn data_and_resource_locations_remain_backward_compatible() {
    let release = layout("release", None);
    let debug = layout("debug", None);
    assert_eq!(release.data(), PathBuf::from("config/daruda"));
    assert_eq!(debug.data(), PathBuf::from("config/daruda-debug"));
    assert_eq!(release.node_install(), debug.node_install());
    assert_eq!(release.flow_locks(), debug.flow_locks());
    assert_eq!(release.remote_locks(), debug.remote_locks());
    assert_ne!(release.logs(), debug.logs());
    assert_ne!(
        release.workspace_state().unwrap(),
        debug.workspace_state().unwrap()
    );
}

#[test]
fn override_contains_all_logs_without_reading_default_logs() {
    for profile in ["release", "debug", "preview"] {
        let storage = layout(profile, Some("portable".into()));
        assert_eq!(storage.data(), PathBuf::from("portable"));
        assert_eq!(storage.node_install(), PathBuf::from("portable/node"));
        assert_eq!(storage.flow_locks(), PathBuf::from("portable/flow-locks"));
        assert_eq!(
            storage.remote_locks(),
            PathBuf::from("portable/remote-locks")
        );
        assert_eq!(storage.logs(), Some(PathBuf::from("portable/logs")));
        assert_eq!(
            storage.workspace_state().unwrap(),
            PathBuf::from("portable/state/workspace")
        );
        assert_eq!(
            storage.diagnostic_sources(),
            [crate::observability::diagnostics::LogSource::Current(
                PathBuf::from("portable/logs")
            )]
        );
    }
}

#[test]
fn different_override_roots_isolate_diagnostics() {
    let first = layout("debug", Some("first".into()));
    let second = layout("debug", Some("second".into()));
    assert_ne!(first.logs(), second.logs());
    assert_ne!(first.diagnostic_sources(), second.diagnostic_sources());
}

#[test]
fn native_logs_group_profiles_under_one_application_directory() {
    for root in [
        "local/daruda/logs",
        "home/Library/Logs/daruda",
        "state/daruda/logs",
    ] {
        let root = PathBuf::from(root);
        for profile in ["release", "debug", "preview"] {
            let storage = layout(profile, None).with_profile_logs(Some(root.clone()), profile);
            assert_eq!(storage.logs(), Some(root.join(profile)));
            assert_eq!(storage.data(), layout(profile, None).data());
            assert_eq!(
                storage.diagnostic_sources(),
                [
                    crate::observability::diagnostics::LogSource::Current(root.join(profile)),
                    crate::observability::diagnostics::LogSource::Compatibility(
                        PathBuf::from("home/.daruda/logs").join(profile)
                    ),
                    crate::observability::diagnostics::LogSource::Compatibility(
                        PathBuf::from("native-logs").join(application_name(profile))
                    ),
                ]
            );
        }
    }
}

#[test]
fn profile_logs_cannot_escape_the_application_directory() {
    for profile in ["", ".", "..", "../other", "/other", "nested/profile"] {
        let storage =
            layout("debug", None).with_profile_logs(Some(PathBuf::from("logs/daruda")), profile);
        assert!(storage.logs().is_none(), "{profile}");
    }
    assert!(
        layout("debug", None)
            .with_profile_logs(None, "debug")
            .logs()
            .is_none()
    );
}

#[test]
fn legacy_logs_are_read_only_diagnostic_sources() {
    let storage = layout("preview", None);
    assert_eq!(
        storage.logs(),
        Some(PathBuf::from("native-logs/daruda-preview"))
    );
    assert_eq!(
        storage.diagnostic_sources(),
        [
            crate::observability::diagnostics::LogSource::Current(PathBuf::from(
                "native-logs/daruda-preview"
            )),
            crate::observability::diagnostics::LogSource::Compatibility(PathBuf::from(
                "home/.daruda/logs/preview"
            )),
        ]
    );
}

#[test]
fn unavailable_native_logs_do_not_fall_back_to_legacy_writes() {
    let storage = StorageLayout::resolve(Path::new("config"), None, None, None, None, "release");
    assert!(storage.logs().is_none());
    assert!(storage.diagnostic_sources().is_empty());
    let isolated = StorageLayout::resolve(
        Path::new("config"),
        None,
        None,
        None,
        Some("isolated".into()),
        "release",
    );
    assert_eq!(isolated.logs(), Some(PathBuf::from("isolated/logs")));
}

#[test]
fn override_trims_empty_values_and_fixes_relative_paths_once() {
    assert!(override_root(None).is_none());
    assert!(override_root(Some("  ".into())).is_none());
    let root = override_root(Some("  test-state  ".into())).unwrap();
    assert_eq!(root, std::env::current_dir().unwrap().join("test-state"));
}

#[test]
fn resolving_storage_does_not_create_or_move_directories() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("not-created");
    let override_path = override_root(Some(root.clone().into_os_string())).unwrap();
    let storage = layout("preview", Some(override_path));
    assert_eq!(storage.logs(), Some(root.join("logs")));
    assert!(!root.exists());
}
