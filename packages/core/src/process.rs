//! Spawning a child, and reaching the tree it forks.
//!
//! Four crates each spelled this out, in two spellings of one POSIX call —
//! so a second platform would have meant four Job Object implementations.
//! Deciding *whether* to kill a tree stays with the caller; the call that
//! does it lives here.

use std::ffi::OsStr;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;
#[cfg(unix)]
use unix as backend;
#[cfg(windows)]
use windows as backend;
#[cfg(not(any(unix, windows)))]
mod unsupported;
#[cfg(not(any(unix, windows)))]
use unsupported as backend;

/// The OS-provided archive extractor. Windows' bsdtar understands Node's ZIP
/// archives; a Git installation's GNU tar may shadow it on PATH.
pub fn archive_command() -> std::process::Command {
    #[cfg(windows)]
    {
        let root = std::env::var_os("SystemRoot").expect("Windows system directory");
        command(std::path::PathBuf::from(root).join("System32/tar.exe"))
    }
    #[cfg(not(windows))]
    {
        command("tar")
    }
}

/// Build a command through the one gate every spawn goes through.
///
/// Windows console subprocesses stay hidden when launched by the GUI.
/// The caller still supplies an executable, not a shell command line.
pub fn command(program: impl AsRef<OsStr>) -> std::process::Command {
    backend::command(program.as_ref())
}

/// `program` as the OS will find it for a child whose `PATH` is `path`
/// (`None`: this process's own).
///
/// Windows runs a bare name only as `<name>.exe`, so a `.cmd` shim — `npx`,
/// an npm-installed `claude`, an editor's CLI — is resolved here through
/// PATHEXT, the way a shell would. Anything already a path, anything not
/// found, and every other platform get `program` back as given.
pub fn resolve_program(program: &OsStr, path: Option<&OsStr>) -> std::path::PathBuf {
    backend::resolve_program(program, path)
}

/// Compare native environment names without imposing Windows rules on Unix.
pub fn env_name_eq(name: &OsStr, expected: &OsStr) -> bool {
    env_name_eq_for(name, expected, cfg!(windows))
}

fn env_name_eq_for(name: &OsStr, expected: &OsStr, windows: bool) -> bool {
    if windows {
        name.eq_ignore_ascii_case(expected)
    } else {
        name == expected
    }
}

/// The `PATH` a list of environment assignments gives a child, if it sets
/// one. The last assignment wins, as it does when they are applied in order;
/// Windows names the variable case-insensitively (`Path`).
pub fn child_path<'a, I, K, V>(assignments: I) -> Option<&'a OsStr>
where
    I: IntoIterator<Item = (&'a K, &'a V)>,
    I::IntoIter: DoubleEndedIterator,
    K: AsRef<OsStr> + ?Sized + 'a,
    V: AsRef<OsStr> + ?Sized + 'a,
{
    assignments
        .into_iter()
        .rev()
        .find(|(name, _)| env_name_eq(name.as_ref(), OsStr::new("PATH")))
        .map(|(_, value)| value.as_ref())
}

/// [`command`] for a program named the way a user types it — see
/// [`resolve_program`]. `path` is the `PATH` the child will be given, when
/// the caller sets one.
pub fn command_on_path(program: impl AsRef<OsStr>, path: Option<&OsStr>) -> std::process::Command {
    command(resolve_program(program.as_ref(), path))
}

/// Ask for a child that can be torn down as a tree. Call before `spawn`,
/// then [`Group::adopt`] on the pid it returns.
///
/// Unix makes it lead its own group, so its pid doubles as a group id.
/// Windows has none to ask for before the process exists, so it starts the
/// child *suspended* and [`Group::adopt`] is what lets it go.
pub fn lead_own_group(command: &mut std::process::Command) {
    backend::lead_own_group(command);
}

/// A spawned child's tear-down handle: everything it goes on to fork belongs
/// to this, and [`Group::kill_tree`] ends all of it.
///
/// Held rather than derived, because Windows cannot derive it: a job object
/// has to exist before the descendants do.
///
/// INVARIANT: keep it at least as long as the child may run. On Windows the
/// job ends when its last handle closes — which is what stops a crash from
/// orphaning the tree, and equally means an early drop kills a live child.
#[derive(Debug)]
pub struct Group(GroupInner);

use backend::GroupInner;

impl Group {
    /// Terminate the tree and reap its root within a bounded wait.
    /// `child` must be this group's unreaped root, adopted after `lead_own_group`.
    pub fn try_terminate_child(&self, child: &mut std::process::Child) -> std::io::Result<()> {
        let termination = self.try_kill_tree();
        // A failed group operation still attempts to stop its unreaped root.
        let _ = child.kill();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            if child.try_wait()?.is_some() {
                return termination;
            }
            if std::time::Instant::now() >= deadline {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Process did not exit after termination",
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    /// Adopt with verified OS setup before resuming.
    /// The caller must kill the child on failure.
    pub fn try_adopt(pid: u32) -> std::io::Result<Self> {
        backend::GroupInner::try_adopt(pid).map(Self)
    }

    /// Verify tree termination; Windows waits at most two seconds for job exit.
    pub fn try_kill_tree(&self) -> std::io::Result<()> {
        self.0.try_kill_tree()
    }
    /// Take the child at `pid` into a group, and on Windows let it run.
    ///
    /// Pairs with [`lead_own_group`], which started it suspended, so it joins
    /// the job before its first instruction. **A child never adopted stays
    /// suspended forever** — nothing fallible may sit in between. On unix
    /// `pid` must also be unreaped, or the group id may name someone else.
    pub fn adopt(pid: u32) -> Self {
        Self(backend::GroupInner::adopt(pid))
    }

    /// End the child and everything under it.
    ///
    /// Failure is not reported: the caller is already tearing the child down,
    /// and the reap that follows is what the caller actually waits on.
    pub fn kill_tree(&self) {
        self.0.kill_tree();
    }
}

/// Whether `pid` still names a live process.
///
/// Signal 0 delivers nothing; it only asks whether the pid is claimed. Same
/// caveat as [`Group::kill_tree`] in reverse — a reaped pid may have been handed to
/// someone else, so this answers "is something there", not "is *that* still
/// there".
pub fn is_alive(pid: u32) -> bool {
    pid != 0 && backend::is_alive(pid)
}

/// Whether a live child belongs to this shell. Windows has no foreground groups;
/// callers use this conservative check only when shell integration has no marker.
pub fn has_descendants(pid: u32) -> std::io::Result<bool> {
    backend::has_descendants(pid)
}

#[cfg(test)]
mod tests;
