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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use daruda_store::project::{ProjectOverride, ProjectState};

    use super::*;

    /// A project rooted at a fresh non-git directory, so it bootstraps
    /// exactly one lane with id 0.
    fn project(id: ProjectId, dir_name: &str) -> (Project, PathBuf) {
        let dir = std::env::temp_dir().join(dir_name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let state = ProjectState {
            schema_version: 0,
            uuid: ProjectUuid::new(),
            root: dir.clone(),
            name: None,
            lanes: Vec::new(),
            last_active_lane_id: 0,
            next_lane_id: 0,
            default_branch: None,
            base_branch: None,
        };
        (
            Project::from_disk(id, &state, &ProjectOverride::default()),
            dir,
        )
    }

    #[test]
    fn a_lane_ref_reaches_its_own_projects_lane_not_a_same_numbered_one() {
        let (first, first_dir) = project(0, "daruda_projects_lookup_a");
        let (mut second, second_dir) = project(1, "daruda_projects_lookup_b");
        second.group_id = Some(7);
        let second_uuid = second.uuid;
        let projects = Projects::from(vec![first, second]);

        // Both projects number their only lane 0; the ref tells them apart.
        let target = LaneRef {
            project: 1,
            lane: 0,
        };
        let lane = projects.lane(target).expect("project 1's lane 0");
        assert!(std::ptr::eq(lane, &projects[1].lanes[0]));
        assert_eq!(projects.by_uuid(second_uuid).map(|p| p.id), Some(1));
        assert!(projects.get(2).is_none());

        let refs: Vec<LaneRef> = projects.lanes().map(|(r, _, _)| r).collect();
        assert_eq!(
            refs,
            vec![
                LaneRef {
                    project: 0,
                    lane: 0
                },
                target
            ]
        );
        assert_eq!(
            projects.ungrouped().map(|p| p.id).collect::<Vec<_>>(),
            vec![0]
        );
        assert_eq!(
            projects.in_group(7).map(|p| p.id).collect::<Vec<_>>(),
            vec![1]
        );

        let _ = std::fs::remove_dir_all(first_dir);
        let _ = std::fs::remove_dir_all(second_dir);
    }

    #[test]
    fn a_path_is_owned_by_the_deepest_lane_holding_it_in_any_project() {
        let (outer, outer_dir) = project(0, "daruda_projects_owning_outer");
        let (mut other, other_dir) = project(1, "daruda_projects_owning_other");
        // A worktree checked out inside the first project's root, filed under
        // the second project: the deeper lane wins even across projects.
        let nested_dir = outer_dir.join("nested");
        std::fs::create_dir_all(&nested_dir).unwrap();
        let nested = Lane::default_for_project(5, nested_dir.clone());
        other.lanes.push(nested);
        let projects = Projects::from(vec![outer, other]);

        let in_nested = nested_dir.join("a.txt");
        let in_outer = outer_dir.join("b.txt");
        let in_other = other_dir.join("c.txt");
        assert_eq!(
            projects.lane_owning(&in_nested),
            Some(LaneRef {
                project: 1,
                lane: 5
            })
        );
        assert_eq!(
            projects.lane_owning(&in_outer),
            Some(LaneRef {
                project: 0,
                lane: 0
            })
        );
        assert_eq!(
            projects.lane_owning(&in_other),
            Some(LaneRef {
                project: 1,
                lane: 0
            })
        );
        assert_eq!(
            projects.lane_owning(std::path::Path::new("/nowhere/at/all")),
            None
        );

        let _ = std::fs::remove_dir_all(outer_dir);
        let _ = std::fs::remove_dir_all(other_dir);
    }
}
