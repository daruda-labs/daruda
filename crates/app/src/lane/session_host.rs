//! Where a lane's agent session attaches: resolving it, assembling the
//! adapter command for it, and validating what the user typed.
//!
//! All three live together because they share one fact — the exact shell
//! quoting [`wrap`] emits. The validation rules below are derived from that
//! string and nothing else, so a change to one is a change to the other.
//!
//! GPUI-free (see the `lane/` module doc).

use daruda_config::{
    AccountEnv, AgentLaunch, SessionHostEntry, SessionHostKind, SessionHostTombstone,
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

/// Build an SSH host from raw form input — trims, then rejects anything
/// [`wrap`] could not quote safely. The only way the UI should construct one.
pub fn sanitized_ssh(
    target: &str,
    session_path: &str,
) -> Result<LaneSessionHost, SessionHostError> {
    Ok(LaneSessionHost::Ssh {
        target: checked_bare_word(target, SessionHostField::Target)?,
        session_path: checked_session_path(session_path)?,
        registry_id: None,
    })
}

/// Build a Docker host from raw form input. See [`sanitized_ssh`].
pub fn sanitized_docker(
    container: &str,
    session_path: &str,
) -> Result<LaneSessionHost, SessionHostError> {
    Ok(LaneSessionHost::Docker {
        container: checked_bare_word(container, SessionHostField::Container)?,
        session_path: checked_session_path(session_path)?,
        registry_id: None,
    })
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
/// Pure over its five inputs rather than taking a `Lane`, so the precedence
/// can be tested without building one.
pub fn effective_session_host(
    session_host: Option<&LaneSessionHost>,
    remote_cwd: Option<&str>,
    launch: &AgentLaunch,
    catalog: &[SessionHostEntry],
    tombstones: &[SessionHostTombstone],
) -> LaneSessionHost {
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
    let Some((id, kind)) = registry_link(&resolved) else {
        return resolved;
    };
    match resolve_catalog_id(id, kind, catalog, tombstones) {
        Some(entry) => apply_catalog_entry(resolved, entry),
        None => resolved, // Orphaned: keep the last-known cached value.
    }
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

/// The command to spawn for a session on `host`.
///
/// The `Ssh`/`Docker` strings are byte-identical to what
/// [`AgentLaunch::wrap`] produced for the same inputs — an already-shipped
/// assembly this change is not trying to alter. The quoting it relies on is
/// what [`sanitized_ssh`] / [`sanitized_docker`] protect.
pub fn wrap(host: &LaneSessionHost, adapter_command: &str) -> String {
    match host {
        LaneSessionHost::Local => adapter_command.to_string(),
        LaneSessionHost::Ssh {
            target,
            session_path,
            ..
        } => format!("ssh {target} sh -c 'cd \"{session_path}\" && {adapter_command}'"),
        LaneSessionHost::Docker {
            container,
            session_path,
            ..
        } => {
            format!("docker exec -i {container} sh -c 'cd \"{session_path}\" && {adapter_command}'")
        }
    }
}

/// Like [`wrap`], but folds `env` in — mirrors [`AgentLaunch::wrap_with_env`]
/// byte-for-byte for the same `(host, adapter_command, env)` triple: `Local`
/// gets the same `KEY='value' ` inject-only prefix (no `unset`, so the
/// launcher token driving Node.js detection stays first on the line);
/// `Ssh`/`Docker` get the same `unset`/`export` splice right after the `cd
/// "…" && ` it just assembled.
pub fn wrap_with_env(host: &LaneSessionHost, adapter_command: &str, env: &AccountEnv) -> String {
    let base = wrap(host, adapter_command);
    match host {
        LaneSessionHost::Local => {
            let mut prefix = String::new();
            for (k, v) in &env.inject {
                prefix.push_str(&format!("{k}='{v}' "));
            }
            format!("{prefix}{base}")
        }
        LaneSessionHost::Ssh { .. } | LaneSessionHost::Docker { .. } => {
            let mut env_script = String::new();
            for s in &env.strip {
                env_script.push_str(&format!("unset {s}; "));
            }
            for (k, v) in &env.inject {
                env_script.push_str(&format!("export {k}=\"{v}\"; "));
            }
            match base.rfind("&& ") {
                Some(idx) => {
                    let (head, tail) = base.split_at(idx + 3);
                    format!("{head}{env_script}{tail}")
                }
                None => base,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADAPTER: &str = "npx -y @agentclientprotocol/codex-acp@latest";

    fn ssh(target: &str, path: &str) -> LaneSessionHost {
        LaneSessionHost::Ssh {
            target: target.into(),
            session_path: path.into(),
            registry_id: None,
        }
    }

    #[test]
    fn an_answered_lane_never_consults_the_legacy_pair() {
        let legacy_launch = AgentLaunch::Ssh {
            adapter_command: ADAPTER.into(),
            host: "old-box".into(),
        };
        let answered = ssh("new-box", "/srv/app");
        assert_eq!(
            effective_session_host(
                Some(&answered),
                Some("/legacy/path"),
                &legacy_launch,
                &[],
                &[]
            ),
            answered
        );
    }

    /// The rev1 defect: turning a remote lane back to local brought its old
    /// `remote_cwd` back to life, because "local" and "unanswered" were the
    /// same value.
    #[test]
    fn answering_local_retires_the_legacy_pair() {
        let legacy_launch = AgentLaunch::Ssh {
            adapter_command: ADAPTER.into(),
            host: "old-box".into(),
        };
        assert_eq!(
            effective_session_host(
                Some(&LaneSessionHost::Local),
                Some("/legacy/path"),
                &legacy_launch,
                &[],
                &[]
            ),
            LaneSessionHost::Local
        );
    }

    #[test]
    fn an_unanswered_lane_still_honours_the_legacy_pair() {
        let launch = AgentLaunch::Ssh {
            adapter_command: ADAPTER.into(),
            host: "box".into(),
        };
        assert_eq!(
            effective_session_host(None, Some("/legacy/path"), &launch, &[], &[]),
            ssh("box", "/legacy/path")
        );
        let launch = AgentLaunch::Docker {
            adapter_command: ADAPTER.into(),
            container: "dev".into(),
        };
        assert_eq!(
            effective_session_host(None, Some("/legacy/path"), &launch, &[], &[]),
            LaneSessionHost::Docker {
                container: "dev".into(),
                session_path: "/legacy/path".into(),
                registry_id: None,
            }
        );
    }

    /// Either half missing means neither applies — a blank `remote_cwd` used to
    /// flow into `wrap` and produce `cd  && …`.
    #[test]
    fn a_half_configured_legacy_pair_stays_local() {
        let remote_launch = AgentLaunch::Ssh {
            adapter_command: ADAPTER.into(),
            host: "box".into(),
        };
        let local_launch = AgentLaunch::Raw(ADAPTER.into());
        for (remote_cwd, launch) in [
            (None, &remote_launch),
            (Some("   "), &remote_launch),
            (Some("/path"), &local_launch),
        ] {
            assert_eq!(
                effective_session_host(None, remote_cwd, launch, &[], &[]),
                LaneSessionHost::Local
            );
        }
    }

    /// Regression anchor: the strings a shipped config already produces must
    /// not move. Mirrors `AgentLaunch::wrap`'s two remote arms.
    #[test]
    fn wrap_matches_the_shipped_assembly() {
        assert_eq!(wrap(&LaneSessionHost::Local, ADAPTER), ADAPTER);
        assert_eq!(
            wrap(&ssh("build-box", "/home/user/project"), ADAPTER),
            format!("ssh build-box sh -c 'cd \"/home/user/project\" && {ADAPTER}'")
        );
        assert_eq!(
            wrap(
                &LaneSessionHost::Docker {
                    container: "dev-1".into(),
                    session_path: "/workspace".into(),
                    registry_id: None,
                },
                ADAPTER
            ),
            format!("docker exec -i dev-1 sh -c 'cd \"/workspace\" && {ADAPTER}'")
        );
    }

    #[test]
    fn adapter_command_reads_through_any_launch_shape() {
        assert_eq!(adapter_command(&AgentLaunch::Raw(ADAPTER.into())), ADAPTER);
        assert_eq!(
            adapter_command(&AgentLaunch::Ssh {
                adapter_command: ADAPTER.into(),
                host: "old-box".into(),
            }),
            ADAPTER
        );
        assert_eq!(
            adapter_command(&AgentLaunch::Docker {
                adapter_command: ADAPTER.into(),
                container: "old-box".into(),
            }),
            ADAPTER
        );
    }

    /// Mirrors `wrap_with_env_prefixes_raw_command` /
    /// `wrap_with_env_raw_emits_no_unset_flags` in `daruda_config`: a `Local`
    /// host gets the same inject-only `KEY='value' ` prefix, never an
    /// `unset`/`/usr/bin/env` — that would hide the launcher token from
    /// `daruda_acp::node::command_needs_node`.
    #[test]
    fn wrap_with_env_local_matches_the_shipped_assembly() {
        let env = AccountEnv {
            inject: vec![(
                "CLAUDE_CONFIG_DIR".into(),
                "/Users/x/Library/Application Support/daruda/acc/alice".into(),
            )],
            strip: vec!["ANTHROPIC_API_KEY"],
        };
        let cmd = wrap_with_env(&LaneSessionHost::Local, "npx -y some-acp", &env);
        assert_eq!(
            cmd,
            "CLAUDE_CONFIG_DIR='/Users/x/Library/Application Support/daruda/acc/alice' npx -y some-acp"
        );
        assert!(!cmd.contains("unset "));
        assert!(!cmd.contains("/usr/bin/env"));
    }

    /// Mirrors `wrap_with_env_ssh_exports_and_unsets` in `daruda_config`.
    #[test]
    fn wrap_with_env_ssh_matches_the_shipped_assembly() {
        let env = AccountEnv {
            inject: vec![("CLAUDE_CONFIG_DIR".into(), "/remote/acc".into())],
            strip: vec!["ANTHROPIC_API_KEY"],
        };
        let cmd = wrap_with_env(&ssh("vm", "/work"), "npx -y some-acp", &env);
        assert!(cmd.contains("export CLAUDE_CONFIG_DIR=\"/remote/acc\""));
        assert!(cmd.contains("unset ANTHROPIC_API_KEY"));
    }

    #[test]
    fn sanitizing_trims_every_field() {
        assert_eq!(
            sanitized_ssh("  build-box \n", "\t/srv/app  "),
            Ok(ssh("build-box", "/srv/app"))
        );
    }

    #[test]
    fn an_empty_field_is_rejected() {
        assert_eq!(
            sanitized_ssh("   ", "/srv/app"),
            Err(SessionHostError::Empty(SessionHostField::Target))
        );
        assert_eq!(
            sanitized_ssh("box", ""),
            Err(SessionHostError::Empty(SessionHostField::SessionPath))
        );
        assert_eq!(
            sanitized_docker("  ", "/srv"),
            Err(SessionHostError::Empty(SessionHostField::Container))
        );
    }

    /// A bare word is unquoted in `wrap`'s output, so anything that could split
    /// it or mean something to the shell is out — a space included.
    #[test]
    fn a_bare_word_rejects_anything_outside_a_shell_word() {
        for bad in [
            "box; rm -rf /",
            "box host",
            "box'x",
            "box\"x",
            "box`x`",
            "box$x",
            "box\\x",
            "box&x",
            "box|x",
            "box\nhost",
        ] {
            assert_eq!(
                sanitized_ssh(bad, "/srv"),
                Err(SessionHostError::Unsafe(SessionHostField::Target)),
                "target {bad:?} must be rejected"
            );
            assert_eq!(
                sanitized_docker(bad, "/srv"),
                Err(SessionHostError::Unsafe(SessionHostField::Container)),
                "container {bad:?} must be rejected"
            );
        }
    }

    #[test]
    fn a_bare_word_accepts_the_shapes_a_host_really_takes() {
        for good in [
            "buildbox",
            "user@host",
            "host.example.com",
            "10.0.0.7",
            "[fe80::1%en0]",
            "my-alias_2",
            "host:2222",
        ] {
            assert!(
                sanitized_ssh(good, "/srv").is_ok(),
                "target {good:?} must be accepted"
            );
        }
    }

    /// The path sits inside `"…"` inside `'…'`, so only what escapes those is
    /// out — a space or a semicolon stays inert and must be allowed.
    #[test]
    fn a_session_path_rejects_only_what_escapes_the_quoting() {
        for bad in [
            "/srv/a'b",
            "/srv/a\"b",
            "/srv/a`b`",
            "/srv/$HOME",
            "/srv/a\\b",
            "/srv/a\nb",
            "/srv/a\rb",
        ] {
            assert_eq!(
                sanitized_ssh("box", bad),
                Err(SessionHostError::Unsafe(SessionHostField::SessionPath)),
                "path {bad:?} must be rejected"
            );
        }
        for good in ["/srv/my project", "/srv/a;b", "/srv/a(b)", "/srv/a&b"] {
            assert!(
                sanitized_ssh("box", good).is_ok(),
                "path {good:?} must be accepted"
            );
        }
    }

    /// Regression: `~/work` used to be accepted, but `wrap` always emits
    /// `cd "…"` — double quotes suppress POSIX tilde expansion, so a saved
    /// `~/work` connected and then failed with "no such file or directory"
    /// on every real remote host.
    #[test]
    fn a_session_path_starting_with_tilde_is_rejected() {
        for bad in ["~/work", "~", "~root/x"] {
            assert_eq!(
                sanitized_ssh("box", bad),
                Err(SessionHostError::Unsafe(SessionHostField::SessionPath)),
                "path {bad:?} must be rejected"
            );
        }
        // A `~` that isn't the leading character is inert — the shell only
        // ever expands a *leading* tilde.
        assert!(sanitized_ssh("box", "/srv/a~b").is_ok());
    }

    fn tombstone(
        old_id: SessionHostId,
        redirected_to: Option<SessionHostId>,
    ) -> SessionHostTombstone {
        SessionHostTombstone {
            old_id,
            kind: SessionHostKind::Ssh {
                target: "irrelevant".into(),
            },
            value: "irrelevant".into(),
            removed_at: 0,
            redirected_to,
        }
    }

    /// Regression anchor: a `registry_id: None` host must resolve
    /// byte-for-byte identically whether the catalog/tombstones are empty or
    /// (as here) non-empty but unrelated — the vast majority of real lanes
    /// never touch the registry at all.
    #[test]
    fn none_registry_id_ignores_the_catalog_entirely() {
        let host = ssh("box", "/srv/app");
        let unrelated_id = SessionHostId::new();
        let catalog = vec![SessionHostEntry {
            id: unrelated_id,
            label: "Something else".into(),
            kind: SessionHostKind::Ssh {
                target: "other-box".into(),
            },
        }];
        let tombstones = vec![tombstone(unrelated_id, None)];
        let launch = AgentLaunch::Raw(ADAPTER.into());
        assert_eq!(
            effective_session_host(Some(&host), None, &launch, &catalog, &tombstones),
            host
        );
    }

    #[test]
    fn a_linked_host_resolves_to_the_catalogs_latest_value() {
        let id = SessionHostId::new();
        let cached = LaneSessionHost::Ssh {
            target: "old-target".into(),
            session_path: "/srv/app".into(),
            registry_id: Some(id),
        };
        let catalog = vec![SessionHostEntry {
            id,
            label: "Build box".into(),
            kind: SessionHostKind::Ssh {
                target: "new-target".into(),
            },
        }];
        let launch = AgentLaunch::Raw(ADAPTER.into());
        let resolved = effective_session_host(Some(&cached), None, &launch, &catalog, &[]);
        assert_eq!(
            resolved,
            LaneSessionHost::Ssh {
                target: "new-target".into(),
                session_path: "/srv/app".into(),
                registry_id: Some(id),
            }
        );
    }

    /// Matching is by id, never by label — a rename in the registry entry
    /// must not orphan every lane that references it.
    #[test]
    fn a_relabeled_entry_still_resolves_by_id() {
        let id = SessionHostId::new();
        let cached = LaneSessionHost::Docker {
            container: "old-name".into(),
            session_path: "/workspace".into(),
            registry_id: Some(id),
        };
        let catalog = vec![SessionHostEntry {
            id,
            label: "Renamed label".into(),
            kind: SessionHostKind::Docker {
                container: "dev-2".into(),
            },
        }];
        let launch = AgentLaunch::Raw(ADAPTER.into());
        let resolved = effective_session_host(Some(&cached), None, &launch, &catalog, &[]);
        assert_eq!(
            resolved,
            LaneSessionHost::Docker {
                container: "dev-2".into(),
                session_path: "/workspace".into(),
                registry_id: Some(id),
            }
        );
    }

    #[test]
    fn an_unresolvable_registry_id_keeps_the_cached_value() {
        let id = SessionHostId::new();
        let cached = LaneSessionHost::Ssh {
            target: "cached-target".into(),
            session_path: "/srv/app".into(),
            registry_id: Some(id),
        };
        let launch = AgentLaunch::Raw(ADAPTER.into());
        let resolved = effective_session_host(Some(&cached), None, &launch, &[], &[]);
        assert_eq!(resolved, cached);
        assert_eq!(registry_link_status(&cached, &[]), LinkStatus::Orphaned);
    }

    /// A hand-edited `config.toml` could give an `Ssh` lane a `registry_id`
    /// that now belongs to a `Docker` entry — must be treated exactly like
    /// "not found", never coerced across kinds.
    #[test]
    fn a_kind_mismatched_entry_is_treated_as_not_found() {
        let id = SessionHostId::new();
        let cached = LaneSessionHost::Ssh {
            target: "cached-target".into(),
            session_path: "/srv/app".into(),
            registry_id: Some(id),
        };
        let catalog = vec![SessionHostEntry {
            id,
            label: "Mismatched".into(),
            kind: SessionHostKind::Docker {
                container: "dev-1".into(),
            },
        }];
        let launch = AgentLaunch::Raw(ADAPTER.into());
        let resolved = effective_session_host(Some(&cached), None, &launch, &catalog, &[]);
        assert_eq!(resolved, cached);
        assert_eq!(
            registry_link_status(&cached, &catalog),
            LinkStatus::Orphaned
        );
    }

    #[test]
    fn a_deleted_entry_resolves_through_a_single_tombstone_redirect() {
        let old_id = SessionHostId::new();
        let new_id = SessionHostId::new();
        let cached = LaneSessionHost::Ssh {
            target: "cached-target".into(),
            session_path: "/srv/app".into(),
            registry_id: Some(old_id),
        };
        let catalog = vec![SessionHostEntry {
            id: new_id,
            label: "Renamed box".into(),
            kind: SessionHostKind::Ssh {
                target: "merged-target".into(),
            },
        }];
        let tombstones = vec![tombstone(old_id, Some(new_id))];
        let launch = AgentLaunch::Raw(ADAPTER.into());
        let resolved = effective_session_host(Some(&cached), None, &launch, &catalog, &tombstones);
        assert_eq!(
            resolved,
            LaneSessionHost::Ssh {
                target: "merged-target".into(),
                session_path: "/srv/app".into(),
                // Unchanged: rewriting the cached `registry_id` to the
                // redirected-to id is the write-back Task 3 owns, not this
                // read-only resolver.
                registry_id: Some(old_id),
            }
        );
    }

    /// The write-back half `resolved_registry_id` owns: unlike
    /// `effective_session_host`'s returned value above (which keeps
    /// `old_id`), this reports the *live* id so a caller can correct the
    /// lane's cache to it.
    #[test]
    fn resolved_registry_id_reports_the_redirected_to_id() {
        let old_id = SessionHostId::new();
        let new_id = SessionHostId::new();
        let cached = LaneSessionHost::Ssh {
            target: "cached-target".into(),
            session_path: "/srv/app".into(),
            registry_id: Some(old_id),
        };
        let catalog = vec![SessionHostEntry {
            id: new_id,
            label: "Renamed box".into(),
            kind: SessionHostKind::Ssh {
                target: "merged-target".into(),
            },
        }];
        let tombstones = vec![tombstone(old_id, Some(new_id))];
        assert_eq!(
            resolved_registry_id(&cached, &catalog, &tombstones),
            Some(new_id)
        );
    }

    #[test]
    fn resolved_registry_id_is_unchanged_on_a_direct_catalog_hit() {
        let id = SessionHostId::new();
        let cached = LaneSessionHost::Ssh {
            target: "cached-target".into(),
            session_path: "/srv/app".into(),
            registry_id: Some(id),
        };
        let catalog = vec![SessionHostEntry {
            id,
            label: "Box".into(),
            kind: SessionHostKind::Ssh {
                target: "latest-target".into(),
            },
        }];
        assert_eq!(resolved_registry_id(&cached, &catalog, &[]), Some(id));
    }

    #[test]
    fn resolved_registry_id_is_none_when_orphaned() {
        let id = SessionHostId::new();
        let cached = LaneSessionHost::Ssh {
            target: "cached-target".into(),
            session_path: "/srv/app".into(),
            registry_id: Some(id),
        };
        assert_eq!(resolved_registry_id(&cached, &[], &[]), None);
    }

    #[test]
    fn resolved_registry_id_is_none_for_a_free_text_or_local_host() {
        let free_text = LaneSessionHost::Ssh {
            target: "vm-work".into(),
            session_path: "/srv/app".into(),
            registry_id: None,
        };
        assert_eq!(resolved_registry_id(&free_text, &[], &[]), None);
        assert_eq!(
            resolved_registry_id(&LaneSessionHost::Local, &[], &[]),
            None
        );
    }

    #[test]
    fn a_chain_of_tombstone_redirects_resolves_to_the_final_catalog_entry() {
        let id_a = SessionHostId::new();
        let id_b = SessionHostId::new();
        let id_c = SessionHostId::new();
        let cached = LaneSessionHost::Docker {
            container: "cached".into(),
            session_path: "/workspace".into(),
            registry_id: Some(id_a),
        };
        let catalog = vec![SessionHostEntry {
            id: id_c,
            label: "Final".into(),
            kind: SessionHostKind::Docker {
                container: "final-container".into(),
            },
        }];
        let tombstones = vec![tombstone(id_a, Some(id_b)), tombstone(id_b, Some(id_c))];
        let launch = AgentLaunch::Raw(ADAPTER.into());
        let resolved = effective_session_host(Some(&cached), None, &launch, &catalog, &tombstones);
        assert_eq!(
            resolved,
            LaneSessionHost::Docker {
                container: "final-container".into(),
                session_path: "/workspace".into(),
                registry_id: Some(id_a),
            }
        );
    }

    /// A tombstone with no `redirected_to` is a dead end, not a hop — the
    /// removal wasn't a merge, so there's nowhere further to look.
    #[test]
    fn a_tombstone_with_no_redirect_leaves_the_id_orphaned() {
        let id = SessionHostId::new();
        let cached = LaneSessionHost::Docker {
            container: "cached".into(),
            session_path: "/workspace".into(),
            registry_id: Some(id),
        };
        let tombstones = vec![tombstone(id, None)];
        let launch = AgentLaunch::Raw(ADAPTER.into());
        let resolved = effective_session_host(Some(&cached), None, &launch, &[], &tombstones);
        assert_eq!(resolved, cached);
    }

    /// Exactly the hop budget: the 10th redirect lands on the catalog entry
    /// and must still resolve.
    #[test]
    fn a_redirect_chain_of_exactly_ten_hops_still_resolves() {
        let ids: Vec<SessionHostId> = (0..11).map(|_| SessionHostId::new()).collect();
        let cached = LaneSessionHost::Ssh {
            target: "cached-target".into(),
            session_path: "/srv/app".into(),
            registry_id: Some(ids[0]),
        };
        let catalog = vec![SessionHostEntry {
            id: ids[10],
            label: "Reachable".into(),
            kind: SessionHostKind::Ssh {
                target: "just-in-time".into(),
            },
        }];
        let tombstones: Vec<SessionHostTombstone> = ids
            .windows(2)
            .map(|pair| tombstone(pair[0], Some(pair[1])))
            .collect();
        let launch = AgentLaunch::Raw(ADAPTER.into());
        let resolved = effective_session_host(Some(&cached), None, &launch, &catalog, &tombstones);
        assert_eq!(
            resolved,
            LaneSessionHost::Ssh {
                target: "just-in-time".into(),
                session_path: "/srv/app".into(),
                registry_id: Some(ids[0]),
            }
        );
    }

    /// One hop past the budget: the same shape as the exactly-ten-hops case,
    /// but the catalog entry sits one redirect further out — must fall back
    /// to the cached value rather than chase indefinitely.
    #[test]
    fn a_redirect_chain_longer_than_ten_hops_falls_back_to_the_cached_value() {
        let ids: Vec<SessionHostId> = (0..12).map(|_| SessionHostId::new()).collect();
        let cached = LaneSessionHost::Ssh {
            target: "cached-target".into(),
            session_path: "/srv/app".into(),
            registry_id: Some(ids[0]),
        };
        let catalog = vec![SessionHostEntry {
            id: ids[11],
            label: "Unreachable".into(),
            kind: SessionHostKind::Ssh {
                target: "too-far".into(),
            },
        }];
        let tombstones: Vec<SessionHostTombstone> = ids
            .windows(2)
            .map(|pair| tombstone(pair[0], Some(pair[1])))
            .collect();
        let launch = AgentLaunch::Raw(ADAPTER.into());
        let resolved = effective_session_host(Some(&cached), None, &launch, &catalog, &tombstones);
        assert_eq!(resolved, cached);
    }

    /// A cyclic tombstone chain (corrupted/hand-edited config) must not hang
    /// the resolver — the hop budget bounds it and the cached value survives.
    #[test]
    fn a_cyclic_tombstone_chain_terminates_and_falls_back() {
        let id_a = SessionHostId::new();
        let id_b = SessionHostId::new();
        let cached = LaneSessionHost::Ssh {
            target: "cached-target".into(),
            session_path: "/srv/app".into(),
            registry_id: Some(id_a),
        };
        let tombstones = vec![tombstone(id_a, Some(id_b)), tombstone(id_b, Some(id_a))];
        let launch = AgentLaunch::Raw(ADAPTER.into());
        let resolved = effective_session_host(Some(&cached), None, &launch, &[], &tombstones);
        assert_eq!(resolved, cached);
    }

    #[test]
    fn registry_link_status_covers_all_three_cases() {
        assert_eq!(
            registry_link_status(&LaneSessionHost::Local, &[]),
            LinkStatus::Unlinked
        );
        assert_eq!(
            registry_link_status(&ssh("box", "/srv"), &[]),
            LinkStatus::Unlinked
        );

        let id = SessionHostId::new();
        let linked = LaneSessionHost::Ssh {
            target: "box".into(),
            session_path: "/srv".into(),
            registry_id: Some(id),
        };
        assert_eq!(registry_link_status(&linked, &[]), LinkStatus::Orphaned);

        let catalog = vec![SessionHostEntry {
            id,
            label: "Build box".into(),
            kind: SessionHostKind::Ssh {
                target: "box".into(),
            },
        }];
        assert_eq!(registry_link_status(&linked, &catalog), LinkStatus::Fresh);
    }
}
