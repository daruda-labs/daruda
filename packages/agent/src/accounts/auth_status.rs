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

use daruda_store::observability::{error_report::ErrorReport, log_writer::LogWriter};

/// How a domain's CLI reports its status, and the arguments that ask for it.
///
/// Paired rather than two fields: arguments whose output format is unknown are
/// useless, and a format with no arguments has nothing to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthStatusProbe {
    /// CLI words run on the prepared adapter.
    pub args: &'static [&'static str],
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
/// The command routinely forks (adapter → the CLI), and the child leads its own
/// group, so its pid doubles as a group id. Reaped afterwards so the handle is
/// not dropped on a live child.
///
/// Only call this where the child is known unreaped — the deadline path. A
/// caller that cannot tell must kill the direct child instead; see the
/// contract on [`daruda_core::process::Group::adopt`].
fn kill_probe_tree(group: &daruda_core::process::Group, child: &mut std::process::Child) {
    // Unreaped — the `wait` below is the reap — so the pid is still this
    // process's to name, which is what the group requires.
    if let Err(error) = group.try_terminate_child(child) {
        LogWriter::log(
            ErrorReport::new("Failed to terminate authentication probe tree")
                .from_error(&error)
                .at(file!(), line!())
                .dedup("account.auth_probe.termination")
                .build(),
        );
    }
}

/// Ask the agent CLI for its current auth status.
///
/// **Blocking**, and it spawns a process. Callers must run this off the UI
/// thread, the same rule `spawn_login` carries. `timeout` bounds only this
/// run: the caller prepares `command` (any install included) beforehand.
///
/// Every failure collapses to `None`: this is a display and diagnostic
/// surface, and a probe that cannot answer must leave the host showing nothing
/// rather than asserting a wrong sign-in method.
pub fn read_auth_status(
    command: &super::CliCommand,
    format: AuthStatusFormat,
    timeout: std::time::Duration,
) -> Option<AuthStatus> {
    let mut cmd = command.command();
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    // Both streams: the reading is not always on stdout. The codex CLI prints
    // its status line to stderr, so discarding it left that domain reporting
    // nothing at all — indistinguishable from being signed out.
    cmd.stderr(std::process::Stdio::piped());
    // Own group, so a probe that hangs can be torn down whole — the command
    // routinely forks (adapter → the CLI), exactly as the login path does.
    daruda_core::process::lead_own_group(&mut cmd);

    let mut child = cmd.spawn().ok()?;
    let group = match daruda_core::process::Group::try_adopt(child.id()) {
        Ok(group) => group,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            LogWriter::log(
                ErrorReport::new("Cannot own authentication probe process")
                    .from_error(&error)
                    .at(file!(), line!())
                    .build(),
            );
            return None;
        }
    };
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {}
            // Cannot tell whether it exited, so it may still be running:
            // tear it down rather than dropping the handle on a live child.
            //
            // The direct child only. `try_wait` most often fails with ECHILD,
            // meaning the child was already reaped — and the group requires
            // an unreaped pid precisely because a reaped one may already name
            // someone else's group.
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
        if std::time::Instant::now() >= deadline {
            kill_probe_tree(&group, &mut child);
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    // Both payloads are a line or two, well under the pipe buffers, so the
    // child cannot have blocked on a full pipe before exiting above.
    let out = child.wait_with_output().ok()?;
    // stdout first: that is where a JSON payload lands, and where a wrapper's own
    // chatter does *not* go — parsing the streams merged would let a warning
    // line break the JSON.
    parse_status(&String::from_utf8_lossy(out.stdout.as_slice()), format)
        .or_else(|| parse_status(&String::from_utf8_lossy(out.stderr.as_slice()), format))
}

#[cfg(test)]
mod tests {
    use super::super::cli_command::from_line;
    use super::*;

    /// The exact payload `claude auth status --json` produced for a
    /// subscription login.
    #[cfg(unix)]
    const SUBSCRIPTION: &str = r#"{
        "loggedIn": true,
        "authMethod": "claude.ai",
        "apiProvider": "firstParty",
        "email": "a@x.com",
        "orgId": "org-1",
        "orgName": "Acme",
        "subscriptionType": "team"
    }"#;

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
}
