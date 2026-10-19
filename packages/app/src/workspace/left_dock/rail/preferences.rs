use daruda_store::project::{LaneRef, ProjectId};
use gpui::{Context, Window};
use unicode_segmentation::UnicodeSegmentation as _;

use crate::surface::strings as s;
use crate::workspace::{EditActiveProjectAlias, ToggleActiveLaneRailGrouping, Workspace};

pub(super) const ALIAS_MAX_GRAPHEMES: usize = 2;

pub(super) fn valid_alias(value: &str) -> bool {
    !value.contains(['\n', '\r']) && value.trim().graphemes(true).count() <= ALIAS_MAX_GRAPHEMES
}

impl Workspace {
    pub(in crate::workspace) fn on_edit_active_project_alias(
        &mut self,
        _: &EditActiveProjectAlias,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_project_alias_editor(self.active.project, window, cx);
    }

    pub(in crate::workspace) fn open_project_alias_editor(
        &mut self,
        project: ProjectId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(model) = self.project_for(project) else {
            return;
        };
        let initial = model.display_alias.clone().unwrap_or_default();
        let workspace = cx.entity().downgrade();
        crate::workspace::dialog_helpers::open_form_modal(
            s::project::alias_title(),
            None,
            move |window, cx| {
                super::alias_modal::AliasModal::new(workspace, project, initial, window, cx)
            },
            window,
            cx,
        );
    }

    pub(in crate::workspace) fn set_project_display_alias(
        &mut self,
        project: ProjectId,
        value: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let value = value
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        if value.as_deref().is_some_and(|value| !valid_alias(value)) {
            return;
        }
        let Some(model) = self.project_for_mut(project) else {
            return;
        };
        if model.display_alias == value {
            return;
        }
        model.display_alias = value;
        self.mutate_durable(cx, |_, _| {});
    }

    pub(in crate::workspace) fn set_lane_rail_grouped(
        &mut self,
        target: LaneRef,
        grouped: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.project_for_mut(target.project) else {
            return;
        };
        if project.lane(target.lane).is_none() {
            return;
        }
        let changed = if grouped {
            project.rail_grouped_lanes.insert(target.lane)
        } else {
            project.rail_grouped_lanes.remove(&target.lane)
        };
        if changed {
            self.mutate_durable(cx, |_, _| {});
        }
    }

    pub(in crate::workspace) fn on_toggle_active_lane_rail_grouping(
        &mut self,
        _: &ToggleActiveLaneRailGrouping,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let grouped = self
            .active_project()
            .is_some_and(|project| project.rail_grouped_lanes.contains(&self.active.lane));
        self.set_lane_rail_grouped(self.active, !grouped, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::valid_alias;

    #[test]
    fn aliases_count_graphemes_and_allow_the_default() {
        for value in ["", "  ", "DA", "개발", "👩‍💻A", "한글"] {
            assert!(valid_alias(value), "{value:?}");
        }
        for value in ["abc", "개발팀", "D\nA", "D\rA"] {
            assert!(!valid_alias(value), "{value:?}");
        }
    }
}
