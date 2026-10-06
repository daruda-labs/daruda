use super::*;

#[cfg(unix)]
fn runtime(directory: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt as _;
    let node = node_binary(directory);
    std::fs::create_dir_all(node.parent().unwrap()).unwrap();
    std::fs::write(&node, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(node, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn canceled_cache_probe_is_not_a_missing_runtime() {
    let directory = tempfile::tempdir().unwrap();
    let context = PreparationContext::new(&|| true, &|_| {});
    assert!(matches!(
        managed_cache_valid_with_context(directory.path(), &context),
        Err(NodeError::Canceled)
    ));
    assert!(
        !managed_cache_valid_with_context(directory.path(), &PreparationContext::default())
            .unwrap()
    );
}

#[cfg(unix)]
#[test]
fn cancellation_during_publication_probe_preserves_the_shared_runtime() {
    let root = tempfile::tempdir().unwrap();
    let installed = root.path().join("installed");
    let extracted = root.path().join("extracted");
    let marker = root.path().join("probe-started");
    runtime(
        &installed,
        &format!(
            ": > {}\nexec /bin/sleep 60",
            shell_words::quote(marker.to_str().unwrap())
        ),
    );
    runtime(&extracted, "exit 0");
    let original = std::fs::read(node_binary(&installed)).unwrap();
    let canceled = || marker.exists();
    let context = PreparationContext::new(&canceled, &|_| {});

    assert!(matches!(
        publish_extracted(&extracted, &installed, &context),
        Err(NodeError::Canceled)
    ));
    assert!(marker.exists(), "cancellation must happen inside the probe");
    assert_eq!(std::fs::read(node_binary(&installed)).unwrap(), original);
    assert!(node_binary(&extracted).is_file());
}

#[cfg(unix)]
#[test]
fn publication_keeps_a_runtime_published_by_another_process() {
    let root = tempfile::tempdir().unwrap();
    let installed = root.path().join("installed");
    let extracted = root.path().join("extracted");
    runtime(&installed, "exit 0 # shared");
    runtime(&extracted, "exit 0 # candidate");
    let original = std::fs::read(node_binary(&installed)).unwrap();
    publish_extracted(&extracted, &installed, &PreparationContext::default()).unwrap();
    assert_eq!(std::fs::read(node_binary(&installed)).unwrap(), original);
    assert!(node_binary(&extracted).is_file());
}
