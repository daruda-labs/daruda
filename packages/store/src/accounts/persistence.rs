//! `accounts.json` load/save via the shared JSON helpers.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

use fs4::fs_std::FileExt as _;

use crate::accounts::{AccountsState, SCHEMA_VERSION};
use crate::observability::error_report::{ErrorReport, ErrorSeverity};
use crate::observability::log_writer::LogWriter;
use crate::persistence::{LoadOutcome, default_data_dir, load_json_file, save_json_atomic};

pub fn accounts_path_in(data_dir: &Path) -> PathBuf {
    data_dir.join("accounts.json")
}

fn accounts_lock_path_in(data_dir: &Path) -> PathBuf {
    data_dir.join("accounts.json.lock")
}

/// Load `accounts.json`, upgrading an older schema in place. `None` for a
/// missing, unreadable, or newer-than-this-build file.
pub fn load_accounts_in(data_dir: &Path) -> Option<AccountsState> {
    let path = accounts_path_in(data_dir);
    let state = read_accounts_file(&path)?;
    if state.schema_version == SCHEMA_VERSION {
        return Some(state);
    }
    migrate_to_current(data_dir, &path, state)
}

/// Parse the file, rejecting one written by a newer daruda.
fn read_accounts_file(path: &Path) -> Option<AccountsState> {
    match load_json_file::<AccountsState>("accounts", path) {
        LoadOutcome::Parsed(state) if state.schema_version <= SCHEMA_VERSION => Some(state),
        _ => None,
    }
}

/// Stamp `state` at the current schema and write it back.
///
/// v1's `default_by_provider` is dropped rather than carried over: on disk an
/// explicit "Set default" and a first account auto-promoted by the login flow
/// were the same field, so a preserved value is more likely the bug than the
/// user's intent. Starting empty means new panes are System until one click.
fn migrate_to_current(
    data_dir: &Path,
    path: &Path,
    mut state: AccountsState,
) -> Option<AccountsState> {
    state.schema_version = SCHEMA_VERSION;
    // Another window may have written the file between our read and this
    // write; take its result rather than overwriting it with ours.
    match load_json_file::<AccountsState>("accounts", path) {
        LoadOutcome::Parsed(disk) if disk.schema_version > SCHEMA_VERSION => return None,
        LoadOutcome::Parsed(disk) if disk.schema_version == SCHEMA_VERSION => return Some(disk),
        _ => {}
    }
    if let Err(e) = save_accounts_in(data_dir, &state) {
        // The in-memory state is still correct, so the caller gets it either
        // way; the next successful save re-attempts the upgrade.
        LogWriter::log(
            ErrorReport::new("Failed to save accounts.json after a schema migration")
                .severity(ErrorSeverity::Warning)
                .from_error(&e)
                .at(file!(), line!())
                .dedup("store.accounts.migrate_save")
                .build(),
        );
    }
    Some(state)
}

/// Load `accounts.json` for a write that will replace it.
///
/// Unlike [`load_accounts_in`], only a *missing* file reads as the empty list.
/// One that is present but unreadable, or written by a newer daruda, is an
/// error: saving over it would erase every account it holds, and the file may
/// yet be fixed by hand or read by the build that wrote it.
fn load_for_write(data_dir: &Path) -> std::io::Result<AccountsState> {
    let path = accounts_path_in(data_dir);
    let newer = || {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "accounts.json was written by a newer daruda; left untouched",
        )
    };
    match load_json_file::<AccountsState>("accounts", &path) {
        LoadOutcome::Missing => Ok(AccountsState::default()),
        LoadOutcome::Corrupt => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "accounts.json could not be read; left untouched",
        )),
        LoadOutcome::Parsed(state) if state.schema_version > SCHEMA_VERSION => Err(newer()),
        LoadOutcome::Parsed(state) if state.schema_version == SCHEMA_VERSION => Ok(state),
        LoadOutcome::Parsed(state) => migrate_to_current(data_dir, &path, state).ok_or_else(newer),
    }
}

pub fn save_accounts_in(data_dir: &Path, state: &AccountsState) -> std::io::Result<()> {
    let path = accounts_path_in(data_dir);
    save_json_atomic(data_dir, &path, state)
}

/// Load, mutate, and save `accounts.json` while holding a sibling lock file.
///
/// The plain load/save helpers remain available for callers that only read or
/// already hold stronger coordination. Account UI flows that merge user edits
/// into disk state should use this to avoid the read-modify-write race between
/// open windows.
pub fn mutate_accounts_in<R>(
    data_dir: &Path,
    mutate: impl FnOnce(&mut AccountsState) -> R,
) -> std::io::Result<(AccountsState, R)> {
    std::fs::create_dir_all(data_dir)?;
    let lock_file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(accounts_lock_path_in(data_dir))?;
    lock_file.lock_exclusive()?;

    let mut state = load_for_write(data_dir)?;
    let result = mutate(&mut state);
    save_accounts_in(data_dir, &state)?;
    Ok((state, result))
}

pub fn load_accounts() -> Option<AccountsState> {
    load_accounts_in(&default_data_dir())
}

pub fn save_accounts(state: &AccountsState) -> std::io::Result<()> {
    save_accounts_in(&default_data_dir(), state)
}

pub fn mutate_accounts<R>(
    mutate: impl FnOnce(&mut AccountsState) -> R,
) -> std::io::Result<(AccountsState, R)> {
    mutate_accounts_in(&default_data_dir(), mutate)
}
