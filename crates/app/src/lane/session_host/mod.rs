//! Where a lane's agent session attaches: resolving it, assembling the
//! adapter command for it, and validating what the user typed.
//!
//! All three live together because they share one fact — the exact shell
//! quoting [`wrap`] emits, which is
//! [`daruda_config::assemble_launch_command`]'s to decide. The validation
//! rules below are derived from that string and nothing else, so a change to
//! one is a change to the other.
//!
//! GPUI-free (see the `lane/` module doc).

#[cfg(test)]
mod tests;

use daruda_config::{
    AccountEnv, AgentLaunch, LaunchTransport, SessionHostEntry, SessionHostKind,
    SessionHostTombstone, assemble_launch_command,
};
use daruda_store::project::{LaneSessionHost, SessionHostId};

/// Which field of a [`LaneSessionHost`] a [`SessionHostError`] is about, so the
/// UI can point at the right input without parsing a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionHostField {
    /// `Ssh::target`.
    Target,
    /// `Docker::container`.
    Container,
    /// The working directory on the other machine.
    SessionPath,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionHostError {
    /// Nothing left after trimming.
    Empty(SessionHostField),
    /// Holds a character that would escape the quoting [`wrap`] relies on, or
    /// (for a bare word) split it into more than one shell argument.
    Unsafe(SessionHostField),
}

impl std::fmt::Display for SessionHostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let field = match self.field() {
            SessionHostField::Target => "target",
            SessionHostField::Container => "container",
            SessionHostField::SessionPath => "session path",
        };
        match self {
            SessionHostError::Empty(_) => write!(f, "{field} is empty"),
            SessionHostError::Unsafe(_) => write!(f, "{field} has an unusable character"),
        }
    }
}

impl SessionHostError {
    pub fn field(self) -> SessionHostField {
        match self {
            SessionHostError::Empty(field) | SessionHostError::Unsafe(field) => field,
        }
    }

    /// The user-facing reason, so every surface that refuses a host — a form,
    /// a connect, a flow — says the same thing about the same value.
    pub fn localized(self) -> String {
        use crate::surface::strings as s;
        match self {
            Self::Empty(SessionHostField::Target) => s::session_host_err_target_empty(),
            Self::Empty(SessionHostField::Container) => s::session_host_err_container_empty(),
            Self::Empty(SessionHostField::SessionPath) => s::session_host_err_session_path_empty(),
            Self::Unsafe(SessionHostField::Target) => s::session_host_err_target_unsafe(),
            Self::Unsafe(SessionHostField::Container) => s::session_host_err_container_unsafe(),
            Self::Unsafe(SessionHostField::SessionPath) => {
                s::session_host_err_session_path_unsafe()
            }
        }
    }
}

/// A resolved host [`wrap`] must not be handed: the value, and which part of
/// it the quoting cannot carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnusableSessionHost {
    pub host: LaneSessionHost,
    pub reason: SessionHostError,
}

/// Characters a bare shell word may hold. An allowlist rather than a denylist:
/// `target`/`container` land unquoted in [`wrap`]'s output, where anything
/// outside this set either splits the word or is a shell metacharacter.
/// Covers `user@host`, an SSH config alias, a dotted name, an IPv4 literal, a
/// bracketed IPv6 one, and every character Docker allows in a container name.
fn is_bare_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@' | ':' | '[' | ']' | '%')
}

/// Characters that would break out of `wrap`'s `'cd "…" && …'` nesting. A
/// denylist here (unlike a bare word) because a path legitimately holds spaces,
/// parentheses and the like, and the double quotes keep those inert — only
/// these six plus a line break can reach the shell:
/// - `'` ends the outer single-quoted `sh -c` argument
/// - `"` ends the inner quoted path
/// - `` ` ``, `$`, `\` stay special to the remote shell inside double quotes
fn breaks_quoting(c: char) -> bool {
    matches!(c, '\'' | '"' | '`' | '$' | '\\' | '\n' | '\r')
}

/// `pub(crate)` so the Settings session-host registry editor
/// (`settings_window::sections::session_hosts`) can validate a
/// `target`/`container` field the same way [`sanitized_ssh`]/[`sanitized_docker`]
/// do, without going through a full `LaneSessionHost` (which also needs a
/// `session_path` the registry editor has no field for).
pub(crate) fn checked_bare_word(
    value: &str,
    field: SessionHostField,
) -> Result<String, SessionHostError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(SessionHostError::Empty(field));
    }
    if !trimmed.chars().all(is_bare_word_char) {
        return Err(SessionHostError::Unsafe(field));
    }
    Ok(trimmed.to_string())
}

fn checked_session_path(value: &str) -> Result<String, SessionHostError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(SessionHostError::Empty(SessionHostField::SessionPath));
    }
    // A leading `~` looks like home-directory shorthand, but `wrap`'s
    // `cd "…"` always double-quotes the path — POSIX tilde expansion only
    // applies to an *unquoted* leading `~`, so `cd "~/work"` looks for a
    // literal directory named `~` and fails. `$HOME/…` isn't a safe
    // alternative either: `$` is one of the characters `breaks_quoting`
    // rejects below, precisely because unrestricted variable/command
    // substitution inside the quotes is the injection risk this validator
    // exists to close. Reject the shorthand outright rather than ship a
    // value that silently can't connect — the absolute path is the only
    // form that's both safe and correct here.
    if trimmed.starts_with('~') {
        return Err(SessionHostError::Unsafe(SessionHostField::SessionPath));
    }
    if trimmed.chars().any(breaks_quoting) {
        return Err(SessionHostError::Unsafe(SessionHostField::SessionPath));
    }
    Ok(trimmed.to_string())
}

/// `host` if [`wrap`] can quote it — the enforcement that makes
/// [`assemble_launch_command`]'s stated precondition true for *every* host
/// it is handed, not just the ones a form built.
///
/// Three inputs reach [`effective_session_host`] without ever passing
/// [`sanitized_ssh`] / [`sanitized_docker`]: an agent's
/// `[[agents]].ssh.host` / `.docker.container` and a lane's legacy
/// `remote_cwd` (the fallback pair), and a `[[session_hosts]]` entry's
/// `target`/`container`, which [`apply_catalog_entry`] writes over whatever
/// the lane's form had checked. All three take a hand-edited config file to
/// go bad — this is a guardrail, not a defense against untrusted input.
///
/// Refused rather than downgraded to `Local`: `Local` is a choice the user
/// made, and a host that runs elsewhere is a containment they rely on.
/// Answering `Local` here would give one value two meanings and move the
/// session onto this machine with nothing but a log line to say so. Both
/// callers that resolve a host already surface a refusal to the user.
fn checked_host(host: LaneSessionHost) -> Result<LaneSessionHost, UnusableSessionHost> {
    let (word, field, session_path) = match &host {
        LaneSessionHost::Local => return Ok(host),
        LaneSessionHost::Ssh {
            target,
            session_path,
            ..
        } => (target, SessionHostField::Target, session_path),
        LaneSessionHost::Docker {
            container,
            session_path,
            ..
        } => (container, SessionHostField::Container, session_path),
    };
    match checked_bare_word(word, field).and_then(|_| checked_session_path(session_path)) {
        Ok(_) => Ok(host),
        Err(reason) => Err(UnusableSessionHost { host, reason }),
    }
}

/// Build an SSH host from raw form input — trims, then rejects anything
/// [`wrap`] could not quote safely. The only way the UI should construct one.
///
/// `registry_id` is a parameter rather than a field a caller patches
/// afterwards: the link is part of what makes the host, and a builder that
/// hardcoded `None` left every call site re-deriving the same overwrite step.
pub fn sanitized_ssh(
    target: &str,
    session_path: &str,
    registry_id: Option<SessionHostId>,
) -> Result<LaneSessionHost, SessionHostError> {
    Ok(LaneSessionHost::Ssh {
        target: checked_bare_word(target, SessionHostField::Target)?,
        session_path: checked_session_path(session_path)?,
        registry_id,
    })
}

/// Build a Docker host from raw form input. See [`sanitized_ssh`].
pub fn sanitized_docker(
    container: &str,
    session_path: &str,
    registry_id: Option<SessionHostId>,
) -> Result<LaneSessionHost, SessionHostError> {
    Ok(LaneSessionHost::Docker {
        container: checked_bare_word(container, SessionHostField::Container)?,
        session_path: checked_session_path(session_path)?,
        registry_id,
    })
}

/// Build the host a registry `entry` describes, linked to that entry — the
/// one path every "user picked a registered host" form takes, so the kind
/// dispatch and the `registry_id` that makes the link resolvable can't drift
/// apart between forms.
pub fn from_registry_entry(
    entry: &SessionHostEntry,
    session_path: &str,
) -> Result<LaneSessionHost, SessionHostError> {
    match &entry.kind {
        SessionHostKind::Ssh { target } => sanitized_ssh(target, session_path, Some(entry.id)),
        SessionHostKind::Docker { container } => {
            sanitized_docker(container, session_path, Some(entry.id))
        }
    }
}

/// Which [`SessionHostKind`] a resolved `Ssh`/`Docker` host's `registry_id`
/// must match in the catalog. A `registry_id` only ever resolves against an
/// entry of the *same* kind — a kind mismatch (e.g. from a hand-edited
/// `config.toml`) is treated exactly like "entry not found", never coerced
/// across kinds or allowed to panic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HostKind {
    Ssh,
    Docker,
}

impl HostKind {
    fn matches(self, kind: &SessionHostKind) -> bool {
        matches!(
            (self, kind),
            (HostKind::Ssh, SessionHostKind::Ssh { .. })
                | (HostKind::Docker, SessionHostKind::Docker { .. })
        )
    }
}

/// `host`'s registry link, if it has one — `None` for `Local` and for a
/// free-text `Ssh`/`Docker` host (`registry_id: None`, either never linked to
/// the catalog or persisted before the registry existed).
fn registry_link(host: &LaneSessionHost) -> Option<(SessionHostId, HostKind)> {
    match host {
        LaneSessionHost::Local => None,
        LaneSessionHost::Ssh { registry_id, .. } => registry_id.map(|id| (id, HostKind::Ssh)),
        LaneSessionHost::Docker { registry_id, .. } => registry_id.map(|id| (id, HostKind::Docker)),
    }
}

fn find_in_catalog(
    id: SessionHostId,
    kind: HostKind,
    catalog: &[SessionHostEntry],
) -> Option<&SessionHostEntry> {
    catalog
        .iter()
        .find(|entry| entry.id == id && kind.matches(&entry.kind))
}

/// Defends against a cyclic or corrupted hand-edited `config.toml`: a
/// tombstone redirect chain never chases more hops than this before giving
/// up and reporting the id as unresolved.
const MAX_REDIRECT_HOPS: u8 = 10;

/// Resolve `id` against `catalog`, chasing a [`SessionHostTombstone`]
/// `redirected_to` chain (up to [`MAX_REDIRECT_HOPS`] hops) when `id` isn't
/// currently in the catalog. `None` when the id was never removed, the chain
/// dead-ends (no `redirected_to`), or the hop budget runs out — a cycle
/// simply burns the budget and falls out the same way.
fn resolve_catalog_id<'a>(
    id: SessionHostId,
    kind: HostKind,
    catalog: &'a [SessionHostEntry],
    tombstones: &[SessionHostTombstone],
) -> Option<&'a SessionHostEntry> {
    let mut current = id;
    let mut hops = 0u8;
    loop {
        if let Some(entry) = find_in_catalog(current, kind, catalog) {
            return Some(entry);
        }
        if hops >= MAX_REDIRECT_HOPS {
            return None;
        }
        current = tombstones
            .iter()
            .find(|t| t.old_id == current)
            .and_then(|t| t.redirected_to)?;
        hops += 1;
    }
}

/// Overwrite `host`'s `target`/`container` with `entry`'s current value —
/// the "resolve to the latest registered value" behavior. `session_path` and
/// `registry_id` are left exactly as `host` carried them: the id may have
/// been reached via a tombstone redirect rather than being `host`'s own, and
/// writing that back onto the cached value is Task 3's job, not this
/// resolver's.
fn apply_catalog_entry(host: LaneSessionHost, entry: &SessionHostEntry) -> LaneSessionHost {
    match (host, &entry.kind) {
        (
            LaneSessionHost::Ssh {
                session_path,
                registry_id,
                ..
            },
            SessionHostKind::Ssh { target },
        ) => LaneSessionHost::Ssh {
            target: target.clone(),
            session_path,
            registry_id,
        },
        (
            LaneSessionHost::Docker {
                session_path,
                registry_id,
                ..
            },
            SessionHostKind::Docker { container },
        ) => LaneSessionHost::Docker {
            container: container.clone(),
            session_path,
            registry_id,
        },
        // `entry` only ever comes from `resolve_catalog_id`, which already
        // filtered by `HostKind`, so this arm is unreachable in practice.
        // Fall back to the original host rather than panic.
        (host, _) => host,
    }
}

/// Whether a [`LaneSessionHost`]'s registry link is currently live — a pure
/// "is this id findable in `catalog` right now" read for UI display (e.g. a
/// stale-host banner). Does **not** chase tombstone redirects itself; that's
/// [`effective_session_host`]'s job when it needs a working connection, not
/// this status check's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkStatus {
    /// `registry_id` resolves in `catalog` right now (kind-matched).
    Fresh,
    /// `registry_id: Some(id)`, but `id` does not currently resolve in
    /// `catalog` — deleted, or kind-mismatched.
    Orphaned,
    /// No registry link to begin with: `Local`, or a free-text `Ssh`/`Docker`
    /// host with `registry_id: None`.
    Unlinked,
}

pub fn registry_link_status(host: &LaneSessionHost, catalog: &[SessionHostEntry]) -> LinkStatus {
    match registry_link(host) {
        None => LinkStatus::Unlinked,
        Some((id, kind)) => match find_in_catalog(id, kind, catalog) {
            Some(_) => LinkStatus::Fresh,
            None => LinkStatus::Orphaned,
        },
    }
}

/// The host a lane's session actually attaches to.
///
/// `session_host` being `Some` means the user answered this on the lane, so the
/// legacy pair is never consulted again — **including when they answered
/// `Local`**. Without that distinction, turning a remote lane back to local
/// would silently resurrect its old `remote_cwd`.
///
/// When the resolved host carries a `registry_id`, it is re-resolved against
/// `catalog` (falling back to a tombstone `redirected_to` chase — see
/// [`resolve_catalog_id`] — when the id isn't directly in the catalog
/// anymore) so `target`/`container` always reflect the latest registered
/// value rather than a stale cached copy; a `registry_id` that resolves
/// nowhere leaves the cached value untouched (Orphaned, not broken). A
/// `registry_id: None` host — the vast majority of real lanes today — skips
/// all of this and returns exactly what the precedence logic below already
/// produced, byte-for-byte.
///
/// Whatever the precedence lands on is finally passed through
/// [`checked_host`], so a host a form never checked — the legacy pair's two
/// halves, a hand-edited catalog entry — can never reach [`wrap`] carrying a
/// character its quoting cannot survive. Such a host is returned as the
/// error, not replaced: see [`checked_host`] for why it is not `Local`.
///
/// Pure over its five inputs rather than taking a `Lane`, so the precedence
/// can be tested without building one.
pub fn effective_session_host(
    session_host: Option<&LaneSessionHost>,
    remote_cwd: Option<&str>,
    launch: &AgentLaunch,
    catalog: &[SessionHostEntry],
    tombstones: &[SessionHostTombstone],
) -> Result<LaneSessionHost, UnusableSessionHost> {
    let resolved = if let Some(host) = session_host {
        host.clone()
    } else {
        // Legacy: the host lived on the agent and the path on the lane, so both
        // halves have to be present for either to mean anything.
        let path = remote_cwd.map(str::trim).filter(|p| !p.is_empty());
        match (launch, path) {
            (AgentLaunch::Ssh { host, .. }, Some(path)) => LaneSessionHost::Ssh {
                target: host.clone(),
                session_path: path.to_string(),
                registry_id: None,
            },
            (AgentLaunch::Docker { container, .. }, Some(path)) => LaneSessionHost::Docker {
                container: container.clone(),
                session_path: path.to_string(),
                registry_id: None,
            },
            _ => LaneSessionHost::Local,
        }
    };
    let resolved = match registry_link(&resolved) {
        None => resolved,
        Some((id, kind)) => match resolve_catalog_id(id, kind, catalog, tombstones) {
            Some(entry) => apply_catalog_entry(resolved, entry),
            None => resolved, // Orphaned: keep the last-known cached value.
        },
    };
    checked_host(resolved)
}

/// The registry id `host`'s catalog link currently resolves to — `host`'s
/// own `registry_id` when it matches the catalog directly, or the
/// tombstone-redirected id when the original entry was deleted and merged
/// into another (see [`resolve_catalog_id`]). `None` when `host` carries no
/// registry link at all, or the link is unresolvable (deleted with no
/// redirect, kind mismatch, or a chain that never lands in the catalog).
///
/// Read-only like [`effective_session_host`], which deliberately leaves a
/// tombstone-redirected id untouched on its returned value (see that
/// function's doc). This is the other half a write-back caller
/// (`connect_agent_chat`) combines with it: correcting a lane's cached
/// `registry_id` to this value collapses a future resolution back to a
/// direct catalog hit instead of repeating the tombstone chase forever.
pub fn resolved_registry_id(
    host: &LaneSessionHost,
    catalog: &[SessionHostEntry],
    tombstones: &[SessionHostTombstone],
) -> Option<SessionHostId> {
    let (id, kind) = registry_link(host)?;
    resolve_catalog_id(id, kind, catalog, tombstones).map(|entry| entry.id)
}

/// The bare adapter command a launch runs, independent of any host embedded
/// in the launch itself — the input [`wrap`] expects, so the lane's own host
/// decides where it runs rather than whatever host (if any) `launch` names.
///
/// **Not for a `Raw` command carrying the legacy `{{cwd}}` token**
/// (`launch.needs_remote_cwd()` true via that token): that command is a
/// hand-written `cd`-then-launch string predating this host axis entirely,
/// and returning it here would drop the token unexpanded into another `cd`
/// wrapper. Callers must route that case through [`AgentLaunch::wrap`]
/// directly — see `connect_agent_chat`.
pub fn adapter_command(launch: &AgentLaunch) -> &str {
    match launch {
        AgentLaunch::Raw(command) => command,
        AgentLaunch::Ssh {
            adapter_command, ..
        }
        | AgentLaunch::Docker {
            adapter_command, ..
        } => adapter_command,
    }
}

/// The transport `host` names, in the vocabulary
/// [`assemble_launch_command`] speaks. `LaneSessionHost` is a persisted app
/// type `daruda_config` cannot see, so this is the one translation between
/// the two.
fn transport(host: &LaneSessionHost) -> LaunchTransport<'_> {
    match host {
        LaneSessionHost::Local => LaunchTransport::Local,
        LaneSessionHost::Ssh {
            target,
            session_path,
            ..
        } => LaunchTransport::Ssh {
            target,
            session_path,
        },
        LaneSessionHost::Docker {
            container,
            session_path,
            ..
        } => LaunchTransport::Docker {
            container,
            session_path,
        },
    }
}

/// The command to spawn for a session on `host`.
///
/// The quoting it relies on is what [`sanitized_ssh`] / [`sanitized_docker`]
/// protect.
pub fn wrap(host: &LaneSessionHost, adapter_command: &str) -> String {
    wrap_with_env(host, adapter_command, &AccountEnv::ambient())
}

/// Like [`wrap`], but folds `env` in. Identical to
/// [`AgentLaunch::wrap_with_env`] for the same `(host, adapter_command,
/// env)` triple by construction, not by hand: both are
/// [`assemble_launch_command`], which owns the quoting — see its doc for how
/// each transport applies `env`, why `Local` stays inject-only, and the
/// preconditions it does *not* check (env names, `session_path`, the
/// transport's bare word). Two of those three are this module's to hold:
/// [`sanitized_ssh`] / [`sanitized_docker`] at every form, and
/// [`checked_host`] on every host [`effective_session_host`] resolves, which
/// covers the entrances no form guards. Env names are validated where
/// they enter the config model.
pub fn wrap_with_env(host: &LaneSessionHost, adapter_command: &str, env: &AccountEnv) -> String {
    assemble_launch_command(transport(host), adapter_command, env)
}
