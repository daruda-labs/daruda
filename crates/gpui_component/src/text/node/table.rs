//! Intrinsic column sizing shared by every row of a rich-text table.

use super::{
    ColumnumnAlign, LinkClickHandlerFn, NodeContext, NodeRenderOptions, STRUCTURAL_LINE_ALPHA,
    TABLE_HEADER_FILL_ALPHA, Table,
};
use crate::{ActiveTheme as _, scroll::ScrollableElement as _};
use gpui::{
    App, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce,
    ScrollHandle, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _, rems,
};
use std::sync::Arc;

mod measure;
#[cfg(test)]
mod tests;

#[derive(IntoElement)]
pub(super) struct TableElement {
    table: Table,
    options: NodeRenderOptions,
    node_cx: NodeContext,
    link_click_handler: Option<Arc<LinkClickHandlerFn>>,
}

impl TableElement {
    pub(super) fn new(
        table: Table,
        options: NodeRenderOptions,
        node_cx: NodeContext,
        link_click_handler: Option<Arc<LinkClickHandlerFn>>,
    ) -> Self {
        Self {
            table,
            options,
            node_cx,
            link_click_handler,
        }
    }
}

impl RenderOnce for TableElement {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        // RenderOnce runs in request_layout, after ancestor text styles have
        // been applied, including a table nested inside a quote or list.
        let columns = measure::columns(&self.table, self.options, window, cx);
        let minimum: gpui::Pixels = columns.iter().map(|column| column.min).sum();
        let flexible = columns.iter().any(|column| column.max > column.min);
        let line_color = self.options.tint(STRUCTURAL_LINE_ALPHA, cx);
        let header_fill = self.options.tint(TABLE_HEADER_FILL_ALPHA, cx);
        let scroll = window
            .use_keyed_state("table-scroll", cx, |_, _| ScrollHandle::default())
            .read(cx)
            .clone();
        // The same basis and growth factor on every row preserve shared
        // boundaries. Only wrappable columns need space beyond minimum.
        let tracks = columns
            .iter()
            .enumerate()
            .map(|(ix, column)| {
                let grow = if flexible {
                    column.max - column.min
                } else {
                    column.max
                };
                (column.min, f32::from(grow), self.table.column_align(ix))
            })
            .collect::<Vec<_>>();
        let rows = self
            .table
            .children
            .iter()
            .enumerate()
            .map(|(row_ix, row)| {
                let cells = tracks
                    .iter()
                    .enumerate()
                    .map(|(ix, &(min, grow, align))| {
                        div()
                            .id(("cell", row_ix * tracks.len() + ix))
                            .debug_selector(move || format!("markdown-table-cell-{row_ix}-{ix}"))
                            .flex_basis(min)
                            // Stating the minimum retires flexbox's automatic
                            // one, which asks the text for a min-content width
                            // and gets its whole unwrapped line back.
                            .min_w(min)
                            .flex_shrink_0()
                            .map(|mut this| {
                                this.style().flex_grow = Some(grow);
                                this
                            })
                            .when(row_ix == 0, |this| {
                                this.bg(header_fill).font_weight(FontWeight::BOLD)
                            })
                            .when(align == ColumnumnAlign::Center, |this| this.text_center())
                            .when(align == ColumnumnAlign::Right, |this| this.text_right())
                            .when(ix > 0, |this| this.border_l_1().border_color(line_color))
                            .when(row_ix > 0, |this| {
                                this.border_t_1().border_color(line_color)
                            })
                            .px_2()
                            .py_1()
                            .when_some(row.children.get(ix), |this, cell| {
                                this.child(div().w_full().min_w_0().overflow_hidden().child(
                                    cell.children.render(
                                        self.options,
                                        &self.node_cx,
                                        self.link_click_handler.as_ref(),
                                        window,
                                        cx,
                                    ),
                                ))
                            })
                    })
                    .collect::<Vec<_>>();
                div().flex().items_stretch().w_full().children(cells)
            })
            .collect::<Vec<_>>();

        div().w_full().min_w_0().pb(rems(1.)).child(
            div()
                .id("table")
                .debug_selector(|| "markdown-table".into())
                .w_full()
                .min_w_0()
                .relative()
                .border_1()
                .border_color(line_color)
                .rounded(cx.theme().radius)
                .overflow_hidden()
                .child(
                    div()
                        .id("table-viewport")
                        .debug_selector(|| "markdown-table-viewport".into())
                        .w_full()
                        .overflow_x_scroll()
                        // Without this a wheel that only moves in y is spent on
                        // the one axis that does scroll, so reading past a table
                        // in the transcript drags its columns sideways.
                        .map(|mut this| {
                            this.style().restrict_scroll_to_axis = Some(true);
                            this
                        })
                        .track_scroll(&scroll)
                        .child(div().w_full().min_w(minimum).children(rows)),
                )
                .horizontal_scrollbar(&scroll),
        )
    }
}
