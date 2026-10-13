//! Public API tests for agent assemble.

use daruda_config::{AccountEnv, LaunchTransport, assemble_launch_command};
const ADAPTER: &str = "npx -y some-acp";

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

fn env(inject: &[(&str, &str)], strip: &[&'static str]) -> AccountEnv {
    AccountEnv {
        inject: inject
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect(),
        strip: strip.to_vec(),
    }
}
