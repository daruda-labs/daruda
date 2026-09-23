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

/// One row rhythm for groups, projects and lanes; each owns its interactions.
pub(super) fn row() -> Div {
    div()
        .px(px(theme::LANE_ROW_PAD_X))
        .py(px(theme::DOCK_TREE_ROW_PAD_Y))
        .min_h(px(theme::DOCK_TREE_ROW_HEIGHT))
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
        let cell_start = theme::LANE_ACTIVE_BORDER_W + theme::LANE_ROW_INSET_L;
        let label_start = cell_start + theme::STATUS_INDICATOR_CELL_WIDTH + theme::LANE_ROW_GAP;
        assert_eq!(label_start, theme::LANE_PROJECT_NAME_INSET);
        let folder_centre = theme::LANE_PROJECT_NAME_INSET
            - theme::LANE_LABEL_GAP
            - theme::LANE_PROJECT_ICON_SIZE / 2.0;
        let cell_centre = cell_start + theme::STATUS_INDICATOR_CELL_WIDTH / 2.0;
        assert_eq!(cell_centre, folder_centre);
    }
}
