//! The profiles and inputs each flow file declares, read once per version of
//! the file.
//!
//! A control listing names every flow's profiles and inputs, and it runs on
//! the main thread for every `daruda_flow_list` call. Reading and parsing each
//! file every time costs a slow or network volume the listing's whole
//! latency; a file's modification time and length say whether it changed,
//! for the price of a `stat`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::SystemTime;

use daruda_flow::input::InputDecl;

/// What a file looked like when its declarations were read.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Version {
    modified: Option<SystemTime>,
    len: u64,
}

/// What one file declares about how it is started.
#[derive(Default)]
struct Declared {
    profiles: Vec<String>,
    inputs: Vec<InputDecl>,
}

/// Each file's declarations, under the version they were read from.
type ReadDeclarations = HashMap<PathBuf, (Version, Declared)>;

static READ: LazyLock<Mutex<ReadDeclarations>> = LazyLock::new(Default::default);

/// The profiles the file at `path` declares, in the order the desktop picker
/// offers them.
///
/// A file that cannot be read or parsed answers none: the run that follows
/// fails on the same read and names it properly, and refusing at a profile
/// check would report a parse error as a profile problem.
pub(in crate::workspace) fn declared_profiles(path: &Path) -> Vec<String> {
    declared(path, |declared| declared.profiles.clone())
}

/// The inputs the file at `path` declares, in the order its author wrote
/// them. None for a file that cannot be read or parsed, for the reason
/// [`declared_profiles`] gives.
pub(in crate::workspace) fn declared_inputs(path: &Path) -> Vec<InputDecl> {
    declared(path, |declared| declared.inputs.clone())
}

/// Both of what `path` declares, from one read, so a check of one against
/// the other cannot straddle a change to the file.
pub(in crate::workspace) fn declared_flow(path: &Path) -> (Vec<String>, Vec<InputDecl>) {
    declared(path, |declared| {
        (declared.profiles.clone(), declared.inputs.clone())
    })
}

/// Why a start's profile does not fit what the flow declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum ProfileRefusal {
    /// The flow declares profiles and the start names none.
    Needed,
    /// The start names a profile the flow does not declare.
    Unknown,
}

/// Whether `profile` fits a flow declaring `profiles`. A flow that declares
/// some runs only with one of them named: a name it does not declare is
/// refused rather than run under the defaults.
pub(in crate::workspace) fn profile_refusal(
    profiles: &[String],
    profile: Option<&str>,
) -> Option<ProfileRefusal> {
    match profile {
        None if !profiles.is_empty() => Some(ProfileRefusal::Needed),
        Some(profile) if !profiles.iter().any(|p| p == profile) => Some(ProfileRefusal::Unknown),
        _ => None,
    }
}

/// `pick` applied to what `path` declares, read again only when the file
/// changed. Picked under the lock so a caller copies just what it uses.
fn declared<T>(path: &Path, pick: impl FnOnce(&Declared) -> T) -> T {
    let Ok(meta) = std::fs::metadata(path) else {
        return pick(&Declared::default());
    };
    let version = Version {
        modified: meta.modified().ok(),
        len: meta.len(),
    };
    let mut read = READ.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((seen, declared)) = read.get(path)
        && *seen == version
    {
        return pick(declared);
    }
    let declared = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| daruda_flow::parse::parse_flow_file(&text).ok())
        .map(|file| Declared {
            profiles: file.profiles.into_keys().collect(),
            inputs: file.inputs,
        })
        .unwrap_or_default();
    let picked = pick(&declared);
    read.insert(path.to_path_buf(), (version, declared));
    picked
}

#[cfg(test)]
mod tests {
    use super::{declared_inputs, declared_profiles};

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

    #[test]
    fn inputs_are_read_in_the_order_they_are_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("review.yaml");
        std::fs::write(
            &path,
            "version: 1\ninputs:\n  pr: { type: number }\n  focus: {}\nnodes: []\n",
        )
        .unwrap();
        let names: Vec<_> = declared_inputs(&path).into_iter().map(|d| d.name).collect();
        assert_eq!(names, ["pr", "focus"]);
    }

    /// Only what the file declares is read: a graph that would not build
    /// still offers its profiles, and a file that does not parse offers none,
    /// leaving the error to the submission that names it.
    #[test]
    fn unreadable_or_invalid_sources_declare_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("flow.yaml");
        assert!(declared_profiles(&path).is_empty());
        std::fs::write(&path, "not: [valid").unwrap();
        assert!(declared_profiles(&path).is_empty());
        std::fs::write(
            &path,
            "version: 1\nprofiles:\n  cheap: {}\nnodes:\n  - id: a\n    kind: command\n    deps: [gone]\n    run: \"true\"\n",
        )
        .unwrap();
        assert_eq!(declared_profiles(&path), ["cheap"]);
    }
}
