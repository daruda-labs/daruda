//! File metadata collected with the cached directory listing, never per row.

use std::{path::PathBuf, time::SystemTime};

use crate::workspace::flow_paths::{FlowSources, FoundFlow};

#[derive(Default)]
pub(in crate::workspace) struct FlowListing {
    pub files: Vec<FoundFlow>,
    pub modified: Vec<(PathBuf, SystemTime)>,
}

impl FlowListing {
    pub fn read(sources: &FlowSources) -> Self {
        let files = sources.list_flows();
        let modified = files
            .iter()
            .filter_map(|found| {
                let time = std::fs::metadata(&found.path).ok()?.modified().ok()?;
                Some((found.path.clone(), time))
            })
            .collect();
        Self { files, modified }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::flow_paths::{FlowOrigin, flows_dir};

    #[test]
    fn metadata_obeys_the_same_source_precedence_as_the_listing() {
        let temp = tempfile::tempdir().unwrap();
        let sources = FlowSources {
            lane: temp.path().join("lane"),
            project: Some(temp.path().join("project")),
            global: temp.path().join("global"),
        };
        for (dir, _) in sources.dirs() {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("ship.yaml"), "version: 1\n").unwrap();
        }
        let listing = FlowListing::read(&sources);
        assert_eq!(listing.files.len(), 1);
        assert_eq!(listing.files[0].origin, FlowOrigin::Repo);
        assert_eq!(listing.modified.len(), 1);
        assert_eq!(
            listing.modified[0].0,
            flows_dir(&sources.lane).join("ship.yaml")
        );
    }
}
