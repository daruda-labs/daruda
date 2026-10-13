//! Tests for the public `daruda_agent::accounts::sweep` API.

use daruda_agent::accounts::layout::account_config_dir;
use daruda_agent::accounts::layout::accounts_root;
use daruda_agent::accounts::sweep::*;
use std::time::Duration;

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
fn sweep_unlinks_a_codex_shaped_orphan_without_following_its_links() {
    let system_home = tempfile::tempdir().expect("system home");
    std::fs::write(system_home.path().join("AGENTS.md"), b"user data").expect("user file");

    let tmp = tempfile::tempdir().expect("tempdir");
    let data_dir = tmp.path();
    let orphan = account_config_dir(data_dir, daruda_store::accounts::AccountId::new());
    std::fs::create_dir_all(&orphan).expect("create orphan dir");
    daruda_core::path::symlink(system_home.path(), orphan.join("linked")).expect("symlink");

    sweep_orphan_dirs(data_dir, &[], Duration::ZERO);

    assert!(!orphan.exists(), "the orphan dir must be swept");
    assert_eq!(
        std::fs::read(system_home.path().join("AGENTS.md")).expect("target survives"),
        b"user data",
        "the sweep must unlink, never follow, a link into the user's home"
    );
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
