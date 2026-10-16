//! Flat tree containers. Selection belongs to the lane, never its ancestors.

use crate::ui::theme;
use gpui::{AnyElement, Div, Hsla, ParentElement, Styled, div, px};

/// Groups interleave with ungrouped projects, so only the outline tells
/// where a group ends. Standalone roots stay bare and share the gutter.
pub(super) fn group(header: AnyElement, body: AnyElement, outline: Hsla) -> Div {
    tree_root()
        .my(px(theme::LANE_GROUP_OUTLINE_MARGIN_Y))
        .p(px(theme::LANE_GROUP_OUTLINE_PAD))
        .rounded(px(theme::LANE_GROUP_OUTLINE_RADIUS))
        .border(px(theme::LANE_GROUP_OUTLINE_W))
        .border_color(outline)
        .child(header)
        .child(body)
}

pub(super) fn project(body: AnyElement) -> Div {
    tree_root().child(body)
}

fn tree_root() -> Div {
    div().flex().flex_col().mx(px(theme::LANE_TREE_MARGIN_X))
}

/// Coupled columns shared by project marks and lane status cells.
pub(in crate::workspace::left_dock) struct RowMetrics {
    pub leading_width: f32,
    pub leading_gap: f32,
    pub lane_inset: f32,
    pub subrow_indent: f32,
}

impl RowMetrics {
    pub fn new(metrics: theme::UiMetrics) -> Self {
        let leading_width = metrics.target(theme::CONTROL_TARGET_SIZE);
        let leading_gap = metrics.space(theme::LANE_LABEL_GAP);
        Self {
            leading_width,
            leading_gap,
            lane_inset: metrics.space(theme::LANE_ROW_PAD_X) - theme::LANE_ACTIVE_BORDER_W,
            subrow_indent: leading_width + leading_gap,
        }
    }
}

/// One row rhythm for groups, projects and lanes; each owns its interactions.
pub(super) fn row(metrics: theme::UiMetrics) -> Div {
    div()
        .px(px(metrics.space(theme::LANE_ROW_PAD_X)))
        .py(px(metrics.space(theme::DOCK_TREE_ROW_PAD_Y)))
        .min_h(px(metrics.row(theme::DOCK_TREE_ROW_HEIGHT)))
        .rounded(px(theme::LANE_ROW_RADIUS))
}

#[cfg(test)]
mod tests {
    use crate::ui::theme;

    #[test]
    fn tree_gutter_clears_scrollbar() {
        const {
            assert!(theme::LANE_TREE_MARGIN_X >= theme::SCROLLBAR_MARGIN_R + theme::SCROLLBAR_W);
        }
    }

    #[test]
    fn lane_column_lines_up_with_project_header() {
        for size in daruda_config::UiSize::ALL {
            let metrics = theme::UiMetrics(size);
            let row = super::RowMetrics::new(metrics);
            let project_start = metrics.space(theme::LANE_ROW_PAD_X);
            let lane_start = theme::LANE_ACTIVE_BORDER_W + row.lane_inset;
            assert_eq!(lane_start, project_start, "leading slots align at {size:?}");
            assert_eq!(
                lane_start + row.subrow_indent,
                project_start + row.leading_width + row.leading_gap,
                "labels and session rows align at {size:?}"
            );
            assert!(row.leading_width >= metrics.icon(theme::STATUS_INDICATOR_SIZE));
        }
    }
}
