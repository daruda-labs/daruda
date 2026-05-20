//! Project / group reordering driven by the left-dock DnD pipeline.
//!
//! Worktrees stay inside their parent project (see
//! [`super::Workspace::reorder_worktree`]); this module handles the
//! other two payload kinds from the multi-project plan:
//!
//! - Projects move freely between groups and the top-level pool. The
//!   target's group membership is inherited so dropping a project onto
//!   another project re-parents it to that project's group (or demotes
//!   it to ungrouped).
//! - Groups only live at the top level, alongside ungrouped projects,
//!   sharing one `tab_order` pool.
//!
//! Every op resolves the destination pool, performs an in-place move,
//! and renumbers `tab_order` as a contiguous `0..N`. Renumbering on
//! every drop keeps the persisted state free of stale gaps and removes
//! ties between the shared top-level pool and per-group pools.

use daruda_store::project::{GroupId, ProjectId};
use gpui::Context;

use super::Workspace;

/// Top-level row identity. Groups and ungrouped projects share a
/// single `tab_order` pool, so DnD ops that touch the top level need
/// to address either kind interchangeably.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum TopRow {
    Group(GroupId),
    Project(ProjectId),
}

impl Workspace {
    /// Move project `from` onto the slot occupied by `to`, inheriting
    /// `to`'s `group_id`. Dropping onto a grouped project re-parents
    /// `from` into that group; dropping onto an ungrouped project
    /// demotes `from` to the top-level pool. No-ops when either id is
    /// unknown or the move is a self-drop.
    ///
    /// Direction-aware within the same pool: dragging downward (from
    /// precedes to) lands `from` AFTER `to`, dragging upward lands
    /// `from` BEFORE `to`. Cross-pool drops always land at `to`'s slot
    /// (before `to`). This matches the typical list-DnD expectation
    /// that "drop X onto Y's slot" makes X take Y's row regardless of
    /// drag direction. Implementation trick: inserting `from` at
    /// `to`'s ORIGINAL index gives both behaviors uniformly — when
    /// `from` precedes `to` in the same pool, the retain shifts `to`
    /// down one slot, so inserting at `to`'s original index lands
    /// after the now-shifted `to`; in every other case, `to` keeps its
    /// index and the insert lands before it.
    pub(in crate::workspace) fn reorder_project_before(
        &mut self,
        from: ProjectId,
        to: ProjectId,
        cx: &mut Context<Self>,
    ) {
        if from == to {
            return;
        }
        let Some(to_group) = self
            .projects
            .iter()
            .find(|p| p.id == to)
            .map(|p| p.group_id)
        else {
            return;
        };
        let Some(from_group_old) = self
            .projects
            .iter()
            .find(|p| p.id == from)
            .map(|p| p.group_id)
        else {
            return;
        };

        // Stage the destination pool's new ordering before touching any
        // project record. `retain` is a no-op when `from` lives in a
        // different pool (cross-pool move), so the same pipeline serves
        // both same-group and cross-group cases.
        match to_group {
            Some(gid) => {
                let original = self.group_member_order(gid);
                let Some(to_pos) = original.iter().position(|id| *id == to) else {
                    unreachable!("`to` belongs to group `gid`");
                };
                let mut new = original.clone();
                new.retain(|id| *id != from);
                new.insert(to_pos, from);
                if from_group_old == to_group && new == original {
                    return;
                }
                if let Some(p) = self.projects.iter_mut().find(|p| p.id == from) {
                    p.group_id = to_group;
                }
                self.write_group_member_order(&new);
            }
            None => {
                let original = self.top_row_order();
                let Some(to_pos) = original
                    .iter()
                    .position(|row| matches!(row, TopRow::Project(id) if *id == to))
                else {
                    unreachable!(
                        "`to` is ungrouped (to_group is None) so it must appear in the top-row pool"
                    );
                };
                let mut new = original.clone();
                new.retain(|row| !matches!(row, TopRow::Project(id) if *id == from));
                new.insert(to_pos, TopRow::Project(from));
                if from_group_old == to_group && new == original {
                    return;
                }
                if let Some(p) = self.projects.iter_mut().find(|p| p.id == from) {
                    p.group_id = None;
                }
                self.write_top_row_order(&new);
            }
        }

        // When `from` left its previous pool, that pool's tab_order is
        // now sparse; renumber so the persisted ordering stays
        // 0..N-contiguous and matches the snapshot's sort.
        if from_group_old != to_group {
            self.renumber_pool(from_group_old);
        }

        // Empty closure: see group_ops.rs:83 for the rationale (multi-stage
        // mutation already complete, intermediate borrows cannot cross the
        // closure boundary).
        self.mutate_durable(cx, |_, _| {});
        cx.notify();
    }

    /// Append project `from` to the end of `target` group. No-op when
    /// the project or group is unknown, or when `from` already lives in
    /// `target` (drop on the group header it already belongs to is a
    /// silent self-target).
    pub(in crate::workspace) fn move_project_to_group_end(
        &mut self,
        from: ProjectId,
        target: GroupId,
        cx: &mut Context<Self>,
    ) {
        if !self.groups.iter().any(|g| g.id == target) {
            return;
        }
        let Some(from_group_old) = self
            .projects
            .iter()
            .find(|p| p.id == from)
            .map(|p| p.group_id)
        else {
            return;
        };
        if from_group_old == Some(target) {
            return;
        }

        if let Some(p) = self.projects.iter_mut().find(|p| p.id == from) {
            p.group_id = Some(target);
        }

        let mut pool = self.group_member_order(target);
        pool.retain(|id| *id != from);
        pool.push(from);
        self.write_group_member_order(&pool);

        self.renumber_pool(from_group_old);

        self.mutate_durable(cx, |_, _| {});
        cx.notify();
    }

    /// Move group `from` onto the slot occupied by `before` in the
    /// shared top-level pool. `before` may be another group or an
    /// ungrouped project. No-op when `from == before`, when `before`
    /// is a grouped project (groups never sit inside other groups), or
    /// when either id is unknown.
    ///
    /// Direction-aware (see [`Self::reorder_project_before`] for the
    /// full rationale): dragging downward lands `from` AFTER `before`,
    /// dragging upward lands `from` BEFORE `before`. Inserting at
    /// `before`'s ORIGINAL index gives both behaviors uniformly — the
    /// retain only shifts `before` down when `from` precedes it, so
    /// inserting at the captured original index lands either after the
    /// shifted `before` (down) or at the unchanged slot of `before`
    /// (up).
    pub(in crate::workspace) fn reorder_group_before_top_row(
        &mut self,
        from: GroupId,
        before: TopRow,
        cx: &mut Context<Self>,
    ) {
        if matches!(before, TopRow::Group(id) if id == from) {
            return;
        }
        if !self.groups.iter().any(|g| g.id == from) {
            return;
        }
        if !self.top_row_exists(before) {
            return;
        }

        let original = self.top_row_order();
        let Some(to_pos) = original.iter().position(|row| *row == before) else {
            unreachable!("`before` was verified in the top-row pool above");
        };
        let mut new = original.clone();
        new.retain(|row| !matches!(row, TopRow::Group(id) if *id == from));
        new.insert(to_pos, TopRow::Group(from));
        if new == original {
            return;
        }
        self.write_top_row_order(&new);

        self.mutate_durable(cx, |_, _| {});
        cx.notify();
    }

    /// Ordered top-level pool: groups + ungrouped projects sorted by
    /// `tab_order`. The list does not stay live — callers mutate the
    /// returned `Vec` and write it back via `write_top_row_order`.
    fn top_row_order(&self) -> Vec<TopRow> {
        let mut rows: Vec<(TopRow, u32)> = self
            .groups
            .iter()
            .map(|g| (TopRow::Group(g.id), g.tab_order))
            .chain(
                self.projects
                    .iter()
                    .filter(|p| p.group_id.is_none())
                    .map(|p| (TopRow::Project(p.id), p.tab_order)),
            )
            .collect();
        rows.sort_by_key(|r| r.1);
        rows.into_iter().map(|r| r.0).collect()
    }

    /// Whether `row` corresponds to a live group or an ungrouped project.
    fn top_row_exists(&self, row: TopRow) -> bool {
        match row {
            TopRow::Group(id) => self.groups.iter().any(|g| g.id == id),
            TopRow::Project(id) => self
                .projects
                .iter()
                .any(|p| p.id == id && p.group_id.is_none()),
        }
    }

    /// Write `rows` back as 0..N `tab_order` values on the matching
    /// group / project records.
    fn write_top_row_order(&mut self, rows: &[TopRow]) {
        for (i, row) in rows.iter().enumerate() {
            let order = i as u32;
            match row {
                TopRow::Group(id) => {
                    if let Some(g) = self.groups.iter_mut().find(|g| g.id == *id) {
                        g.tab_order = order;
                    }
                }
                TopRow::Project(id) => {
                    if let Some(p) = self.projects.iter_mut().find(|p| p.id == *id) {
                        p.tab_order = order;
                    }
                }
            }
        }
    }

    /// Ordered member projects of `group`, sorted by `tab_order`.
    fn group_member_order(&self, group: GroupId) -> Vec<ProjectId> {
        let mut entries: Vec<(ProjectId, u32)> = self
            .projects
            .iter()
            .filter(|p| p.group_id == Some(group))
            .map(|p| (p.id, p.tab_order))
            .collect();
        entries.sort_by_key(|e| e.1);
        entries.into_iter().map(|e| e.0).collect()
    }

    /// Write `ids` back as 0..N `tab_order` values on the matching
    /// projects. Callers stage the full ordering of one group's members
    /// via [`Self::group_member_order`] before calling this; projects
    /// not listed in `ids` keep their existing `tab_order`.
    fn write_group_member_order(&mut self, ids: &[ProjectId]) {
        for (i, id) in ids.iter().enumerate() {
            if let Some(p) = self.projects.iter_mut().find(|p| p.id == *id) {
                p.tab_order = i as u32;
            }
        }
    }

    /// Renumber the pool identified by `group_id` (`None` = top level).
    /// Used after a project leaves its previous pool so the remaining
    /// members collapse back to a contiguous `0..N`.
    fn renumber_pool(&mut self, group_id: Option<GroupId>) {
        match group_id {
            Some(gid) => {
                let pool = self.group_member_order(gid);
                self.write_group_member_order(&pool);
            }
            None => {
                let pool = self.top_row_order();
                self.write_top_row_order(&pool);
            }
        }
    }
}
