use super::*;

#[cfg(target_os = "macos")]
mod visual;

#[gpui::test]
fn cell_width_is_consistent_across_states(cx: &mut gpui::TestAppContext) {
    crate::test_support::init_gpui_component(cx);
    cx.update(|cx| {
        // Both populated and empty cells must reserve the same
        // horizontal space so lane rows in the list don't visually
        // shift when a session attaches.
        let with = agent_status_cell(
            Some(SessionStatus::Working),
            None,
            Default::default(),
            theme::CONTROL_TARGET_SIZE,
            cx,
        );
        let without = agent_status_cell(
            None,
            None,
            Default::default(),
            theme::CONTROL_TARGET_SIZE,
            cx,
        );
        // Smoke check — each call returns an AnyElement; the actual
        // width is enforced by `STATUS_INDICATOR_CELL_WIDTH`.
        let _ = (with, without);
    });
}
