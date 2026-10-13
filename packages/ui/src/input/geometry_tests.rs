use super::*;
use crate::select::{self, SelectOption, SelectState};
use crate::test_support::init_gpui_component;
use gpui::{
    AppContext as _, Bounds, Context, Pixels, Render, TestAppContext, VisualTestContext, Window,
};

#[derive(Clone, Copy)]
enum Kind {
    Standard,
    Form,
    Search,
    PaneSearch,
}

struct Field {
    selector: &'static str,
    kind: Kind,
    state: Entity<InputState>,
    height: f32,
    inset_y: f32,
}

struct Probe {
    fields: Vec<Field>,
    standard_select: Entity<SelectState>,
    form_select: Entity<SelectState>,
}

impl Render for Probe {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A non-default rem and inherited typography must not resize fields.
        window.set_rem_size(px(20.));
        let surface = d::current(cx);
        let mut column = div()
            .w(px(240.))
            .flex()
            .flex_col()
            .text_size(px(32.))
            .line_height(px(40.));
        for field in &self.fields {
            let control = match field.kind {
                Kind::Standard => input(&field.state, cx, 0).into_any_element(),
                Kind::Form => form_input(&field.state, cx, 0).into_any_element(),
                Kind::Search => search_input(&field.state, cx, ()).into_any_element(),
                Kind::PaneSearch => search_input_on(
                    &field.state,
                    surface.modal_input_bg,
                    surface.text_primary,
                    cx,
                    (),
                )
                .into_any_element(),
            };
            let selector = field.selector;
            column = column.child(
                div()
                    .id(selector)
                    .debug_selector(move || selector.into())
                    .flex_shrink_0()
                    .child(control),
            );
        }
        column
            .child(
                crate::button("standard-action", "Save")
                    .debug_selector(|| "standard-action".into()),
            )
            .child(
                crate::button_compact("compact-action", "Fetch")
                    .debug_selector(|| "compact-action".into()),
            )
            .child(
                crate::field_trigger(
                    "field-trigger",
                    "A long field value that must leave room for its source",
                    div().flex_none().child("Default"),
                    cx,
                )
                .debug_selector(|| "field-trigger".into()),
            )
            .child(
                div()
                    .id("standard-select")
                    .debug_selector(|| "standard-select".into())
                    .child(select::select(&self.standard_select, cx, 0).w_full()),
            )
            .child(
                div()
                    .id("form-select")
                    .debug_selector(|| "form-select".into())
                    .child(select::form_select(&self.form_select, cx, 0).w_full()),
            )
            .child(
                div()
                    .id("standard-value")
                    .debug_selector(|| "standard-value".into())
                    .child(readonly_input("Project", cx)),
            )
            .child(
                div()
                    .id("form-value")
                    .debug_selector(|| "form-value".into())
                    .child(readonly_form_input("Project", cx)),
            )
    }
}

fn verify_geometry(cx: &mut TestAppContext, light: bool) {
    init_gpui_component(cx);
    if light {
        cx.update(|cx| {
            cx.set_global(
                d::DarudaTheme::from_json(include_str!(
                    "../../../../assets/themes/daruda_light.json"
                ))
                .unwrap(),
            );
            d::apply_daruda_palette(cx);
        });
    }
    let mut probe_entity = None;
    let window = cx.add_window(|window, cx| {
        let probe = cx.new(|cx| {
            let specs = [
                ("standard", Kind::Standard, "Name", 28., 4., None),
                ("empty", Kind::Standard, "", 28., 4., None),
                ("masked", Kind::Standard, "secret", 28., 4., None),
                ("disabled", Kind::Standard, "Locked", 28., 4., None),
                ("form", Kind::Form, "Name", 36., 8., None),
                ("search", Kind::Search, "query", 28., 4., None),
                ("pane-search", Kind::PaneSearch, "query", 28., 4., None),
                (
                    "standard-multi",
                    Kind::Standard,
                    "One\nTwo",
                    48.,
                    4.,
                    Some((2, 4)),
                ),
                ("form-multi", Kind::Form, "One\nTwo", 56., 8., Some((2, 4))),
                (
                    "capped",
                    Kind::Form,
                    "One\nTwo\nThree\nFour",
                    56.,
                    8.,
                    Some((1, 2)),
                ),
                (
                    "long",
                    Kind::Standard,
                    "A long single line that extends beyond the available field width",
                    28.,
                    4.,
                    None,
                ),
            ];
            let fields = specs
                .into_iter()
                .map(|(selector, kind, value, height, inset_y, rows)| {
                    let state = cx.new(|cx| {
                        let mut state = InputState::new(window, cx)
                            .placeholder("Placeholder")
                            .masked(selector == "masked");
                        if let Some((min, max)) = rows {
                            state = state.auto_grow(min, max);
                        }
                        state.set_value(value, window, cx);
                        state.set_disabled(selector == "disabled", cx);
                        state
                    });
                    Field {
                        selector,
                        kind,
                        state,
                        height,
                        inset_y,
                    }
                })
                .collect();
            let mut make_select = || {
                cx.new(|cx| {
                    select::state_with_options(
                        vec![SelectOption::simple("Project")],
                        Some(&SharedString::from("Project")),
                        window,
                        cx,
                    )
                })
            };
            Probe {
                fields,
                standard_select: make_select(),
                form_select: make_select(),
            }
        });
        probe_entity = Some(probe.clone());
        gpui_component::Root::new(probe, window, cx)
    });
    let probe = probe_entity.unwrap();
    let mut vcx = VisualTestContext::from_window(window.into(), cx);
    vcx.run_until_parked();
    for _ in 0..2 {
        vcx.update(|window, _| window.refresh());
        vcx.run_until_parked();
    }
    let bounds: Vec<Bounds<Pixels>> = [
        "standard",
        "empty",
        "masked",
        "disabled",
        "form",
        "search",
        "pane-search",
        "standard-multi",
        "form-multi",
        "capped",
        "long",
    ]
    .map(|selector| vcx.debug_bounds(selector).unwrap())
    .into();
    window
        .update(&mut vcx, |_, _, cx| {
            let probe = probe.read(cx);
            for (field, outer) in probe.fields.iter().zip(&bounds) {
                assert_eq!(
                    outer.size.height,
                    px(field.height),
                    "{} outer height",
                    field.selector
                );
                let state = field.state.read(cx);
                let mut text = state.last_bounds().unwrap();
                // Painted text bounds include the caret-following scroll offset.
                text.origin -= state.scroll_handle().offset();
                let inset_x = if matches!(field.kind, Kind::PaneSearch) {
                    9.
                } else {
                    10.
                };
                assert!(
                    (text.left() - outer.left() - px(inset_x)).as_f32().abs() < 0.01,
                    "{} left inset",
                    field.selector
                );
                assert_eq!(
                    text.top() - outer.top(),
                    px(field.inset_y),
                    "{} top inset",
                    field.selector
                );
                assert_eq!(
                    outer.bottom() - text.bottom(),
                    px(field.inset_y),
                    "{} bottom inset",
                    field.selector
                );
                assert_eq!(state.is_disabled(), field.selector == "disabled");
            }
        })
        .unwrap();
    for (selector, height) in [
        ("standard-action", 28.),
        ("compact-action", 20.),
        ("field-trigger", 28.),
        ("standard-select", 28.),
        ("standard-value", 28.),
        ("form-select", 36.),
        ("form-value", 36.),
    ] {
        assert_eq!(
            vcx.debug_bounds(selector).unwrap().size.height,
            px(height),
            "{selector}"
        );
    }
    window
        .update(&mut vcx, |_, window, cx| {
            let state = probe.read(cx).fields[0].state.clone();
            state.update(cx, |state, cx| state.focus(window, cx))
        })
        .unwrap();
    vcx.run_until_parked();
    assert_eq!(
        vcx.debug_bounds("standard").unwrap(),
        bounds[0],
        "focus must not shift field geometry"
    );
}

#[gpui::test]
fn field_geometry_is_stable_in_dark_theme(cx: &mut TestAppContext) {
    verify_geometry(cx, false);
}

#[gpui::test]
fn field_geometry_is_stable_in_light_theme(cx: &mut TestAppContext) {
    verify_geometry(cx, true);
}
