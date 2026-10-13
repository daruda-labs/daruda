//! Public API tests for accounts auth status.

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
