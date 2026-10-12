//! Read-only observations: taking a snapshot must not schedule another paint.

use gpui::{ListOffset, Pixels, TestAppContext, WindowHandle, px};

use super::super::super::AgentChatView;

#[derive(Debug)]
struct Geometry {
    row: usize,
    top: Pixels,
    height: Pixels,
}

#[derive(Debug)]
pub(super) struct Snapshot {
    pub scroll: ListOffset,
    pub following: bool,
    pub paints: u32,
    rows: usize,
    viewport_height: Pixels,
    visible: Vec<Geometry>,
}

pub(super) fn snapshot(handle: WindowHandle<AgentChatView>, cx: &TestAppContext) -> Snapshot {
    handle
        .read_with(cx, |view, _| {
            let list = &view.list_state;
            assert_eq!(list.item_count(), view.rows.len(), "list/projection count");
            let viewport = list.viewport_bounds();
            assert!(viewport.size.height > px(0.), "list must have painted");
            let visible: Vec<_> = view
                .rows
                .iter()
                .enumerate()
                .filter(|(_, row)| !row.hidden)
                .filter_map(|(row, _)| {
                    let bounds = list.bounds_for_item(row)?;
                    (bounds.bottom() > viewport.top() && bounds.top() < viewport.bottom())
                        .then_some(Geometry {
                            row,
                            top: bounds.top() - viewport.top(),
                            height: bounds.size.height,
                        })
                })
                .collect();
            assert!(!visible.is_empty(), "visible rows must be measured");
            Snapshot {
                scroll: list.logical_scroll_top(),
                following: list.is_following_tail(),
                paints: view.render_count.get(),
                rows: view.rows.len(),
                viewport_height: viewport.size.height,
                visible,
            }
        })
        .expect("snapshot window is live")
}

#[track_caller]
pub(super) fn assert_pixels(actual: Pixels, expected: Pixels, label: &str) {
    const EPSILON: f32 = 0.5;
    assert!(
        (actual - expected).abs() <= px(EPSILON),
        "{label}: {actual:?} != {expected:?}"
    );
}

impl Snapshot {
    #[track_caller]
    pub fn assert_matches(&self, expected: &Self, context: &str) {
        assert_eq!(self.rows, expected.rows, "{context}");
        assert_eq!(self.following, expected.following, "{context}");
        assert_eq!(self.scroll.item_ix, expected.scroll.item_ix, "{context}");
        assert_pixels(
            self.scroll.offset_in_item,
            expected.scroll.offset_in_item,
            context,
        );
        assert_pixels(self.viewport_height, expected.viewport_height, context);
        assert_eq!(
            self.visible.len(),
            expected.visible.len(),
            "{context}\nactual: {self:#?}\nexpected: {expected:#?}"
        );
        for (actual, expected) in self.visible.iter().zip(&expected.visible) {
            assert_eq!(actual.row, expected.row, "{context}");
            let label = format!("row {}: {context}", actual.row);
            assert_pixels(actual.top, expected.top, &label);
            assert_pixels(actual.height, expected.height, &label);
        }
    }
}
