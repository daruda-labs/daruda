use super::*;

#[test]
fn reports_exit_stderr_without_echoing_environment() {
    let error = output(
        Command::new(test_process::executable())
            .args(["--stderr", "broken", "--exit", "126"])
            .env("TEST_SECRET", "hidden"),
        &PreparationContext::default(),
    )
    .unwrap_err();
    assert!(error.detail.contains("126"));
    assert!(error.detail.contains("broken"));
    assert!(!error.detail.contains("hidden"));
}

#[test]
fn hung_command_is_bounded() {
    let error = output_with_timeout(
        Command::new(test_process::executable()).args(["--sleep-ms", "10000"]),
        &PreparationContext::default(),
        Duration::from_millis(30),
    )
    .unwrap_err();
    assert_eq!(error.kind, PreparationKind::Timeout);
}

#[test]
fn cancellation_interrupts_a_running_process() {
    let started = Instant::now();
    let canceled = || started.elapsed() > Duration::from_millis(30);
    let context = PreparationContext::new(&canceled, &|_| {});
    let error = output(
        Command::new(test_process::executable()).args(["--sleep-ms", "10000"]),
        &context,
    )
    .unwrap_err();
    assert_eq!(error.kind, PreparationKind::Canceled);
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn classifies_structured_npm_errors() {
    for (code, kind) in [
        ("ENOTFOUND", PreparationKind::Network),
        ("EINTEGRITY", PreparationKind::Integrity),
        ("ETARGET", PreparationKind::Configuration),
    ] {
        let json = serde_json::json!({"error": {"code": code}});
        let error = output(
            Command::new(test_process::executable()).args([
                "--stdout",
                &json.to_string(),
                "--exit",
                "1",
            ]),
            &PreparationContext::default(),
        )
        .unwrap_err();
        assert_eq!(error.kind, kind);
    }
}

#[test]
fn output_is_bounded_while_the_child_runs() {
    let error = output(
        Command::new(test_process::executable()).arg("--flood"),
        &PreparationContext::default(),
    )
    .unwrap_err();
    assert_eq!(error.kind, PreparationKind::Process);
    assert!(error.detail.contains("limit"));
}

#[test]
fn stderr_keeps_only_a_bounded_tail() {
    let bytes = smol::block_on(capture(
        smol::io::Cursor::new(vec![b'x'; OUTPUT_LIMIT * 4]),
        true,
    ))
    .unwrap();
    assert_eq!(bytes.len(), OUTPUT_LIMIT);
}

#[cfg(unix)]
#[test]
fn timeout_kills_descendants_before_they_can_keep_writing() {
    let root = tempfile::tempdir().unwrap();
    let marker = root.path().join("orphan-wrote");
    let quoted = shell_words::quote(marker.to_str().unwrap());
    let script = format!("(sleep 0.4; printf orphan > {quoted}) & wait");
    let error = output_with_timeout(
        Command::new("/bin/sh").args(["-c", &script]),
        &PreparationContext::default(),
        Duration::from_millis(50),
    )
    .unwrap_err();
    assert_eq!(error.kind, PreparationKind::Timeout);
    std::thread::sleep(Duration::from_millis(500));
    assert!(!marker.exists(), "the installer child survived its timeout");
}
