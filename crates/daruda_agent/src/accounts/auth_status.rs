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

/// One `auth status --json` reading.
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
    cmd.stderr(std::process::Stdio::null());
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
            Err(_) => return None,
        }
        if std::time::Instant::now() >= deadline {
            #[cfg(unix)]
            {
                let pgid = child.id() as libc::pid_t;
                // SAFETY: the child leads its own group (set above) and is not
                // reaped — `try_wait` said so on this iteration — so the pid is
                // still ours and the negative form reaches that group alone.
                unsafe {
                    libc::kill(-pgid, libc::SIGKILL);
                }
            }
            let _ = child.kill();
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    // The payload is a few hundred bytes, well under the pipe buffer, so the
    // child cannot have blocked on a full stdout before exiting above.
    let out = child.wait_with_output().ok()?;
    parse_auth_status(&String::from_utf8_lossy(out.stdout.as_slice()))
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
    #[cfg(unix)]
    #[test]
    fn a_hanging_probe_gives_up() {
        let started = std::time::Instant::now();
        let status = read_auth_status(
            "/bin/sh -c 'sleep 30'",
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
