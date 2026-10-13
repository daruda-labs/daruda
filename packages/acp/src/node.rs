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
use crate::preparation::PreparationContext;
use crate::preparation::process::output;

/// Pinned Node.js version for the managed install. A single pinned version keeps
/// the download URL and integrity check deterministic; bump it periodically to a
/// current LTS. Matches the version zed pins so the URL is known-good.
const MANAGED_NODE_VERSION: &str = "v24.11.0";

/// Minimum acceptable system Node.js version. Below this, a system install is
/// treated as absent and the managed runtime is used instead — old Node.js
/// tends to fail the adapter with cryptic errors, which is worse than a
/// one-time managed download.
const MIN_NODE_VERSION: Version = Version::new(20, 0, 0);

#[cfg(test)]
mod cache_tests;
mod command;
mod layout;
pub(crate) mod resolved;

use command::split_env_prefixed_tokens;
pub use command::{command_diagnostic, command_needs_node, first_command_token};

/// Base URL for the official Node.js distribution.
const NODE_DIST_BASE: &str = "https://nodejs.org/dist";

/// Overall timeout for the (large) tarball / checksum downloads.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);
const DOWNLOAD_IO_TIMEOUT: Duration = Duration::from_secs(5);
const DOWNLOAD_CHUNK: usize = 64 * 1024;
const INSTALL_POLL: Duration = Duration::from_millis(25);

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
                let bin_dir = bin_dir(node_dir);
                let (env_assignments, tokens) = split_env_prefixed_tokens(command);
                let has_cache_override = env_assignments
                    .iter()
                    .any(|(name, _)| name == "npm_config_cache");
                let launcher = tokens.first().cloned().unwrap_or_default();
                let abs_launcher = managed_launcher(node_dir, &launcher);
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
    /// The host closed the pane or stopped the flow during preparation.
    #[error("runtime preparation canceled")]
    Canceled,
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
    ensure_node_with_context(install_root, progress, &PreparationContext::default())
}

pub(crate) fn ensure_node_with_context(
    install_root: &Path,
    progress: &mut dyn FnMut(NodeProgress),
    context: &PreparationContext<'_>,
) -> Result<NodeRuntime, NodeError> {
    checkpoint(context)?;
    if detect_system_node_with_context(context) {
        progress(NodeProgress::UsingSystemNode);
        return Ok(NodeRuntime::System);
    }

    let (os, arch) = node_platform()?;
    let node_dir = managed_node_dir(install_root, os, arch);

    progress(NodeProgress::CheckingCache);
    checkpoint(context)?;
    if managed_cache_valid_with_context(&node_dir, context)? {
        return Ok(NodeRuntime::Managed { node_dir });
    }

    install_managed_with_context(install_root, os, arch, progress, context)
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
#[cfg(test)]
fn detect_system_node() -> bool {
    detect_system_node_with_context(&PreparationContext::default())
}

fn detect_system_node_with_context(context: &PreparationContext<'_>) -> bool {
    let Ok(node) = which::which("node") else {
        return false;
    };
    // `npx` must exist too — the adapter is launched through it.
    if which::which("npx").is_err() {
        return false;
    }
    let Ok(version) = output(
        daruda_core::process::command(&node).arg("--version"),
        context,
    ) else {
        return false;
    };
    let version_ok = match parse_node_version(&version) {
        Some(version) => version >= MIN_NODE_VERSION,
        None => false,
    };
    version_ok && system_node_arch_matches_host_with_context(&node, context)
}

/// `true` if `node`'s own `process.arch` matches the host's actual CPU
/// architecture (per [`node_platform`]'s `(os, arch)` mapping). `false` on
/// any probe failure — an architecture we can't confirm is treated as a
/// mismatch, favoring the always-correct managed download over a guess.
#[cfg(test)]
fn system_node_arch_matches_host(node: &Path) -> bool {
    system_node_arch_matches_host_with_context(node, &PreparationContext::default())
}

fn system_node_arch_matches_host_with_context(
    node: &Path,
    context: &PreparationContext<'_>,
) -> bool {
    let Ok((_, expected_arch)) = node_platform() else {
        return false;
    };
    let Ok(arch) = output(
        daruda_core::process::command(node).args(["-e", "process.stdout.write(process.arch)"]),
        context,
    ) else {
        return false;
    };
    arch.trim() == expected_arch
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
        "windows" => "win32",
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
    let os = if os == "win32" { "win" } else { os };
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

/// The directory holding a managed runtime's executables.
///
/// The one place this layout is written down — a nodejs.org distribution
/// does not use `bin/` everywhere, so the arm for that belongs here and
/// nowhere else.
pub fn bin_dir(node_dir: &Path) -> PathBuf {
    layout::NodeLayout::current().bin_dir(node_dir)
}

/// `PATH` with a managed runtime's executables in front of it.
///
/// Callers hand this straight to a child's environment. Going through here
/// rather than formatting the string keeps two things right that are easy to
/// get wrong: the separator is the platform's, and the layout above stays in
/// one place.
pub fn path_with_node(node_dir: &Path) -> String {
    prepend_to_path(&bin_dir(node_dir))
}

/// The `node` binary inside an extracted managed node directory.
fn node_binary(node_dir: &Path) -> PathBuf {
    managed_launcher(node_dir, "node")
}

pub(crate) fn managed_launcher(node_dir: &Path, launcher: &str) -> PathBuf {
    layout::NodeLayout::current().launcher(node_dir, launcher)
}

/// `true` if a managed install exists and its `node` runs. Cheap validity gate
/// that also self-heals a truncated / corrupt extraction (it re-downloads).
#[cfg(test)]
fn managed_cache_valid(node_dir: &Path) -> bool {
    managed_cache_valid_with_context(node_dir, &PreparationContext::default())
        .expect("uncancelled cache probe")
}

fn managed_cache_valid_with_context(
    node_dir: &Path,
    context: &PreparationContext<'_>,
) -> Result<bool, NodeError> {
    let node = node_binary(node_dir);
    let result = output(
        daruda_core::process::command(&node).arg("--version"),
        context,
    );
    checkpoint(context)?;
    match result {
        Err(error) if error.kind == crate::preparation::PreparationKind::Canceled => {
            Err(NodeError::Canceled)
        }
        result => Ok(result.is_ok()),
    }
}

/// Download, verify, and extract the pinned Node.js into `install_root`,
/// returning the [`NodeRuntime::Managed`]. Serialized by [`INSTALL_LOCK`]; the
/// cache is re-checked after acquiring so a queued caller reuses a just-finished
/// install instead of downloading again.
#[cfg(test)]
fn install_managed(
    install_root: &Path,
    os: &str,
    arch: &str,
    progress: &mut dyn FnMut(NodeProgress),
) -> Result<NodeRuntime, NodeError> {
    install_managed_with_context(
        install_root,
        os,
        arch,
        progress,
        &PreparationContext::default(),
    )
}

fn checkpoint(context: &PreparationContext<'_>) -> Result<(), NodeError> {
    context.check().map_err(|_| NodeError::Canceled)
}

fn install_managed_with_context(
    install_root: &Path,
    os: &str,
    arch: &str,
    progress: &mut dyn FnMut(NodeProgress),
    context: &PreparationContext<'_>,
) -> Result<NodeRuntime, NodeError> {
    let started = std::time::Instant::now();
    let _guard = loop {
        checkpoint(context)?;
        match INSTALL_LOCK.try_lock() {
            Ok(guard) => break guard,
            Err(std::sync::TryLockError::Poisoned(error)) => break error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {}
        }
        if started.elapsed() >= DOWNLOAD_TIMEOUT {
            return Err(NodeError::Download(
                "waiting for the runtime install lock timed out".into(),
            ));
        }
        std::thread::sleep(INSTALL_POLL);
    };

    let node_dir = managed_node_dir(install_root, os, arch);
    progress(NodeProgress::CheckingCache);
    if managed_cache_valid_with_context(&node_dir, context)? {
        return Ok(NodeRuntime::Managed { node_dir });
    }

    let extension = if os == "win32" { "zip" } else { "tar.gz" };
    let file_name = format!("{}.{extension}", managed_folder_name(os, arch));
    let archive_url = format!("{NODE_DIST_BASE}/{MANAGED_NODE_VERSION}/{file_name}");
    let shasums_url = format!("{NODE_DIST_BASE}/{MANAGED_NODE_VERSION}/SHASUMS256.txt");

    progress(NodeProgress::Downloading);
    let archive = http_get_bytes(&archive_url, context)?;

    progress(NodeProgress::Verifying);
    let shasums = String::from_utf8(http_get_bytes(&shasums_url, context)?)
        .map_err(|error| NodeError::Download(error.to_string()))?;
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
    let result = publish_managed(&archive, os, arch, &staging, &node_dir, context);
    let _ = std::fs::remove_dir_all(&staging);
    result?;

    checkpoint(context)?;
    if !managed_cache_valid_with_context(&node_dir, context)? {
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
    context: &PreparationContext<'_>,
) -> Result<(), NodeError> {
    extract_archive_with_context(archive, staging, context)?;
    publish_extracted(
        &staging.join(managed_folder_name(os, arch)),
        node_dir,
        context,
    )
}

fn publish_extracted(
    extracted: &Path,
    node_dir: &Path,
    context: &PreparationContext<'_>,
) -> Result<(), NodeError> {
    checkpoint(context)?;
    if node_dir.exists() && !managed_cache_valid_with_context(node_dir, context)? {
        checkpoint(context)?;
        let _ = std::fs::remove_dir_all(node_dir);
    }
    checkpoint(context)?;
    match std::fs::rename(extracted, node_dir) {
        Ok(()) => Ok(()),
        // Another process published a valid install between our checks and the
        // rename — keep theirs rather than fail.
        Err(_) if managed_cache_valid_with_context(node_dir, context)? => Ok(()),
        Err(e) => Err(NodeError::Extract(format!("publishing node: {e}"))),
    }
}

/// GET `url` and return the body bytes (no size cap — the tarball is tens of MB).
fn http_get_bytes(url: &str, context: &PreparationContext<'_>) -> Result<Vec<u8>, NodeError> {
    checkpoint(context)?;
    let agent = ureq::AgentBuilder::new()
        .timeout(DOWNLOAD_TIMEOUT)
        .timeout_connect(DOWNLOAD_IO_TIMEOUT)
        .timeout_read(DOWNLOAD_IO_TIMEOUT)
        .build();
    let response = agent
        .get(url)
        .call()
        .map_err(|e| NodeError::Download(e.to_string()))?;
    let mut buf = Vec::new();
    let mut reader = response.into_reader();
    let mut chunk = [0; DOWNLOAD_CHUNK];
    loop {
        checkpoint(context)?;
        let n = reader
            .read(&mut chunk)
            .map_err(|e| NodeError::Download(e.to_string()))?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    Ok(buf)
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
    prepend_to(bin_dir, std::env::var_os("PATH"))
}

/// The pure half, so a test states the `PATH` it means instead of setting
/// one: `set_var` is unsound once the process has other threads, and a test
/// binary always does.
fn prepend_to(bin_dir: &Path, existing: Option<std::ffi::OsString>) -> String {
    match existing {
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

/// Extract Node's platform archive (ZIP on Windows, gzip tarball on Unix).
/// The system extractor detects the format from the staged bytes.
#[cfg(test)]
fn extract_archive(bytes: &[u8], dest: &Path) -> Result<(), NodeError> {
    extract_archive_with_context(bytes, dest, &PreparationContext::default())
}

fn extract_archive_with_context(
    bytes: &[u8],
    dest: &Path,
    context: &PreparationContext<'_>,
) -> Result<(), NodeError> {
    let tmp = dest.join(format!(".node-download-{}.archive", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(|e| NodeError::Extract(format!("staging archive: {e}")))?;
    let result = output(
        daruda_core::process::archive_command()
            .arg("-xf")
            .arg(&tmp)
            .arg("-C")
            .arg(dest),
        context,
    );
    let _ = std::fs::remove_file(&tmp);
    checkpoint(context)?;
    match result {
        Ok(_) => Ok(()),
        Err(e) => Err(NodeError::Extract(format!("running tar: {e}"))),
    }
}

#[cfg(test)]
mod tests;
