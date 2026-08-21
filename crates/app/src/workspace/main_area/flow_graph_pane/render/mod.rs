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
use crate::ui::theme::palette;

pub(super) mod toolbar;

use self::toolbar::toolbar;

impl Render for FlowGraphView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = div().size_full().track_focus(&self.focus_handle);
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
                let has_selection = !self.selected_nodes(cx).is_empty();
                let unsaved_form = self.has_unsaved_form(cx);
                body.child(
                    div()
                        .size_full()
                        .flex()
                        .flex_row()
                        .child(
                            div()
                                .relative()
                                .flex_1()
                                .h_full()
                                .child(canvas.clone())
                                .child(toolbar(has_selection, unsaved_form, cx)),
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
            vec![s::flow_graph_read_failed(
                &path.display().to_string(),
                message,
            )]
        }
        FlowGraphError::Parse { detail } => vec![s::flow_graph_parse_failed(detail)],
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
