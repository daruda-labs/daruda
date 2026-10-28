//! Public API tests for path.

use daruda_core::path::*;
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;

#[test]
fn an_empty_file_in_new_nested_directories_can_be_published() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("new").join("nested");
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("log");
    let file = std::fs::File::create(&path).unwrap();

    sync_file_entry(&file, &path).unwrap();
    assert!(std::fs::read(&path).unwrap().is_empty());
}

#[test]
fn a_renamed_file_is_published_using_its_final_writable_handle() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("temporary");
    let target = temp.path().join("snapshot");
    std::fs::write(&source, b"state").unwrap();
    std::fs::rename(&source, &target).unwrap();
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(&target)
        .unwrap();

    sync_file_entry(&file, &target).unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"state");
}

#[cfg(windows)]
#[test]
fn publication_refuses_a_handle_without_write_access() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("file");
    std::fs::write(&path, b"state").unwrap();
    let file = std::fs::File::open(&path).unwrap();

    assert!(sync_file_entry(&file, &path).is_err());
}

#[cfg(unix)]
#[test]
fn an_owner_only_file_is_created_narrow_and_an_existing_one_is_narrowed() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let fresh = dir.path().join("fresh.log");
    open_owner_only(
        std::fs::OpenOptions::new().create(true).append(true),
        &fresh,
    )
    .unwrap();
    assert_eq!(mode_of(&fresh), 0o600);

    let old = dir.path().join("old.log");
    std::fs::write(&old, b"before").unwrap();
    std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o644)).unwrap();
    open_owner_only(std::fs::OpenOptions::new().append(true), &old).unwrap();
    assert_eq!(mode_of(&old), 0o600);
    assert_eq!(
        std::fs::read(&old).unwrap(),
        b"before",
        "appending keeps the bytes"
    );
}

#[test]
fn a_path_is_the_same_as_itself_even_when_it_does_not_exist() {
    let gone = Path::new("/daruda/no/such/dir");
    assert!(same_path(gone, gone));
    assert!(!same_path(gone, Path::new("/daruda/no/other")));
}

/// The shape every caller has: one side through a symlink, the other
/// resolved — FSEvents reporting `/private/tmp` for a root typed `/tmp`.
#[cfg(unix)]
#[test]
fn a_symlinked_spelling_names_the_same_place() {
    let temp = tempfile::tempdir().unwrap();
    let real = temp.path().join("real");
    std::fs::create_dir(&real).unwrap();
    let link = temp.path().join("link");
    symlink(&real, &link).unwrap();
    let real = canonicalize(&real).unwrap();

    assert!(same_path(&link, &real));
    std::fs::write(real.join("f"), b"x").unwrap();
    assert_eq!(strip_root(&real.join("f"), &link), Some(PathBuf::from("f")));
    assert_eq!(strip_root(&link.join("f"), &real), Some(PathBuf::from("f")));
    // Removed already: only the root can still be resolved.
    assert_eq!(
        strip_root(&real.join("gone"), &link),
        Some(PathBuf::from("gone"))
    );
    assert!(is_within(&real, &link));
    assert!(!is_within(temp.path(), &link));
}

#[test]
fn canonicalize_resolves_a_relative_path_to_an_absolute_one() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("f");
    std::fs::write(&file, b"x").unwrap();

    let resolved = canonicalize(&file).unwrap();

    assert!(resolved.is_absolute());
    assert!(!resolved.to_string_lossy().starts_with(r"\\?\"));
}

/// What callers compare against: two spellings of one file must come back
/// as the same path, which is the whole reason they canonicalize.
#[test]
fn two_spellings_of_one_path_canonicalize_alike() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("d");
    std::fs::create_dir(&dir).unwrap();
    let file = dir.join("f");
    std::fs::write(&file, b"x").unwrap();

    let direct = canonicalize(&file).unwrap();
    let roundabout = canonicalize(dir.join("..").join("d").join("f")).unwrap();

    assert_eq!(direct, roundabout);
}

#[cfg(windows)]
#[test]
fn a_long_canonical_child_is_within_a_short_regular_root() {
    use std::os::windows::ffi::OsStrExt as _;

    let temp = tempfile::tempdir().unwrap();
    let root = canonicalize(temp.path()).unwrap();
    assert!(!root.to_string_lossy().starts_with(r"\\?\"));
    let extended = std::fs::canonicalize(&root).unwrap();
    let mut relative = std::path::PathBuf::new();
    while extended.join(&relative).as_os_str().encode_wide().count() <= 300 {
        relative.push("nested-directory");
    }
    std::fs::create_dir_all(extended.join(&relative)).unwrap();
    let child = canonicalize(extended.join(&relative)).unwrap();
    assert!(child.to_string_lossy().starts_with(r"\\?\"));

    assert!(is_within(&child, &root));
    assert_eq!(strip_root(&child, &root), Some(relative.clone()));
    assert_eq!(strip_root(&root.join(&relative), &root), Some(relative));
    assert!(!is_within(&root, &child));
}

#[cfg(windows)]
#[test]
fn verbatim_dot_names_do_not_alias_regular_names_during_containment() {
    let temp = tempfile::tempdir().unwrap();
    let root = canonicalize(temp.path()).unwrap();
    std::fs::create_dir(root.join("repo")).unwrap();
    let extended = std::fs::canonicalize(&root).unwrap();
    let distinct = extended.join("repo.").join("child");
    std::fs::create_dir_all(&distinct).unwrap();

    assert!(!is_within(&distinct, &root.join("repo")));
    assert_eq!(strip_root(&distinct, &root.join("repo")), None);
}

#[test]
fn canonicalizing_a_missing_path_is_an_error() {
    let temp = tempfile::tempdir().unwrap();

    assert!(canonicalize(temp.path().join("nope")).is_err());
}

#[test]
fn creates_missing_directory_and_its_parents() {
    let temp = tempfile::tempdir().unwrap();
    let nested = temp.path().join("a").join("b");

    create_owner_only_dir(&nested).unwrap();

    assert!(nested.is_dir());
}

#[test]
fn creating_an_existing_directory_succeeds() {
    let temp = tempfile::tempdir().unwrap();

    create_owner_only_dir(temp.path()).unwrap();

    assert!(temp.path().is_dir());
}

/// The caller's contract: an existing directory keeps the mode it
/// already had, so a shared data directory is never re-permissioned.
#[cfg(unix)]
#[test]
fn an_existing_directory_keeps_its_mode() {
    use std::os::unix::fs::PermissionsExt as _;

    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("preexisting");
    std::fs::create_dir(&dir).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();

    create_owner_only_dir(&dir).unwrap();

    let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o755);
}

#[test]
fn a_link_to_a_directory_resolves_through_it() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("d");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("f"), b"x").unwrap();
    let link = temp.path().join("link");

    symlink(&dir, &link).unwrap();

    assert!(is_symlink(&link));
    assert_eq!(std::fs::read(link.join("f")).unwrap(), b"x");
}

#[test]
fn a_link_to_a_file_reads_as_the_file() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("f");
    std::fs::write(&file, b"x").unwrap();
    let link = temp.path().join("link");

    symlink(&file, &link).unwrap();

    assert!(is_symlink(&link));
    assert_eq!(std::fs::read(&link).unwrap(), b"x");
}

/// Callers plant links ahead of the file they point at, so a target that
/// does not exist yet must still get its link.
#[test]
fn a_dangling_link_is_still_created() {
    let temp = tempfile::tempdir().unwrap();
    let link = temp.path().join("link");

    symlink(temp.path().join("never-written"), &link).unwrap();

    assert!(is_symlink(&link));
    assert!(
        !link.exists(),
        "`exists` follows the link, which goes nowhere"
    );
}

/// A relative target is relative to the link's directory — which is also
/// where the platform that has to pick a kind must look.
#[test]
fn a_relative_target_resolves_against_the_link_s_directory() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("d");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("f"), b"x").unwrap();
    let link = temp.path().join("rel");

    symlink("d", &link).unwrap();

    assert_eq!(std::fs::read_link(&link).unwrap(), Path::new("d"));
    assert_eq!(std::fs::read(link.join("f")).unwrap(), b"x");
}

#[test]
fn removing_a_directory_link_leaves_the_directory() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("d");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("f"), b"x").unwrap();
    let link = temp.path().join("link");
    symlink(&dir, &link).unwrap();

    remove_symlink(&link).unwrap();

    assert!(std::fs::symlink_metadata(&link).is_err());
    assert_eq!(std::fs::read(dir.join("f")).unwrap(), b"x");
}

#[test]
fn removing_a_file_link_leaves_the_file() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("f");
    std::fs::write(&file, b"x").unwrap();
    let link = temp.path().join("link");
    symlink(&file, &link).unwrap();

    remove_symlink(&link).unwrap();

    assert!(std::fs::symlink_metadata(&link).is_err());
    assert_eq!(std::fs::read(&file).unwrap(), b"x");
}

#[test]
fn a_dangling_link_is_removed_too() {
    let temp = tempfile::tempdir().unwrap();
    let link = temp.path().join("link");
    symlink(temp.path().join("never-written"), &link).unwrap();

    remove_symlink(&link).unwrap();

    assert!(std::fs::symlink_metadata(&link).is_err());
}

/// The guard: a real file handed to a link remover survives.
#[test]
fn a_real_file_is_refused_not_deleted() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("f");
    std::fs::write(&file, b"x").unwrap();

    let err = remove_symlink(&file).unwrap_err();

    assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(std::fs::read(&file).unwrap(), b"x");
}

#[cfg(unix)]
fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

fn is_symlink(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}
