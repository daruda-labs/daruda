use super::*;

fn location(run_dir: &Path) -> super::super::result::FlowRunLocation {
    super::super::result::FlowRunLocation {
        run_dir: run_dir.to_path_buf(),
        lock_dir: Some(run_dir.join("lock")),
    }
}
#[test]
fn corrupt_logs_are_unreadable_and_removed_directories_are_gone() {
    let dir = tempfile::tempdir().unwrap();

    let path = daruda_flow::run_log::log_file(dir.path());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"broken\nbroken\n").unwrap();
    let snapshot = flow_disk(&location(dir.path()));
    assert!(matches!(snapshot.state,RunState::Unreadable {reason} if !reason.is_empty()));
    assert_eq!(std::fs::read(&path).unwrap(), b"broken\nbroken\n");
    assert!(matches!(
        flow_disk(&location(&dir.path().join("missing"))).state,
        RunState::Gone
    ));
}

#[test]
fn open_logs_use_the_real_lock_holder_and_unknown_identity_is_not_a_crash() {
    let dir = tempfile::tempdir().unwrap();

    let location = location(dir.path());
    let id = dir.path().file_name().unwrap().to_str().unwrap();
    let lock = daruda_flow::lock::RunLock::acquire(
        location.lock_dir.as_ref().unwrap(),
        id,
        &daruda_core::process::is_alive,
    )
    .unwrap();
    assert!(matches!(
        flow_disk(&location).state,
        RunState::Running { .. }
    ));
    lock.release().unwrap();
    let unknown = super::super::result::FlowRunLocation {
        lock_dir: None,
        ..location
    };
    assert!(matches!(flow_disk(&unknown).state, RunState::Unknown));
}

#[test]
fn terminal_output_pointers_follow_engine_run_directory_and_spec_dependencies() {
    let dir = tempfile::tempdir().unwrap();
    let spec = daruda_flow::parse::parse_flow_file("version: 1\nname: A flow\nnodes:\n  - id: first\n    kind: agent\n    prompt: first\n    output: first.md\n  - id: last\n    deps: [first]\n    kind: agent\n    prompt: last\n    output: last.md\n").unwrap();
    let pointers = flow_pointers(dir.path(), Some(&spec), None);
    assert_eq!(pointers.outputs, vec![dir.path().join("last.md")]);
    assert_eq!(
        flow_pointers(dir.path(), Some(&spec), Some(&"first".into())).outputs,
        vec![dir.path().join("first.md")]
    );
    assert_eq!(
        pointers.report,
        Some(dir.path().join(daruda_flow::record::RUN_REPORT_FILE))
    );
}

#[test]
fn review_regression_legacy_selection_preserves_outputs_and_reason() {
    let dir = tempfile::tempdir().unwrap();
    let yaml = "version: 1\nname: Legacy\nnodes:\n  - id: a\n    kind: agent\n    prompt: a\n    output: a.md\n  - id: b\n    deps: [a]\n    kind: agent\n    prompt: b\n    output: b.md\n";
    std::fs::write(dir.path().join("run.yaml"), yaml).unwrap();
    let journal = b"{\"kind\":\"started\",\"v\":1,\"until\":\"a\",\"pinned\":[\"a\"]}\n";
    std::fs::write(dir.path().join("progress.jsonl"), journal).unwrap();
    std::fs::write(dir.path().join("DONE"), "").unwrap();
    let snapshot = flow_disk(&location(dir.path()));
    assert_eq!(snapshot.why.as_deref(), Some("until: a; pinned: a"));
    let RunState::Ended { pointers, .. } = snapshot.state else {
        panic!("ended");
    };
    assert_eq!(pointers.outputs, vec![dir.path().join("a.md")]);
    assert_eq!(
        std::fs::read(dir.path().join("progress.jsonl")).unwrap(),
        journal
    );
    assert!(!daruda_flow::run_log::reader::has_log(dir.path()));
}

#[test]
fn full_legacy_runs_keep_all_terminal_output_pointers() {
    let dir = tempfile::tempdir().unwrap();
    let yaml = "version: 1\nnodes:\n  - id: a\n    kind: agent\n    prompt: a\n    output: a.md\n  - id: b\n    deps: [a]\n    kind: agent\n    prompt: b\n    output: b.md\n  - id: c\n    kind: agent\n    prompt: c\n    output: c.md\n";
    std::fs::write(dir.path().join("run.yaml"), yaml).unwrap();
    std::fs::write(
        dir.path().join("progress.jsonl"),
        "{\"kind\":\"started\",\"v\":1}\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("DONE"), "").unwrap();
    let snapshot = flow_disk(&location(dir.path()));
    assert!(snapshot.why.is_none());
    let RunState::Ended { pointers, .. } = snapshot.state else {
        panic!("ended");
    };
    assert_eq!(
        pointers.outputs,
        vec![dir.path().join("b.md"), dir.path().join("c.md")]
    );
    assert!(!daruda_flow::run_log::reader::has_log(dir.path()));
}

#[test]
fn legacy_selection_survives_a_resume_that_only_opened_the_modern_log() {
    let dir = tempfile::tempdir().unwrap();
    let yaml = "version: 1\nname: Legacy\nnodes:\n  - id: a\n    kind: agent\n    prompt: a\n    output: a.md\n  - id: b\n    deps: [a]\n    kind: agent\n    prompt: b\n    output: b.md\n";
    std::fs::write(dir.path().join("run.yaml"), yaml).unwrap();
    let journal = b"{\"kind\":\"started\",\"v\":1,\"until\":\"a\",\"pinned\":[\"a\"]}\n";
    std::fs::write(dir.path().join("progress.jsonl"), journal).unwrap();
    std::fs::write(dir.path().join("DONE"), "").unwrap();
    let log = daruda_flow::run_log::log_file(dir.path());
    std::fs::create_dir_all(log.parent().unwrap()).unwrap();
    let opened = b"{\"v\":1,\"seq\":1,\"at_ms\":0,\"kind\":\"opened\",\"epoch\":1,\"build\":\"test\",\"schema\":2}\n";
    std::fs::write(&log, opened).unwrap();
    let snapshot = flow_disk(&location(dir.path()));
    assert_eq!(snapshot.why.as_deref(), Some("until: a; pinned: a"));
    let RunState::Ended { pointers, .. } = snapshot.state else {
        panic!("ended");
    };
    assert_eq!(pointers.outputs, vec![dir.path().join("a.md")]);
    assert_eq!(
        std::fs::read(dir.path().join("progress.jsonl")).unwrap(),
        journal
    );
    assert_eq!(std::fs::read(&log).unwrap(), opened);
}
