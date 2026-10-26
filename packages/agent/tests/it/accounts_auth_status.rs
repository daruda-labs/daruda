//! Public API tests for accounts auth status.

use daruda_agent::accounts::CliCommand;
use daruda_agent::accounts::auth_status::*;

/// The exact payload `claude auth status --json` produced for a
/// subscription login.
const SUBSCRIPTION: &str = r#"{
    "loggedIn": true,
    "authMethod": "claude.ai",
    "apiProvider": "firstParty",
    "email": "a@x.com",
    "orgId": "org-1",
    "orgName": "Acme",
    "subscriptionType": "team"
}"#;

#[test]
fn the_captured_payload_reports_how_the_user_signed_in() {
    let status = parse_auth_status(SUBSCRIPTION).expect("parses");
    assert!(status.logged_in);
    assert_eq!(status.auth_method.as_deref(), Some("claude.ai"));
    assert_eq!(status.api_provider.as_deref(), Some("firstParty"));
    assert_eq!(status.subscription_type.as_deref(), Some("team"));
    assert_eq!(status.organization.as_deref(), Some("Acme"));
    assert!(status.names_a_method());
}

/// What a config dir with no credentials answers. `none` is an absence,
/// not a method — showing it verbatim would read as a third kind of login.
#[test]
fn a_signed_out_config_dir_names_no_method() {
    let status = parse_auth_status(r#"{"loggedIn": false, "authMethod": "none"}"#).expect("parses");
    assert!(!status.logged_in);
    assert_eq!(status.auth_method, None);
    assert!(!status.names_a_method());
}

/// The value set is the provider's to grow. An unfamiliar method has to
/// survive verbatim — mapping to a known set would report a metered login
/// as unknown, which is the one mistake that costs the user money.
#[test]
fn an_unfamiliar_method_survives_verbatim() {
    let status =
        parse_auth_status(r#"{"loggedIn": true, "authMethod": "console"}"#).expect("parses");
    assert_eq!(status.auth_method.as_deref(), Some("console"));
}

/// A CLI too old to report the method is signed in but unreadable — which
/// is not the same as signed out.
#[test]
fn a_reading_without_a_method_is_still_a_reading() {
    let status = parse_auth_status(r#"{"loggedIn": true}"#).expect("parses");
    assert!(status.logged_in);
    assert!(!status.names_a_method());
}

/// Every form the codex CLI is built to print, taken from its own binary.
/// The one that matters is the API-key line — a metered sign-in reported as
/// unknown is the mistake that costs money.
#[test]
fn the_prose_forms_report_the_method_they_name() {
    for (line, expected) in [
        ("Logged in using ChatGPT", Some("ChatGPT")),
        ("Logged in using an API key - sk-…", Some("an API key")),
        (
            "Logged in using Amazon Bedrock API key",
            Some("Amazon Bedrock API key"),
        ),
        (
            "Logged in using personal access token",
            Some("personal access token"),
        ),
    ] {
        let status = parse_prose_auth_status(line).expect("parses");
        assert!(status.logged_in, "{line}");
        assert_eq!(status.auth_method.as_deref(), expected, "{line}");
    }
}

#[test]
fn the_prose_signed_out_form_is_a_reading_too() {
    let status = parse_prose_auth_status("Not logged in").expect("parses");
    assert!(!status.logged_in);
    assert!(!status.names_a_method());
}

/// A form this build has never seen still has to survive — the set is the
/// provider's to grow.
#[test]
fn an_unfamiliar_prose_form_survives_verbatim() {
    let status = parse_prose_auth_status("Logged in using Something New").expect("parses");
    assert_eq!(status.auth_method.as_deref(), Some("Something New"));
}

/// The line is found among whatever else the CLI printed.
#[test]
fn the_prose_line_is_found_among_other_output() {
    let status =
        parse_prose_auth_status("checking...\n  Logged in using ChatGPT\ndone").expect("parses");
    assert_eq!(status.auth_method.as_deref(), Some("ChatGPT"));
}

#[test]
fn prose_with_no_status_line_is_not_a_reading() {
    assert_eq!(parse_prose_auth_status("some unrelated output"), None);
}

#[test]
fn a_non_object_payload_is_not_a_reading() {
    for raw in ["", "not json", "[]", "\"a\"", "null"] {
        assert_eq!(parse_auth_status(raw), None, "{raw:?}");
    }
}

#[test]
fn unknown_fields_are_ignored() {
    let status = parse_auth_status(r#"{"loggedIn": true, "somethingNew": 1}"#).expect("parses");
    assert!(status.logged_in);
}

fn from_line(line: &str) -> daruda_agent::accounts::CliCommand {
    let mut words = shell_words::split(line).expect("a well-formed test command");
    daruda_agent::accounts::CliCommand {
        program: words.remove(0).into(),
        args: words,
        env: Vec::new(),
    }
}

/// An unknown field must not sink the whole reading.
#[cfg(unix)]
#[test]
fn a_real_probe_reads_what_the_command_printed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("status ' $USER `echo expanded`.json");
    std::fs::write(&path, SUBSCRIPTION).expect("fixture");
    let status = read_auth_status(
        &from_line(&format!(
            "/bin/cat {}",
            daruda_core::shell::quote::quote_path(&path, daruda_core::shell::Shell::Posix)
        )),
        AuthStatusFormat::Json,
        std::time::Duration::from_secs(10),
    )
    .expect("the probe parses what the command printed");
    assert!(status.logged_in);
    assert_eq!(status.auth_method.as_deref(), Some("claude.ai"));
}

/// A probe that never answers must not hold the caller — and must not
/// leave the tree it spawned behind either.
/// The reading is not always on stdout — the codex CLI puts its status line
/// on stderr. Reading only stdout reported that whole domain as having no
/// sign-in method, which the display cannot tell from signed out.
#[cfg(unix)]
#[test]
fn a_reading_on_stderr_is_still_found() {
    let status = read_auth_status(
        &from_line("/bin/sh -c 'echo Logged in using ChatGPT 1>&2'"),
        AuthStatusFormat::Prose,
        std::time::Duration::from_secs(10),
    )
    .expect("the probe reads the stream the CLI actually used");
    assert_eq!(status.auth_method.as_deref(), Some("ChatGPT"));
}

/// stdout wins when both carry something, so a wrapper's chatter on stderr
/// cannot displace a real payload.
#[cfg(unix)]
#[test]
fn stdout_wins_over_stderr() {
    let status = read_auth_status(
        &from_line(
            "/bin/sh -c 'echo Logged in using ChatGPT; echo Logged in using an API key 1>&2'",
        ),
        AuthStatusFormat::Prose,
        std::time::Duration::from_secs(10),
    )
    .expect("parses");
    assert_eq!(status.auth_method.as_deref(), Some("ChatGPT"));
}

#[cfg(unix)]
#[test]
fn a_hanging_probe_gives_up() {
    let started = std::time::Instant::now();
    let status = read_auth_status(
        &from_line("/bin/sh -c 'sleep 30'"),
        AuthStatusFormat::Json,
        std::time::Duration::from_millis(200),
    );
    assert_eq!(status, None);
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}

#[cfg(unix)]
#[test]
fn descendant_pipes_do_not_outlive_the_probe_deadline() {
    let started = std::time::Instant::now();
    let status = read_auth_status(
        &from_line("/bin/sh -c 'sleep 2 & echo Logged in using ChatGPT'"),
        AuthStatusFormat::Prose,
        std::time::Duration::from_millis(200),
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    assert_eq!(status, None);
}

#[cfg(unix)]
#[test]
fn a_timed_out_pipe_owner_is_terminated_even_after_its_parent_exits() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("survived ' $USER `probe`");
    let quoted = daruda_core::shell::quote::quote_path(&marker, daruda_core::shell::Shell::Posix);
    let command = CliCommand {
        program: "/bin/sh".into(),
        args: vec![
            "-c".into(),
            format!("(sleep 1; printf survived > {quoted}) & echo Logged in using ChatGPT"),
        ],
        env: Vec::new(),
    };
    assert_eq!(
        read_auth_status(
            &command,
            AuthStatusFormat::Prose,
            std::time::Duration::from_millis(200),
        ),
        None,
    );
    std::thread::sleep(std::time::Duration::from_millis(1200));
    assert!(
        !marker.exists(),
        "the inherited pipe must not keep its owner alive"
    );
}

#[cfg(unix)]
#[test]
fn excessive_stdout_does_not_block_a_stderr_reading() {
    let status = read_auth_status(
        &from_line("/bin/sh -c 'head -c 131072 /dev/zero; echo Logged in using ChatGPT 1>&2'"),
        AuthStatusFormat::Prose,
        std::time::Duration::from_secs(1),
    )
    .expect("stderr is drained while excessive stdout is rejected");
    assert_eq!(status.auth_method.as_deref(), Some("ChatGPT"));
}

#[cfg(unix)]
#[test]
fn excessive_output_does_not_break_the_process_before_its_reading() {
    let command = CliCommand {
        program: "/bin/sh".into(),
        args: vec![
            "-c".into(),
            r#"i=0; while [ "$i" -lt 128 ]; do printf '%01024d' 0; i=$((i+1)); done; echo Logged in using ChatGPT >&2"#.into(),
        ],
        env: Vec::new(),
    };
    let status = read_auth_status(
        &command,
        AuthStatusFormat::Prose,
        std::time::Duration::from_secs(5),
    )
    .expect("discarding excess stdout must let the same process report on stderr");
    assert_eq!(status.auth_method.as_deref(), Some("ChatGPT"));
}

#[test]
fn a_command_that_does_not_exist_is_not_a_reading() {
    assert_eq!(
        read_auth_status(
            &from_line("/nonexistent/daruda-probe"),
            AuthStatusFormat::Json,
            std::time::Duration::from_secs(1)
        ),
        None
    );
}
