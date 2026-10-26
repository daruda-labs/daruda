use gpui::{
    Bounds, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, TestAppContext, point, px,
    size,
};

use crate::session::interval_tree::LineRange;
use crate::view::{TerminalView, TerminalViewEvent};
use crate::{TerminalConfig, TerminalDims, TerminalSession};

#[gpui::test]
fn annotation_mouse_handlers_agree_with_menu_lookup_in_scrolled_inset_pane(
    cx: &mut TestAppContext,
) {
    let mut session = TerminalSession::new(
        TerminalDims { cols: 40, rows: 3 },
        TerminalConfig::default(),
    )
    .unwrap();
    session
        .feed(b"zero\r\none\r\ntwo\r\nthree\r\nfour\r\nfive\r\nsix\r\nseven\r\n")
        .unwrap();
    session.scroll_viewport(-2).unwrap();
    let top = session.viewport_row_offset();
    assert!(top > 0, "the fixture must require a viewport offset");
    let marked_row = top + 1;
    assert!(
        marked_row < session.line_buffer().wrapped_row_count(session.cols()),
        "the marked row must be scrollback, not the live grid"
    );
    let coord = session.screen_row_to_line_coord(marked_row).unwrap();
    let mark = session
        .add_annotation(LineRange::new(coord, coord), "history note".into())
        .unwrap();
    let handle = cx.add_window(|_, cx| TerminalView::new(session, cx.focus_handle()));
    let entity = handle.root(cx).unwrap();
    let mut events = cx.events(&entity);

    handle
        .update(cx, |view, window, cx| {
            view.apply_inset(12.0, 20.0);
            let layout = view
                .cell_layout(window)
                .expect("terminal font has cell metrics");
            let origin = point(px(100.0), px(200.0));
            view.state.last_bounds = Some(Bounds::new(origin, size(px(500.0), px(200.0))));
            view.refresh_viewport_preserving_selection();
            let (inset_x, inset_y) = view.inset();
            let marked = point(
                origin.x + px(inset_x + layout.cell_width * 0.5),
                origin.y + px(inset_y + layout.line_height * 1.5),
            );
            let unmarked = point(marked.x, origin.y + px(inset_y + layout.line_height * 0.5));

            view.on_mouse_move(
                &MouseMoveEvent {
                    position: marked,
                    ..Default::default()
                },
                window,
                cx,
            );
            assert_eq!(view.state.hovered_annotation, Some(mark));
            assert_eq!(
                view.annotation_at_window_position(marked, window),
                Some(mark)
            );
            view.on_mouse_down(
                &MouseDownEvent {
                    button: MouseButton::Left,
                    position: marked,
                    click_count: 2,
                    ..Default::default()
                },
                window,
                cx,
            );

            view.on_mouse_move(
                &MouseMoveEvent {
                    position: unmarked,
                    ..Default::default()
                },
                window,
                cx,
            );
            assert_eq!(view.state.hovered_annotation, None);
            assert_eq!(view.annotation_at_window_position(unmarked, window), None);
            view.on_mouse_down(
                &MouseDownEvent {
                    button: MouseButton::Left,
                    position: unmarked,
                    click_count: 2,
                    ..Default::default()
                },
                window,
                cx,
            );
            view.on_mouse_up(
                &MouseUpEvent {
                    button: MouseButton::Left,
                    position: unmarked,
                    ..Default::default()
                },
                window,
                cx,
            );
        })
        .unwrap();

    assert!(
        matches!(events.try_recv(), Ok(TerminalViewEvent::AnnotationDoubleClicked { id }) if id == mark)
    );
    assert!(
        events.try_recv().is_err(),
        "the unmarked double-click must not emit an annotation edit"
    );
}
