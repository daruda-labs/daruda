//! Mutual exclusion for runs sharing one working directory. Two runs in
//! one tree would interleave file writes, so exactly one lock exists per
//! `cwd` and it is taken atomically.
//!
//! A live holder is never reclaimed, however old it is. Age cannot tell a
//! long run apart from a lock whose pid the OS has handed to something
//! else, and the two mistakes are not symmetric: reclaiming a running run
//! corrupts a working tree, while refusing a reused pid costs a run that
//! the user recovers by clearing a lock the error names. Runs may declare
//! no deadline at all, so there is no age above which a holder is
//! certainly gone.

use crate::error::{FlowIoError, IoSite};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

/// Whether a process is still running. The engine does not ask the OS —
/// the host already tracks processes and answers. Injecting it also makes
/// every case here testable.
pub type IsAlive<'a> = &'a dyn Fn(u32) -> bool;

/// The lock file's name inside the working directory.
const LOCK_FILE: &str = ".lock";

/// The name that serialises reclaims of a stale lock.
const TAKEOVER_FILE: &str = ".lock.takeover";

/// A reclaim is a delete and a create, so anything holding the takeover
/// name longer than this died mid-way. Deliberately short: the cost of
/// waiting it out is a refused run, and the cost of it being long is a
/// directory nobody can reclaim.
const TAKEOVER_STALE_AFTER: std::time::Duration = std::time::Duration::from_secs(60);

mod doing {
    pub const TAKE: &str = "taking the run lock";
    pub const TAKEOVER: &str = "serialising a stale-lock reclaim";
    pub const RECLAIM: &str = "reclaiming a stale run lock";
    pub const RELEASE: &str = "releasing the run lock";
}

/// Who holds the lock. Serialised as YAML — this crate already parses YAML
/// and a second format would be a second thing to keep working.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LockHolder {
    pub pid: u32,
    pub run_id: String,
    /// Wall-clock start. Not used to decide staleness — it is what the
    /// error shows a user deciding whether to clear a lock by hand.
    pub started_unix_secs: u64,
}

impl LockHolder {
    /// Same process, same run, same start. Two runs of one flow in one
    /// process differ by `run_id`; a reused pid differs by start time.
    fn is_same_run_as(&self, other: &Self) -> bool {
        self.pid == other.pid
            && self.run_id == other.run_id
            && self.started_unix_secs == other.started_unix_secs
    }
}

#[derive(Debug)]
pub enum LockError {
    Held(LockHolder),
    Io(FlowIoError),
}

/// Released on `release()`. Not on drop: releasing needs to report I/O
/// failure, and `Drop` cannot.
#[derive(Debug)]
pub struct RunLock {
    path: PathBuf,
    /// What this run wrote. Compared on release, because deleting by path
    /// alone would delete whatever is there — including a lock a different
    /// run has since taken.
    holder: LockHolder,
}

impl RunLock {
    /// Take the working directory for `run_id`, reclaiming a lock whose
    /// holder is gone. Every step races, so creation is atomic and the
    /// reclaim is serialised — see `take` and `reclaim`.
    pub fn acquire(dir: &Path, run_id: &str, is_alive: IsAlive<'_>) -> Result<Self, LockError> {
        let path = dir.join(LOCK_FILE);
        match take(&path, run_id) {
            Ok(holder) => return Ok(Self { path, holder }),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
            Err(source) => return Err(io(doing::TAKE, path, source)),
        }

        // A holder we cannot read or parse is not evidence of a live run.
        if let Some(holder) = read_holder(dir)
            && is_alive(holder.pid)
        {
            return Err(LockError::Held(holder));
        }

        reclaim(dir, &path, run_id, is_alive)
    }

    /// Give the directory back, but only if it is still this run's to give.
    ///
    /// A run whose lock was reclaimed while it was still going would
    /// otherwise delete the lock of whoever took over — turning one
    /// mistaken reclaim into an unlocked directory. Finding someone else's
    /// lock is not an error: this run's is already gone.
    ///
    /// Behind the takeover guard, because checking and deleting are two
    /// steps: a reclaimer that installs a new holder between them would
    /// have its lock deleted by the very check meant to protect it. The
    /// guard is the same one `reclaim` takes, so the two serialise against
    /// each other rather than each against itself.
    pub fn release(self) -> Result<(), FlowIoError> {
        let takeover = self.path.with_file_name(TAKEOVER_FILE);
        // A guard we cannot take means a reclaim is in flight. It is about
        // to replace this lock anyway, so leaving it is both correct and
        // the safe direction.
        let Ok(guard) = TakeoverGuard::acquire(&takeover, &self.holder.run_id) else {
            return Ok(());
        };
        let mine = read_at(&self.path).is_some_and(|current| current.is_same_run_as(&self.holder));
        let outcome = if mine {
            match std::fs::remove_file(&self.path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
                Err(e) => Err(io_error(doing::RELEASE, self.path.clone(), e)),
            }
        } else {
            Ok(())
        };
        drop(guard);
        outcome
    }
}

/// Create the lock file, holder and all, or fail with `AlreadyExists`.
///
/// The name appears only once the contents are complete: it is written to
/// a private file first and `hard_link`ed into place, which fails atomically
/// if the name is taken. Creating `.lock` directly would publish an empty
/// file that a racer reads as unparseable — and therefore stale — in the
/// window before the write lands, so both would end up holding it.
fn take(path: &Path, run_id: &str) -> std::io::Result<LockHolder> {
    let holder = LockHolder {
        pid: std::process::id(),
        run_id: run_id.to_string(),
        started_unix_secs: now_unix_secs(),
    };
    let text = yaml_serde::to_string(&holder)
        .map_err(|e| std::io::Error::new(ErrorKind::InvalidData, e))?;

    let staging = staging_path(path);
    // Truncating rather than creating exclusively: the name carries this
    // process's pid, so only a dead run's leftover can be in the way.
    let mut file = std::fs::File::create(&staging)?;
    let published = file
        .write_all(text.as_bytes())
        .and_then(|()| std::fs::hard_link(&staging, path));
    let _ = std::fs::remove_file(&staging);
    published.map(|()| holder)
}

/// A staging name no other live process can collide on: the pid separates
/// processes and the counter separates this process's own threads.
fn staging_path(path: &Path) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    path.with_file_name(format!("{LOCK_FILE}.staging.{}.{n}", std::process::id()))
}

/// Delete a stale lock and take it.
///
/// Deleting and creating are two steps, so two reclaimers that both delete
/// before either creates would both end up holding — the exact corruption
/// the lock exists to prevent. One more atomic name serialises them:
/// whoever links the takeover file does the reclaim and the rest back off.
fn reclaim(
    dir: &Path,
    path: &Path,
    run_id: &str,
    is_alive: IsAlive<'_>,
) -> Result<RunLock, LockError> {
    let takeover = path.with_file_name(TAKEOVER_FILE);
    let guard = TakeoverGuard::acquire(&takeover, run_id)?;

    // Serialised now, so ask again with the same test the caller used: a
    // reclaimer that got here first has already installed a live holder,
    // and deleting that would undo their run.
    if let Some(holder) = read_holder(dir)
        && is_alive(holder.pid)
    {
        drop(guard);
        return Err(LockError::Held(holder));
    }

    if let Err(source) = std::fs::remove_file(path)
        && source.kind() != ErrorKind::NotFound
    {
        drop(guard);
        return Err(io(doing::RECLAIM, path.to_path_buf(), source));
    }
    let taken = take(path, run_id);
    drop(guard);
    match taken {
        Ok(holder) => Ok(RunLock {
            path: path.to_path_buf(),
            holder,
        }),
        Err(source) => Err(io(doing::RECLAIM, path.to_path_buf(), source)),
    }
}

/// Held for the few microseconds a reclaim takes, then removed. Dropping
/// rather than an explicit release: a failure to clean it up is recovered
/// by the next reclaimer's age check, and there is nothing a caller could
/// usefully do about it.
struct TakeoverGuard(PathBuf);

impl TakeoverGuard {
    fn acquire(path: &Path, run_id: &str) -> Result<Self, LockError> {
        match take(path, run_id) {
            Ok(_) => return Ok(Self(path.to_path_buf())),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
            Err(source) => return Err(io(doing::TAKEOVER, path.to_path_buf(), source)),
        }
        // A takeover is microseconds of work, so anything older is a
        // reclaimer that died holding it. Age alone, never liveness — a pid
        // check here would recurse into the question this guard serialises.
        let stale = read_at(path)
            .map(|h| age_secs(&h) >= TAKEOVER_STALE_AFTER.as_secs())
            .unwrap_or(true);
        if !stale {
            return Err(LockError::Held(holder_or_unknown(
                path.parent().unwrap_or(Path::new("")),
            )));
        }
        let _ = std::fs::remove_file(path);
        match take(path, run_id) {
            Ok(_) => Ok(Self(path.to_path_buf())),
            // Lost the retry — someone else is reclaiming, so this run does
            // not get the directory. Retrying in a loop would let two
            // reclaimers delete each other's guard forever.
            Err(e) if e.kind() == ErrorKind::AlreadyExists => Err(LockError::Held(
                holder_or_unknown(path.parent().unwrap_or(Path::new(""))),
            )),
            Err(source) => Err(io(doing::TAKEOVER, path.to_path_buf(), source)),
        }
    }
}

impl Drop for TakeoverGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// The holder named by the lock file, or `None` when there is no file or
/// its contents are not a `LockHolder`. Public because deriving a run's
/// status reads the same file, and a second copy of this format would be a
/// second thing to keep working.
pub fn read_holder(dir: &Path) -> Option<LockHolder> {
    read_at(&dir.join(LOCK_FILE))
}

/// The same read against a full path, for the takeover file.
fn read_at(path: &Path) -> Option<LockHolder> {
    let text = std::fs::read_to_string(path).ok()?;
    yaml_serde::from_str(&text).ok()
}

/// The winner of a reclaim race may not have written its file yet, so an
/// unreadable holder still means the directory is taken. `pid: 0` marks
/// the holder as unidentified rather than naming a process we never read.
fn holder_or_unknown(dir: &Path) -> LockHolder {
    read_holder(dir).unwrap_or(LockHolder {
        pid: 0,
        run_id: String::new(),
        started_unix_secs: 0,
    })
}

fn age_secs(holder: &LockHolder) -> u64 {
    now_unix_secs().saturating_sub(holder.started_unix_secs)
}

/// The lock's age is the one thing here measured on the wall clock:
/// `Instant` cannot be serialised, and a stale lock outlives the process
/// that would have held the monotonic reference.
fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn io(doing: &'static str, path: PathBuf, source: std::io::Error) -> LockError {
    LockError::Io(io_error(doing, path, source))
}

fn io_error(doing: &'static str, path: PathBuf, source: std::io::Error) -> FlowIoError {
    FlowIoError {
        site: IoSite::Run,
        doing,
        path,
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dead(_pid: u32) -> bool {
        false
    }
    fn alive(_pid: u32) -> bool {
        true
    }
    /// The truthful predicate for a test: this process is running, nothing
    /// else in the fixture is. `alive`/`dead` are blunt enough to hide a
    /// race, because they answer the same for a lock we just wrote.
    fn only_us(pid: u32) -> bool {
        pid == std::process::id()
    }

    /// Plant a lock file with contents no acquisition would produce —
    /// only a test needs to name a holder it is not.
    fn write_holder(dir: &Path, holder: &LockHolder) {
        std::fs::write(
            dir.join(LOCK_FILE),
            yaml_serde::to_string(holder).expect("serialise"),
        )
        .expect("write");
    }

    #[test]
    fn a_lock_is_taken_and_released() {
        let dir = tempfile::tempdir().expect("tempdir");
        let lock = RunLock::acquire(dir.path(), "01J", &dead).expect("free");
        assert!(dir.path().join(".lock").is_file());
        lock.release().expect("release");
        assert!(!dir.path().join(".lock").exists());
    }

    /// The whole point: the second caller must not get in.
    #[test]
    fn a_second_acquire_against_a_live_holder_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _held = RunLock::acquire(dir.path(), "01J", &alive).expect("free");
        match RunLock::acquire(dir.path(), "01K", &alive) {
            Err(LockError::Held(holder)) => {
                assert_eq!(holder.run_id, "01J");
                assert_eq!(holder.pid, std::process::id());
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    /// A crashed run leaves its lock behind forever otherwise. Taking it
    /// over is a delete-then-create, so two reclaimers race on the create
    /// and exactly one wins.
    #[test]
    fn a_lock_left_by_a_dead_process_is_taken_over() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _stale = RunLock::acquire(dir.path(), "01J", &alive).expect("free");
        std::mem::forget(_stale); // leave the file behind, as a crash would

        let taken = RunLock::acquire(dir.path(), "01K", &dead).expect("stale is reclaimable");
        assert_eq!(read_holder(dir.path()).expect("readable").run_id, "01K");
        taken.release().expect("release");
    }

    /// Age never overrides liveness. A run may declare no deadline, so an
    /// ancient live holder can be a legitimate long run — and reclaiming
    /// one puts two agents in a working tree, which is worse than refusing
    /// a run whose lock the user can clear from the error.
    #[test]
    fn an_ancient_lock_with_a_live_pid_is_refused_not_reclaimed() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_holder(
            dir.path(),
            &LockHolder {
                pid: std::process::id(),
                run_id: "01J".to_string(),
                started_unix_secs: 0, // 1970
            },
        );
        match RunLock::acquire(dir.path(), "01K", &alive) {
            Err(LockError::Held(holder)) => assert_eq!(holder.run_id, "01J"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    /// The damage a mistaken reclaim would otherwise multiply: the run that
    /// lost its lock must not delete the lock of whoever holds it now.
    #[test]
    fn releasing_after_someone_else_took_the_lock_leaves_theirs_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ours = RunLock::acquire(dir.path(), "01J", &dead).expect("free");
        write_holder(
            dir.path(),
            &LockHolder {
                pid: std::process::id(),
                run_id: "01K".to_string(),
                started_unix_secs: now_unix_secs(),
            },
        );

        ours.release()
            .expect("releasing what is no longer ours is not an error");

        assert_eq!(
            read_holder(dir.path()).expect("still locked").run_id,
            "01K",
            "the new holder's lock must survive"
        );
    }

    /// Every sequential test here passes against a check-then-create too,
    /// because nothing else is running between the check and the create.
    /// Only real racers show whether acquisition is atomic.
    #[test]
    fn only_one_of_many_simultaneous_acquires_wins() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        const RACERS: usize = 8;

        for round in 0..200 {
            let dir = tempfile::tempdir().expect("tempdir");
            let start = std::sync::Barrier::new(RACERS);
            let winners = AtomicUsize::new(0);
            let (d, w, b) = (&dir, &winners, &start);
            std::thread::scope(|s| {
                for i in 0..RACERS {
                    s.spawn(move || {
                        b.wait();
                        if RunLock::acquire(d.path(), &i.to_string(), &alive).is_ok() {
                            w.fetch_add(1, Ordering::SeqCst);
                        }
                    });
                }
            });
            assert_eq!(
                winners.load(Ordering::SeqCst),
                1,
                "round {round}: more than one run holds the directory"
            );
        }
    }

    /// A lock file that is not parseable is not evidence of a live run.
    /// Refusing forever on a truncated write would wedge the directory.
    #[test]
    fn an_unparseable_lock_is_taken_over() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path()).expect("mkdir");
        std::fs::write(dir.path().join(".lock"), "{ truncated").expect("write");
        RunLock::acquire(dir.path(), "01K", &alive)
            .expect("garbage is reclaimable")
            .release()
            .expect("release");
    }

    /// The reclaim path races too, and it is the one the takeover guard
    /// exists for: without it, reclaimers that all delete before any of
    /// them creates all end up holding. Measured at 8 racers, that leaked
    /// a second holder in roughly 4% of rounds.
    #[test]
    fn only_one_of_many_simultaneous_reclaims_wins() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        const RACERS: usize = 8;

        for round in 0..200 {
            let dir = tempfile::tempdir().expect("tempdir");
            write_holder(
                dir.path(),
                &LockHolder {
                    // A pid nothing in this fixture answers for, which is
                    // what makes the planted lock reclaimable.
                    pid: 999_999,
                    run_id: "crashed".to_string(),
                    started_unix_secs: 0,
                },
            );
            let start = std::sync::Barrier::new(RACERS);
            let winners = AtomicUsize::new(0);
            let (d, w, b) = (&dir, &winners, &start);
            std::thread::scope(|s| {
                for i in 0..RACERS {
                    s.spawn(move || {
                        b.wait();
                        if RunLock::acquire(d.path(), &i.to_string(), &only_us).is_ok() {
                            w.fetch_add(1, Ordering::SeqCst);
                        }
                    });
                }
            });
            assert_eq!(
                winners.load(Ordering::SeqCst),
                1,
                "round {round}: more than one run reclaimed the directory"
            );
        }
    }
}
