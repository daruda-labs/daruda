//! Transient native fixtures for the collapsed rail and its session chooser.

use crate::workspace::{Workspace, layout::DockSnapshot};
use gpui::{Context, Window, point, px};

impl Workspace {
    pub(in crate::workspace) fn seed_left_dock_groups_for_shot(
        &mut self,
        worktrees: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.seed_left_dock_for_shot(false, false, false, window, cx);
        let development = self.add_group("Development".into(), None, cx);
        let open_source = self.add_group("Open source".into(), None, cx);
        self.move_project_to_group(self.active.project, Some(development), cx);
        if let Some(project) = self.active_project_mut() {
            let mut empty = crate::lane::Lane::default_for_project(77, project.root.clone());
            empty.name = Some("playground".into());
            empty.tab_order = 1;
            project.lanes.push(empty);
        }
        for (name, group) in [
            ("Vibe", development),
            ("UI Kit", open_source),
            ("Orbit", open_source),
        ] {
            let mut project = crate::project::Project::bootstrap_placeholder(
                self.next_project_id,
                self.data_dir.clone(),
            );
            self.next_project_id += 1;
            project.name = name.into();
            project.group_id = Some(group);
            project.tab_order = self.projects.len() as u32;
            self.projects.push(project);
        }
        let snap = self.prepare_left_dock_snapshot(cx);
        self.docks.left.update(cx, |dock, cx| {
            dock.stage(DockSnapshot::Left(Box::new(snap)));
            cx.notify();
        });
        if worktrees {
            self.choose_project_from_rail(
                self.active.project,
                gpui::Bounds::new(
                    point(
                        px(0.0),
                        px(crate::ui::theme::TITLE_BAR_HEIGHT
                            + crate::ui::theme::PAD_XS * 2.0
                            + crate::ui::theme::CONTROL_TARGET_SIZE * 5.0
                            + crate::ui::theme::GAP_LG
                            + crate::ui::theme::CONTROL_TARGET_SIZE),
                    ),
                    gpui::size(
                        px(crate::ui::theme::DOCK_LEFT_RAIL_W),
                        px(crate::ui::theme::DOCK_RAIL_ROW_H),
                    ),
                ),
                window,
                cx,
            );
        }
        cx.notify();
    }

    pub(in crate::workspace) fn seed_left_dock_for_shot(
        &mut self,
        chooser: bool,
        expanded: bool,
        six_sessions: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ensure_project_for_shot();
        if let Some(project) = self.project_for_mut(self.active.project) {
            project.name = crate::surface::constants::APP_NAME.into();
        }
        if let Some(lane) = self.lane_for_mut(self.active) {
            lane.name = Some("sidebar-study".into());
        }
        if self.active_runtime().tabs.is_empty() {
            self.add_tab(window, cx);
        }
        self.seed_tab_indicators_for_shot(window, cx);
        if six_sessions {
            self.seed_tab_indicators_for_shot(window, cx);
        }
        self.docks.left.update(cx, |d, _| d.is_open = expanded);
        self.docks.left_view = daruda_store::project::LeftDockView::Lanes;
        self.main_area.pending_resize = true;
        let snap = self.prepare_left_dock_snapshot(cx);
        self.docks.left.update(cx, |d, cx| {
            d.stage(DockSnapshot::Left(Box::new(snap)));
            cx.notify();
        });
        if chooser {
            self.choose_lane_from_rail(
                self.active,
                gpui::Bounds::new(
                    point(
                        px(0.0),
                        px(crate::ui::theme::TITLE_BAR_HEIGHT + crate::ui::theme::DOCK_RAIL_ROW_H),
                    ),
                    gpui::size(
                        px(crate::ui::theme::DOCK_LEFT_RAIL_W),
                        px(crate::ui::theme::DOCK_RAIL_ROW_H),
                    ),
                ),
                window,
                cx,
            );
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn scenario_names_are_reachable() {
        use crate::workspace::screenshot_scenario::ScreenshotScenario;
        for name in [
            "left-dock-groups",
            "left-dock-worktrees",
            "left-dock-sessions",
            "left-dock-rail",
            "left-dock-expanded",
            "left-dock-sessions-six",
        ] {
            assert!(ScreenshotScenario::from_cli_name(name).is_some());
        }
    }
}
