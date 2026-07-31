//! Node.js runtime provisioning for the ACP adapter — GPUI-free.
//!
//! The Claude ACP adapter is spawned as `npx -y <pkg>` (see
//! [`crate::connection::AdapterCommand`]), which needs Node.js. A macOS `.app`
//! launched from Finder inherits only launchd's minimal `PATH`; the login-shell
//! `PATH` hydration in the host app fixes the *"node is installed but off PATH"*
//! case, but a machine with **no Node.js at all** still fails the spawn with a
//! raw `os error 2`.
//!
//! This module closes that gap the way zed's `node_runtime` does: reuse the
//! system Node.js when it is present and recent enough, otherwise download a
//! pinned Node.js from `nodejs.org/dist` into an app-managed directory —
//! integrity-checked against the published `SHASUMS256.txt` — so the adapter
//! runs with zero user setup.
//!
//! Blocking on purpose (system `tar` + synchronous `ureq`): it runs on the
//! host's background executor, mirroring the blocking git-CLI layer. Nothing
//! here touches GPUI.
//!
//! Both runtimes also redirect `npm_config_cache` to an app-owned directory
//! (`<install_root>/npx-cache`) instead of the default `~/.npm` — see
//! [`npx_cache_dir`]'s doc comment for why the shared, unversioned default is
//! unsafe to trust. One cost of that isolation: an existing install upgrading
//! into this cache for the first time reinstalls each configured agent's npm
//! package once (a fresh directory, not a migration of the old one — see
//! [`npx_cache_dir`]) instead of reusing whatever was already warm in
//! `~/.npm/_npx`.

use std::fmt::Write as _;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use agent_client_protocol::AcpAgentConfig;
use semver::Version;
use sha2::{Digest, Sha256};

use crate::connection::AdapterCommand;

/// Pinned Node.js version for the managed install. A single pinned version keeps
/// the download URL and integrity check deterministic; bump it periodically to a
/// current LTS. Matches the version zed pins so the URL is known-good.
const MANAGED_NODE_VERSION: &str = "v24.11.0";

/// Minimum acceptable system Node.js version. Below this, a system install is
/// treated as absent and the managed runtime is used instead — old Node.js
/// tends to fail the adapter with cryptic errors, which is worse than a
/// one-time managed download.
const MIN_NODE_VERSION: Version = Version::new(20, 0, 0);

/// Base URL for the official Node.js distribution.
const NODE_DIST_BASE: &str = "https://nodejs.org/dist";

/// Overall timeout for the (large) tarball / checksum downloads.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// Serializes managed installs across concurrent lane connections so two panes
/// starting at once download once, not twice (the second waiter re-checks the
/// cache and reuses it). System detection needs no lock.
static INSTALL_LOCK: Mutex<()> = Mutex::new(());

/// A usable Node.js runtime for the ACP adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeRuntime {
    /// The user's own Node.js, found on `PATH` and recent enough. The adapter
    /// runs via the default `npx -y <pkg>` command, unchanged.
    System,
    /// An app-managed Node.js under the install root. `node_dir` is the
    /// extracted `node-<ver>-<os>-<arch>` directory; its `bin/` holds `node` and
    /// `npx`.
    Managed { node_dir: PathBuf },
}

/// `true` when launching `command` needs a Node.js runtime provisioned.
///
/// A JSON stdio config (trimmed string starts with `{`) is a self-contained
/// transport that names its own executable, so it needs nothing. Any other
/// binary is assumed already runnable. Only a leading `npx` / `node` token —
/// the Node-family launchers — triggers provisioning.
#[must_use]
pub fn command_needs_node(command: &str) -> bool {
    let trimmed = command.trim_start();
    if trimmed.starts_with('{') {
        return false;
    }
    matches!(
        launcher_after_env_prefix(trimmed).as_deref(),
        Some("npx" | "node")
    )
}

/// The command's first real token — the executable name, after stripping any
/// leading `NAME=value` env-prefix assignments (see `daruda_config`'s
/// `AgentLaunch::wrap_with_env`). Exposed for the Settings window's
/// local-PATH check, which needs the same env-prefix-aware tokenization
/// [`command_needs_node`] uses but tests the token against a different set
/// (`npx`/`uvx`) than the Node-only launcher check.
#[must_use]
pub fn first_command_token(command: &str) -> Option<String> {
    launcher_after_env_prefix(command)
}

fn launcher_after_env_prefix(command: &str) -> Option<String> {
    let (_, tokens) = split_env_prefixed_tokens(command);
    tokens.into_iter().next()
}

/// Split `command` into its leading `NAME=value` env-prefix assignments and
/// the remaining command tokens.
///
/// Tokenizes with [`shell_words::split`] (quote-aware) rather than
/// [`str::split_whitespace`] so a quoted assignment value with an embedded
/// space — e.g. `CLAUDE_CONFIG_DIR='/Users/x/Library/Application
/// Support/daruda/acc/alice'`, produced by `daruda_config`'s
/// `AgentLaunch::wrap_with_env` (`Raw` branch) — is tokenized as a single
/// word and parsed as one assignment with its value's quotes stripped,
/// instead of exploding into two command tokens partway through the path.
/// This mirrors how the eventual consumer, `AcpAgent::from_str`, re-tokenizes
/// the same string (see [`prefix_with_host_arch_env`]'s doc comment) — both
/// must agree on where the env prefix ends and the command begins. An
/// unquoted `NAME=value` token still parses exactly as before, since
/// `shell_words::split` treats it as one plain word when it contains no
/// whitespace to begin with.
///
/// Falls back to a plain whitespace split on a `shell_words` parse error
/// (e.g. an unmatched quote in a hand-written command) so a malformed string
/// still yields *some* tokens instead of dropping the command entirely.
fn split_env_prefixed_tokens(command: &str) -> (Vec<(String, String)>, Vec<String>) {
    let words = shell_words::split(command)
        .unwrap_or_else(|_| command.split_whitespace().map(str::to_string).collect());

    let mut env = Vec::new();
    let mut command_tokens = Vec::new();
    let mut saw_command = false;

    for word in words {
        if !saw_command && let Some((name, value)) = parse_env_assignment(&word) {
            env.push((name.to_string(), value.to_string()));
            continue;
        }
        saw_command = true;
        command_tokens.push(word);
    }

    (env, command_tokens)
}

fn parse_env_assignment(token: &str) -> Option<(&str, &str)> {
    let eq_pos = token.find('=')?;
    if eq_pos == 0 {
        return None;
    }

    let name = &token[..eq_pos];
    let value = &token[eq_pos + 1..];

    let mut chars = name.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic() && first != '_' {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }

    Some((name, value))
}

impl NodeRuntime {
    /// Wrap an agent launch command for this runtime.
    ///
    /// `install_root` is the same app-managed directory passed to
    /// [`ensure_node`] — used here only to derive [`npx_cache_dir`], not to
    /// locate a Node.js install (a `System` runtime needs no such thing).
    ///
    /// System, for an `npx` / `node` command, prepends an arch-pinning and
    /// cache-isolating env prefix — see [`prefix_with_host_arch_env`]. Any
    /// other System command passes through unchanged; `node` / `npx` are
    /// already on the hydrated `PATH`. Managed, for an `npx` / `node`
    /// command, rewrites it to a JSON stdio config: the leading launcher
    /// token becomes the **absolute** path inside the managed `bin/`, the
    /// remaining tokens are its args, a `PATH` prepending the managed `bin/`
    /// is injected so the launcher, and the adapter it spawns, find the
    /// managed `node`, and (unless the command already sets its own
    /// `npm_config_cache`) an isolated cache dir is injected too — see
    /// [`npx_cache_dir`]. JSON (not a bash string) is used because the
    /// managed path can contain spaces (macOS `Application Support`), which
    /// bash-word splitting would break. A Managed runtime with any other
    /// command passes it through unchanged.
    ///
    /// Runtime selection only — the auth-env strip is *not* applied here;
    /// [`crate::launch_env::prepare_adapter_command`] applies it once to
    /// whatever shape this returns.
    ///
    /// Only simple whitespace-tokenized `npx -y <pkg>` / `node <script> …` forms
    /// are rewritten; a caller needing more control supplies a JSON stdio config
    /// directly.
    #[must_use]
    pub fn wrap_command(&self, command: &str, install_root: &Path) -> AdapterCommand {
        match self {
            NodeRuntime::System if command_needs_node(command) => {
                AdapterCommand(prefix_with_host_arch_env(command, install_root))
            }
            NodeRuntime::System => AdapterCommand(command.to_string()),
            NodeRuntime::Managed { node_dir } if command_needs_node(command) => {
                let bin_dir = node_dir.join("bin");
                let (env_assignments, tokens) = split_env_prefixed_tokens(command);
                let has_cache_override = env_assignments
                    .iter()
                    .any(|(name, _)| name == "npm_config_cache");
                let launcher = tokens.first().cloned().unwrap_or_default();
                let abs_launcher = bin_dir.join(&launcher);
                let args = tokens.into_iter().skip(1).collect::<Vec<_>>();
                let path = prepend_to_path(&bin_dir);
                let mut config = AcpAgentConfig::new(abs_launcher)
                    .args(args)
                    .envs(env_assignments)
                    .env("PATH", path);
                // Skipped when the command's own env prefix already sets
                // `npm_config_cache` — respect that explicit override rather
                // than silently discard it, since setting ours here would
                // overwrite the entry that prefix already put in the map.
                if !has_cache_override {
                    config = config.env(
                        "npm_config_cache",
                        npx_cache_dir(install_root).to_string_lossy().into_owned(),
                    );
                }
                // `AcpAgent::from_str` parses a leading `{` as a JSON
                // `AcpAgentConfig`, so serializing the SDK's own launch type
                // gives a shell-safe command that needs no re-translation
                // downstream. serde can't fail on this owned value.
                let json =
                    serde_json::to_string(&config).expect("AcpAgentConfig serializes to JSON");
                AdapterCommand(json)
            }
            NodeRuntime::Managed { .. } => AdapterCommand(command.to_string()),
        }
    }
}

/// Progress milestones during [`ensure_node`], for a host status line. The
/// managed download is the only slow path (tens of MB on first run), so the
/// host can show "preparing runtime…" instead of an apparent hang.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeProgress {
    /// System Node.js was found and will be used — no download.
    UsingSystemNode,
    /// Checking the managed cache for an existing install.
    CheckingCache,
    /// Downloading the Node.js archive.
    Downloading,
    /// Verifying the archive checksum.
    Verifying,
    /// Extracting the archive.
    Extracting,
}

/// Failure modes of node provisioning. `Display` is user-facing (surfaced as a
/// toast + status line by the host), so messages name the remedy.
#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    /// The current OS/architecture has no managed Node.js build.
    #[error(
        "no managed Node.js build for this platform ({0}); install Node.js from https://nodejs.org and restart daruda"
    )]
    UnsupportedPlatform(String),
    /// The download (tarball or checksums) failed.
    #[error(
        "couldn't download Node.js ({0}) — check your internet connection, or install Node.js from https://nodejs.org and restart daruda"
    )]
    Download(String),
    /// The downloaded archive did not match its published checksum.
    #[error("the downloaded Node.js archive failed its integrity check; not installing it")]
    Checksum {
        /// Checksum published in `SHASUMS256.txt`.
        expected: String,
        /// Checksum computed over the downloaded bytes.
        actual: String,
    },
    /// Extraction or on-disk setup failed.
    #[error("couldn't set up the downloaded Node.js: {0}")]
    Extract(String),
}

/// Ensure a usable Node.js runtime, preferring the user's own install.
///
/// Order: system Node.js (recent enough) → managed cache → download + verify +
/// extract. `install_root` is the app-managed directory that holds managed
/// installs (injected so tests and profiles stay isolated). `progress` is
/// called at each milestone.
pub fn ensure_node(
    install_root: &Path,
    progress: &mut dyn FnMut(NodeProgress),
) -> Result<NodeRuntime, NodeError> {
    if detect_system_node() {
        progress(NodeProgress::UsingSystemNode);
        return Ok(NodeRuntime::System);
    }

    let (os, arch) = node_platform()?;
    let node_dir = managed_node_dir(install_root, os, arch);

    progress(NodeProgress::CheckingCache);
    if managed_cache_valid(&node_dir) {
        return Ok(NodeRuntime::Managed { node_dir });
    }

    install_managed(install_root, os, arch, progress)
}

/// `true` if a system `node` is on `PATH`, at least [`MIN_NODE_VERSION`], and
/// built for the host's actual CPU architecture.
///
/// The arch check matters: an x86_64 Node.js under Rosetta on Apple Silicon
/// still reports `process.arch: "x64"`, so `npx` installs any wrong-arch
/// *native binary* optionalDependency (e.g. the ACP adapter's bundled Claude
/// Code engine) — which then hangs silently under translation. Rejecting a
/// mismatched system Node.js here routes to the managed download instead,
/// which [`node_platform`] always builds for the real host arch.
fn detect_system_node() -> bool {
    let Ok(node) = which::which("node") else {
        return false;
    };
    // `npx` must exist too — the adapter is launched through it.
    if which::which("npx").is_err() {
        return false;
    }
    let Ok(output) = std::process::Command::new(&node).arg("--version").output() else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let version_ok = match parse_node_version(&String::from_utf8_lossy(&output.stdout)) {
        Some(version) => version >= MIN_NODE_VERSION,
        None => false,
    };
    version_ok && system_node_arch_matches_host(&node)
}

/// `true` if `node`'s own `process.arch` matches the host's actual CPU
/// architecture (per [`node_platform`]'s `(os, arch)` mapping). `false` on
/// any probe failure — an architecture we can't confirm is treated as a
/// mismatch, favoring the always-correct managed download over a guess.
fn system_node_arch_matches_host(node: &Path) -> bool {
    let Ok((_, expected_arch)) = node_platform() else {
        return false;
    };
    let Ok(output) = std::process::Command::new(node)
        .args(["-e", "process.stdout.write(process.arch)"])
        .output()
    else {
        return false;
    };
    output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == expected_arch
}

/// Parse `node --version` output (`v24.11.0\n`) into a [`Version`].
fn parse_node_version(output: &str) -> Option<Version> {
    Version::parse(output.trim().trim_start_matches('v')).ok()
}

/// Node's `(os, arch)` tokens for the current platform, as used in the
/// distribution file names. `Err` on an unsupported platform.
pub(crate) fn node_platform() -> Result<(&'static str, &'static str), NodeError> {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "linux" => "linux",
        other => return Err(NodeError::UnsupportedPlatform(format!("os: {other}"))),
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => return Err(NodeError::UnsupportedPlatform(format!("arch: {other}"))),
    };
    Ok((os, arch))
}

/// Prepend `npm_config_cpu=<arch> npm_config_os=<os> npm_config_cache=<dir>`
/// — npm's env-var form of `--cpu`/`--os`/`--cache` — to `command`.
///
/// `npm_config_cpu`/`npm_config_os` make `npx` install a platform-specific
/// native-binary optionalDependency for the real host arch even if the
/// running Node.js itself reports a different `process.arch` (e.g. x64
/// under Rosetta); this is install-time defense-in-depth alongside
/// [`detect_system_node`]'s own arch check, and does not fix a package whose
/// *runtime* also self-detects arch off the live process rather than off
/// what got installed. `npm_config_cache` redirects npx's cache to
/// [`npx_cache_dir`] for the same isolation reason `Managed` needs it — see
/// that function's doc comment.
///
/// Prepending (rather than appending) means a command whose own env prefix
/// already sets one of these three names — legal per
/// [`split_env_prefixed_tokens`], e.g. an `npm_config_cache=... npx ...`
/// override — still wins: `AcpAgent::from_str` parses *all* leading
/// `NAME=value` tokens left to right into one env list applied via
/// sequential last-write-wins `Command::env` calls, so whichever assignment
/// for a given name appears **later** in the string (the command's own,
/// here) takes effect.
///
/// Best-effort — an unsupported `node_platform()` just leaves `command`
/// unprefixed.
///
/// `npm_config_cache`'s value is shell-quoted (`shell_words::quote`): it is
/// [`npx_cache_dir`], a child of `install_root`, which on macOS is always
/// under `~/Library/Application Support` — a path containing a space. This
/// whole prefixed string is later re-tokenized by `AcpAgent::from_str` via
/// `shell_words::split`, which treats an unquoted space as a word boundary;
/// an unquoted cache path there splits `npm_config_cache=...` into two
/// tokens, and the env-parsing loop stops at the first token without `=`,
/// treating that fragment as the command to spawn instead of `npx` — an
/// `ENOENT` that looks like a missing Node.js rather than a quoting bug.
fn prefix_with_host_arch_env(command: &str, install_root: &Path) -> String {
    match node_platform() {
        Ok((os, arch)) => {
            let cache = npx_cache_dir(install_root);
            format!(
                "npm_config_cpu={arch} npm_config_os={os} npm_config_cache={} {command}",
                shell_words::quote(&cache.to_string_lossy())
            )
        }
        Err(_) => command.to_string(),
    }
}

/// `node-<ver>-<os>-<arch>` — the distribution folder / archive stem.
fn managed_folder_name(os: &str, arch: &str) -> String {
    format!("node-{MANAGED_NODE_VERSION}-{os}-{arch}")
}

/// The extracted managed node directory under `install_root`.
fn managed_node_dir(install_root: &Path, os: &str, arch: &str) -> PathBuf {
    install_root.join(managed_folder_name(os, arch))
}

/// App-owned `npm_config_cache` target, used by both runtimes — a sibling of
/// the versioned managed `node_dir` under `install_root` (so it is shared
/// across a `MANAGED_NODE_VERSION` bump, not tied to one extracted Node.js
/// tree), but populated by whichever Node.js — system or managed — is
/// currently selected.
///
/// Needs to be app-owned rather than the default, shared `~/.npm`: a
/// wrong-arch Node.js run elsewhere on the machine (a stray terminal
/// command, another tool, or a session predating this isolation) can leave a
/// wrong-arch native-binary `optionalDependency` cached under `~/.npm/_npx`
/// for a given package spec; daruda reusing that slot — even with a
/// correctly-arch-selected runtime of its own — reproduces the same "native
/// binary not found" failure. Isolating the cache means only daruda's own,
/// already-arch-verified runtime ever writes into it.
///
/// Trade-off: this directory starts empty, so the very first launch after
/// adopting this isolation (or after a fresh install) reinstalls each
/// configured agent's npm package once via `npx`, instead of reusing
/// whatever was already warm in the old shared `~/.npm/_npx`. Deliberately
/// not seeded from that old cache — the whole point of isolating it is to
/// stop trusting cache entries daruda didn't itself just write.
fn npx_cache_dir(install_root: &Path) -> PathBuf {
    install_root.join("npx-cache")
}

/// The `node` binary inside an extracted managed node directory.
fn node_binary(node_dir: &Path) -> PathBuf {
    node_dir.join("bin").join("node")
}

/// `true` if a managed install exists and its `node` runs. Cheap validity gate
/// that also self-heals a truncated / corrupt extraction (it re-downloads).
fn managed_cache_valid(node_dir: &Path) -> bool {
    let node = node_binary(node_dir);
    match std::process::Command::new(&node).arg("--version").output() {
        Ok(output) => output.status.success(),
        Err(_) => false,
    }
}

/// Download, verify, and extract the pinned Node.js into `install_root`,
/// returning the [`NodeRuntime::Managed`]. Serialized by [`INSTALL_LOCK`]; the
/// cache is re-checked after acquiring so a queued caller reuses a just-finished
/// install instead of downloading again.
fn install_managed(
    install_root: &Path,
    os: &str,
    arch: &str,
    progress: &mut dyn FnMut(NodeProgress),
) -> Result<NodeRuntime, NodeError> {
    let _guard = INSTALL_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let node_dir = managed_node_dir(install_root, os, arch);
    progress(NodeProgress::CheckingCache);
    if managed_cache_valid(&node_dir) {
        return Ok(NodeRuntime::Managed { node_dir });
    }

    let file_name = format!("{}.tar.gz", managed_folder_name(os, arch));
    let archive_url = format!("{NODE_DIST_BASE}/{MANAGED_NODE_VERSION}/{file_name}");
    let shasums_url = format!("{NODE_DIST_BASE}/{MANAGED_NODE_VERSION}/SHASUMS256.txt");

    progress(NodeProgress::Downloading);
    let archive = http_get_bytes(&archive_url)?;

    progress(NodeProgress::Verifying);
    let shasums = http_get_string(&shasums_url)?;
    let expected = find_checksum(&shasums, &file_name)
        .ok_or_else(|| NodeError::Download(format!("no checksum listed for {file_name}")))?;
    let actual = sha256_hex(&archive);
    if actual != expected {
        return Err(NodeError::Checksum {
            expected: expected.to_string(),
            actual,
        });
    }

    progress(NodeProgress::Extracting);
    std::fs::create_dir_all(install_root)
        .map_err(|e| NodeError::Extract(format!("creating {}: {e}", install_root.display())))?;

    // Extract into a private staging dir, then publish with an atomic rename.
    // The install root is shared across profiles *and* instances (see
    // `node_install_dir`), so `INSTALL_LOCK` — a process-local mutex — does not
    // serialize a second process. Extracting straight into `node_dir` would let
    // a concurrent installer observe or clobber a half-written tree; staging +
    // rename means each process builds its own copy and exactly one publish
    // wins, the other reuses it.
    let staging = install_root.join(format!(".staging-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)
        .map_err(|e| NodeError::Extract(format!("creating staging dir: {e}")))?;
    let result = publish_managed(&archive, os, arch, &staging, &node_dir);
    let _ = std::fs::remove_dir_all(&staging);
    result?;

    if !managed_cache_valid(&node_dir) {
        return Err(NodeError::Extract(
            "the extracted Node.js did not run".to_string(),
        ));
    }
    Ok(NodeRuntime::Managed { node_dir })
}

/// Extract `archive` into `staging` and publish the resulting node tree to
/// `node_dir`. Publishing is a same-filesystem `rename`, atomic when `node_dir`
/// is absent. A concurrent process that published a valid install first wins
/// (its tree is kept); a stale/corrupt `node_dir` is removed before the rename
/// (that removal is serialized within the process by `INSTALL_LOCK`, and a lost
/// cross-process race is caught by the post-rename re-validation).
fn publish_managed(
    archive: &[u8],
    os: &str,
    arch: &str,
    staging: &Path,
    node_dir: &Path,
) -> Result<(), NodeError> {
    extract_tar_gz(archive, staging)?;
    let extracted = staging.join(managed_folder_name(os, arch));
    if node_dir.exists() && !managed_cache_valid(node_dir) {
        let _ = std::fs::remove_dir_all(node_dir);
    }
    match std::fs::rename(&extracted, node_dir) {
        Ok(()) => Ok(()),
        // Another process published a valid install between our checks and the
        // rename — keep theirs rather than fail.
        Err(_) if managed_cache_valid(node_dir) => Ok(()),
        Err(e) => Err(NodeError::Extract(format!("publishing node: {e}"))),
    }
}

/// GET `url` and return the body bytes (no size cap — the tarball is tens of MB).
fn http_get_bytes(url: &str) -> Result<Vec<u8>, NodeError> {
    let agent = ureq::AgentBuilder::new().timeout(DOWNLOAD_TIMEOUT).build();
    let response = agent
        .get(url)
        .call()
        .map_err(|e| NodeError::Download(e.to_string()))?;
    let mut buf = Vec::new();
    response
        .into_reader()
        .read_to_end(&mut buf)
        .map_err(|e| NodeError::Download(e.to_string()))?;
    Ok(buf)
}

/// GET `url` and return the body as a string (checksums file — small).
fn http_get_string(url: &str) -> Result<String, NodeError> {
    let agent = ureq::AgentBuilder::new().timeout(DOWNLOAD_TIMEOUT).build();
    agent
        .get(url)
        .call()
        .map_err(|e| NodeError::Download(e.to_string()))?
        .into_string()
        .map_err(|e| NodeError::Download(e.to_string()))
}

/// Find the checksum for `file_name` in a `SHASUMS256.txt` body (each line is
/// `<hex>  <filename>`).
fn find_checksum<'a>(shasums: &'a str, file_name: &str) -> Option<&'a str> {
    shasums.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?;
        (name == file_name).then_some(hash)
    })
}

/// Lowercase hex SHA-256 of `bytes`.
fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// `bin_dir:$PATH`, or just `bin_dir` when `PATH` is unset. Uses OS path joining
/// so a `bin_dir` with a separator-conflicting char is handled correctly.
fn prepend_to_path(bin_dir: &Path) -> String {
    match std::env::var_os("PATH") {
        Some(existing) => {
            let joined = std::iter::once(bin_dir.to_path_buf())
                .chain(std::env::split_paths(&existing))
                .collect::<Vec<_>>();
            std::env::join_paths(joined)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| bin_dir.to_string_lossy().into_owned())
        }
        None => bin_dir.to_string_lossy().into_owned(),
    }
}

/// Extract a gzip tarball (`bytes`) into `dest` via the system `tar`. macOS and
/// Linux both ship a `tar` with gzip support; shelling out avoids pulling a
/// `tar`/`flate2` crate for the one extraction, mirroring the blocking git-CLI
/// layer. The bytes are staged to a temp file (same filesystem as `dest`) since
/// `tar` reads a path.
fn extract_tar_gz(bytes: &[u8], dest: &Path) -> Result<(), NodeError> {
    let tmp = dest.join(format!(".node-download-{}.tar.gz", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(|e| NodeError::Extract(format!("staging archive: {e}")))?;
    let status = std::process::Command::new("tar")
        .arg("-xzf")
        .arg(&tmp)
        .arg("-C")
        .arg(dest)
        .status();
    let _ = std::fs::remove_file(&tmp);
    match status {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(NodeError::Extract(format!("tar exited with {status}"))),
        Err(e) => Err(NodeError::Extract(format!("running tar: {e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::ADAPTER_NPM_PACKAGE;
    use agent_client_protocol::AcpAgent;
    use std::str::FromStr;

    #[test]
    fn command_needs_node_only_for_npx_and_node_launchers() {
        assert!(command_needs_node(
            "npx -y @agentclientprotocol/claude-agent-acp@latest"
        ));
        assert!(command_needs_node("node /path/adapter.js"));
        // Leading whitespace before the launcher must not hide it.
        assert!(command_needs_node("   npx -y pkg"));
        // Registry commands may prefix environment assignments before npx.
        assert!(command_needs_node(
            "AUGMENT_DISABLE_AUTO_UPDATE=1 npx -y pkg --acp"
        ));
        assert!(command_needs_node("A=1 B=false node /path/adapter.js"));

        // A non-Node binary runs itself.
        assert!(!command_needs_node("/usr/local/bin/codex-acp"));
        assert!(!command_needs_node("A=1 /usr/local/bin/codex-acp"));
        assert!(!command_needs_node("A=1"));
        // A JSON config config is a self-contained transport.
        assert!(!command_needs_node(
            r#"{"type":"config","command":"codex","args":[]}"#
        ));
        // Empty string needs nothing.
        assert!(!command_needs_node(""));
    }

    #[test]
    fn split_env_prefixed_tokens_dequotes_a_single_quoted_spaced_value() {
        // Regression: `AgentLaunch::wrap_with_env`'s `Raw` branch
        // single-quotes an env value that can contain a space (a real
        // account config dir under `default_data_dir()`, e.g. macOS
        // `~/Library/Application Support/...`). Naive `split_whitespace`
        // would break this into two command tokens; the quote-aware split
        // must parse it as one assignment with the quotes stripped and the
        // internal space intact.
        let command =
            "CLAUDE_CONFIG_DIR='/Users/x/Library/Application Support/daruda/acc/alice' npx -y pkg";
        let (env, tokens) = split_env_prefixed_tokens(command);
        assert_eq!(
            env,
            vec![(
                "CLAUDE_CONFIG_DIR".to_string(),
                "/Users/x/Library/Application Support/daruda/acc/alice".to_string()
            )]
        );
        assert_eq!(
            tokens,
            vec!["npx".to_string(), "-y".to_string(), "pkg".to_string()]
        );
    }

    #[test]
    fn split_env_prefixed_tokens_dequotes_a_double_quoted_spaced_value() {
        let command = r#"CLAUDE_CONFIG_DIR="/Users/x/Library/Application Support/daruda/acc/alice" npx -y pkg"#;
        let (env, tokens) = split_env_prefixed_tokens(command);
        assert_eq!(
            env,
            vec![(
                "CLAUDE_CONFIG_DIR".to_string(),
                "/Users/x/Library/Application Support/daruda/acc/alice".to_string()
            )]
        );
        assert_eq!(
            tokens,
            vec!["npx".to_string(), "-y".to_string(), "pkg".to_string()]
        );
    }

    #[test]
    fn split_env_prefixed_tokens_still_parses_an_unquoted_assignment() {
        // Regression: the pre-existing, still-legal unquoted `NAME=value`
        // prefix (e.g. `AUGMENT_DISABLE_AUTO_UPDATE=1 npx ...`) must keep
        // working after switching the tokenizer from `split_whitespace` to
        // `shell_words::split`.
        let command = "AUGMENT_DISABLE_AUTO_UPDATE=1 npx -y pkg --acp";
        let (env, tokens) = split_env_prefixed_tokens(command);
        assert_eq!(
            env,
            vec![("AUGMENT_DISABLE_AUTO_UPDATE".to_string(), "1".to_string())]
        );
        assert_eq!(
            tokens,
            vec![
                "npx".to_string(),
                "-y".to_string(),
                "pkg".to_string(),
                "--acp".to_string()
            ]
        );
    }

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
            install_root.join("npx-cache").display()
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
        assert_eq!(config.command(), node_dir.join("bin").join("npx"));
        assert_eq!(config.arguments(), vec!["-y", ADAPTER_NPM_PACKAGE]);
        let path = config.environment().get("PATH").expect("PATH env present");
        assert!(
            path.starts_with(&node_dir.join("bin").to_string_lossy().into_owned()),
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
        assert_eq!(config.command(), node_dir.join("bin").join("node"));
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
        assert_eq!(config.command(), node_dir.join("bin").join("npx"));
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
        assert_eq!(config.command(), node_dir.join("bin").join("npx"));
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
        assert_eq!(node_binary(&dir), dir.join("bin").join("node"));
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
        // SAFETY: single-threaded test; restored right after reading.
        let saved = std::env::var_os("PATH");
        unsafe {
            std::env::set_var("PATH", "/usr/bin:/bin");
        }
        let result = prepend_to_path(Path::new("/managed/bin"));
        match saved {
            Some(v) => unsafe { std::env::set_var("PATH", v) },
            None => unsafe { std::env::remove_var("PATH") },
        }
        assert_eq!(result, "/managed/bin:/usr/bin:/bin");
    }

    #[test]
    fn extract_tar_gz_unpacks_a_real_archive() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir(&src).unwrap();
        std::fs::write(src.join("hello.txt"), b"hi").unwrap();

        // Build a gzip tarball with the system tar, then extract it back.
        let tarball = dir.path().join("out.tar.gz");
        let status = std::process::Command::new("tar")
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
        extract_tar_gz(&bytes, &dest).expect("extraction succeeds");
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
    fn write_fake_node_reporting(path: &Path, output: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, format!("#!/bin/sh\nprintf '%s' '{output}'\n")).unwrap();
        let mut perms = std::fs::metadata(path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(path, perms).unwrap();
    }

    #[test]
    fn system_node_arch_matches_host_true_when_reported_arch_matches() {
        let (_, expected) = node_platform().expect("supported test platform");
        let dir = tempfile::tempdir().unwrap();
        let fake_node = dir.path().join("node");
        write_fake_node_reporting(&fake_node, expected);
        assert!(system_node_arch_matches_host(&fake_node));
    }

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
                assert!(node_dir.join("bin").join("npx").exists(), "npx extracted");
                assert!(managed_cache_valid(&node_dir), "extracted node runs");
            }
            other => panic!("expected managed runtime, got {other:?}"),
        }
        assert!(seen.contains(&NodeProgress::Downloading));
        assert!(seen.contains(&NodeProgress::Verifying));
        assert!(seen.contains(&NodeProgress::Extracting));
    }
}
