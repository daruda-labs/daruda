//! Mutually exclusive worktree/session choices and the left-dock popup lifecycle.

mod entries;
mod render;
#[cfg(feature = "screenshot")]
mod screenshot;
mod selection;
#[cfg(test)]
mod tests;

use crate::workspace::{Workspace, layout::DockSnapshot, main_area::pane_tree::PaneId};
use daruda_agent::SessionStatus;
use daruda_store::project::{LaneRef, ProjectId};
use entries::{Entry, Target};
use gpui::{
    Bounds, Context, FocusHandle, KeyDownEvent, MouseDownEvent, Pixels, SharedString, Window,
};

pub(in crate::workspace) use render::{chooser_overlay, named_rows};

#[derive(Clone, PartialEq)]
pub(in crate::workspace) struct SessionRow {
    pub id: String,
    pub pane: PaneId,
    pub title: SharedString,
    pub status: SessionStatus,
}

pub(in crate::workspace) struct Chooser {
    target: Target,
    pub trigger_bounds: Bounds<Pixels>,
    pub focus: FocusHandle,
    pub previous_focus: Option<FocusHandle>,
    pub scroll: gpui::ScrollHandle,
    viewport: gpui::Size<Pixels>,
    origin: ChooserOrigin,
}

#[derive(PartialEq)]
struct ChooserOrigin {
    active: LaneRef,
    page: Option<crate::workspace::pages::Page>,
    view: daruda_store::project::LeftDockView,
    expanded: bool,
}

impl Workspace {
    fn session_chooser_origin(&self, cx: &gpui::App) -> ChooserOrigin {
        ChooserOrigin {
            active: self.active,
            page: self.active_page(),
            view: self.docks.left_view,
            expanded: self.docks.left.read(cx).is_open,
        }
    }

    pub(in crate::workspace) fn choose_lane_from_rail(
        &mut self,
        lane: LaneRef,
        trigger_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let rows = self.chooser_rows(lane, cx);
        if rows.len() < 2 {
            self.close_session_chooser(false, window, cx);
            let was_active = self.active == lane;
            self.activate_lane(lane, window, cx);
            if was_active && self.has_focused_pane() {
                self.focus_pane(self.active_runtime().focused_pane_id, window, cx);
            }
            return;
        }
        let selected = rows
            .iter()
            .find(|r| lane == self.active && r.pane == self.active_runtime().focused_pane_id)
            .unwrap_or(&rows[0])
            .pane;
        self.open_rail_chooser(
            Target::Sessions { lane, selected },
            trigger_bounds,
            window,
            cx,
        );
    }

    pub(in crate::workspace) fn choose_project_from_rail(
        &mut self,
        project: ProjectId,
        trigger_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(model) = self.project_for(project) else {
            return;
        };
        let selected = if self.active.project == project {
            self.active.lane
        } else {
            model.last_active_lane_id
        };
        self.open_rail_chooser(
            Target::Worktrees {
                project,
                selected: Some(selected),
            },
            trigger_bounds,
            window,
            cx,
        );
    }

    fn open_rail_chooser(
        &mut self,
        target: Target,
        trigger_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .docks
            .session_chooser
            .as_ref()
            .is_some_and(|c| c.target.same_list(target))
        {
            self.close_session_chooser(true, window, cx);
            return;
        }
        let previous_focus = self
            .docks
            .session_chooser
            .take()
            .and_then(|c| c.previous_focus)
            .or_else(|| window.focused(cx));
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        self.docks.session_chooser = Some(Chooser {
            target,
            trigger_bounds,
            focus,
            previous_focus,
            scroll: gpui::ScrollHandle::new(),
            viewport: window.viewport_size(),
            origin: self.session_chooser_origin(cx),
        });
        cx.notify();
    }

    fn activate_rail_choice(&mut self, entry: &Entry, window: &mut Window, cx: &mut Context<Self>) {
        match entry {
            Entry::Session { lane, row } => self.activate_dock_session(*lane, row, window, cx),
            Entry::Worktree { lane, .. } => {
                let Some(bounds) = self
                    .docks
                    .session_chooser
                    .as_ref()
                    .map(|c| c.trigger_bounds)
                else {
                    return;
                };
                if self.lane_for(*lane).is_some() {
                    self.choose_lane_from_rail(*lane, bounds, window, cx);
                } else {
                    self.close_session_chooser(true, window, cx);
                }
            }
        }
    }

    pub(in crate::workspace) fn chooser_rows(
        &self,
        lane: LaneRef,
        cx: &gpui::App,
    ) -> Vec<SessionRow> {
        let dock = self.docks.left.read(cx);
        match &dock.snap {
            DockSnapshot::Left(snap) => snap.named_sessions.get(&lane).cloned().unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    pub(in crate::workspace) fn on_session_chooser_mouse_down_out(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Outside presses run during capture, before the trigger's click.
        // Keep its open state intact so that click can toggle it closed.
        if event.button == gpui::MouseButton::Left
            && self
                .docks
                .session_chooser
                .as_ref()
                .is_some_and(|chooser| chooser.trigger_bounds.contains(&event.position))
        {
            return;
        }
        self.close_session_chooser(false, window, cx);
    }

    pub(in crate::workspace) fn close_session_chooser(
        &mut self,
        restore: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(chooser) = self.docks.session_chooser.take() {
            if (restore || chooser.focus.is_focused(window))
                && let Some(focus) = chooser.previous_focus
            {
                focus.focus(window, cx);
            }
            cx.notify();
        }
    }

    pub(in crate::workspace) fn reconcile_session_chooser(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings_view().is_some() {
            self.close_session_chooser(false, window, cx);
            return;
        }
        let Some(chooser) = &self.docks.session_chooser else {
            return;
        };
        let rows = self.rail_chooser_entries(chooser.target, cx);
        let valid = match chooser.target {
            Target::Sessions { lane, .. } => self.lane_for(lane).is_some() && !rows.is_empty(),
            Target::Worktrees { project, .. } => self.project_for(project).is_some(),
        };
        if chooser.origin != self.session_chooser_origin(cx)
            || !valid
            || chooser.viewport != window.viewport_size()
        {
            self.close_session_chooser(true, window, cx);
        } else if !rows.iter().any(|r| chooser.target.is_selected(r))
            && let Some(first) = rows.first()
        {
            self.docks
                .session_chooser
                .as_mut()
                .unwrap()
                .target
                .select(first);
        }
    }

    pub(in crate::workspace) fn on_session_chooser_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(chooser) = &self.docks.session_chooser else {
            return;
        };
        let Some((key, _)) = crate::workspace::command::picker_key::picker_keystroke(event) else {
            return;
        };
        let rows = self.rail_chooser_entries(chooser.target, cx);
        match key {
            "escape" => self.close_session_chooser(true, window, cx),
            "enter" => {
                if let Some(row) = rows.iter().find(|r| chooser.target.is_selected(r)).cloned() {
                    self.activate_rail_choice(&row, window, cx);
                }
            }
            "up" | "down" if !rows.is_empty() => {
                let current = rows.iter().position(|r| chooser.target.is_selected(r));
                let delta = if key == "up" { -1 } else { 1 };
                let index = super::wrap_step(current, delta, rows.len());
                let chooser = self.docks.session_chooser.as_mut().unwrap();
                chooser.target.select(&rows[index]);
                chooser.scroll.scroll_to_item(index);
                cx.notify();
            }
            _ => return,
        }
        cx.stop_propagation();
    }
}
