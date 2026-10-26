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

mod probe;

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
    let (stdout, stderr) = probe::output(command, timeout)?;
    // stdout has priority; parsing streams separately keeps wrapper chatter
    // on stderr from corrupting a JSON payload.
    parse_status(&String::from_utf8_lossy(&stdout), format)
        .or_else(|| parse_status(&String::from_utf8_lossy(&stderr), format))
}
