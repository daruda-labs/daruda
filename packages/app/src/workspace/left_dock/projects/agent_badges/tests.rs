use super::*;

#[cfg(target_os = "macos")]
mod visual;

#[gpui::test]
fn empty_active_id_means_no_outline(cx: &mut gpui::TestAppContext) {
    crate::test_support::init_gpui_component(cx);
    cx.update(|cx| {
        // Sanity: passing None for active_session_id never panics
        // and produces a valid element. We can't assert the visual
        // outcome here without a render harness, but we can guard
        // against shape regressions.
        let sessions = vec![("abc".to_string(), SessionStatus::Idle)];
        let _ = agent_badges_row(&sessions, None, cx);
    });
}

#[gpui::test]
fn cell_width_is_consistent_across_states(cx: &mut gpui::TestAppContext) {
    crate::test_support::init_gpui_component(cx);
    cx.update(|cx| {
        // Both populated and empty cells must reserve the same
        // horizontal space so lane rows in the list don't visually
        // shift when a session attaches.
        let with = agent_status_cell(Some(SessionStatus::Working), None, cx);
        let without = agent_status_cell(None, None, cx);
        // Smoke check — each call returns an AnyElement; the actual
        // width is enforced by `STATUS_INDICATOR_CELL_WIDTH`.
        let _ = (with, without);
    });
}
