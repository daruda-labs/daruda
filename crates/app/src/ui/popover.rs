//! Popover — re-export. Anchored panel opened from a trigger element, for
//! browsing surfaces that are not command menus (the status-bar Ports chip):
//! clicks inside keep it open; outside click / Escape dismiss it. For a list
//! of commands use `.dropdown_menu` + [`crate::ui::menu_builder`] instead —
//! menu items are expected to dismiss on click, panels are not.
//!
//! Opening takes keyboard focus, so the click that opens one is consumed by the
//! trigger. A surrounding surface that reads a click as "activate me" would
//! otherwise take that focus straight back — see the test below.

pub use gpui_component::popover::{Popover, PopoverState};

#[cfg(test)]
mod tests {
    use std::rc::Rc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use gpui::{
        AppContext as _, Bounds, Context, InteractiveElement as _, IntoElement, Modifiers,
        MouseButton, ParentElement as _, Render, SharedString, Styled as _, TestAppContext,
        VisualTestContext, Window, WindowBounds, WindowOptions, div, point, px, size,
    };

    use crate::test_support::init_gpui_component;
    use crate::ui::DropdownMenu as _;

    /// Which primitive builds the trigger. `Dropdown` is here because
    /// `DropdownMenuPopover` is itself built on `Popover`, so it inherits the
    /// trigger's press handling — the part worth pinning.
    #[derive(Clone, Copy)]
    enum Kind {
        Panel,
        Dropdown,
    }

    /// A trigger inside a surface that reads any left press as "activate me" —
    /// the shape of a daruda pane, whose wrapper calls `focus_pane_on_click` and,
    /// for an agent chat, moves keyboard focus to the bottom input and connects
    /// an idle session.
    struct TriggerProbe {
        kind: Kind,
        activations: Rc<AtomicUsize>,
    }

    impl Render for TriggerProbe {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let activations = self.activations.clone();
            let trigger = crate::ui::button("probe-chip", "Fold: Auto");
            let control = match self.kind {
                Kind::Panel => super::Popover::new(SharedString::from("probe-popover"))
                    .trigger(trigger)
                    .content(move |_, _window, _cx| div().child("panel").into_any_element())
                    .into_any_element(),
                Kind::Dropdown => trigger
                    .dropdown_menu(|menu, _window, _cx| menu)
                    .into_any_element(),
            };
            div()
                .size_full()
                .on_mouse_down(MouseButton::Left, move |_, _window, _cx| {
                    activations.fetch_add(1, Ordering::SeqCst);
                })
                .child(control)
        }
    }

    /// Press the trigger and report how many times the surface under it was
    /// activated.
    fn surface_activations(cx: &mut TestAppContext, kind: Kind) -> usize {
        init_gpui_component(cx);
        let activations = Rc::new(AtomicUsize::new(0));
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(400.), px(300.)));
        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        let probe = activations.clone();
        let window = cx
            .update(|cx| {
                cx.open_window(opts, |_window, cx| {
                    cx.new(|_cx| TriggerProbe {
                        kind,
                        activations: probe,
                    })
                })
            })
            .expect("window opens");
        let mut vcx = VisualTestContext::from_window(window.into(), cx);
        vcx.run_until_parked();
        vcx.update(|window, _| window.refresh());
        vcx.run_until_parked();

        // Inside the trigger chip, which sits at the surface's top-left.
        vcx.simulate_click(point(px(10.), px(8.)), Modifiers::default());
        vcx.run_until_parked();

        activations.load(Ordering::SeqCst)
    }

    /// Pressing a popover trigger must not also read as activating the surface
    /// the trigger sits on. Opening takes keyboard focus, and the surface's own
    /// left-press listener runs after it — it would take that focus right back.
    #[gpui::test]
    fn pressing_a_panel_trigger_does_not_activate_the_surface_under_it(cx: &mut TestAppContext) {
        assert_eq!(
            surface_activations(cx, Kind::Panel),
            0,
            "the trigger's press reached the surface and activated it"
        );
    }

    /// The same contract through `.dropdown_menu()`, which wraps `Popover` — so
    /// the agent chat's `Recent steps` chip and every menu chip elsewhere get it
    /// from the same place rather than each re-deriving it.
    #[gpui::test]
    fn pressing_a_dropdown_trigger_does_not_activate_the_surface_under_it(cx: &mut TestAppContext) {
        assert_eq!(
            surface_activations(cx, Kind::Dropdown),
            0,
            "the dropdown trigger's press reached the surface and activated it"
        );
    }
}
