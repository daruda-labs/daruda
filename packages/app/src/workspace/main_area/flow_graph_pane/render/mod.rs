//! Drawing the pane: the canvas beside its inspector, or the reason there is
//! no graph to draw.
//!
//! Split out because `impl Render` belongs in its own file, and because what a
//! frame is made of is a different question from how the pane gets built or
//! what a click on a card means.

use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Window,
    div, px,
};

use super::{FlowGraphError, FlowGraphState, FlowGraphView, Selection, form};
use crate::surface::strings as s;
use crate::ui::cursor::CursorReachExt as _;
use crate::ui::theme::palette;

pub(super) mod toolbar;

use self::toolbar::{ToolbarState, toolbar};

impl Render for FlowGraphView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = div()
            .size_full()
            .flex()
            .flex_col()
            .track_focus(&self.focus_handle)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .px(px(palette::PAD_STANDARD))
                    .py(px(palette::PAD_XS))
                    .child(
                        crate::ui::button("flow-back-to-list", s::flow::back_to_list())
                            .debug_selector(|| "flow-back-to-list".into())
                            .tab_stop(true)
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.emit(super::FlowGraphEvent::BackToList)
                            })),
                    ),
            );
        match &self.state {
            FlowGraphState::Graph { canvas, .. } => {
                // A row rather than an overlay: floating the inspector over the
                // graph would hide the cards it is about.
                //
                // Its width is reserved whether or not anything is selected. On
                // selection alone the canvas would narrow, the graph re-fit into
                // what is left, and the picture shift under the pointer that
                // just clicked — a shift-click on a second card would miss it.
                let inspector = match self.selection(cx) {
                    Selection::One(_) => match &self.form {
                        Some(form) => form::render(form, cx).into_any_element(),
                        None => form::render_empty(cx).into_any_element(),
                    },
                    Selection::Many(nodes) => form::render_many(nodes.len(), cx).into_any_element(),
                    // Nothing to click is not the same as nothing clicked yet.
                    Selection::None if self.is_empty_graph() => {
                        form::render_no_nodes(cx).into_any_element()
                    }
                    Selection::None => form::render_empty(cx).into_any_element(),
                };
                // The toolbar goes inside the canvas half, not the pane: over the
                // pane it would sit on the inspector column instead of the graph.
                let state = ToolbarState {
                    has_selection: !self.selected_nodes(cx).is_empty(),
                    unsaved_form: self.has_unsaved_form(cx),
                    until: self.selected_node(cx),
                    pin: self.pin_action(cx),
                };
                body.child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .flex()
                        .flex_row()
                        .child(
                            div()
                                .relative()
                                .flex_1()
                                .h_full()
                                // An open hand says a drag would move the view
                                // rather than whatever it lands on; a closed
                                // one says it is moving, and reaches the whole
                                // window because a pan redraws this every
                                // frame.
                                .cursor_reach(self.pan_cursor_shown)
                                .child(canvas.clone())
                                .child(toolbar(state, cx)),
                        )
                        .child(inspector),
                )
            }
            FlowGraphState::Unreadable(err) => body
                .flex()
                .flex_col()
                .gap(px(palette::FLOW_GRAPH_CARD_ROW_GAP))
                .p(px(palette::FLOW_GRAPH_CARD_PAD))
                .text_size(px(palette::FLOW_GRAPH_META_FONT_SIZE))
                .text_color(crate::ui::theme::current(cx).text_muted)
                .children(error_lines(err).into_iter().map(|line| div().child(line))),
        }
    }
}

/// One line per thing wrong. A validation failure reports every issue the
/// stage saw, and collapsing them to the first would hide the rest.
fn error_lines(err: &FlowGraphError) -> Vec<String> {
    match err {
        FlowGraphError::Read { path, message } => {
            vec![s::flow::graph_read_failed(
                path.display().to_string(),
                message,
            )]
        }
        FlowGraphError::Parse { detail } => vec![s::flow::graph_parse_failed(detail)],
        FlowGraphError::Validate { issues } => issues.clone(),
    }
}

impl FlowGraphView {
    /// A flow that loaded but holds no nodes — `nodes: []`, which the engine
    /// accepts. There is nothing to click, so the inspector says to add one.
    fn is_empty_graph(&self) -> bool {
        match &self.state {
            FlowGraphState::Graph { model, .. } => model.nodes.is_empty(),
            FlowGraphState::Unreadable(_) => false,
        }
    }
}
