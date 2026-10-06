//! Assembling the shell command line that launches an ACP adapter.
//!
//! The single owner of that quoting. Two callers reach it from opposite
//! sides of the GPUI boundary: [`crate::AgentLaunch`], where the host is
//! part of the launch itself, and the app's `lane::session_host`, where the
//! host lives on the lane instead. Both hand over the same four pieces —
//! transport, session path, adapter command, account env — so the strings
//! they emit are identical by construction rather than by hand.

use crate::account_env::AccountEnv;

/// Where an assembled adapter command runs. The app's `LaneSessionHost` is a
/// GPUI-side type this crate cannot see, so a caller passes the pieces it
/// carries rather than the host itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchTransport<'a> {
    /// This machine — the adapter command runs with no wrapper at all.
    Local,
    /// Another machine over SSH, `cd`-ed into `session_path` first.
    Ssh {
        target: &'a str,
        session_path: &'a str,
    },
    /// An already-running container, `cd`-ed into `session_path` first.
    Docker {
        container: &'a str,
        session_path: &'a str,
    },
}

/// POSIX single-quotes `value` for interpolation into a shell command,
/// escaping an embedded `'` as `'\''` (close the quote, emit an escaped
/// literal quote, reopen). Inside single quotes a shell performs no
/// expansion at all, so this is safe for a value containing `"`, `$`,
/// backticks or `\`.
fn shell_single_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// Whether `name` is an environment-variable name daruda may put into a
/// launch command: `[A-Za-z_][A-Za-z0-9_]*`, the POSIX portable charset.
///
/// This is the contract the whole pipeline runs on. Nothing downstream can
/// consume a wider name — `daruda_acp::node`'s env-assignment parser accepts
/// exactly this set and treats anything else as the launcher token, and
/// `daruda_acp::launch_env` emits `-u <name>` unquoted on the strength of it.
/// [`assemble_launch_command`] quotes only the *value*, so a name carrying
/// `;`, a space or a backtick would escape the remote `sh -c` script and run
/// as a command of its own.
///
/// Enforced wherever an env pair enters the config model — the `[[agents]]`
/// load path (`super::sanitized_env`) and the Settings Environment field
/// (`settings_window::sections::agent_env::stated_env` in the app crate) —
/// which is what lets the assembler treat it as a precondition.
#[must_use]
pub fn is_valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Build the command line that launches `adapter_command` through
/// `transport` with `env` applied. Pass [`AccountEnv::ambient`] for "no
/// managed account" — every arm then degenerates to the plain, un-enveloped
/// form.
///
/// The two transports apply `env` differently, and the difference is
/// load-bearing:
///
/// - **Local** emits **only** the `env.inject` `KEY='value' ` prefix. The
///   string has to stay parseable by
///   `daruda_acp::node::command_needs_node`, which reads the launcher token
///   after the assignments, so an `/usr/bin/env -u …` prefix here would hide
///   it and skip Node.js provisioning. `env.strip` is applied instead at
///   final launch assembly, in
///   `daruda_acp::launch_env::prepare_adapter_command`, once the runtime is
///   resolved.
/// - **Ssh / Docker** fold both halves into the remote shell as
///   `unset`/`export` statements ahead of the adapter, since there is no
///   local process to set an environment on.
///
/// Every injected value is single-quoted, and the remote script is quoted
/// again as a whole, so a value holding `"`, `'`, `$`, a backtick or `\`
/// survives both the launch string's own re-tokenization
/// (`daruda_acp::node::split_env_prefixed_tokens` / `AcpAgent::from_str`)
/// and the remote shell.
///
/// # Preconditions
///
/// This function quotes the values it is handed; it does not quote the two
/// things a caller must have validated first, because quoting either one
/// would change what they mean:
///
/// - **Every `env` name satisfies [`is_valid_env_name`].** A name is emitted
///   bare on both sides of the `=`, so one carrying `;` or whitespace would
///   run as a command inside the remote `sh -c` script. The config model
///   enforces this at every entry point — see that predicate's doc.
/// - **`session_path` and the transport's `target` / `container` contain no
///   shell metacharacter.** `session_path` is only double-quoted, so `$`, a
///   backtick and `\` stay live to the remote shell; `target` / `container`
///   are bare words. The app validates all three in
///   `lane::session_host` — `checked_session_path` / `checked_bare_word` at
///   every form that types one, and again on every host
///   `effective_session_host` resolves, which is what covers the values that
///   arrive from a hand-edited config instead of a form.
///
/// `adapter_command` is deliberately *not* a precondition: it is spliced in
/// as the shell command line it is meant to be. It must therefore be a shell
/// command line — a JSON stdio config is not one, and a caller holding a
/// non-empty `env` must refuse that shape rather than assemble it (the app
/// does, in `agent::launch_resolve`).
pub fn assemble_launch_command(
    transport: LaunchTransport<'_>,
    adapter_command: &str,
    env: &AccountEnv,
) -> String {
    match transport {
        LaunchTransport::Local => {
            let mut prefix = String::new();
            for (k, v) in &env.inject {
                prefix.push_str(&format!("{k}={} ", shell_single_quote(v)));
            }
            format!("{prefix}{adapter_command}")
        }
        LaunchTransport::Ssh {
            target,
            session_path,
        } => remote(&format!("ssh {target}"), session_path, adapter_command, env),
        LaunchTransport::Docker {
            container,
            session_path,
        } => remote(
            &format!("docker exec -i {container}"),
            session_path,
            adapter_command,
            env,
        ),
    }
}

/// A transport prefix plus an `sh -c` script that `cd`s into `session_path`,
/// applies `env`, then runs the adapter. The script is single-quoted once as
/// a whole, so a `'` inside it cannot close the wrapper early.
fn remote(prefix: &str, session_path: &str, adapter_command: &str, env: &AccountEnv) -> String {
    let mut env_script = String::new();
    for s in &env.strip {
        env_script.push_str(&format!("unset {s}; "));
    }
    for (k, v) in &env.inject {
        env_script.push_str(&format!("export {k}={}; ", shell_single_quote(v)));
    }
    let script = format!("cd \"{session_path}\" && {env_script}{adapter_command}");
    format!("{prefix} sh -c {}", shell_single_quote(&script))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADAPTER: &str = "npx -y some-acp";

    fn env(inject: &[(&str, &str)], strip: &[&'static str]) -> AccountEnv {
        AccountEnv {
            inject: inject
                .iter()
                .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                .collect(),
            strip: strip.to_vec(),
        }
    }

    /// The ambient env must leave every transport's string exactly as its
    /// no-env form.
    #[test]
    fn the_ambient_env_adds_nothing_to_any_transport() {
        let ambient = AccountEnv::ambient();
        assert_eq!(
            assemble_launch_command(LaunchTransport::Local, ADAPTER, &ambient),
            ADAPTER
        );
        assert_eq!(
            assemble_launch_command(
                LaunchTransport::Ssh {
                    target: "build-box",
                    session_path: "/home/user/project",
                },
                ADAPTER,
                &ambient,
            ),
            format!("ssh build-box sh -c 'cd \"/home/user/project\" && {ADAPTER}'")
        );
        assert_eq!(
            assemble_launch_command(
                LaunchTransport::Docker {
                    container: "dev-1",
                    session_path: "/workspace",
                },
                ADAPTER,
                &ambient,
            ),
            format!("docker exec -i dev-1 sh -c 'cd \"/workspace\" && {ADAPTER}'")
        );
    }

    #[test]
    fn local_emits_an_inject_only_quoted_prefix() {
        let cmd = assemble_launch_command(
            LaunchTransport::Local,
            ADAPTER,
            &env(
                &[("CLAUDE_CONFIG_DIR", "/data/acc/alice")],
                &["ANTHROPIC_API_KEY"],
            ),
        );
        assert_eq!(
            cmd,
            format!("CLAUDE_CONFIG_DIR='/data/acc/alice' {ADAPTER}")
        );
        assert!(!cmd.contains("unset "), "{cmd}");
        assert!(!cmd.contains("/usr/bin/env"), "{cmd}");
    }

    /// One tokenization layer for `Local` (the launcher re-splits the
    /// prefix), two for the remote arms (the outer split, then the remote
    /// shell's own) — the value must come back byte-identical from each.
    #[test]
    fn a_hostile_value_survives_every_transports_quoting_layers() {
        // The trailing `;` is deliberate: it collides with the separator the
        // remote script puts after every `export`, so it proves the recovery
        // below strips exactly one and not the value's own.
        for value in [r#"{"a":1} it's $HOME `pwd` \ "q""#, "ends-with;"] {
            let built = env(&[("K", value)], &[]);

            let local = assemble_launch_command(LaunchTransport::Local, ADAPTER, &built);
            let tokens = shell_words::split(&local).expect("Local stays one POSIX command");
            assert_eq!(
                tokens.first().map(String::as_str),
                Some(&*format!("K={value}"))
            );

            for transport in [
                LaunchTransport::Ssh {
                    target: "vm",
                    session_path: "/work",
                },
                LaunchTransport::Docker {
                    container: "dev",
                    session_path: "/work",
                },
            ] {
                let cmd = assemble_launch_command(transport, ADAPTER, &built);
                let outer = shell_words::split(&cmd).expect("outer command tokenizes");
                let script = outer
                    .iter()
                    .position(|t| t == "-c")
                    .and_then(|i| outer.get(i + 1))
                    .unwrap_or_else(|| panic!("no `-c <script>` in {outer:?}"));
                let inner = shell_words::split(script).expect("remote shell tokenizes the script");
                let assignment = inner
                    .iter()
                    .find(|t| t.starts_with("K="))
                    .unwrap_or_else(|| panic!("no `K=...` in {inner:?}"));
                let got = assignment
                    .strip_prefix("K=")
                    .expect("assignment has a value");
                // Exactly one separator is always emitted, so requiring it
                // (rather than stripping it when present) cannot swallow a
                // `;` that belongs to the value.
                let got = got.strip_suffix(';').unwrap_or_else(|| {
                    panic!("`export K=…; ` must end its word with the separator: {got:?}")
                });
                assert_eq!(got, value);
            }
        }
    }

    #[test]
    fn single_quoting_closes_reopens_around_an_embedded_quote() {
        assert_eq!(shell_single_quote("it's"), r"'it'\''s'");
        assert_eq!(shell_single_quote(r#"a"b$c`d\e"#), r#"'a"b$c`d\e'"#);
    }
}
