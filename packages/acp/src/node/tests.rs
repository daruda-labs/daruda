use super::*;
use crate::connection::ADAPTER_NPM_PACKAGE;
use agent_client_protocol::AcpAgent;
use std::str::FromStr;

/// Fixed test `install_root`, distinct from `node_dir` (which itself
/// lives under a real one in production) so assertions can tell the two
/// paths apart.
fn test_install_root() -> PathBuf {
    PathBuf::from("/data/daruda/node")
}

#[test]
fn system_runtime_prefixes_arch_and_cache_env_for_an_npx_command() {
    let cmd = "npx -y @agentclientprotocol/claude-agent-acp@latest";
    let (os, arch) = node_platform().expect("supported test platform");
    let install_root = test_install_root();
    let expected = format!(
        "npm_config_cpu={arch} npm_config_os={os} npm_config_cache={} {cmd}",
        shell_words::quote(&install_root.join("npx-cache").to_string_lossy())
    );
    assert_eq!(
        NodeRuntime::System.wrap_command(cmd, &install_root).0,
        expected
    );
}

#[test]
fn system_runtime_lets_an_explicit_npm_config_cache_prefix_win() {
    // The regression this guards: a caller-supplied `npm_config_cache=...`
    // prefix (legal per `split_env_prefixed_tokens`, same as the
    // `AUGMENT_DISABLE_AUTO_UPDATE=1 npx ...` form) must not be silently
    // discarded by daruda's own isolation default.
    let cmd = "npm_config_cache=/custom/cache npx -y pkg";
    let install_root = test_install_root();
    let command = NodeRuntime::System.wrap_command(cmd, &install_root);
    // Both assignments are present (ours first, the override last); the
    // override wins downstream because `AcpAgent::from_str` applies the
    // env list via sequential last-write-wins `Command::env` calls.
    let last_cache_assignment = command
        .0
        .split_whitespace()
        .filter_map(|tok| tok.strip_prefix("npm_config_cache="))
        .next_back()
        .expect("npm_config_cache present");
    assert_eq!(last_cache_assignment, "/custom/cache");
}

#[test]
fn system_runtime_command_survives_a_path_with_spaces_and_round_trips() {
    // Regression: `install_root` mirrors the real `node_install_dir()`,
    // which on macOS always lands under `~/Library/Application Support`
    // — a space in the path. Before quoting, `AcpAgent::from_str`'s
    // `shell_words::split` would break the unquoted cache value into two
    // tokens and mistake the second fragment for the command, spawning
    // it instead of `npx` and failing with ENOENT.
    let install_root = PathBuf::from("/Users/x/Library/Application Support/daruda/node");
    let cmd = format!("npx -y {ADAPTER_NPM_PACKAGE}");
    let wrapped = NodeRuntime::System.wrap_command(&cmd, &install_root);

    let agent = AcpAgent::from_str(&wrapped.0).expect("wrapped command parses");
    let config = agent.into_config();
    assert_eq!(config.command(), PathBuf::from("npx"));
    assert_eq!(config.arguments(), vec!["-y", ADAPTER_NPM_PACKAGE]);
    let cache = config
        .environment()
        .get("npm_config_cache")
        .expect("npm_config_cache env present");
    assert_eq!(
        cache.as_str(),
        npx_cache_dir(&install_root).to_string_lossy()
    );
}

#[test]
fn system_runtime_preserves_a_single_quoted_spaced_env_prefix() {
    // End-to-end regression for the two-sided fix: `AgentLaunch::
    // wrap_with_env`'s `Raw` branch single-quotes an injected env value
    // that can contain a space (a real account config dir under
    // `default_data_dir()`). The wrapped command must still detect
    // `npx` as the launcher, and the final string must still parse back
    // into one env assignment with the space intact via
    // `AcpAgent::from_str`.
    let install_root = test_install_root();
    let cmd = format!(
        "CLAUDE_CONFIG_DIR='/Users/x/Library/Application Support/daruda/acc/alice' npx -y {ADAPTER_NPM_PACKAGE}"
    );
    assert!(command_needs_node(&cmd));

    let wrapped = NodeRuntime::System.wrap_command(&cmd, &install_root);
    let agent = AcpAgent::from_str(&wrapped.0).expect("wrapped command parses");
    let config = agent.into_config();
    assert_eq!(config.command(), PathBuf::from("npx"));
    assert_eq!(config.arguments(), vec!["-y", ADAPTER_NPM_PACKAGE]);
    let config_dir = config
        .environment()
        .get("CLAUDE_CONFIG_DIR")
        .expect("CLAUDE_CONFIG_DIR env present");
    assert_eq!(
        config_dir,
        "/Users/x/Library/Application Support/daruda/acc/alice"
    );
}

#[test]
fn system_runtime_passes_a_non_node_command_through_unchanged() {
    let cmd = "/usr/local/bin/codex-acp --flag";
    assert_eq!(
        NodeRuntime::System
            .wrap_command(cmd, &test_install_root())
            .0,
        cmd
    );
}

#[test]
fn managed_runtime_command_round_trips_to_absolute_npx_with_path_env() {
    let node_dir = PathBuf::from("/data/daruda/node/node-v24.11.0-darwin-arm64");
    let cmd = format!("npx -y {ADAPTER_NPM_PACKAGE}");
    let command = NodeRuntime::Managed {
        node_dir: node_dir.clone(),
    }
    .wrap_command(&cmd, &test_install_root());

    // The command must be JSON (leading `{`) so no shell splitting happens.
    assert!(command.0.trim_start().starts_with('{'), "{}", command.0);

    // And it must parse back into a config transport with the absolute npx
    // path, the adapter package args, and a PATH env prepending the bin dir.
    let agent = AcpAgent::from_str(&command.0).expect("managed command parses");
    let config = agent.into_config();
    assert_eq!(config.command(), managed_launcher(&node_dir, "npx"));
    assert_eq!(config.arguments(), vec!["-y", ADAPTER_NPM_PACKAGE]);
    let path = config.environment().get("PATH").expect("PATH env present");
    assert!(
        path.starts_with(&bin_dir(&node_dir).to_string_lossy().into_owned()),
        "PATH must start with the managed bin dir, got {path}"
    );
}

#[test]
fn managed_runtime_sets_an_isolated_npm_config_cache() {
    // The regression this guards: a stray wrong-arch Node.js elsewhere on
    // the machine can populate the *default* `~/.npm/_npx` cache slot for
    // a package spec with the wrong-arch native binary; daruda's own
    // correctly-arch-selected managed run must never reuse that slot.
    let node_dir = PathBuf::from("/data/daruda/node/node-v24.11.0-darwin-arm64");
    let cmd = format!("npx -y {ADAPTER_NPM_PACKAGE}");
    let install_root = test_install_root();
    let command = NodeRuntime::Managed { node_dir }.wrap_command(&cmd, &install_root);
    let agent = AcpAgent::from_str(&command.0).expect("managed command parses");
    let config = agent.into_config();
    let cache = config
        .environment()
        .get("npm_config_cache")
        .expect("npm_config_cache env present");
    assert_eq!(
        cache.as_str(),
        install_root.join("npx-cache").to_string_lossy()
    );
}

#[test]
fn managed_runtime_lets_an_explicit_npm_config_cache_prefix_win() {
    let node_dir = PathBuf::from("/data/daruda/node/node-v24.11.0-darwin-arm64");
    let command = NodeRuntime::Managed { node_dir }.wrap_command(
        "npm_config_cache=/custom/cache npx -y pkg",
        &test_install_root(),
    );
    let agent = AcpAgent::from_str(&command.0).expect("managed command parses");
    let config = agent.into_config();
    // The env is a map, so a duplicate entry is unrepresentable; what the
    // `has_cache_override` guard still buys is that the app-owned value
    // does not *overwrite* the caller's.
    assert_eq!(
        config
            .environment()
            .get("npm_config_cache")
            .map(String::as_str),
        Some("/custom/cache"),
        "the caller's explicit npm_config_cache must survive"
    );
}

#[test]
fn managed_runtime_rewrites_the_node_launcher_token() {
    let node_dir = PathBuf::from("/data/daruda/node/node-v24.11.0-darwin-arm64");
    let command = NodeRuntime::Managed {
        node_dir: node_dir.clone(),
    }
    .wrap_command("node /path/adapter.js --flag", &test_install_root());
    let agent = AcpAgent::from_str(&command.0).expect("managed node command parses");
    let config = agent.into_config();
    assert_eq!(config.command(), managed_launcher(&node_dir, "node"));
    assert_eq!(config.arguments(), vec!["/path/adapter.js", "--flag"]);
}

#[test]
fn managed_runtime_preserves_env_prefix_when_rewriting_npx() {
    let node_dir = PathBuf::from("/data/daruda/node/node-v24.11.0-darwin-arm64");
    let command = NodeRuntime::Managed {
        node_dir: node_dir.clone(),
    }
    .wrap_command(
        "AUGMENT_DISABLE_AUTO_UPDATE=1 npx -y @augmentcode/auggie@0.32.0 --acp",
        &test_install_root(),
    );
    let agent = AcpAgent::from_str(&command.0).expect("managed env command parses");
    let config = agent.into_config();
    assert_eq!(config.command(), managed_launcher(&node_dir, "npx"));
    assert_eq!(
        config.arguments(),
        vec!["-y", "@augmentcode/auggie@0.32.0", "--acp"]
    );
    assert_eq!(
        config
            .environment()
            .get("AUGMENT_DISABLE_AUTO_UPDATE")
            .map(String::as_str),
        Some("1"),
        "registry env var must be preserved"
    );
    assert!(
        config.environment().contains_key("PATH"),
        "managed Node PATH must still be injected"
    );
}

#[test]
fn managed_runtime_passes_non_node_command_through() {
    let node_dir = PathBuf::from("/data/daruda/node/node-v24.11.0-darwin-arm64");
    let cmd = "/usr/local/bin/codex-acp";
    assert_eq!(
        NodeRuntime::Managed { node_dir }
            .wrap_command(cmd, &test_install_root())
            .0,
        cmd
    );
}

#[test]
fn managed_command_survives_a_path_with_spaces() {
    // macOS `Application Support` has a space — the JSON form must keep the
    // absolute npx path intact where a bash string would split it.
    let node_dir = PathBuf::from(
        "/Users/x/Library/Application Support/daruda/node/node-v24.11.0-darwin-arm64",
    );
    let cmd = format!("npx -y {ADAPTER_NPM_PACKAGE}");
    let command = NodeRuntime::Managed {
        node_dir: node_dir.clone(),
    }
    .wrap_command(&cmd, &test_install_root());
    let agent = AcpAgent::from_str(&command.0).expect("command with spaces parses");
    let config = agent.into_config();
    assert_eq!(config.command(), managed_launcher(&node_dir, "npx"));
}

#[test]
fn parse_node_version_reads_v_prefixed_output() {
    assert_eq!(
        parse_node_version("v24.11.0\n"),
        Some(Version::new(24, 11, 0))
    );
    assert_eq!(
        parse_node_version("  v20.0.0  "),
        Some(Version::new(20, 0, 0))
    );
    assert_eq!(parse_node_version("not a version"), None);
}

#[test]
fn min_version_gate_rejects_old_and_accepts_current() {
    assert!(parse_node_version("v18.20.0").unwrap() < MIN_NODE_VERSION);
    assert!(parse_node_version("v20.0.0").unwrap() >= MIN_NODE_VERSION);
    assert!(parse_node_version("v22.0.0").unwrap() >= MIN_NODE_VERSION);
    assert!(parse_node_version("v24.11.0").unwrap() >= MIN_NODE_VERSION);
}

#[test]
fn managed_dir_and_binary_paths() {
    let root = PathBuf::from("/data/node");
    let dir = managed_node_dir(&root, "darwin", "arm64");
    assert_eq!(
        dir,
        root.join(format!("node-{MANAGED_NODE_VERSION}-darwin-arm64"))
    );
    let relative = if cfg!(windows) {
        "node.exe"
    } else {
        "bin/node"
    };
    assert_eq!(node_binary(&dir), dir.join(relative));
}

#[test]
fn find_checksum_matches_exact_filename() {
    let shasums = "\
aaaa1111  node-v24.11.0-darwin-arm64.tar.gz
bbbb2222  node-v24.11.0-darwin-x64.tar.gz
cccc3333  node-v24.11.0-linux-x64.tar.xz
";
    assert_eq!(
        find_checksum(shasums, "node-v24.11.0-darwin-arm64.tar.gz"),
        Some("aaaa1111")
    );
    assert_eq!(
        find_checksum(shasums, "node-v24.11.0-darwin-x64.tar.gz"),
        Some("bbbb2222")
    );
    // A prefix match must not leak across filenames.
    assert_eq!(find_checksum(shasums, "node-v24.11.0-darwin.tar.gz"), None);
    assert_eq!(find_checksum(shasums, "missing.tar.gz"), None);
}

#[test]
fn sha256_hex_is_lowercase_and_correct() {
    // SHA-256 of the empty string.
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    // SHA-256 of "abc".
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn prepend_to_path_puts_bin_dir_first() {
    let result = prepend_to(
        Path::new("/managed/bin"),
        Some(std::env::join_paths(["/usr/bin", "/bin"]).unwrap()),
    );

    assert_eq!(
        std::env::split_paths(&result).collect::<Vec<_>>(),
        [
            PathBuf::from("/managed/bin"),
            PathBuf::from("/usr/bin"),
            PathBuf::from("/bin")
        ]
    );
}

#[test]
fn prepend_to_an_unset_path_is_just_the_bin_dir() {
    let result = prepend_to(Path::new("/managed/bin"), None);

    assert_eq!(result, "/managed/bin");
}

/// The separator is the platform's, not a `:` someone typed. This is what
/// the hand-rolled `format!("{}:{existing}")` in the app got wrong, and
/// routing that caller through here is what this asserts is enough.
#[test]
fn path_with_node_joins_with_the_platform_separator() {
    let result = prepend_to(
        &bin_dir(Path::new("/managed")),
        Some(std::ffi::OsString::from("/usr/bin")),
    );

    let sep = if cfg!(windows) { ';' } else { ':' };
    let (first, rest) = result.split_once(sep).expect("two entries");
    assert_eq!(Path::new(first), bin_dir(Path::new("/managed")));
    assert_eq!(rest, "/usr/bin");
}

/// Layout lives in one accessor, so everything derived from it agrees.
#[test]
fn the_node_binary_sits_in_the_bin_dir() {
    let root = Path::new("/managed");

    assert!(node_binary(root).starts_with(bin_dir(root)));
}

#[test]
fn extract_archive_unpacks_a_real_archive() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir(&src).unwrap();
    std::fs::write(src.join("hello.txt"), b"hi").unwrap();

    // Build a gzip tarball with the system tar, then extract it back.
    let tarball = dir.path().join("out.tar.gz");
    let status = daruda_core::process::command("tar")
        .arg("-czf")
        .arg(&tarball)
        .arg("-C")
        .arg(&src)
        .arg("hello.txt")
        .status()
        .unwrap();
    assert!(status.success());
    let bytes = std::fs::read(&tarball).unwrap();

    let dest = dir.path().join("dest");
    std::fs::create_dir(&dest).unwrap();
    extract_archive(&bytes, &dest).expect("extraction succeeds");
    assert_eq!(std::fs::read(dest.join("hello.txt")).unwrap(), b"hi");
}

#[test]
fn ensure_node_prefers_system_when_available() {
    // The CI / dev host running these tests has a recent node on PATH, so
    // ensure_node must pick System and never touch the (nonexistent) root.
    if !detect_system_node() {
        // No system node here — skip rather than assert on the environment.
        return;
    }
    let mut seen = Vec::new();
    let runtime = ensure_node(Path::new("/nonexistent-daruda-node-root"), &mut |p| {
        seen.push(p)
    })
    .expect("system node is used");
    assert_eq!(runtime, NodeRuntime::System);
    assert_eq!(seen, vec![NodeProgress::UsingSystemNode]);
}

/// Executable shell script standing in for `node -e "process.stdout
/// .write(process.arch)"`, so arch-matching can be tested without
/// depending on a real Node.js or the test host's own arch.
// WORKAROUND: `node_platform` has no Windows arm yet, so the true case
// cannot even name an expected arch there — and this stub is a POSIX
// shell script. Both lift together when the managed-node Windows layout
// (.zip, node.exe at the root) lands; the stub becomes a .cmd then.
#[cfg(unix)]
fn write_fake_node_reporting(path: &Path, output: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, format!("#!/bin/sh\nprintf '%s' '{output}'\n")).unwrap();
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).unwrap();
}

#[cfg(unix)]
#[test]
fn system_node_arch_matches_host_true_when_reported_arch_matches() {
    let (_, expected) = node_platform().expect("supported test platform");
    let dir = tempfile::tempdir().unwrap();
    let fake_node = dir.path().join("node");
    write_fake_node_reporting(&fake_node, expected);
    assert!(system_node_arch_matches_host(&fake_node));
}

#[cfg(unix)]
#[test]
fn system_node_arch_matches_host_false_on_arch_mismatch() {
    let dir = tempfile::tempdir().unwrap();
    let fake_node = dir.path().join("node");
    write_fake_node_reporting(&fake_node, "definitely-not-a-real-arch");
    assert!(!system_node_arch_matches_host(&fake_node));
}

#[test]
fn system_node_arch_matches_host_false_on_missing_binary() {
    assert!(!system_node_arch_matches_host(Path::new(
        "/nonexistent-daruda-node-probe-binary"
    )));
}

/// End-to-end check of the managed path against the *real* nodejs.org: it
/// downloads the pinned build, verifies it against the published checksum,
/// extracts it with the system `tar`, and confirms the extracted `node`
/// runs. Ignored by default (network + tens of MB); run explicitly with
/// `cargo test -p daruda_acp -- --ignored install_managed`.
#[test]
#[ignore = "network: downloads a real Node.js from nodejs.org"]
fn install_managed_downloads_verifies_and_runs() {
    let dir = tempfile::tempdir().unwrap();
    let (os, arch) = node_platform().expect("supported test platform");
    let mut seen = Vec::new();
    let runtime =
        install_managed(dir.path(), os, arch, &mut |p| seen.push(p)).expect("managed install");
    match runtime {
        NodeRuntime::Managed { node_dir } => {
            assert!(node_binary(&node_dir).exists(), "node binary extracted");
            assert!(managed_launcher(&node_dir, "npx").exists(), "npx extracted");
            assert!(managed_cache_valid(&node_dir), "extracted node runs");
            for launcher in ["node", "npx"] {
                let prepared = crate::launch_config::finalize_command(
                    NodeRuntime::Managed {
                        node_dir: node_dir.clone(),
                    }
                    .wrap_command(&format!("{launcher} --version"), dir.path()),
                    &["TEST_STRIPPED".into()],
                );
                let config = AcpAgent::from_str(&prepared.0).unwrap().into_config();
                let output = daruda_core::process::command(config.command())
                    .args(config.arguments())
                    .envs(config.environment())
                    .env("TEST_STRIPPED", "hidden")
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{launcher}: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                assert!(!output.stdout.is_empty());
            }
        }
        other => panic!("expected managed runtime, got {other:?}"),
    }
    assert!(seen.contains(&NodeProgress::Downloading));
    assert!(seen.contains(&NodeProgress::Verifying));
    assert!(seen.contains(&NodeProgress::Extracting));
}
