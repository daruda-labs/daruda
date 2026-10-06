//! Browsing dispatches bind a worktree before any dialog or editor is opened.

use std::path::Path;

use daruda_store::project::LaneRef;
use gpui::{App, Context, Window};

use super::{FlowBrowserSnapshot, FlowGrouping, FlowScope, FlowTab, FlowTarget, RunFilter};
use crate::surface::strings as s;
use crate::workspace::{
    Workspace, flow_paths::FlowOrigin, layout::diff_policy::Handle, pages::Page,
};

impl Workspace {
    pub(in crate::workspace) fn stage_flow_page(&mut self, cx: &Context<Self>) {
        self.flow_browser.page_snapshot =
            (self.active_page() == Some(Page::Flows)).then(|| super::FlowPageSnapshot {
                workspace: cx.weak_entity(),
                flows: self.flow_rows_matching(|lane| lane == self.flow_browser_lane()),
                flow_lane: self.flow_browser_lane(),
                flow_history: self.flow_history_for_panel(),
                flow_files: self.flow_list_for_panel(),
                flow_browser: self.flow_browser_snapshot(cx),
                flows_with_unsaved_edits: self.flows_with_unsaved_edits(cx),
            });
    }

    pub(in crate::workspace) fn flow_browser_lane(&self) -> LaneRef {
        self.flow_browser.state.scope.resolve(self.active)
    }

    pub(in crate::workspace) fn flow_browser_snapshot(&self, cx: &App) -> FlowBrowserSnapshot {
        let search = &self.flow_browser.searches[self.flow_browser.state.tab.index()];
        FlowBrowserSnapshot {
            state: self.flow_browser.state.clone(),
            targets: self
                .projects
                .iter()
                .flat_map(|project| {
                    project.lanes.iter().map(|lane| {
                        let target = LaneRef {
                            project: project.id,
                            lane: lane.id,
                        };
                        FlowTarget {
                            lane: target,
                            project: project.name.clone(),
                            label: self.lane_label_for(target),
                            current: target == self.active,
                        }
                    })
                })
                .collect(),
            search: Handle(search.clone()),
            query: search.read(cx).value().to_string(),
            modified: self
                .flow_list
                .get(self.flow_browser_lane())
                .map(|listing| listing.modified.clone())
                .unwrap_or_default(),
        }
    }

    pub(in crate::workspace) fn set_flow_scope(
        &mut self,
        scope: FlowScope,
        cx: &mut Context<Self>,
    ) {
        if self.lane_for(scope.resolve(self.active)).is_none() {
            return;
        }
        self.flow_browser.state.scope = scope;
        self.invalidate_flow_list();
        self.invalidate_flow_history(self.flow_browser_lane());
        self.respawn_flow_watcher(cx);
        self.reset_flow_scroll();
        cx.notify();
    }

    pub(in crate::workspace) fn set_flow_tab(&mut self, tab: FlowTab, cx: &mut Context<Self>) {
        self.flow_browser.state.tab = tab;
        if let Some(page) = self
            .workspace_page
            .as_mut()
            .filter(|p| p.page == Page::Flows)
        {
            page.scroll = self.flow_browser.scrolls[tab.index()].clone();
        }
        cx.notify();
    }

    pub(in crate::workspace) fn set_flow_origin(
        &mut self,
        origin: Option<FlowOrigin>,
        cx: &mut Context<Self>,
    ) {
        self.flow_browser.state.origin = origin;
        self.reset_flow_scroll();
        cx.notify();
    }

    pub(in crate::workspace) fn set_flow_run_filter(
        &mut self,
        filter: RunFilter,
        cx: &mut Context<Self>,
    ) {
        self.flow_browser.state.run_filter = filter;
        self.reset_flow_scroll();
        cx.notify();
    }

    pub(in crate::workspace) fn set_flow_grouping(
        &mut self,
        grouping: FlowGrouping,
        cx: &mut Context<Self>,
    ) {
        self.flow_browser.state.grouping = grouping;
        cx.notify();
    }

    pub(in crate::workspace) fn toggle_flow_group(
        &mut self,
        origin: FlowOrigin,
        cx: &mut Context<Self>,
    ) {
        self.flow_browser.state.toggle_group(origin);
        cx.notify();
    }

    fn reset_flow_scroll(&self) {
        self.flow_browser.scrolls[self.flow_browser.state.tab.index()]
            .set_offset(gpui::point(gpui::Pixels::ZERO, gpui::Pixels::ZERO));
    }

    pub(in crate::workspace) fn clear_flow_search(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.flow_browser.searches[self.flow_browser.state.tab.index()].update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.reset_flow_scroll();
        cx.notify();
    }

    pub(in crate::workspace) fn clear_flow_filters(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.flow_browser.state.tab {
            FlowTab::Definitions => self.flow_browser.state.origin = None,
            FlowTab::Runs => self.flow_browser.state.run_filter = RunFilter::All,
        }
        self.clear_flow_search(window, cx);
    }

    pub(in crate::workspace) fn show_flow_questions(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_flow_tab(FlowTab::Runs, cx);
        self.set_flow_run_filter(RunFilter::Asking, cx);
        self.clear_flow_search(window, cx);
    }

    pub(in crate::workspace) fn prompt_new_flow(
        &mut self,
        lane: LaneRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.lane_for(lane).is_none() {
            return;
        }
        self.create_flow_in(lane, &s::flow::untitled(), window, cx);
    }

    /// Opening an editor explicitly enters its worktree; changing scope never does.
    pub(in crate::workspace) fn open_browsed_flow(
        &mut self,
        lane: LaneRef,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.lane_for(lane).is_none() {
            return;
        }
        if self.active != lane {
            self.activate_lane(lane, window, cx);
        }
        self.open_flow_graph(path, window, cx);
    }

    pub(in crate::workspace) fn open_browsed_report(
        &mut self,
        lane: LaneRef,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.lane_for(lane).is_none() {
            return;
        }
        if self.active != lane {
            self.activate_lane(lane, window, cx);
        }
        self.open_flow_report(path, window, cx);
    }

    pub(in crate::workspace) fn edit_browsed_flow_name(
        &mut self,
        lane: LaneRef,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.lane_for(lane).is_none() {
            return;
        }
        self.open_browsed_flow(lane, path, window, cx);
        if let Some(id) = self.find_flow_graph_pane(path)
            && let Some((_, view)) = self.flow_graph_of_pane(id)
        {
            view.update(cx, |view, cx| view.focus_name(window, cx));
        }
    }

    pub(in crate::workspace) fn run_browsed_flow(
        &mut self,
        lane: LaneRef,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .flows_with_unsaved_edits(cx)
            .iter()
            .any(|held| held == path)
        {
            self.report_error(
                daruda_store::observability::error_report::ErrorReport::new(s::flow::needs_save())
                    .severity(daruda_store::observability::error_report::ErrorSeverity::Warning)
                    .at(file!(), line!())
                    .dedup("flow.browser.unsaved")
                    .build(),
                cx,
            );
            return;
        }
        let _refused_on_screen = self.run_flow_at(
            lane,
            path,
            crate::workspace::command::flow_picker::FlowPurpose::Run,
            crate::workspace::flow_request::FlowSelection::default(),
            window,
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn scope_labels_keep_project_and_worktree_identity() {
        let state = super::FlowTarget {
            lane: super::LaneRef {
                project: 1,
                lane: 0,
            },
            project: "Project".into(),
            label: "Project / main".into(),
            current: true,
        };
        assert!(state.label.contains(&state.project));
    }
}
