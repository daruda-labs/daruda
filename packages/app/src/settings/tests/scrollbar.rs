use super::*;
use gpui::{ScrollHandle, VisualTestContext, point, px, size};

fn click_track(handle: &ScrollHandle, cx: &mut VisualTestContext) {
    let bounds = handle.bounds();
    cx.simulate_click(
        point(bounds.right() - px(6.), bounds.bottom() - px(24.)),
        Default::default(),
    );
    cx.run_until_parked();
}

#[gpui::test]
fn body_scrollbar_tracks_the_new_section_without_an_extra_repaint(cx: &mut TestAppContext) {
    let (window, settings) = build_window(cx);
    cx.simulate_window_resize(window.into(), size(px(800.), px(500.)));
    let handle = settings.read_with(cx, |settings, _| settings.scroll_handle.clone());
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.run_until_parked();

    for section in [
        BuiltinSection::Font,
        BuiltinSection::General,
        BuiltinSection::Font,
    ] {
        vcx.update(|window, cx| {
            settings.update(cx, |settings, cx| {
                settings.focus_section(section, window, cx)
            });
        });
        vcx.run_until_parked();
        assert_eq!(handle.offset().y, px(0.));
        click_track(&handle, &mut vcx);
        if section == BuiltinSection::Font {
            assert!(handle.max_offset().y > px(0.));
            assert!(
                handle.offset().y < px(0.),
                "the new page must have a live track"
            );
        } else {
            assert_eq!(handle.max_offset().y, px(0.));
            assert_eq!(handle.offset().y, px(0.), "a fitting page must not scroll");
        }
    }
}

#[gpui::test]
fn sidebar_scrollbar_scrolls_independently_in_a_short_window(cx: &mut TestAppContext) {
    let (window, settings) = build_window(cx);
    cx.simulate_window_resize(window.into(), size(px(800.), px(420.)));
    let (sidebar, body) = settings.read_with(cx, |settings, _| {
        (
            settings.sidebar_scroll_handle.clone(),
            settings.scroll_handle.clone(),
        )
    });
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.run_until_parked();

    assert!(sidebar.max_offset().y > px(0.));
    click_track(&sidebar, &mut vcx);
    assert!(
        sidebar.offset().y < px(0.),
        "overflowing navigation must have a live track"
    );
    assert_eq!(body.offset().y, px(0.));
}
