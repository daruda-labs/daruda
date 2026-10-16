//! Flushing a node's files before the log says they exist (O1). Best effort,
//! and off the run thread: a resume re-checks every passed output and reruns
//! a node whose file is gone, so a flush that fails costs a rerun, never a
//! wrong result.

use std::path::{Path, PathBuf};

use crate::NodeId;

/// Flush a passed node's output and the directory that names it.
pub(super) async fn sync_file(path: PathBuf) {
    smol::unblock(move || {
        // SILENT-OK: see the module note; a lost output is found on resume.
        let _ = std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .and_then(|file| file.sync_all());
        if let Some(parent) = path.parent() {
            // SILENT-OK: as above.
            let _ = daruda_core::path::sync_dir(parent);
        }
    })
    .await;
}

/// Flush the directories an archive moved files out of and into.
pub(super) async fn sync_dirs(set: &[(NodeId, Option<PathBuf>)], log_dir: &Path) {
    let mut dirs: Vec<PathBuf> = set
        .iter()
        .filter_map(|(_, output)| output.as_deref()?.parent().map(Path::to_path_buf))
        .collect();
    dirs.push(log_dir.to_path_buf());
    dirs.sort();
    dirs.dedup();
    smol::unblock(move || {
        for dir in dirs {
            // SILENT-OK: see the module note.
            let _ = daruda_core::path::sync_dir(&dir);
        }
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flushing_leaves_the_file_as_it_was() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("out.md");
        std::fs::write(&path, "kept\n").expect("write");
        smol::block_on(sync_file(path.clone()));
        smol::block_on(sync_dirs(&[("out".into(), Some(path.clone()))], dir.path()));
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "kept\n");
    }

    /// A node that wrote nothing has nothing to flush, which is not an error.
    #[test]
    fn a_missing_file_is_not_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        smol::block_on(sync_file(dir.path().join("never-written.md")));
    }
}
