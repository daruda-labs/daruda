use super::*;
use crate::ui::Disableable as _;
use gpui::{
    Context, InteractiveElement as _, IntoElement, Render, TestAppContext, VisualTestContext,
    Window,
};

#[derive(Default)]
struct Probe {
    checked: bool,
    changes: usize,
}

impl Probe {
    fn toggle(&mut self, cx: &mut Context<Self>) {
        self.checked = !self.checked;
        self.changes += 1;
        cx.notify();
    }
}

impl Render for Probe {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .child(
                switch("switch-live", self.checked, cx)
                    .debug_selector(|| "switch-live".into())
                    .on_click(cx.listener(|this, _, _, cx| this.toggle(cx))),
            )
            .child(
                switch("switch-disabled", false, cx)
                    .disabled(true)
                    .debug_selector(|| "switch-disabled".into())
                    .on_click(cx.listener(|this, _, _, cx| this.toggle(cx))),
            )
    }
}

#[gpui::test]
fn switch_supports_click_and_keyboard_but_not_disabled_activation(cx: &mut TestAppContext) {
    crate::test_support::init_gpui_component(cx);
    let window = cx.add_window(|_, _| Probe::default());
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.run_until_parked();
    vcx.update(|window, _| window.refresh());
    vcx.run_until_parked();
    let live = vcx.debug_bounds("switch-live").unwrap();
    let disabled = vcx.debug_bounds("switch-disabled").unwrap();
    vcx.simulate_click(live.center(), Default::default());
    vcx.run_until_parked();
    assert_eq!(
        window
            .read_with(&vcx, |p, _| (p.checked, p.changes))
            .unwrap(),
        (true, 1)
    );
    vcx.update(|window, cx| window.focus_next(cx));
    vcx.run_until_parked();
    // GPUI's keystroke helper emits only KeyDown; clicks activate on KeyUp.
    vcx.simulate_keystrokes("space");
    vcx.simulate_event(gpui::KeyUpEvent {
        keystroke: gpui::Keystroke::parse("space").unwrap(),
    });
    vcx.run_until_parked();
    assert_eq!(
        window
            .read_with(&vcx, |p, _| (p.checked, p.changes))
            .unwrap(),
        (false, 2)
    );
    vcx.simulate_keystrokes("enter");
    vcx.simulate_event(gpui::KeyUpEvent {
        keystroke: gpui::Keystroke::parse("enter").unwrap(),
    });
    vcx.run_until_parked();
    vcx.simulate_click(disabled.center(), Default::default());
    vcx.run_until_parked();
    assert_eq!(
        window
            .read_with(&vcx, |p, _| (p.checked, p.changes))
            .unwrap(),
        (true, 3)
    );
}

#[test]
fn compact_thumb_fits_its_track() {
    for m in [&SETTINGS, &COMPACT] {
        assert!(m.thumb + 2.0 * m.inset <= m.h);
        assert!(m.w <= m.target_w && m.h <= m.target_h);
    }
}
