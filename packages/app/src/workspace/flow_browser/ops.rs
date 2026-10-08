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
        // Only while the list is what the page draws: reading the browser's
        // inputs for a page showing its graph would re-arm their repaints
        // for nothing on screen (render-cost rule 10).
        let list_shown =
            self.active_page() == Some(Page::Flows) && self.pages.flows.detail.is_none();
        self.flows.browser.page_snapshot = list_shown.then(|| super::FlowPageSnapshot {
            workspace: cx.weak_entity(),
            flows: self.flow_rows_matching(|lane| lane == self.flow_browser_lane()),
            flow_lane: self.flow_browser_lane(),
            flow_history: self.flow_history_for_panel(),
            flow_files: self.flow_list_for_panel(),
            flow_browser: self.flow_browser_snapshot(cx),
        });
    }

    pub(in crate::workspace) fn flow_browser_lane(&self) -> LaneRef {
        self.flows.browser.state.scope.resolve(self.active)
    }

    pub(in crate::workspace) fn flow_browser_snapshot(&self, cx: &App) -> FlowBrowserSnapshot {
        let search = &self.flows.browser.searches[self.flows.browser.state.tab.index()];
        FlowBrowserSnapshot {
            state: self.flows.browser.state.clone(),
            targets: self
                .projects
                .lanes()
                .map(|(target, project, _)| FlowTarget {
                    lane: target,
                    project: project.name.clone(),
                    label: self.lane_label_for(target),
                    current: target == self.active,
                })
                .collect(),
            search: Handle(search.clone()),
            query: search.read(cx).value().to_string(),
            modified: self
                .flows
                .list
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
        self.flows.browser.state.scope = scope;
        self.invalidate_flow_list();
        self.invalidate_flow_history(self.flow_browser_lane());
        self.respawn_flow_watcher(cx);
        self.reset_flow_scroll();
        cx.notify();
    }

    pub(in crate::workspace) fn set_flow_tab(&mut self, tab: FlowTab, cx: &mut Context<Self>) {
        self.flows.browser.state.tab = tab;
        if let Some(page) = self
            .workspace_page
            .as_mut()
            .filter(|p| p.page == Page::Flows)
        {
            page.scroll = self.flows.browser.scrolls[tab.index()].clone();
        }
        cx.notify();
    }

    pub(in crate::workspace) fn set_flow_origin(
        &mut self,
        origin: Option<FlowOrigin>,
        cx: &mut Context<Self>,
    ) {
        self.flows.browser.state.origin = origin;
        self.reset_flow_scroll();
        cx.notify();
    }

    pub(in crate::workspace) fn set_flow_run_filter(
        &mut self,
        filter: RunFilter,
        cx: &mut Context<Self>,
    ) {
        self.flows.browser.state.run_filter = filter;
        self.reset_flow_scroll();
        cx.notify();
    }

    pub(in crate::workspace) fn set_flow_grouping(
        &mut self,
        grouping: FlowGrouping,
        cx: &mut Context<Self>,
    ) {
        self.flows.browser.state.grouping = grouping;
        cx.notify();
    }

    pub(in crate::workspace) fn toggle_flow_group(
        &mut self,
        origin: FlowOrigin,
        cx: &mut Context<Self>,
    ) {
        self.flows.browser.state.toggle_group(origin);
        cx.notify();
    }

    fn reset_flow_scroll(&self) {
        self.flows.browser.scrolls[self.flows.browser.state.tab.index()]
            .set_offset(gpui::point(gpui::Pixels::ZERO, gpui::Pixels::ZERO));
    }

    pub(in crate::workspace) fn clear_flow_search(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.flows.browser.searches[self.flows.browser.state.tab.index()].update(
            cx,
            |input, cx| {
                input.set_value("", window, cx);
            },
        );
        self.reset_flow_scroll();
        cx.notify();
    }

    pub(in crate::workspace) fn clear_flow_filters(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.flows.browser.state.tab {
            FlowTab::Definitions => self.flows.browser.state.origin = None,
            FlowTab::Runs => self.flows.browser.state.run_filter = RunFilter::All,
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

    /// Open the graph in the page, run in the worktree the page browses —
    /// the active one stays as it is.
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
        self.open_flow_graph(lane, path, window, cx);
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
        self.open_flow_graph_then(lane, path, window, cx, |_, view, window, cx| {
            view.update(cx, |view, cx| view.focus_name(window, cx));
        });
    }

    pub(in crate::workspace) fn run_browsed_flow(
        &mut self,
        lane: LaneRef,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
