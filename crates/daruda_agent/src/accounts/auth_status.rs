//! What the agent CLI says about the credentials it is currently using.
//!
//! Read by asking the CLI itself (`auth status --json`) rather than by
//! inspecting the credential store, because the two answer different
//! questions. The store says whether *an* OAuth blob is present; the CLI says
//! **which way** the user signed in — and that is the fact daruda cannot
//! otherwise obtain: the ACP adapter never forwards it, and a login the user
//! performed themselves leaves no record daruda wrote.
//!
//! It also answers scoped and ambient credentials with one mechanism. The
//! store reader cannot: a Claude account's Keychain service name is derived
//! from its config dir, so the ambient home has no scoped item and always
//! reads as absent (see `AccountRecipe::has_credentials`).
//!
//! `auth_method` and the rest are pass-through strings, deliberately not
//! mapped to an enum — the same reasoning as
//! [`PlanInfo`](super::credentials::PlanInfo): providers add values without
//! notice, and a stale enum would report a metered login as unknown.

/// How a domain's CLI reports its status, and the arguments that ask for it.
///
/// Paired rather than two fields: arguments whose output format is unknown are
/// useless, and a format with no arguments has nothing to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthStatusProbe {
    /// Suffix appended to the agent's launch command.
    pub args: &'static str,
    pub format: AuthStatusFormat,
}

/// What the CLI prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthStatusFormat {
    /// A JSON object — the richest form, carrying identity and plan too.
    Json,
    /// One human sentence. Carries the method and nothing else, so a domain
    /// reporting this way fills in less.
    Prose,
}

/// One `auth status` reading.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthStatus {
    /// Whether the CLI considers these credentials usable at all.
    pub logged_in: bool,
    /// How the user signed in, verbatim — e.g. `claude.ai` for a subscription
    /// login. Absent on a CLI too old to report it.
    pub auth_method: Option<String>,
    /// Which API backend is in play, verbatim — e.g. `firstParty`.
    pub api_provider: Option<String>,
    /// Plan tier, verbatim — e.g. `team`.
    pub subscription_type: Option<String>,
    pub email: Option<String>,
    pub organization: Option<String>,
}

impl AuthStatus {
    /// Whether this reading names a sign-in method at all. A CLI too old to
    /// report one leaves the host with nothing to show, which is different
    /// from being signed out.
    #[must_use]
    pub fn names_a_method(&self) -> bool {
        self.auth_method.is_some()
    }
}

/// Parse a reading in `format`.
#[must_use]
pub fn parse_status(raw: &str, format: AuthStatusFormat) -> Option<AuthStatus> {
    match format {
        AuthStatusFormat::Json => parse_auth_status(raw),
        AuthStatusFormat::Prose => parse_prose_auth_status(raw),
    }
}

/// Prefix the prose form puts before the method it used.
const PROSE_SIGNED_IN: &str = "Logged in using ";

/// Marker for a CLI that reports being signed out.
const PROSE_SIGNED_OUT: &str = "Not logged in";

/// Parse a one-sentence status line.
///
/// The method is whatever follows the prefix, up to a trailing detail the CLI
/// appends after a dash — taken verbatim rather than matched against the forms
/// seen so far (`ChatGPT`, `an API key`, `Amazon Bedrock API key`, `personal
/// access token`), for the same reason the JSON form is: the value set is the
/// provider's to grow, and an unmatched metered login must not read as
/// unknown.
#[must_use]
pub fn parse_prose_auth_status(raw: &str) -> Option<AuthStatus> {
    let line = raw
        .lines()
        .map(str::trim)
        .find(|l| l.contains(PROSE_SIGNED_IN) || l.contains(PROSE_SIGNED_OUT))?;
    if let Some(rest) = line.split_once(PROSE_SIGNED_IN).map(|(_, rest)| rest) {
        let method = rest.split(" - ").next().unwrap_or(rest).trim();
        return Some(AuthStatus {
            logged_in: true,
            auth_method: (!method.is_empty()).then(|| method.to_owned()),
            ..AuthStatus::default()
        });
    }
    Some(AuthStatus::default())
}

/// Parse one `auth status --json` payload.
///
/// Every field is optional: this is a diagnostic surface, not a contract, and
/// a CLI that drops or renames one must degrade to "unknown" rather than to a
/// wrong answer. `None` only when the payload is not a JSON object at all.
#[must_use]
pub fn parse_auth_status(raw: &str) -> Option<AuthStatus> {
    let v: serde_json::Value = serde_json::from_str(raw.trim()).ok()?;
    if !v.is_object() {
        return None;
    }
    Some(AuthStatus {
        logged_in: v["loggedIn"].as_bool().unwrap_or(false),
        auth_method: field(&v, &["authMethod"]),
        api_provider: field(&v, &["apiProvider"]),
        subscription_type: field(&v, &["subscriptionType"]),
        email: field(&v, &["email"]),
        // Both spellings have been seen; the name is what a user reads.
        organization: field(&v, &["organizationName", "orgName"]),
    })
}

/// First of `names` present as a non-blank string.
///
/// `"none"` is filtered out with the blanks: the CLI uses it for "no method"
/// on a signed-out config dir, which is an absence, not a method to display.
fn field(v: &serde_json::Value, names: &[&str]) -> Option<String> {
    names
        .iter()
        .filter_map(|n| v[*n].as_str())
        .map(str::trim)
        .find(|s| !s.is_empty() && *s != "none")
        .map(str::to_string)
}

/// Tear down a probe and everything it forked.
///
/// The command routinely forks (`npx` → the CLI), and the child leads its own
/// group, so its pid doubles as a group id. Reaped afterwards so the handle is
/// not dropped on a live child.
fn kill_probe_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let pgid = child.id() as libc::pid_t;
        // SAFETY: the child led its own group from the spawn above and has not
        // been reaped, so the pid is still this process's to name and the
        // negative form reaches that group alone.
        unsafe {
            libc::kill(-pgid, libc::SIGKILL);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// Ask the agent CLI for its current auth status.
///
/// **Blocking**, and it spawns a process — the command is the agent's launch
/// line, which for a default install goes through `npx`. Callers must run this
/// off the UI thread, the same rule `spawn_login` carries.
///
/// Every failure collapses to `None`: this is a display and diagnostic
/// surface, and a probe that cannot answer must leave the host showing nothing
/// rather than asserting a wrong sign-in method.
pub fn read_auth_status(
    command: &str,
    format: AuthStatusFormat,
    inject_env: &[(String, String)],
    strip_env: &[&str],
    timeout: std::time::Duration,
) -> Option<AuthStatus> {
    let tokens = shell_words::split(command).ok()?;
    let (program, args) = tokens.split_first()?;

    let mut cmd = std::process::Command::new(program);
    cmd.args(args);
    for name in strip_env {
        cmd.env_remove(name);
    }
    for (name, value) in inject_env {
        cmd.env(name, value);
    }
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    // Both streams: the reading is not always on stdout. The codex CLI prints
    // its status line to stderr, so discarding it left that domain reporting
    // nothing at all — indistinguishable from being signed out.
    cmd.stderr(std::process::Stdio::piped());
    // Own group, so a probe that hangs can be torn down whole — the command
    // routinely forks (`npx` → the CLI), exactly as the login path does.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        cmd.process_group(0);
    }

    let mut child = cmd.spawn().ok()?;
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {}
            // Cannot tell whether it exited, so it may still be running:
            // tear it down rather than dropping the handle on a live child,
            // which would leave the tree — and its pid — behind.
            Err(_) => {
                kill_probe_tree(&mut child);
                return None;
            }
        }
        if std::time::Instant::now() >= deadline {
            kill_probe_tree(&mut child);
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    // Both payloads are a line or two, well under the pipe buffers, so the
    // child cannot have blocked on a full pipe before exiting above.
    let out = child.wait_with_output().ok()?;
    // stdout first: that is where a JSON payload lands, and where `npx`'s own
    // chatter does *not* go — parsing the streams merged would let a warning
    // line break the JSON.
    parse_status(&String::from_utf8_lossy(out.stdout.as_slice()), format)
        .or_else(|| parse_status(&String::from_utf8_lossy(out.stderr.as_slice()), format))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let status =
            parse_auth_status(r#"{"loggedIn": false, "authMethod": "none"}"#).expect("parses");
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
        let status = parse_prose_auth_status("checking...\n  Logged in using ChatGPT\ndone")
            .expect("parses");
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

    /// An unknown field must not sink the whole reading.
    #[cfg(unix)]
    #[test]
    fn a_real_probe_reads_what_the_command_printed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("status.json");
        std::fs::write(&path, SUBSCRIPTION).expect("fixture");
        let status = read_auth_status(
            &format!("/bin/cat {}", path.display()),
            AuthStatusFormat::Json,
            &[],
            &[],
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
            "/bin/sh -c 'echo Logged in using ChatGPT 1>&2'",
            AuthStatusFormat::Prose,
            &[],
            &[],
            std::time::Duration::from_secs(10),
        )
        .expect("the probe reads the stream the CLI actually used");
        assert_eq!(status.auth_method.as_deref(), Some("ChatGPT"));
    }

    /// stdout wins when both carry something, so `npx`'s chatter on stderr
    /// cannot displace a real payload.
    #[cfg(unix)]
    #[test]
    fn stdout_wins_over_stderr() {
        let status = read_auth_status(
            "/bin/sh -c 'echo Logged in using ChatGPT; echo Logged in using an API key 1>&2'",
            AuthStatusFormat::Prose,
            &[],
            &[],
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
            "/bin/sh -c 'sleep 30'",
            AuthStatusFormat::Json,
            &[],
            &[],
            std::time::Duration::from_millis(200),
        );
        assert_eq!(status, None);
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }

    #[test]
    fn a_command_that_does_not_exist_is_not_a_reading() {
        assert_eq!(
            read_auth_status(
                "/nonexistent/daruda-probe",
                AuthStatusFormat::Json,
                &[],
                &[],
                std::time::Duration::from_secs(1)
            ),
            None
        );
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let status = parse_auth_status(r#"{"loggedIn": true, "somethingNew": 1}"#).expect("parses");
        assert!(status.logged_in);
    }
}
