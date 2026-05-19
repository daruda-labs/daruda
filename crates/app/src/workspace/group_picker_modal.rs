//! Picker modal for the Command Palette's "Move Project to Group…"
//! entry. Wraps `crate::ui::list` so the user picks an existing group
//! (or the "Ungrouped" sentinel row) instead of typing a free-text
//! group name — which previously meant a typo silently created a
//! brand-new group (C-2 review).
//!
//! `Esc` / `ListEvent::Cancel` dismisses; `Enter` / `ListEvent::Confirm`
//! routes the pick through [`Workspace::move_project_to_group`]. When
//! the workspace has no groups defined the picker still renders with
//! just the "Ungrouped" row, which is the legitimate "move out of any
//! group" path — useful even before the first group exists.

use crate::ui::theme;
use daruda_store::project::{GroupId, ProjectId};
use gpui::{
    App, Context, Entity, FocusHandle, Focusable, IntoElement, KeyDownEvent, Render, SharedString,
    Subscription, WeakEntity, Window, div, prelude::*, px,
};

use crate::ui::WindowExt as _;
use crate::ui::list::{FilteredItem, FilteredListState, ListEvent, list, searchable_list_state};
use crate::workspace::ModalView;
use crate::workspace::Workspace;

/// One picker row. `target == None` means "Ungrouped" (clear
/// `Project::group_id`); `Some(id)` means "move into this group".
#[derive(Clone)]
pub struct GroupPickItem {
    pub target: Option<GroupId>,
    pub label_text: SharedString,
}

impl FilteredItem for GroupPickItem {
    fn label(&self) -> SharedString {
        self.label_text.clone()
    }
}

pub struct GroupPickerModal {
    panel_focus_handle: FocusHandle,
    list_state: Entity<FilteredListState<GroupPickItem>>,
    workspace: WeakEntity<Workspace>,
    project_id: ProjectId,
    _list_sub: Subscription,
}

impl GroupPickerModal {
    /// Build the picker rows from a workspace snapshot. The current
    /// group membership is annotated with `(current)` so the user
    /// sees which row would be a no-op before pressing Enter. Called
    /// *before* opening the modal so the borrow doesn't tangle with
    /// the modal's constructor (G2).
    pub fn build_items(workspace: &Workspace, project_id: ProjectId) -> Vec<GroupPickItem> {
        let current = workspace
            .projects
            .iter()
            .find(|p| p.id == project_id)
            .and_then(|p| p.group_id);

        let mut items = Vec::with_capacity(workspace.groups.len() + 1);
        items.push(GroupPickItem {
            target: None,
            label_text: SharedString::from(if current.is_none() {
                "Ungrouped (current)"
            } else {
                "Ungrouped"
            }),
        });
        for g in &workspace.groups {
            let label = if current == Some(g.id) {
                format!("{} (current)", g.name)
            } else {
                g.name.clone()
            };
            items.push(GroupPickItem {
                target: Some(g.id),
                label_text: SharedString::from(label),
            });
        }
        items
    }

    pub fn new(
        workspace: WeakEntity<Workspace>,
        project_id: ProjectId,
        items: Vec<GroupPickItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let list_state = cx.new(|cx| searchable_list_state(items, window, cx));

        let _list_sub = cx.subscribe_in(
            &list_state,
            window,
            move |this, state, ev: &ListEvent, window, cx| match ev {
                ListEvent::Confirm(ix) => {
                    let target = state.read(cx).delegate().item_at(*ix).map(|i| i.target);
                    this.dispatch(target, window, cx);
                }
                ListEvent::Cancel => this.dismiss(window, cx),
                ListEvent::Select(_) => {}
            },
        );

        Self {
            panel_focus_handle: cx.focus_handle(),
            list_state,
            workspace,
            project_id,
            _list_sub,
        }
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut App) {
        window.close_dialog(cx);
    }

    /// Route the pick to `move_project_to_group` and close the dialog.
    /// Outer `None` = list returned no item (shouldn't happen — the
    /// list always has the Ungrouped row); inner `target` is the
    /// `Option<GroupId>` the workspace method expects.
    fn dispatch(
        &mut self,
        target: Option<Option<GroupId>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.close_dialog(cx);
        let Some(target) = target else {
            return;
        };
        let project_id = self.project_id;
        if let Some(ws) = self.workspace.upgrade() {
            ws.update(cx, |ws, cx| {
                ws.move_project_to_group(project_id, target, cx);
            });
        }
    }
}

impl Focusable for GroupPickerModal {
    fn focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        // Land focus inside the list's query input so keystrokes
        // filter rows immediately.
        self.list_state.focus_handle(cx)
    }
}

impl ModalView for GroupPickerModal {}

impl Render for GroupPickerModal {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .key_context("GroupPickerModal")
            .track_focus(&self.panel_focus_handle)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                if ev.keystroke.key.as_str() == "escape" {
                    this.dismiss(window, cx);
                    cx.stop_propagation();
                }
            }))
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(list(&self.list_state))
    }
}
