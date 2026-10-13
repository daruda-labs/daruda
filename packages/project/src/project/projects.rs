//! The projects one window has open, and the lookups every caller used to
//! spell out over a `Vec<Project>`.
//!
//! A lane id is unique only inside its project, so the lookups that cross
//! projects take a [`LaneRef`] and hand one back.

use std::ops::{Deref, DerefMut};

use daruda_store::project::{GroupId, LaneRef, ProjectId, ProjectUuid};

use super::Project;
use crate::lane::Lane;
use crate::lane::availability::LaneAvailability;

/// A window's open projects, in left-dock order.
#[derive(Debug, Default)]
pub struct Projects(Vec<Project>);

impl Projects {
    pub fn get(&self, id: ProjectId) -> Option<&Project> {
        self.0.iter().find(|p| p.id == id)
    }

    pub fn get_mut(&mut self, id: ProjectId) -> Option<&mut Project> {
        self.0.iter_mut().find(|p| p.id == id)
    }

    /// The project open under `uuid`, the durable name a task and a
    /// persisted state file use for it.
    pub fn by_uuid(&self, uuid: ProjectUuid) -> Option<&Project> {
        self.0.iter().find(|p| p.uuid == uuid)
    }

    pub fn lane(&self, target: LaneRef) -> Option<&Lane> {
        self.get(target.project)?.lane(target.lane)
    }

    pub fn lane_mut(&mut self, target: LaneRef) -> Option<&mut Lane> {
        self.get_mut(target.project)?.lane_mut(target.lane)
    }

    /// Every lane of every project, each with the ref that names it.
    pub fn lanes(&self) -> impl Iterator<Item = (LaneRef, &Project, &Lane)> {
        self.0.iter().flat_map(|project| {
            project.lanes.iter().map(move |lane| {
                let target = LaneRef {
                    project: project.id,
                    lane: lane.id,
                };
                (target, project, lane)
            })
        })
    }

    /// The lane `path` lies in, across every project — the deepest, so a
    /// worktree nested inside another lane's checkout wins over its parent.
    /// Spellings are compared as one place, since a path may come through a
    /// symlink. Only a lane whose root is present can own a file; `None` when
    /// no such lane holds it.
    pub fn lane_owning(&self, path: &std::path::Path) -> Option<LaneRef> {
        self.lanes()
            .filter(|(_, _, lane)| lane.availability == LaneAvailability::Present)
            .filter(|(_, _, lane)| daruda_core::path::is_within(path, &lane.path))
            .max_by_key(|(_, _, lane)| lane.path.components().count())
            .map(|(target, _, _)| target)
    }

    /// Projects in no group — the ones the left dock ranks beside groups.
    pub fn ungrouped(&self) -> impl Iterator<Item = &Project> {
        self.0.iter().filter(|p| p.group_id.is_none())
    }

    pub fn in_group(&self, group: GroupId) -> impl Iterator<Item = &Project> {
        self.0.iter().filter(move |p| p.group_id == Some(group))
    }

    pub fn push(&mut self, project: Project) {
        self.0.push(project);
    }

    pub fn retain(&mut self, keep: impl FnMut(&Project) -> bool) {
        self.0.retain(keep);
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }
}

/// Read and reorder access as a slice — iteration, `len`, indexing and
/// sorting stay what they were on the `Vec`.
impl Deref for Projects {
    type Target = [Project];

    fn deref(&self) -> &[Project] {
        &self.0
    }
}

impl DerefMut for Projects {
    fn deref_mut(&mut self) -> &mut [Project] {
        &mut self.0
    }
}

impl From<Vec<Project>> for Projects {
    fn from(projects: Vec<Project>) -> Self {
        Self(projects)
    }
}

impl FromIterator<Project> for Projects {
    fn from_iter<I: IntoIterator<Item = Project>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl<'a> IntoIterator for &'a Projects {
    type Item = &'a Project;
    type IntoIter = std::slice::Iter<'a, Project>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl<'a> IntoIterator for &'a mut Projects {
    type Item = &'a mut Project;
    type IntoIter = std::slice::IterMut<'a, Project>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter_mut()
    }
}
