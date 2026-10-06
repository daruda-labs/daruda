//! Lane / project directory availability recompute + the single write
//! site that flips a lane's [`LaneAvailability`].
//!
//! A lane (or project) root can vanish or become unreadable between
//! sessions. These setters classify the live filesystem and stamp the
//! runtime flag so the read side (file-tree scan, watcher, PTY spawn)
//! short-circuits instead of spamming directory-read errors. The flag
//! is never serialized — it is rebuilt on restore, on activation, and
//! whenever a root load fails.

use std::path::PathBuf;

use daruda_store::project::{LaneRef, ProjectId};

use crate::lane::availability::{LaneAvailability, classify_dir};

use super::Workspace;

impl Workspace {
    /// Single write site for a lane's availability. No-op when the ref
    /// no longer resolves (lane removed between schedule and apply).
    pub(in crate::workspace) fn set_lane_availability(&mut self, r: LaneRef, a: LaneAvailability) {
        if let Some(l) = self.lane_for_mut(r) {
            l.availability = a;
        }
    }

    /// Single write site for a project's availability. No-op when the
    /// id no longer resolves (project closed between schedule and apply).
    pub(in crate::workspace) fn set_project_availability(
        &mut self,
        id: ProjectId,
        a: LaneAvailability,
    ) {
        if let Some(p) = self.project_for_mut(id) {
            p.availability = a;
        }
    }

    /// Re-classify one lane's root *and* its owning project's root against
    /// the live filesystem. The coupling is intentional: every
    /// `activate_lane` calls this, and the project header must reflect the
    /// live filesystem too. Paths are collected before mutating so no
    /// immutable borrow is held while calling the `&mut self` setters.
    pub(in crate::workspace) fn recompute_availability_for(&mut self, r: LaneRef) {
        let lane_path = self.lane_for(r).map(|l| l.path.clone());
        let project_root = self.project_for(r.project).map(|p| p.root.clone());

        if let Some(path) = lane_path {
            let a = classify_dir(&path);
            self.set_lane_availability(r, a);
        }
        if let Some(root) = project_root {
            let a = classify_dir(&root);
            self.set_project_availability(r.project, a);
        }
    }

    /// Re-classify every project root and every lane root. Run on
    /// restore (the persisted set may reference directories that no
    /// longer exist) so the read side has correct flags before the
    /// pane-rebuild loop touches any path.
    pub(in crate::workspace) fn recompute_availability(&mut self) {
        // Collect refs + paths first; the classify + setters below take
        // `&mut self`, so we cannot hold a borrow of `self.projects`.
        let mut lane_targets: Vec<(LaneRef, PathBuf)> = Vec::new();
        let mut project_targets: Vec<(ProjectId, PathBuf)> = Vec::new();
        for project in &self.projects {
            project_targets.push((project.id, project.root.clone()));
            for lane in &project.lanes {
                let r = LaneRef {
                    project: project.id,
                    lane: lane.id,
                };
                lane_targets.push((r, lane.path.clone()));
            }
        }
        for (r, path) in lane_targets {
            let a = classify_dir(&path);
            self.set_lane_availability(r, a);
        }
        for (id, root) in project_targets {
            let a = classify_dir(&root);
            self.set_project_availability(id, a);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use std::path::Path;
    use tempfile::TempDir;

    /// Build a Workspace rooted at `root` with a single project/lane.
    /// Returns the entity plus the persistence-dir guard (bound by the
    /// caller so its on-disk dir lives for the test). The `add_window`
    /// handle is dropped here — the GPUI test executor keeps the entity
    /// alive independently, so the returned `Entity<Workspace>` stays usable.
    fn workspace_at(cx: &mut TestAppContext, root: &Path) -> (gpui::Entity<Workspace>, TempDir) {
        crate::test_support::init_gpui_component(cx);
        let config = daruda_config::Config::default();
        let project = daruda_store::project::Project::from_path(root);
        let data_dir = tempfile::tempdir().unwrap();
        let data_path = data_dir.path().to_path_buf();
        let wh = cx.add_window(|window, cx| {
            Workspace::new_with_project_for_test(&config, Some(project), data_path, window, cx)
        });
        (wh.root(cx).unwrap(), data_dir)
    }

    #[gpui::test]
    fn recompute_for_present_lane_sets_present(cx: &mut TestAppContext) {
        let temp = tempfile::tempdir().unwrap();
        let (ws, _data_dir) = workspace_at(cx, temp.path());
        ws.update(cx, |ws, _cx| {
            let r = ws.active_ref();
            ws.recompute_availability_for(r);
            assert_eq!(
                ws.lane_for(r).unwrap().availability,
                LaneAvailability::Present
            );
            assert_eq!(
                ws.project_for(r.project).unwrap().availability,
                LaneAvailability::Present
            );
        });
    }

    #[gpui::test]
    fn recompute_for_removed_lane_dir_sets_missing(cx: &mut TestAppContext) {
        let temp = tempfile::tempdir().unwrap();
        let (ws, _data_dir) = workspace_at(cx, temp.path());
        // Remove the lane root out from under it, then reclassify. The
        // persistence dir (`_data_dir`) is a separate tempdir and stays.
        drop(temp);
        ws.update(cx, |ws, _cx| {
            let r = ws.active_ref();
            ws.recompute_availability_for(r);
            assert_eq!(
                ws.lane_for(r).unwrap().availability,
                LaneAvailability::Missing
            );
            assert_eq!(
                ws.project_for(r.project).unwrap().availability,
                LaneAvailability::Missing
            );
        });
    }

    #[gpui::test]
    fn recompute_all_classifies_every_lane(cx: &mut TestAppContext) {
        let temp = tempfile::tempdir().unwrap();
        let (ws, _data_dir) = workspace_at(cx, temp.path());
        ws.update(cx, |ws, _cx| {
            ws.recompute_availability();
            let r = ws.active_ref();
            assert_eq!(
                ws.lane_for(r).unwrap().availability,
                LaneAvailability::Present
            );
        });
    }

    #[gpui::test]
    fn set_lane_availability_noop_for_unknown_ref(cx: &mut TestAppContext) {
        let temp = tempfile::tempdir().unwrap();
        let (ws, _data_dir) = workspace_at(cx, temp.path());
        ws.update(cx, |ws, _cx| {
            let bogus = LaneRef {
                project: 999,
                lane: 999,
            };
            // Must not panic and must leave existing lanes untouched.
            ws.set_lane_availability(bogus, LaneAvailability::Missing);
            let r = ws.active_ref();
            assert_eq!(
                ws.lane_for(r).unwrap().availability,
                LaneAvailability::Present
            );
        });
    }
}
