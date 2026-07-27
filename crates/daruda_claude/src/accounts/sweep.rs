//! Startup orphan sweep: remove per-account config dirs that never got
//! promoted to a tracked `ManagedAccount` (login cancelled or crashed
//! mid-login). Pure/GPUI-free (filesystem + Keychain best-effort).

use std::path::Path;
use std::time::{Duration, SystemTime};

use daruda_store::accounts::AccountId;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;

use super::credentials::delete_scoped_credentials;
use super::layout::accounts_root;

/// Startup sweep: remove per-account config dirs under [`accounts_root`]
/// that never got promoted to a `ManagedAccount` (login cancelled or
/// the app crashed mid-login). A subdirectory is an orphan when its name
/// doesn't match any id in `known` — this also catches garbage/leftover
/// dirs whose name was never a valid UUID to begin with, since those
/// can't match either.
///
/// `grace` spares an orphan candidate whose directory is younger than
/// this window: `add_managed_account`/`reauthenticate_account` create the
/// account's config dir immediately but only persist it to `accounts.json`
/// (and so into `known`) on success, so a login still in flight — in
/// another window, blocking on human OAuth for up to its own timeout —
/// looks identical to an orphan to every *other* window's constructor
/// sweep. A dir left behind by a genuine orphan (the app crashed mid-login,
/// or the login timed out and `finish_login_failed` never got to run) is
/// always older than any login could still be running, since a live login
/// is bounded by that same timeout — so passing the login timeout itself
/// as `grace` is exactly the cutoff that spares every in-flight login
/// while still sweeping every real orphan. Callers should pass the same
/// timeout constant the login flow itself uses, so the two can't drift.
///
/// A dir whose modified time can't be read (metadata failure, or a
/// modified time that is somehow in the future relative to now) is
/// SPARED rather than swept — an unknown age is treated as "possibly
/// in-flight" rather than "definitely old", since sweeping a live
/// login's dir is silent data loss but leaving a genuine orphan behind
/// one extra restart is not.
///
/// Best-effort: any individual dir's Keychain or filesystem failure is
/// skipped, never panics, and never aborts the sweep of the remaining
/// dirs. Only touches entries directly under `accounts_root(data_dir)` —
/// never anything outside it.
pub fn sweep_orphan_dirs(data_dir: &Path, known: &[AccountId], grace: Duration) {
    let root = accounts_root(data_dir);
    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        // No accounts root yet (no logins have ever run) — nothing to sweep.
        Err(_) => return,
    };
    // `account_config_dir` names each dir after `id.0.to_string()`, so a
    // plain string comparison against that same format is exact — no
    // UUID parsing needed.
    let known_names: Vec<String> = known.iter().map(|id| id.0.to_string()).collect();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let is_known = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| known_names.iter().any(|known_name| known_name == name));
        if is_known {
            continue;
        }
        if is_within_grace(&path, grace) {
            // Possibly an in-flight login in another window — spare it.
            continue;
        }
        delete_scoped_credentials(&path);
        if let Err(e) = std::fs::remove_dir_all(&path) {
            LogWriter::log(
                ErrorReport::new("Failed to remove orphaned account config dir")
                    .from_error(&e)
                    .severity(ErrorSeverity::Warning)
                    .at(file!(), line!())
                    .dedup("account.sweep.remove_dir_failed")
                    .build(),
            );
        }
    }
}

/// Whether `path`'s modified time is younger than `grace` — or unknown,
/// which is treated the same as "younger" (see [`sweep_orphan_dirs`]'s
/// doc for why an unreadable age spares rather than sweeps).
fn is_within_grace(path: &Path, grace: Duration) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return true;
    };
    let Ok(modified) = metadata.modified() else {
        return true;
    };
    match SystemTime::now().duration_since(modified) {
        Ok(elapsed) => elapsed < grace,
        // `modified` is after "now" (clock skew, or a write landing between
        // the two syscalls) — can't be a stale age, treat as young.
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::super::layout::account_config_dir;
    use super::*;

    #[test]
    fn sweep_orphan_dirs_removes_unknown_and_garbage_keeps_known() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let data_dir = tmp.path();
        let root = accounts_root(data_dir);
        std::fs::create_dir_all(&root).expect("create accounts root");

        let known_id = daruda_store::accounts::AccountId::new();
        let known_dir = account_config_dir(data_dir, known_id);
        std::fs::create_dir_all(&known_dir).expect("create known dir");

        let unknown_id = daruda_store::accounts::AccountId::new();
        let unknown_dir = account_config_dir(data_dir, unknown_id);
        std::fs::create_dir_all(&unknown_dir).expect("create unknown dir");

        let garbage_dir = root.join("not-a-uuid");
        std::fs::create_dir_all(&garbage_dir).expect("create garbage dir");

        // Zero grace: every orphan candidate is immediately "old" enough
        // to sweep, regardless of how fresh its mtime is — proves the
        // baseline sweep behavior is unchanged by the grace param.
        sweep_orphan_dirs(data_dir, &[known_id], Duration::ZERO);

        assert!(known_dir.exists(), "known account dir must be preserved");
        assert!(!unknown_dir.exists(), "unknown-uuid dir must be removed");
        assert!(!garbage_dir.exists(), "garbage-named dir must be removed");
    }

    #[test]
    fn sweep_orphan_dirs_no_op_when_root_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        // accounts_root under tmp.path() was never created.
        sweep_orphan_dirs(tmp.path(), &[], Duration::from_secs(5 * 60));
    }

    #[test]
    fn sweep_orphan_dirs_spares_young_orphan_sweeps_old_orphan() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let data_dir = tmp.path();
        let root = accounts_root(data_dir);
        std::fs::create_dir_all(&root).expect("create accounts root");

        // A freshly-created orphan dir (mtime = now) — stands in for an
        // in-flight login's dir in another window.
        let young_id = daruda_store::accounts::AccountId::new();
        let young_dir = account_config_dir(data_dir, young_id);
        std::fs::create_dir_all(&young_dir).expect("create young orphan dir");

        // A large grace window (1 hour, well above "just created") must
        // spare a dir this young — the in-flight-login protection.
        sweep_orphan_dirs(data_dir, &[], Duration::from_secs(60 * 60));
        assert!(
            young_dir.exists(),
            "a dir younger than the grace window must be spared"
        );

        // Zero grace treats the very same dir as old enough to sweep —
        // proves real (grace-expired) orphans are still removed.
        sweep_orphan_dirs(data_dir, &[], Duration::ZERO);
        assert!(
            !young_dir.exists(),
            "with zero grace, an orphan dir must still be swept"
        );
    }
}
