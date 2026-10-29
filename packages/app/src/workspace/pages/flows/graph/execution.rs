//! Read-only execution canvas using the editor's layout and card renderer.

use super::{
    build::build_canvas_graph,
    click::NodeClickPlugin,
    frame::FrameGraphPlugin,
    model::FlowGraphModel,
    node_ids::NodeIds,
    pins::PinSet,
    renderer::{self, CardFacts, FlowNodeRenderer, NODE_TYPE, card_for, flow_theme},
};
use crate::ui::flow_canvas::{BackgroundPlugin, FlowCanvas, GraphPlugin};
use crate::workspace::Workspace;
use daruda_flow::{NodeId, history::RunHistory};
use gpui::{App, AppContext as _, Context, Entity, Window};

pub(in crate::workspace) struct ExecutionGraph {
    pub canvas: Entity<FlowCanvas>,
    ids: NodeIds,
    measured: Option<gpui::Bounds<gpui::Pixels>>,
    surface: (gpui::Hsla, gpui::Hsla),
}

impl ExecutionGraph {
    pub fn new(
        history: &RunHistory,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Option<Self> {
        let model = FlowGraphModel::from_flow(history.flow()?, Vec::new());
        let (mut graph, ids) = build_canvas_graph(&model, &PinSet::default());
        for (id, data) in Self::cards(history, &ids) {
            if let Some(card) = graph.get_node_mut(&id) {
                card.set_data(data);
            }
        }
        let tokens = crate::ui::theme::PaneSurfaceTokens::flow_graph(cx);
        let canvas = cx.new(|cx| {
            FlowCanvas::builder(graph, cx, window)
                .theme(flow_theme(&tokens))
                .node_renderer(
                    NODE_TYPE,
                    FlowNodeRenderer {
                        palette: renderer::CardPalette::of(&tokens),
                    },
                )
                .plugin(BackgroundPlugin::new())
                .plugin(super::execution_pan::ExecutionPanPlugin)
                .plugin(GraphPlugin::new())
                .plugin(NodeClickPlugin::new())
                .plugin(FrameGraphPlugin::new())
                .build()
        });
        Some(Self {
            canvas,
            ids,
            measured: None,
            surface: (tokens.background, tokens.foreground),
        })
    }

    fn cards(
        history: &RunHistory,
        ids: &NodeIds,
    ) -> Vec<(crate::ui::flow_canvas::CanvasNodeId, serde_json::Value)> {
        let Some(flow) = history.flow() else {
            return Vec::new();
        };
        let model = FlowGraphModel::from_flow(flow, Vec::new());
        let mut cards = Vec::new();
        for node in &model.nodes {
            use crate::runs::NodeRunState;
            use daruda_flow::history::NodeStatus;
            let run = match history.node_status(&node.id) {
                NodeStatus::Running { attempt } => NodeRunState::Running { attempt },
                NodeStatus::Passed | NodeStatus::Pinned => NodeRunState::Passed,
                NodeStatus::Failed => NodeRunState::Failed,
                NodeStatus::Fixing => NodeRunState::Fixing,
                NodeStatus::Pending | NodeStatus::Canceled | NodeStatus::Interrupted => {
                    NodeRunState::Pending
                }
            };
            if let Some(id) = ids.canvas(&node.id) {
                let mut data = card_for(
                    node,
                    CardFacts {
                        run,
                        pinned: false,
                        issues: 0,
                        unpinned: None,
                    },
                );
                data.policy.clear();
                let latest = history.attempts(&node.id).last().copied();
                let duration = latest
                    .and_then(|(_, seq)| history.attempt(&node.id, seq))
                    .map(|a| format!("{:.1}s", a.took.as_secs_f64()));
                data.meta = match &node.kind {
                    super::model::GraphNodeKind::Agent { agent, .. } => match duration {
                        Some(duration) => format!("{} · {duration}", agent.id),
                        None => agent.id.clone(),
                    },
                    super::model::GraphNodeKind::Gate { .. } => duration.unwrap_or_default(),
                };
                cards.push((id, serde_json::to_value(data).unwrap_or_default()));
            }
        }
        cards
    }

    pub fn refresh(&self, history: &RunHistory, cx: &mut Context<Workspace>) {
        let cards = Self::cards(history, &self.ids);
        self.canvas.update(cx, |canvas, cx| {
            canvas.dispatch_command(super::commands::StampCards { cards }, cx);
        });
    }

    pub fn matches_theme(&self, cx: &App) -> bool {
        let tokens = crate::ui::theme::PaneSurfaceTokens::flow_graph(cx);
        self.surface == (tokens.background, tokens.foreground)
    }

    pub fn select(&self, node: &NodeId, cx: &mut Context<Workspace>) {
        if let Some(node) = self.ids.canvas(node) {
            self.canvas.update(cx, |canvas, cx| {
                if !canvas.graph().selected_node().contains(&node) {
                    canvas.dispatch_command(super::commands::SelectNode { node }, cx);
                }
            });
        }
    }

    pub fn needs_layout_repaint(&mut self, cx: &App) -> bool {
        let Some(bounds) = self.canvas.read(cx).viewport().window_bounds() else {
            return false;
        };
        if self.measured == Some(bounds) {
            return false;
        }
        self.measured = Some(bounds);
        true
    }

    pub fn selected(&self, cx: &App) -> Option<NodeId> {
        self.canvas
            .read(cx)
            .graph()
            .selected_node()
            .iter()
            .next()
            .and_then(|id| self.ids.flow(*id))
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use crate::workspace::{
        pages::{
            Page,
            flows::execution_fixture::{ExecutionShot, seed},
        },
        tests::build_workspace_with,
    };
    use gpui::TestAppContext;

    #[gpui::test]
    fn execution_cards_paint_after_the_initial_bounds_notification(cx: &mut TestAppContext) {
        let root = tempfile::tempdir().unwrap();
        let dir = seed(root.path(), ExecutionShot::Retry);
        let project = daruda_store::project::Project::from_path(root.path());
        let (window, workspace) =
            build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
        let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);
        super::super::CARDS_DRAWN.with(|n| n.set(0));
        workspace.update_in(&mut vcx, |ws, window, cx| {
            ws.open_page(Page::Flows, window, cx);
            ws.open_run_detail(ws.active, &dir, window, cx);
        });
        vcx.run_until_parked();
        assert!(
            super::super::CARDS_DRAWN.with(|n| n.get()) >= 2,
            "initial bounds must trigger a second paint of execution cards"
        );
        super::super::CARDS_DRAWN.with(|n| n.set(0));
        workspace.update_in(&mut vcx, |_, window, _| window.refresh());
        vcx.run_until_parked();
        assert!(
            super::super::CARDS_DRAWN.with(|n| n.get()) >= 2,
            "global invalidation must retain the measured graph cards"
        );
    }
    #[gpui::test]
    fn background_drag_and_scroll_move_the_execution_canvas(cx: &mut TestAppContext) {
        use crate::workspace::pages::flows::detail::FlowDetailBody;
        use gpui::{MouseButton, ScrollDelta, ScrollWheelEvent, TouchPhase, point, px};
        let root = tempfile::tempdir().unwrap();
        let dir = seed(root.path(), ExecutionShot::Completed);
        let project = daruda_store::project::Project::from_path(root.path());
        let (window, workspace) =
            build_workspace_with(cx, &daruda_config::Config::default(), Some(project));
        let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);
        workspace.update_in(&mut vcx, |ws, window, cx| {
            ws.open_page(Page::Flows, window, cx);
            ws.open_run_detail(ws.active, &dir, window, cx);
        });
        vcx.run_until_parked();
        let canvas = workspace.read_with(&vcx, |ws, _| {
            let FlowDetailBody::Run(run) = &ws.pages.flows.detail.as_ref().unwrap().body else {
                panic!("run");
            };
            run.execution.graph.view().unwrap().canvas.clone()
        });
        let (bounds, offset, zoom) = canvas.read_with(&vcx, |canvas, _| {
            (
                canvas.viewport().window_bounds().unwrap(),
                canvas.viewport().offset(),
                canvas.viewport().zoom(),
            )
        });
        let start = bounds.origin + point(px(10.0), px(10.0));
        let delta = point(px(35.0), px(23.0));
        vcx.simulate_mouse_move(start, None, Default::default());
        vcx.simulate_mouse_down(start, MouseButton::Left, Default::default());
        vcx.simulate_mouse_move(start + delta, Some(MouseButton::Left), Default::default());
        vcx.simulate_mouse_up(start + delta, MouseButton::Left, Default::default());
        vcx.run_until_parked();
        canvas.read_with(&vcx, |canvas, _| {
            assert_eq!(canvas.viewport().offset(), offset + delta);
            assert_eq!(canvas.viewport().zoom(), zoom);
        });
        vcx.simulate_event(ScrollWheelEvent {
            position: start,
            delta: ScrollDelta::Pixels(delta),
            modifiers: Default::default(),
            touch_phase: TouchPhase::Moved,
        });
        vcx.run_until_parked();
        canvas.read_with(&vcx, |canvas, _| {
            assert_eq!(canvas.viewport().offset(), offset + delta + delta);
            assert_eq!(canvas.viewport().zoom(), zoom);
        });
        let lines = point(2.0, -3.0);
        vcx.simulate_event(ScrollWheelEvent {
            position: start,
            delta: ScrollDelta::Lines(lines),
            modifiers: Default::default(),
            touch_phase: TouchPhase::Moved,
        });
        vcx.run_until_parked();
        canvas.read_with(&vcx, |canvas, _| {
            assert_eq!(
                canvas.viewport().offset(),
                offset + delta + delta + point(px(48.0), px(-72.0))
            );
            assert_eq!(canvas.viewport().zoom(), zoom);
        });
        vcx.simulate_event(ScrollWheelEvent {
            position: start,
            delta: ScrollDelta::Pixels(delta),
            modifiers: gpui::Modifiers {
                control: true,
                ..Default::default()
            },
            touch_phase: TouchPhase::Moved,
        });
        vcx.run_until_parked();
        canvas.read_with(&vcx, |canvas, _| assert!(canvas.viewport().zoom() < zoom));
    }
}
