//! The three files a running control surface owns, and who may read them.
//!
//! Ownership is decided by an advisory file lock, never by probing the socket:
//! the OS releases a lock when the process dies, so a crash is handled by the
//! same code path as a clean exit.

use std::path::{Path, PathBuf};

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;

use super::SocketError;

/// Mode for the socket and its directory: owner only.
///
/// A unix socket's file mode *is* its access control — Darwin enforces it on
/// `connect` — and the default umask leaves it world-connectable, which would
/// put all nine app-driving tools behind nothing but the token.
pub(super) const OWNER_ONLY_FILE: u32 = 0o600;

/// macOS `sun_path` is 104 bytes (`sys/un.h`); Linux allows 108. Truncation
/// would bind a different path than the one written to the runtime file, so it
/// fails loudly instead.
#[cfg(target_os = "macos")]
const SUN_PATH_MAX: usize = 104;
#[cfg(not(target_os = "macos"))]
const SUN_PATH_MAX: usize = 108;

/// File names inside the profile's data directory.
const LOCK_FILE: &str = "control.lock";
pub(super) const SOCKET_FILE: &str = "control.sock";
const RUNTIME_FILE: &str = "control.json";

/// What the shim reads to find the socket. Deliberately carries **no token**:
/// the token travels to the agent through its session environment, so a file
/// any local process can read never holds the secret.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Runtime {
    pub socket: PathBuf,
    /// Identifies this run of the app. The shim compares it against what the
    /// handshake answers, so a socket rebound by a *restarted* daruda is
    /// detected instead of silently adopted.
    pub runtime_id: String,
}

pub(crate) fn lock_path(dir: &Path) -> PathBuf {
    dir.join(LOCK_FILE)
}

pub(crate) fn socket_path(dir: &Path) -> PathBuf {
    dir.join(SOCKET_FILE)
}

pub(crate) fn runtime_path(dir: &Path) -> PathBuf {
    dir.join(RUNTIME_FILE)
}

/// Exclusive claim on this profile's control socket.
pub(crate) struct Ownership {
    /// Held for the lifetime of the claim; the OS drops the lock with it.
    _file: std::fs::File,
    socket: PathBuf,
    runtime: PathBuf,
}

impl Ownership {
    /// Take the lock and clear anything a previous run left behind.
    pub(crate) fn acquire(dir: &Path) -> Result<Self, SocketError> {
        use fs4::fs_std::FileExt;
        // Owner-only because the socket inside is reachable by anyone who can
        // traverse the path, and a unix socket's mode *is* its access control.
        daruda_core::path::create_owner_only_dir(dir).map_err(SocketError::Io)?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(lock_path(dir))
            .map_err(SocketError::Io)?;
        // `try_lock_exclusive` answers with a *bool*, not by erroring: `false`
        // means someone else holds it. Treating the `Ok(false)` as success
        // would let a second instance unlink the live socket below.
        match FileExt::try_lock_exclusive(&file) {
            Ok(true) => {}
            Ok(false) => return Err(SocketError::AlreadyRunning),
            Err(e) => return Err(SocketError::Io(e)),
        }

        let socket = socket_path(dir);
        validate_socket_path(&socket)?;
        // Holding the lock proves no live process owns this inode, so a
        // leftover file is a dead one from a crash.
        if socket.exists() {
            std::fs::remove_file(&socket).map_err(SocketError::Io)?;
        }
        Ok(Self {
            _file: file,
            socket,
            runtime: runtime_path(dir),
        })
    }

    pub(crate) fn socket(&self) -> &Path {
        &self.socket
    }

    /// Publish where the socket is, atomically.
    ///
    /// Written through a temporary file and renamed so a shim that reads while
    /// this runs sees either the old contents or the new ones, never half a
    /// file.
    pub(crate) fn publish(&self, runtime_id: &str) -> Result<(), SocketError> {
        let runtime = Runtime {
            socket: self.socket.clone(),
            runtime_id: runtime_id.to_owned(),
        };
        let dir = self.runtime.parent().unwrap_or(&self.runtime);
        let mut temp = tempfile::NamedTempFile::new_in(dir).map_err(SocketError::Io)?;
        serde_json::to_writer(&mut temp, &runtime)
            .map_err(|e| SocketError::Io(std::io::Error::other(e)))?;
        temp.as_file().sync_all().map_err(SocketError::Io)?;
        temp.persist(&self.runtime)
            .map_err(|e| SocketError::Io(e.error))?;
        Ok(())
    }
}

impl Drop for Ownership {
    fn drop(&mut self) {
        // Best effort. The lock release is what actually matters, and the OS
        // does that; a stale socket or runtime file is cleaned up by the next
        // `acquire`, which is the path a crash takes anyway.
        for path in [&self.socket, &self.runtime] {
            if let Err(e) = std::fs::remove_file(path)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                LogWriter::log(
                    ErrorReport::new("Control socket file could not be removed")
                        .severity(ErrorSeverity::Info)
                        .from_error(&e)
                        .at(file!(), line!())
                        .dedup("control.socket.cleanup")
                        .build(),
                );
            }
        }
    }
}

/// Restrict a freshly bound socket to its owner.
///
/// `bind` applies the process umask, which is typically 022 — leaving the
/// socket world-connectable. Since the mode is the only access control a unix
/// socket has, this is what stands between another local user and every tool.
pub(super) fn restrict_socket(path: &Path) -> Result<(), SocketError> {
    use std::os::unix::fs::PermissionsExt as _;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(OWNER_ONLY_FILE))
        .map_err(SocketError::Io)
}

pub(crate) fn validate_socket_path(path: &Path) -> Result<(), SocketError> {
    let len = path.as_os_str().len();
    if len >= SUN_PATH_MAX {
        return Err(SocketError::PathTooLong {
            len,
            max: SUN_PATH_MAX,
        });
    }
    Ok(())
}
