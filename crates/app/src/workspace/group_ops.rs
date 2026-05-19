//! Group CRUD on [`Workspace`]. Groups carry only visual metadata —
//! a name, an optional color, a collapsed flag, and a `tab_order`
//! that shares a pool with ungrouped projects so DnD can interleave
//! them in the left dock without separate sort buckets.
//!
//! Storage already lives in [`Workspace::groups`] as
//! `Vec<daruda_store::project::SerializedGroup>`. Promoting that to a
//! distinct runtime type would buy nothing today; the persisted shape
//! has every field the UI needs. CRUD here mutates it in place and
//! routes through `mark_dirty_and_save` so changes round-trip.
//!
//! Production wiring:
//! - `add_group` / `move_project_to_group` — Command Palette
//!   (`project_palette_ops::on_new_group`,
//!   `on_move_active_project_to_group`).
//! - `toggle_group_collapse` — left-dock group accordion header
//!   (`worktrees/rows.rs`).
//! - `rename_group` / `recolor_group` / `delete_group` — reserved for
//!   the left-dock group context menu; currently exercised only by
//!   `workspace::tests::projects`. The function-scoped
//!   `#[allow(dead_code)]` will fall off as each menu entry lands.

use daruda_store::project::{GroupId, ProjectId, SerializedGroup};
use gpui::Context;

use super::Workspace;

impl Workspace {
    /// Append a new group with a fresh monotonic id and place it at
    /// the end of the shared (group, ungrouped-project) tab-order
    /// pool. Returns the new id so callers (modal, palette) can focus
    /// or rename the entry immediately.
    pub(crate) fn add_group(
        &mut self,
        name: String,
        color: Option<String>,
        cx: &mut Context<Self>,
    ) -> GroupId {
        let id = self.next_group_id;
        // Saturating add protects against the (astronomically
        // unlikely) wraparound. Reuse is forbidden by spec —
        // saturation keeps the counter monotonic even at the edge.
        self.next_group_id = self.next_group_id.saturating_add(1);
        let tab_order = self.next_top_row_tab_order();
        self.groups.push(SerializedGroup {
            id,
            name,
            color,
            tab_order,
            is_collapsed: false,
        });
        self.mark_dirty_and_save(cx);
        id
    }

    /// Next free `tab_order` value across the shared pool (groups +
    /// ungrouped projects). Returns `max + 1`, or `0` when the pool
    /// is empty.
    fn next_top_row_tab_order(&self) -> u32 {
        let from_groups = self.groups.iter().map(|g| g.tab_order);
        let from_ungrouped = self
            .projects
            .iter()
            .filter(|p| p.group_id.is_none())
            .map(|p| p.tab_order);
        from_groups
            .chain(from_ungrouped)
            .max()
            .map(|m| m.saturating_add(1))
            .unwrap_or(0)
    }

    /// Rename the group with `group_id`. No-op when the id is unknown
    /// or the name is unchanged.
    #[allow(dead_code)] // wired by the left-dock group context menu
    pub(crate) fn rename_group(
        &mut self,
        group_id: GroupId,
        name: String,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(group) = self.groups.iter_mut().find(|g| g.id == group_id) else {
            return false;
        };
        if group.name == name {
            return true;
        }
        group.name = name;
        self.mark_dirty_and_save(cx);
        true
    }

    /// Set or clear the group's accent color. `None` removes the
    /// override (left-dock falls back to the default chip color).
    #[allow(dead_code)] // wired by the left-dock group context menu
    pub(crate) fn recolor_group(
        &mut self,
        group_id: GroupId,
        color: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(group) = self.groups.iter_mut().find(|g| g.id == group_id) else {
            return;
        };
        if group.color == color {
            return;
        }
        group.color = color;
        self.mark_dirty_and_save(cx);
    }

    /// Toggle the group's collapsed flag in the left dock.
    pub(crate) fn toggle_group_collapse(&mut self, group_id: GroupId, cx: &mut Context<Self>) {
        let Some(group) = self.groups.iter_mut().find(|g| g.id == group_id) else {
            return;
        };
        group.is_collapsed = !group.is_collapsed;
        self.mark_dirty_and_save(cx);
    }

    /// Remove a group. Member projects are demoted to ungrouped
    /// (`group_id = None`) so they keep their place in the shared
    /// tab-order pool. No data loss — only the visual grouping
    /// disappears.
    #[allow(dead_code)] // wired by the left-dock group context menu
    pub(crate) fn delete_group(&mut self, group_id: GroupId, cx: &mut Context<Self>) {
        let existed = self.groups.iter().any(|g| g.id == group_id);
        if !existed {
            return;
        }
        for project in self.projects.iter_mut() {
            if project.group_id == Some(group_id) {
                project.group_id = None;
            }
        }
        self.groups.retain(|g| g.id != group_id);
        self.mark_dirty_and_save(cx);
    }

    /// Move a project into (or out of) a group. Passing
    /// `target = None` makes the project ungrouped. Unknown
    /// `project_id` / `group_id` silently no-op so a stale UI click
    /// (e.g. a context menu fired after the group was deleted) cannot
    /// crash the workspace.
    pub(crate) fn move_project_to_group(
        &mut self,
        project_id: ProjectId,
        target: Option<GroupId>,
        cx: &mut Context<Self>,
    ) {
        if let Some(gid) = target
            && !self.groups.iter().any(|g| g.id == gid)
        {
            return;
        }
        let Some(project) = self.projects.iter_mut().find(|p| p.id == project_id) else {
            return;
        };
        if project.group_id == target {
            return;
        }
        project.group_id = target;
        self.mark_dirty_and_save(cx);
    }
}
