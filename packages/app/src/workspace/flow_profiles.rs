//! The profiles each flow file declares, read once per version of the file.
//!
//! A control listing names every flow's profiles, and it runs on the main
//! thread for every `daruda_flow_list` call. Reading and parsing each file
//! every time costs a slow or network volume the listing's whole latency;
//! a file's modification time and length say whether it changed, for the
//! price of a `stat`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::SystemTime;

/// What a file looked like when its profiles were read.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Version {
    modified: Option<SystemTime>,
    len: u64,
}

/// Each file's profiles, under the version they were read from.
type ReadProfiles = HashMap<PathBuf, (Version, Vec<String>)>;

static READ: LazyLock<Mutex<ReadProfiles>> = LazyLock::new(Default::default);

/// The profiles the file at `path` declares, in the order the desktop picker
/// offers them.
///
/// A file that cannot be read or parsed answers none: the run that follows
/// fails on the same read and names it properly, and refusing at a profile
/// check would report a parse error as a profile problem.
pub(in crate::workspace) fn declared_profiles(path: &Path) -> Vec<String> {
    let Ok(meta) = std::fs::metadata(path) else {
        return Vec::new();
    };
    let version = Version {
        modified: meta.modified().ok(),
        len: meta.len(),
    };
    let mut read = READ.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((seen, profiles)) = read.get(path)
        && *seen == version
    {
        return profiles.clone();
    }
    let profiles = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| daruda_flow::load::profiles(&text).ok())
        .unwrap_or_default();
    read.insert(path.to_path_buf(), (version, profiles.clone()));
    profiles
}

#[cfg(test)]
mod tests {
    use super::declared_profiles;

    const WITH: &str = "version: 1\nprofiles:\n  cheap: {}\nnodes: []\n";
    const WITHOUT: &str = "version: 1\nnodes: []\n";

    /// A rewritten file is read again; an unreadable one declares nothing.
    #[test]
    fn a_changed_file_is_read_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ship.yaml");
        std::fs::write(&path, WITH).unwrap();
        assert_eq!(declared_profiles(&path), vec!["cheap".to_string()]);
        std::fs::write(&path, WITHOUT).unwrap();
        assert!(declared_profiles(&path).is_empty(), "the length changed");
        assert!(declared_profiles(&dir.path().join("gone.yaml")).is_empty());
    }
}
