use gpui::{Context, IntoElement, SharedString, Window, div, prelude::*, px};

use super::SummaryLine;

use std::cell::Cell;

use super::StretchSlot;

/// Records which builders ran, so laziness is observable: a slot that shows
/// one side must not pay for parsing the other's markdown body.
fn probe<'a>(ran: &'a Cell<u8>, bit: u8, text: &'static str) -> super::SummaryBuilder<'a> {
    Box::new(move || {
        ran.set(ran.get() | bit);
        Some(SummaryLine::plain(text))
    })
}

#[test]
fn a_summary_slot_shows_nothing_once_expanded() {
    let ran = Cell::new(0);
    let slot = StretchSlot::Summary(probe(&ran, 1, "preview"));
    assert!(slot.builder_for(true).is_none());
    assert_eq!(ran.get(), 0, "an expanded summary is never built");

    let ran = Cell::new(0);
    let slot = StretchSlot::Summary(probe(&ran, 1, "preview"));
    let line = slot
        .builder_for(false)
        .and_then(|b| b())
        .expect("collapsed");
    assert_eq!(line.text, "preview");
    assert_eq!(ran.get(), 1);
}

#[test]
fn a_spacer_shows_nothing_in_either_state() {
    assert!(StretchSlot::Spacer.builder_for(false).is_none());
    assert!(StretchSlot::Spacer.builder_for(true).is_none());
}

/// A title dropped straight into the stretch slot spans it; the same title
/// through [`interactive_title`] shrink-wraps its glyphs.
///
/// This is the hitbox contract behind `FoldHeader::with_interactive_title`:
/// [`stretch_container`] is a block box (a gpui `div` defaults to
/// `display: block`), so a lone child takes its full width — and the diff
/// header's file path carries a click handler that opens the file, which
/// therefore fired on a click anywhere in the empty run out to the trailing
/// icons. Both sides are asserted because the spanning case is the *default*
/// gpui behaviour: without it a passing shrink-wrap assertion could just
/// mean the probe never got a wide slot to span in the first place.
#[gpui::test]
async fn only_an_interactive_title_shrink_wraps_its_slot(cx: &mut gpui::TestAppContext) {
    use gpui::{
        AppContext as _, Bounds, InteractiveElement as _, Styled as _, VisualTestContext,
        WindowBounds, WindowOptions, point, size,
    };

    const SLOT_W: f32 = 600.;

    struct SlotProbe;

    impl gpui::Render for SlotProbe {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            // Two stretch slots side by side in a column, each pinned to the
            // same width, so the only difference is how the title is placed.
            div()
                .flex()
                .flex_col()
                .w(px(SLOT_W))
                .child(
                    super::stretch_container(false)
                        .debug_selector(|| "plain-slot".into())
                        .child(
                            div()
                                .debug_selector(|| "plain-title".into())
                                .child(SharedString::from("src/main.rs")),
                        ),
                )
                .child(
                    super::stretch_container(false)
                        .debug_selector(|| "wrapped-slot".into())
                        .child(super::interactive_title(
                            div()
                                .debug_selector(|| "wrapped-title".into())
                                .child(SharedString::from("src/main.rs")),
                        )),
                )
        }
    }

    crate::test_support::init_gpui_component(cx);
    let bounds = Bounds::new(point(px(0.), px(0.)), size(px(SLOT_W), px(400.)));
    let window = cx
        .update(|cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |_window, cx| cx.new(|_cx| SlotProbe),
            )
        })
        .expect("window opens");
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.run_until_parked();
    // Force a second frame so `debug_bounds` reads settled layout, not the
    // construction-time paint (same rationale as `ui::markdown`'s probe).
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();

    let width = |vcx: &mut VisualTestContext, sel: &'static str| {
        vcx.debug_bounds(sel)
            .unwrap_or_else(|| panic!("{sel} painted"))
            .size
            .width
    };
    let slot_w = width(&mut vcx, "plain-slot");
    let plain_w = width(&mut vcx, "plain-title");
    let wrapped_w = width(&mut vcx, "wrapped-title");

    assert_eq!(
        plain_w, slot_w,
        "a bare block child spans the stretch slot ({plain_w:?} of {slot_w:?})"
    );
    assert!(
        wrapped_w < slot_w / 2.,
        "an interactive title must shrink-wrap its glyphs, not the slot \
             ({wrapped_w:?} of {slot_w:?})"
    );
}
