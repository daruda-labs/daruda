//! Where a lane's agent session attaches: resolving it, assembling the
//! adapter command for it, and validating what the user typed.
//!
//! All three live together because they share one fact — the exact shell
//! quoting [`wrap`] emits. The validation rules below are derived from that
//! string and nothing else, so a change to one is a change to the other.
//!
//! GPUI-free (see the `lane/` module doc).

use daruda_config::{AccountEnv, AgentLaunch};
use daruda_store::project::LaneSessionHost;

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

fn checked_bare_word(value: &str, field: SessionHostField) -> Result<String, SessionHostError> {
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
    })
}

/// The host a lane's session actually attaches to.
///
/// `session_host` being `Some` means the user answered this on the lane, so the
/// legacy pair is never consulted again — **including when they answered
/// `Local`**. Without that distinction, turning a remote lane back to local
/// would silently resurrect its old `remote_cwd`.
///
/// Pure over its three inputs rather than taking a `Lane`, so the precedence
/// can be tested without building one.
pub fn effective_session_host(
    session_host: Option<&LaneSessionHost>,
    remote_cwd: Option<&str>,
    launch: &AgentLaunch,
) -> LaneSessionHost {
    if let Some(host) = session_host {
        return host.clone();
    }
    // Legacy: the host lived on the agent and the path on the lane, so both
    // halves have to be present for either to mean anything.
    let path = remote_cwd.map(str::trim).filter(|p| !p.is_empty());
    match (launch, path) {
        (AgentLaunch::Ssh { host, .. }, Some(path)) => LaneSessionHost::Ssh {
            target: host.clone(),
            session_path: path.to_string(),
        },
        (AgentLaunch::Docker { container, .. }, Some(path)) => LaneSessionHost::Docker {
            container: container.clone(),
            session_path: path.to_string(),
        },
        _ => LaneSessionHost::Local,
    }
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
        } => format!("ssh {target} sh -c 'cd \"{session_path}\" && {adapter_command}'"),
        LaneSessionHost::Docker {
            container,
            session_path,
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
            effective_session_host(Some(&answered), Some("/legacy/path"), &legacy_launch),
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
                &legacy_launch
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
            effective_session_host(None, Some("/legacy/path"), &launch),
            ssh("box", "/legacy/path")
        );
        let launch = AgentLaunch::Docker {
            adapter_command: ADAPTER.into(),
            container: "dev".into(),
        };
        assert_eq!(
            effective_session_host(None, Some("/legacy/path"), &launch),
            LaneSessionHost::Docker {
                container: "dev".into(),
                session_path: "/legacy/path".into(),
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
                effective_session_host(None, remote_cwd, launch),
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
        for good in [
            "/srv/my project",
            "/srv/a;b",
            "/srv/a(b)",
            "/srv/a&b",
            "~/work",
        ] {
            assert!(
                sanitized_ssh("box", good).is_ok(),
                "path {good:?} must be accepted"
            );
        }
    }
}
