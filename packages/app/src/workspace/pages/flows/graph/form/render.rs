//! The inspector column — the selected node's fields, and what stands in for
//! them when there is nothing (or too much) selected.
//!
//! A view builder, not a state machine: the buttons emit
//! [`super::super::FlowGraphEvent`] and the workspace does the work, because a
//! view that wrote to the file would be a second place that knows how.

use crate::ui::ButtonVariants as _;
use gpui::{Context, IntoElement};

use super::{KindChoice, NodeForm, Refusal};

/// The inspector: the selected node's fields, a save and a revert.
///
/// A view builder, not a state machine — the buttons emit
/// [`super::FlowGraphEvent`] and the workspace does the work, because a view
/// that wrote to the file would be a second place that knows how.
pub(in super::super) fn render(
    form: &NodeForm,
    status: &super::super::form_bridge::FormStatus,
    cx: &mut Context<super::super::FlowGraphView>,
) -> impl IntoElement {
    use crate::surface::strings as s;
    use crate::ui::select::form_select as select;
    use crate::ui::theme::{current, palette};
    use crate::ui::{Disableable as _, button, button_icon, button_primary};
    use gpui::{
        InteractiveElement as _, ParentElement as _, StatefulInteractiveElement as _, Styled as _,
        div, px,
    };

    // Status hues stay the UI theme's — an error is an error on any surface.
    let (primary, error) = (
        crate::ui::theme::current(cx).text_primary,
        current(cx).banner_error_text,
    );
    let refusal = form.refusal(cx);

    let mut rows = div()
        .flex()
        .flex_col()
        .gap(px(palette::FLOW_INSPECTOR_GAP))
        .child(field_with_note(
            form,
            super::notes::FormField::Id,
            s::flow::form_id_label(),
            None,
            field(form.id_state(), cx, 1),
            cx,
        ))
        .child(field_with_note(
            form,
            super::notes::FormField::Deps,
            s::flow::form_deps_label(),
            None,
            dependency_picker(form, cx),
            cx,
        ));

    // Tab indices continue across the kind-specific fields in visual order,
    // reusing the same pool for both kinds — the person lands on the same
    // logical slot whichever node they picked.
    let body = form.body_states(cx);
    rows = rows.child(field_column(
        s::flow::form_kind_label(),
        select(body.kind_select, cx, 3),
        cx,
    ));
    rows = rows.children((body.kind == KindChoice::Agent).then(|| agent_section(form, cx)));
    rows = match body.kind {
        KindChoice::Agent => rows
            .child(source_row(
                body.prompt,
                s::flow::form_prompt_label(),
                s::flow::form_prompt_file_label(),
                14,
                cx,
            ))
            .children(super::is_file_source(body.prompt, cx).then(|| {
                button("flow-prompt-file", s::flow::form_pick_prompt())
                    .on_click(cx.listener(|view, _, window, cx| view.pick_prompt_file(window, cx)))
            }))
            .children((!super::is_file_source(body.prompt, cx)).then(|| {
                button("flow-prompt-expand", s::flow::form_expand_prompt())
                    .on_click(cx.listener(|view, _, window, cx| view.expand_prompt(window, cx)))
            }))
            .children((!super::is_file_source(body.prompt, cx)).then(|| result_picker(form, cx)))
            .child(field_with_note(
                form,
                super::notes::FormField::Output,
                s::flow::form_output_label(),
                Some(s::flow::form_output_help()),
                field(body.output, cx, 18),
                cx,
            )),
        KindChoice::Command => rows
            .child(field_column(
                s::flow::form_run_label(),
                field(body.run, cx, 14),
                cx,
            ))
            .child(result_picker(form, cx)),
    };

    rows = rows.child(execution_section(form, cx));

    if let Some(refusal) = &refusal {
        let message = match refusal {
            Refusal::EmptyId => s::flow::form_id_required(),
            Refusal::InvalidId => s::flow::form_id_invalid(),
            Refusal::Timeout(text) => s::flow::form_timeout_unreadable(text),
            Refusal::Attempts(text) => s::flow::form_attempts_unreadable(text),
            Refusal::Turns(text) => s::flow::form_turns_unreadable(text),
            Refusal::OutputRequired => s::flow::form_output_required(),
            Refusal::RunRequired => s::flow::form_run_required(),
        };
        rows = rows.child(
            div()
                .text_size(px(palette::FLOW_GRAPH_META_FONT_SIZE))
                .text_color(error)
                .child(message),
        );
    }

    // An untouched form's refusal is about the file as it is, which does not
    // stop saving the title or another node.
    let can_save = status.unsaved && (refusal.is_none() || !status.node_local);
    let footer = div()
        .flex()
        .flex_row()
        .flex_none()
        .p(px(palette::FLOW_INSPECTOR_PAD))
        .border_t_1()
        .border_color(crate::ui::theme::current(cx).border)
        .gap(px(palette::GAP_SM))
        .child(
            button_primary("flow-form-save", s::flow::form_save())
                .disabled(!can_save)
                .on_click(cx.listener(|_, _, _window, cx| {
                    cx.emit(super::super::FlowGraphEvent::Save);
                })),
        )
        .child(
            button("flow-form-revert", s::flow::form_revert_node())
                .disabled(!status.node_revertable)
                .on_click(cx.listener(|view, _, window, cx| view.revert_selected_node(window, cx))),
        )
        // Deleting the node this form is about, from the form it is about it.
        // Also on the pane's menu, which is where a person who has not selected
        // anything looks.
        .child(
            button_icon("flow-form-delete", crate::ui::icons::DELETE, cx)
                .tooltip(s::flow::delete_node())
                .on_click(cx.listener(|_, _, _window, cx| {
                    cx.emit(super::super::FlowGraphEvent::Delete);
                })),
        );

    column(cx)
        .flex()
        .flex_col()
        .overflow_hidden()
        .child(
            // The node's own name, as a heading: without an explicit colour it
            // inherits the panel's muted body tone and reads as another label.
            div()
                .flex_none()
                .p(px(palette::FLOW_INSPECTOR_PAD))
                .text_size(px(palette::FLOW_GRAPH_ID_FONT_SIZE))
                .text_color(primary)
                .child(form.node.clone().into_string()),
        )
        // Directly under the heading, not after the fields: the column scrolls,
        // and a capture found the banner below the fold — which is the same as
        // not saying anything.
        .children(form.banner().map(|banner| {
            div()
                .text_size(px(palette::FLOW_GRAPH_META_FONT_SIZE))
                .text_color(error)
                .child(banner.to_string())
        }))
        .child(
            div()
                .id("flow-inspector-fields")
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .overflow_y_scroll()
                .p(px(palette::FLOW_INSPECTOR_PAD))
                .child(rows),
        )
        .child(footer)
}

fn dependency_picker(form: &NodeForm, cx: &mut Context<super::super::FlowGraphView>) -> gpui::Div {
    use crate::surface::strings as s;
    use crate::ui::{button, select::form_select, theme};
    use gpui::{ParentElement as _, Styled as _, div, px};
    let deps = form.fields(cx).deps;
    div()
        .flex()
        .flex_col()
        .gap(px(theme::GAP_SM))
        .child(div().flex().flex_wrap().gap(px(theme::GAP_SM)).children(
            deps.iter().enumerate().map(|(index, dep)| {
                let dep = dep.clone();
                button(("flow-dep", index), s::flow::form_dep_chip(&dep))
                    .tooltip(s::flow::form_remove_dep(&dep))
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.remove_form_dep(&dep, window, cx)
                    }))
            }),
        ))
        .children(deps.is_empty().then(|| {
            div()
                .text_color(theme::current(cx).text_muted)
                .child(s::flow::form_no_deps())
        }))
        .child(form_select(&form.dep_picker, cx, 2))
}

fn result_picker(
    form: &NodeForm,
    cx: &mut Context<super::super::FlowGraphView>,
) -> impl IntoElement {
    use crate::ui::{select::form_select, theme};
    use gpui::{ParentElement as _, Styled as _, div, px};
    div()
        .flex()
        .flex_col()
        .gap(px(theme::GAP_SM))
        .child(form_select(&form.result_picker, cx, 17))
        .child(
            div()
                .text_size(px(theme::FLOW_GRAPH_META_FONT_SIZE))
                .text_color(theme::current(cx).text_muted)
                .child(crate::surface::strings::flow::form_result_help()),
        )
}

/// A labelled box, with a standing description of the field and the engine's
/// reason under it when there is one for this box. The note is what turns "the
/// flow would not load" into "this field"; the help is the rule that holds even
/// when nothing is wrong, which is the only place an author meets it.
fn field_with_note(
    form: &NodeForm,
    field: super::notes::FormField,
    label: String,
    help: Option<String>,
    body: impl IntoElement,
    cx: &gpui::App,
) -> impl IntoElement {
    use crate::ui::theme::{current, palette};
    use gpui::{ParentElement as _, Styled as _, div, px};

    let error = current(cx).banner_error_text;
    let muted = crate::ui::theme::current(cx).text_muted;
    div()
        .flex()
        .flex_col()
        .gap(px(palette::FLOW_GRAPH_CARD_ROW_GAP))
        .child(field_column(label, body, cx))
        .children(help.map(|help| {
            div()
                .text_size(px(palette::FLOW_GRAPH_META_FONT_SIZE))
                .text_color(muted)
                .whitespace_normal()
                .child(help)
        }))
        .children(form.note_for(field).map(|note| {
            div()
                .text_size(px(palette::FLOW_GRAPH_META_FONT_SIZE))
                .text_color(error)
                .child(note.to_string())
        }))
}

/// An either-or field: the select that says where the text lives, and the box
/// for whichever half it names. Both boxes stay alive behind it, so switching
/// and switching back does not lose what was typed.
fn source_row(
    states: &super::SourceStates,
    inline_label: String,
    file_label: String,
    tab: isize,
    cx: &mut Context<super::super::FlowGraphView>,
) -> impl IntoElement {
    use crate::ui::select::form_select as select;
    use crate::ui::theme::palette;
    use gpui::{ParentElement as _, Styled as _, div, px};

    let from_file = super::is_file_source(states, cx);
    div()
        .flex()
        .flex_col()
        .gap(px(palette::FLOW_INSPECTOR_GAP))
        .child(field_column(
            if from_file { file_label } else { inline_label },
            select(&states.choice, cx, tab),
            cx,
        ))
        .child(if from_file {
            field(&states.file, cx, tab + 1).into_any_element()
        } else {
            field(&states.inline, cx, tab + 1).into_any_element()
        })
}

/// Which fields the acting policy shows.
#[derive(Copy, Clone, PartialEq, Eq)]
enum FailShape {
    Retry,
    Repair,
}

/// What the node does when it fails: the policy, and the fields the chosen one
/// needs. `halt` needs none, so the rows below simply are not there.
fn fail_section(
    states: &super::FailStates,
    shape: FailShape,
    cx: &mut Context<super::super::FlowGraphView>,
) -> impl IntoElement {
    use crate::surface::strings as s;
    use crate::ui::select::form_select as select;
    use crate::ui::theme::palette;
    use gpui::{ParentElement as _, Styled as _, div, px};

    let acting = super::acting(states, cx);
    let mut section = div()
        .flex()
        .flex_col()
        .gap(px(palette::FLOW_INSPECTOR_GAP))
        .child(field_column(
            s::flow::form_fail_label(),
            select(&states.policy, cx, 25),
            cx,
        ));
    if !acting {
        return section;
    }
    section = match shape {
        FailShape::Retry => section.child(source_row(
            &states.hint,
            s::flow::form_hint_label(),
            s::flow::form_hint_file_label(),
            26,
            cx,
        )),
        FailShape::Repair => section
            .child(field_column(
                s::flow::form_fix_label(),
                field(&states.fix, cx, 26),
                cx,
            ))
            .child(field_column(
                s::flow::form_rerun_label(),
                field(&states.rerun, cx, 27),
                cx,
            )),
    };
    section
        .child(field_column(
            s::flow::form_attempts_label(),
            field(&states.max_attempts, cx, 28),
            cx,
        ))
        .child(field_column(
            s::flow::form_wait_label(),
            field(&states.wait, cx, 29),
            cx,
        ))
}

/// The agent picker stays visible; optional overrides share one disclosure.
fn agent_section(
    form: &NodeForm,
    cx: &mut Context<super::super::FlowGraphView>,
) -> impl IntoElement {
    use crate::surface::strings as s;
    use crate::ui::select::form_select as select;
    use crate::ui::theme::{current, palette};
    use gpui::{ParentElement as _, Styled as _, div, px};

    let muted = crate::ui::theme::current(cx).text_muted;
    let open = form.agent_open();
    let header = crate::ui::button("flow-form-agent", s::flow::form_agent_section())
        .ghost()
        .justify_start()
        .tab_stop(true)
        .tab_index(6)
        .icon(crate::ui::icons::icon(if open {
            crate::ui::icons::EXPAND_MORE
        } else {
            crate::ui::icons::CHEVRON_RIGHT
        }))
        .on_click(cx.listener(|view, _, _, cx| view.toggle_agent_section(cx)));

    let error = current(cx).banner_error_text;
    let mut section = div()
        .flex()
        .flex_col()
        .gap(px(palette::FLOW_INSPECTOR_GAP))
        .child(field_column(
            s::flow::form_agent_id_label(),
            form.agent_choices[0].clone(),
            cx,
        ))
        .child(header)
        .children(form.note_for(super::notes::FormField::Agent).map(|note| {
            div()
                .text_size(px(palette::FLOW_GRAPH_META_FONT_SIZE))
                .text_color(error)
                .child(note.to_string())
        }));
    if open {
        let states = form.agent_states();
        section = section
            .child(field_column(
                s::flow::form_agent_model_label(),
                form.agent_choices[1].clone(),
                cx,
            ))
            .child(field_column(
                s::flow::form_agent_mode_label(),
                form.agent_choices[2].clone(),
                cx,
            ))
            .child(field_column(
                s::flow::form_agent_effort_label(),
                field(&states.effort, cx, 12),
                cx,
            ))
            .child(
                div()
                    .text_size(px(palette::FLOW_GRAPH_META_FONT_SIZE))
                    .text_color(muted)
                    .child(s::flow::form_effort_help()),
            )
            .child(field_column(
                s::flow::form_agent_permission_label(),
                select(&states.permission, cx, 13),
                cx,
            ));
    }
    section
}

fn execution_section(
    form: &NodeForm,
    cx: &mut Context<super::super::FlowGraphView>,
) -> impl IntoElement {
    use crate::surface::strings as s;
    use crate::ui::{button, theme};
    use gpui::{ParentElement as _, Styled as _, div, px};
    let mut section = div()
        .flex()
        .flex_col()
        .gap(px(theme::FLOW_INSPECTOR_GAP))
        .child(
            button(
                "flow-execution-settings",
                s::flow::form_execution_settings(),
            )
            .ghost()
            .justify_start()
            .tab_stop(true)
            .tab_index(19)
            .icon(crate::ui::icons::icon(if form.settings_open {
                crate::ui::icons::EXPAND_MORE
            } else {
                crate::ui::icons::CHEVRON_RIGHT
            }))
            .on_click(cx.listener(|view, _, _, cx| view.toggle_execution_section(cx))),
        );
    if form.settings_open {
        let body = form.body_states(cx);
        section =
            section
                .child(field_column(
                    s::flow::form_timeout_label(),
                    form.timeout_choice.clone(),
                    cx,
                ))
                .children((body.kind == KindChoice::Agent).then(|| {
                    field_column(s::flow::form_turns_label(), form.turns_choice.clone(), cx)
                }))
                .child(field_with_note(
                    form,
                    super::notes::FormField::Cwd,
                    s::flow::form_cwd_label(),
                    None,
                    field(form.cwd_state(), cx, 24),
                    cx,
                ))
                .child(fail_section(
                    body.on_fail,
                    if body.kind == KindChoice::Agent {
                        FailShape::Retry
                    } else {
                        FailShape::Repair
                    },
                    cx,
                ));
    }
    section
}

/// What the inspector says when a marquee took several cards.
pub(in super::super) fn render_many(
    n: usize,
    cx: &mut Context<super::super::FlowGraphView>,
) -> impl IntoElement {
    note(crate::surface::strings::flow::form_many_selected(n), cx)
}

/// What it says for a flow with no nodes at all.
pub(in super::super) fn render_no_nodes(
    cx: &mut Context<super::super::FlowGraphView>,
) -> impl IntoElement {
    note(crate::surface::strings::flow::form_no_nodes(), cx)
}

/// What it says with nothing selected. The column is there either way — see the
/// note in `FlowGraphView::render` for why it does not come and go.
pub(in super::super) fn render_empty(
    cx: &mut Context<super::super::FlowGraphView>,
) -> impl IntoElement {
    note(crate::surface::strings::flow::form_no_selection(), cx)
}

/// A label and field on the UI theme’s authoring surface.
fn field_column(
    label: impl Into<gpui::SharedString>,
    body: impl gpui::IntoElement,
    cx: &gpui::App,
) -> impl gpui::IntoElement {
    use crate::ui::theme::palette;
    use gpui::{ParentElement as _, Styled as _, div, px};
    div()
        .flex()
        .flex_col()
        .gap(px(palette::MODAL_PANEL_GAP))
        .child(
            div()
                .text_size(px(palette::MODAL_BODY_FONT_SIZE))
                .text_color(crate::ui::theme::current(cx).text_primary)
                .child(label.into()),
        )
        .child(div().w_full().child(body))
}

/// Shared 36px authoring geometry, including multi-line inputs.
fn field<T: crate::ui::input::InputTabSpec>(
    state: &gpui::Entity<crate::ui::InputState>,
    cx: &gpui::App,
    tab: T,
) -> impl gpui::IntoElement {
    crate::ui::form_input(state, cx, tab)
}

/// The inspector column holding one line of muted prose.
fn note(text: String, cx: &mut Context<super::super::FlowGraphView>) -> impl IntoElement {
    use crate::ui::theme::palette;
    use gpui::{ParentElement as _, Styled as _, px};

    let muted = crate::ui::theme::current(cx).text_muted;
    column(cx)
        .p(px(palette::FLOW_INSPECTOR_PAD))
        .text_size(px(palette::FLOW_GRAPH_META_FONT_SIZE))
        .text_color(muted)
        .child(text)
}

/// The column every inspector state shares: one width, one border, one inset.
fn column(cx: &mut Context<super::super::FlowGraphView>) -> gpui::Stateful<gpui::Div> {
    use crate::ui::theme::palette;
    use gpui::{InteractiveElement as _, Styled as _, div, px};

    let tokens = crate::ui::theme::current(cx);
    div()
        .id("flow-inspector")
        .w(px(palette::FLOW_INSPECTOR_W))
        .flex_none()
        .h_full()
        // A short pane cannot show five fields and a prompt box at once, and a
        // form whose Save button is off the bottom edge is unusable — the same
        // reason the right dock's views scroll their bodies.
        .overflow_hidden()
        .border_l_1()
        .border_color(tokens.border)
        .bg(tokens.modal_panel_bg)
}
