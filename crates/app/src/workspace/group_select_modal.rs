//! Select modal for the Command Palette's "Move Project to Group…"
//! entry. Replaces the prior `GroupPickerModal` (searchable list) with
//! a single `ui::select` dropdown — the group list is small and the
//! query box on a picker added noise without aiding discovery.
//!
//! `Esc` / Cancel dismisses; the **Move** button routes the current
//! dropdown value through [`Workspace::move_project_to_group`]. The
//! free-text path the C-2 review flagged stays closed because the
//! dropdown can only emit values present as options — typo-driven
//! group creation is structurally unreachable from this entry. To mint
//! a new group on purpose, use the `New Group` action (`Cmd+Shift+N`).

use crate::ui::theme;
use crate::ui::{
    WindowExt as _, button, button_primary,
    select::{self, SelectOption, SelectState},
};
use crate::workspace::ModalView;
use crate::workspace::Workspace;
use daruda_store::project::{GroupId, ProjectId};
use gpui::{
    App, ClickEvent, Context, Entity, FocusHandle, Focusable, IntoElement, KeyDownEvent, Render,
    SharedString, WeakEntity, Window, div, prelude::*, px,
};

/// Sentinel `SelectOption::value` for the "Ungrouped" row. The
/// dropdown's value type is `SharedString`, so we encode the "no
/// group" choice as this literal and parse remaining values as
/// `GroupId`. Picked over an empty string so log / diff output stays
/// readable when something goes wrong.
const UNGROUPED_VALUE: &str = "ungrouped";

pub struct GroupSelectModal {
    panel_focus_handle: FocusHandle,
    select_state: Entity<SelectState>,
    workspace: WeakEntity<Workspace>,
    project_id: ProjectId,
}

impl GroupSelectModal {
    /// Build the dropdown options + initial value from a workspace
    /// snapshot. The current group membership is annotated with
    /// `(current)` so the user sees which row would be a no-op before
    /// clicking Move. Called *before* opening the modal so the borrow
    /// doesn't tangle with the modal's constructor (G2).
    pub fn build_options(
        workspace: &Workspace,
        project_id: ProjectId,
    ) -> (Vec<SelectOption>, SharedString) {
        let current = workspace
            .projects
            .iter()
            .find(|p| p.id == project_id)
            .and_then(|p| p.group_id);

        let mut opts = Vec::with_capacity(workspace.groups.len() + 1);
        let ungrouped_label = if current.is_none() {
            "Ungrouped (current)"
        } else {
            "Ungrouped"
        };
        opts.push(SelectOption::new(UNGROUPED_VALUE, ungrouped_label));

        for g in &workspace.groups {
            let value = SharedString::from(g.id.to_string());
            let label = if current == Some(g.id) {
                SharedString::from(format!("{} (current)", g.name))
            } else {
                SharedString::from(g.name.clone())
            };
            opts.push(SelectOption { value, label });
        }

        let initial: SharedString = match current {
            None => SharedString::from(UNGROUPED_VALUE),
            Some(id) => SharedString::from(id.to_string()),
        };

        (opts, initial)
    }

    pub fn new(
        workspace: WeakEntity<Workspace>,
        project_id: ProjectId,
        options: Vec<SelectOption>,
        initial: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let select_state =
            cx.new(|cx| select::state_with_options(options, Some(&initial), window, cx));

        Self {
            panel_focus_handle: cx.focus_handle(),
            select_state,
            workspace,
            project_id,
        }
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut App) {
        window.close_dialog(cx);
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.select_state.read(cx).selected_value().cloned();
        let value_str: Option<&str> = value.as_ref().map(|v| v.as_ref());
        let target: Option<GroupId> = match value_str {
            None | Some(UNGROUPED_VALUE) => None,
            Some(s) => match s.parse::<GroupId>() {
                Ok(id) => Some(id),
                Err(_) => {
                    // Options come from this workspace's groups, so
                    // an unparseable value means the option set drifted
                    // mid-modal (group deleted from another path).
                    // Bail without mutating state — the next palette
                    // open rebuilds the option set.
                    window.close_dialog(cx);
                    return;
                }
            },
        };

        let project_id = self.project_id;
        window.close_dialog(cx);
        if let Some(ws) = self.workspace.upgrade() {
            ws.update(cx, |ws, cx| {
                ws.move_project_to_group(project_id, target, cx);
            });
        }
    }
}

impl Focusable for GroupSelectModal {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        // Land focus on the dropdown so Space/Enter opens it
        // immediately — saves a Tab when the user arrives via the
        // command palette or keyboard shortcut.
        self.select_state.focus_handle(cx)
    }
}

impl ModalView for GroupSelectModal {}

impl Render for GroupSelectModal {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let footer = div()
            .flex()
            .flex_row()
            .justify_end()
            .gap(px(theme::MODAL_FOOTER_GAP))
            .mt(px(theme::MODAL_FOOTER_MARGIN_TOP))
            .child(
                button("group-select-cancel", "Cancel").on_click(cx.listener(
                    |this, _: &ClickEvent, window, cx| {
                        this.dismiss(window, cx);
                    },
                )),
            )
            .child(
                button_primary("group-select-move", "Move").on_click(cx.listener(
                    |this, _: &ClickEvent, window, cx| {
                        this.submit(window, cx);
                    },
                )),
            );

        div()
            .flex()
            .flex_col()
            .key_context("GroupSelectModal")
            .track_focus(&self.panel_focus_handle)
            .tab_group()
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                if ev.keystroke.key.as_str() == "escape" {
                    this.dismiss(window, cx);
                    cx.stop_propagation();
                }
            }))
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(select::select(&self.select_state, cx, 0_isize))
            .child(footer)
    }
}
