//! Column-driven list geometry shared by headers and body rows.

use gpui::{AnyElement, App, Div, ElementId, IntoElement, Stateful, div, prelude::*, px};

use super::theme;

#[derive(Clone, Copy, Debug)]
pub enum Column {
    Fixed(f32),
    FixedUnclipped(f32),
    Flexible(f32),
}

impl Column {
    pub const fn title() -> Self {
        Self::Flexible(theme::list_metrics::TITLE_MIN_W)
    }

    fn min_width(self) -> f32 {
        match self {
            Self::Fixed(width) | Self::FixedUnclipped(width) | Self::Flexible(width) => width,
        }
    }

    fn cell(self) -> Div {
        let cell = div().w(px(self.min_width())).min_w_0();
        match self {
            Self::Fixed(_) => cell.flex_none().truncate(),
            Self::FixedUnclipped(_) => cell.flex_none(),
            Self::Flexible(_) => cell.flex_grow().flex_shrink_0(),
        }
    }
}

/// The caller's key identifies content, without exposing domain types to UI.
pub struct ListTable<K> {
    columns: Vec<(K, Column)>,
}

impl<K: Copy> ListTable<K> {
    pub fn new(columns: impl IntoIterator<Item = (K, Column)>) -> Self {
        Self {
            columns: columns.into_iter().collect(),
        }
    }

    pub fn min_width(&self) -> f32 {
        self.columns
            .iter()
            .map(|(_, column)| column.min_width())
            .sum::<f32>()
            + self.columns.len().saturating_sub(1) as f32 * theme::GAP_LG
            + theme::PAD_LG * 2.0
    }

    pub fn body(&self) -> Div {
        div().flex().flex_col().min_w(px(self.min_width()))
    }

    pub fn row(&self, mut render_cell: impl FnMut(K, Div) -> AnyElement) -> Div {
        div()
            .flex()
            .items_center()
            .gap(px(theme::GAP_LG))
            .px(px(theme::PAD_LG))
            .py(px(theme::PAD_SM))
            .text_size(px(theme::FONT_SIZE_MD))
            .children(
                self.columns
                    .iter()
                    .map(|(key, column)| render_cell(*key, column.cell())),
            )
    }

    pub fn header(&self, render_cell: impl FnMut(K, Div) -> AnyElement, cx: &App) -> Div {
        let t = theme::current(cx);
        self.row(render_cell)
            .text_color(t.text_muted)
            .border_b_1()
            .border_color(t.border)
    }
}

pub fn scroll(id: impl Into<ElementId>, body: impl IntoElement) -> Stateful<Div> {
    div().id(id).overflow_x_scroll().child(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimum_width_follows_the_visible_columns_and_shared_row_insets() {
        let table = ListTable::new([(0, Column::Flexible(200.0)), (1, Column::Fixed(96.0))]);
        assert_eq!(
            table.min_width(),
            200.0 + 96.0 + theme::GAP_LG + 2.0 * theme::PAD_LG
        );
        let narrower = ListTable::new([(0, Column::Flexible(200.0))]);
        assert_eq!(
            table.min_width() - narrower.min_width(),
            96.0 + theme::GAP_LG
        );
    }

    #[test]
    fn an_empty_schema_has_no_negative_gap() {
        assert_eq!(ListTable::<()>::new([]).min_width(), 2.0 * theme::PAD_LG);
    }

    #[test]
    fn unclipped_fixed_cells_keep_their_width_budget() {
        let clipped = ListTable::new([(0, Column::Fixed(96.0))]);
        let unclipped = ListTable::new([(0, Column::FixedUnclipped(96.0))]);
        assert_eq!(clipped.min_width(), unclipped.min_width());
    }
}
