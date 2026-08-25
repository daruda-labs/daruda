//! The inspector column — the selected node's fields, and what stands in for
//! them when there is nothing (or too much) selected.
//!
//! A view builder, not a state machine: the buttons emit
//! [`super::super::FlowGraphEvent`] and the workspace does the work, because a
//! view that wrote to the file would be a second place that knows how.

use gpui::{Context, IntoElement};

use super::{KindChoice, NodeForm, Refusal};

/// The inspector: the selected node's fields, a save and a revert.
///
/// A view builder, not a state machine — the buttons emit
/// [`super::FlowGraphEvent`] and the workspace does the work, because a view
/// that wrote to the file would be a second place that knows how.
pub(in super::super) fn render(
    form: &NodeForm,
    cx: &mut Context<super::super::FlowGraphView>,
) -> impl IntoElement {
    use crate::surface::strings as s;
    use crate::ui::select::select;
    use crate::ui::theme::{current, palette};
    use crate::ui::{Disableable as _, button, button_danger, button_primary};
    use gpui::{ParentElement as _, Styled as _, div, px};

    // Status hues stay the UI theme's — an error is an error on any surface.
    let (primary, error) = (
        crate::ui::theme::PaneSurfaceTokens::flow_graph(cx).foreground,
        current(cx).banner_error_text,
    );
    let dirty = form.is_dirty(cx);
    let refusal = form.refusal(cx);

    let mut rows = div()
        .flex()
        .flex_col()
        .gap(px(palette::FLOW_INSPECTOR_GAP))
        .child(field_with_note(
            form,
            super::notes::FormField::Id,
            s::flow_form_id_label(),
            None,
            field(form.id_state(), cx, 0),
            cx,
        ))
        .child(field_with_note(
            form,
            super::notes::FormField::Deps,
            s::flow_form_deps_label(),
            None,
            field(form.deps_state(), cx, 1),
            cx,
        ));

    // Tab indices continue across the kind-specific fields in visual order,
    // reusing the same pool for both kinds — the person lands on the same
    // logical slot whichever node they picked.
    let body = form.body_states(cx);
    rows = rows.child(field_column(
        s::flow_form_kind_label(),
        select(body.kind_select, cx, 2),
        cx,
    ));
    rows = match body.kind {
        KindChoice::Agent => rows
            .child(source_row(
                body.prompt,
                s::flow_form_prompt_label(),
                s::flow_form_prompt_file_label(),
                3,
                cx,
            ))
            .child(field_with_note(
                form,
                super::notes::FormField::Output,
                s::flow_form_output_label(),
                Some(s::flow_form_output_help()),
                field(body.output, cx, 5),
                cx,
            ))
            .child(fail_section(body.on_fail, FailShape::Retry, cx)),
        KindChoice::Command => rows
            .child(field_column(
                s::flow_form_run_label(),
                field(body.run, cx, 3),
                cx,
            ))
            .child(fail_section(body.on_fail, FailShape::Repair, cx)),
    };

    rows = rows
        .child(field_column(
            s::flow_form_timeout_label(),
            field(form.timeout_state(), cx, 4),
            cx,
        ))
        .child(field_with_note(
            form,
            super::notes::FormField::Cwd,
            s::flow_form_cwd_label(),
            None,
            field(form.cwd_state(), cx, 5),
            cx,
        ))
        // Agent nodes only: `agent:` lives inside the file's `Agent` variant, so
        // a command node cannot carry one — offering the boxes there is
        // offering fields no save could keep. Nothing is hidden by this: the
        // rules that point at this block all name agent nodes, and a repair's
        // missing agent points at the `fix` box instead.
        .children((body.kind == KindChoice::Agent).then(|| agent_section(form, cx)));

    if let Some(refusal) = &refusal {
        let message = match refusal {
            Refusal::EmptyId => s::flow_form_id_required(),
            Refusal::InvalidId => s::flow_form_id_invalid(),
            Refusal::Timeout(text) => s::flow_form_timeout_unreadable(text),
            Refusal::Attempts(text) => s::flow_form_attempts_unreadable(text),
            Refusal::OutputRequired => s::flow_form_output_required(),
            Refusal::RunRequired => s::flow_form_run_required(),
        };
        rows = rows.child(
            div()
                .text_size(px(palette::FLOW_GRAPH_META_FONT_SIZE))
                .text_color(error)
                .child(message),
        );
    }

    let can_save = dirty && refusal.is_none();
    let footer = div()
        .flex()
        .flex_row()
        .gap(px(palette::FLOW_INSPECTOR_GAP))
        .child(
            button_primary("flow-form-save", s::flow_form_save())
                .disabled(!can_save)
                .on_click(cx.listener(|_, _, _window, cx| {
                    cx.emit(super::super::FlowGraphEvent::Save);
                })),
        )
        .child(
            button("flow-form-revert", s::flow_form_revert())
                .disabled(!dirty)
                .on_click(cx.listener(|_, _, _window, cx| {
                    cx.emit(super::super::FlowGraphEvent::Revert);
                })),
        )
        // Deleting the node this form is about, from the form it is about it.
        // Also on the pane's menu, which is where a person who has not selected
        // anything looks.
        .child(
            button_danger("flow-form-delete", s::flow_delete_node()).on_click(cx.listener(
                |_, _, _window, cx| {
                    cx.emit(super::super::FlowGraphEvent::Delete);
                },
            )),
        );

    column(cx)
        .flex()
        .flex_col()
        .gap(px(palette::FLOW_INSPECTOR_GAP))
        .child(
            // The node's own name, as a heading: without an explicit colour it
            // inherits the panel's muted body tone and reads as another label.
            div()
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
        .child(rows)
        .child(footer)
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
    let muted = crate::ui::theme::PaneSurfaceTokens::flow_graph(cx).foreground_muted;
    div()
        .flex()
        .flex_col()
        .gap(px(palette::FLOW_GRAPH_CARD_ROW_GAP))
        .child(field_column(label, body, cx))
        .children(help.map(|help| {
            div()
                .text_size(px(palette::FLOW_GRAPH_META_FONT_SIZE))
                .text_color(muted)
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
    use crate::ui::select::select;
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
    use crate::ui::select::select;
    use crate::ui::theme::palette;
    use gpui::{ParentElement as _, Styled as _, div, px};

    let acting = super::acting(states, cx);
    let mut section = div()
        .flex()
        .flex_col()
        .gap(px(palette::FLOW_INSPECTOR_GAP))
        .child(field_column(
            s::flow_form_fail_label(),
            select(&states.policy, cx, 11),
            cx,
        ));
    if !acting {
        return section;
    }
    section = match shape {
        FailShape::Retry => section.child(source_row(
            &states.hint,
            s::flow_form_hint_label(),
            s::flow_form_hint_file_label(),
            12,
            cx,
        )),
        FailShape::Repair => section
            .child(field_column(
                s::flow_form_fix_label(),
                field(&states.fix, cx, 12),
                cx,
            ))
            .child(field_column(
                s::flow_form_rerun_label(),
                field(&states.rerun, cx, 13),
                cx,
            )),
    };
    section
        .child(field_column(
            s::flow_form_attempts_label(),
            field(&states.max_attempts, cx, 14),
            cx,
        ))
        .child(field_column(
            s::flow_form_wait_label(),
            field(&states.wait, cx, 15),
            cx,
        ))
}

/// The node's own agent, behind a disclosure.
///
/// Closed unless the node already overrides something: most nodes take
/// `defaults`, and five empty boxes above the save button is five rows of
/// nothing. Tab indices continue the same pool — a closed block simply has no
/// slots in it.
fn agent_section(
    form: &NodeForm,
    cx: &mut Context<super::super::FlowGraphView>,
) -> impl IntoElement {
    use crate::surface::strings as s;
    use crate::ui::disclosure;
    use crate::ui::select::select;
    use crate::ui::theme::{current, palette};
    use gpui::{InteractiveElement as _, ParentElement as _, Styled as _, div, px};

    let muted = crate::ui::theme::PaneSurfaceTokens::flow_graph(cx).foreground_muted;
    let open = form.agent_open();
    let header = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(palette::FLOW_GRAPH_CARD_ROW_GAP))
        .cursor_pointer()
        .on_mouse_down(
            gpui::MouseButton::Left,
            cx.listener(|view, _, _window, cx| view.toggle_agent_section(cx)),
        )
        .child(disclosure("flow-form-agent", open).color(muted))
        .child(
            div()
                .text_size(px(palette::FLOW_GRAPH_META_FONT_SIZE))
                .text_color(muted)
                .child(s::flow_form_agent_section()),
        );

    let error = current(cx).banner_error_text;
    let mut section = div()
        .flex()
        .flex_col()
        .gap(px(palette::FLOW_INSPECTOR_GAP))
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
                s::flow_form_agent_id_label(),
                field(&states.id, cx, 6),
                cx,
            ))
            .child(field_column(
                s::flow_form_agent_mode_label(),
                field(&states.mode, cx, 7),
                cx,
            ))
            .child(field_column(
                s::flow_form_agent_model_label(),
                field(&states.model, cx, 8),
                cx,
            ))
            .child(field_column(
                s::flow_form_agent_effort_label(),
                field(&states.effort, cx, 9),
                cx,
            ))
            .child(field_column(
                s::flow_form_agent_permission_label(),
                select(&states.permission, cx, 10),
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
    note(crate::surface::strings::flow_form_many_selected(n), cx)
}

/// What it says for a flow with no nodes at all.
pub(in super::super) fn render_no_nodes(
    cx: &mut Context<super::super::FlowGraphView>,
) -> impl IntoElement {
    note(crate::surface::strings::flow_form_no_nodes(), cx)
}

/// What it says with nothing selected. The column is there either way — see the
/// note in `FlowGraphView::render` for why it does not come and go.
pub(in super::super) fn render_empty(
    cx: &mut Context<super::super::FlowGraphView>,
) -> impl IntoElement {
    note(crate::surface::strings::flow_form_no_selection(), cx)
}

/// A labelled field on the pane's own surface, not the modal one.
///
/// `crate::ui::field_column` draws its label with `crate::ui::Label`, whose own
/// doc says to prefer it "whenever the surrounding widget's natural foreground
/// colour is what you want" — and here it is not: `Label` takes the UI theme's
/// foreground while this pane's background is mirrored from the terminal, so a
/// light UI theme put dark text on a dark pane. Same reasoning as `field`
/// below, one step further out.
///
/// The modal font and gap tokens are kept deliberately: what was wrong is the
/// colour, and re-picking the geometry here would move every row.
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
                .text_color(crate::ui::theme::PaneSurfaceTokens::flow_graph(cx).foreground)
                .child(label.into()),
        )
        .child(div().w_full().child(body))
}

/// A text box on the pane's own surface, not the modal one.
///
/// One place so the boxes cannot drift apart: [`crate::ui::input`]'s default
/// background is a workspace-chrome tone, and this pane follows the terminal.
fn field<T: crate::ui::input::InputTabSpec>(
    state: &gpui::Entity<crate::ui::InputState>,
    cx: &gpui::App,
    tab: T,
) -> impl gpui::IntoElement {
    crate::ui::input_on(
        state,
        crate::ui::theme::PaneSurfaceTokens::flow_graph(cx).tint,
        cx,
        tab,
    )
}

/// The inspector column holding one line of muted prose.
fn note(text: String, cx: &mut Context<super::super::FlowGraphView>) -> impl IntoElement {
    use crate::ui::theme::palette;
    use gpui::{ParentElement as _, Styled as _, px};

    let muted = crate::ui::theme::PaneSurfaceTokens::flow_graph(cx).foreground_muted;
    column(cx)
        .text_size(px(palette::FLOW_GRAPH_META_FONT_SIZE))
        .text_color(muted)
        .child(text)
}

/// The column every inspector state shares: one width, one border, one inset.
fn column(cx: &mut Context<super::super::FlowGraphView>) -> gpui::Stateful<gpui::Div> {
    use crate::ui::theme::palette;
    use gpui::{InteractiveElement as _, StatefulInteractiveElement as _, Styled as _, div, px};

    let tokens = crate::ui::theme::PaneSurfaceTokens::flow_graph(cx);
    div()
        .id("flow-inspector")
        .w(px(palette::FLOW_INSPECTOR_W))
        .flex_none()
        .h_full()
        // A short pane cannot show five fields and a prompt box at once, and a
        // form whose Save button is off the bottom edge is unusable — the same
        // reason the right dock's views scroll their bodies.
        .overflow_y_scroll()
        .p(px(palette::FLOW_INSPECTOR_PAD))
        .border_l_1()
        .border_color(tokens.border_tint)
        .bg(tokens.background)
}
