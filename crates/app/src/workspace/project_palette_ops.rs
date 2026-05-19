//! Action handlers for the Command Palette's multi-project entries —
//! `NewGroup`, `RenameActiveProject`, and `MoveActiveProjectToGroup`.
//!
//! `NewGroup` and `RenameActiveProject` route through
//! [`open_single_field_dialog`] and delegate to
//! [`Workspace::add_group`] / [`Workspace::rename_active_project`].
//!
//! `MoveActiveProjectToGroup` opens [`GroupSelectModal`] — a dropdown
//! over every existing group plus an "Ungrouped" row. Free-text input
//! was the original shape but allowed a typo to silently mint a fresh
//! group (C-2 review); a follow-up searchable picker was tried but the
//! query input added noise without aiding discovery, so the dropdown
//! is the current resting shape. The dropdown constrains the pick to
//! existing rows so accidental group proliferation is no longer
//! reachable from this action. Use `NewGroup` (`Cmd+Shift+N`) to
//! create groups deliberately.

use gpui::{Context, Window};

use super::{MoveActiveProjectToGroup, NewGroup, RenameActiveProject, Workspace};
use crate::workspace::dialog_helpers::{open_form_modal, open_single_field_dialog};
use crate::workspace::group_select_modal::GroupSelectModal;

impl Workspace {
    pub(in crate::workspace) fn on_new_group(
        &mut self,
        _: &NewGroup,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.entity().downgrade();
        open_single_field_dialog(
            weak,
            "New Group",
            "Group name",
            None,
            |ws, value, _window, cx| {
                let Some(name) = value else {
                    return;
                };
                ws.add_group(name, None, cx);
            },
            window,
            cx,
        );
    }

    pub(in crate::workspace) fn on_rename_active_project(
        &mut self,
        _: &RenameActiveProject,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(initial) = self.active_project_name() else {
            return;
        };
        let weak = cx.entity().downgrade();
        open_single_field_dialog(
            weak,
            "Rename Project",
            "Project name",
            Some(&initial),
            |ws, value, _window, cx| {
                let Some(name) = value else {
                    return;
                };
                ws.rename_active_project(name, cx);
            },
            window,
            cx,
        );
    }

    pub(in crate::workspace) fn on_move_active_project_to_group(
        &mut self,
        _: &MoveActiveProjectToGroup,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project_id) = self.active_project().map(|p| p.id) else {
            return;
        };
        // Build the dropdown options + initial value now, while the
        // workspace borrow is in scope — passing them into the modal
        // constructor avoids a re-entrant read from inside the entity
        // (G2 / pitfall §4).
        let (options, initial) = GroupSelectModal::build_options(self, project_id);
        let weak = cx.entity().downgrade();
        open_form_modal(
            "Move Project to Group",
            None,
            move |window, cx| GroupSelectModal::new(weak, project_id, options, initial, window, cx),
            window,
            cx,
        );
    }
}

// State mutations dispatched by these handlers are covered by the
// existing `workspace::tests::projects` and `workspace::tests::dnd`
// suites against the underlying ops methods. The palette handlers
// themselves are pure plumbing (open_single_field_dialog +
// delegation), so no separate inline tests are needed here.
