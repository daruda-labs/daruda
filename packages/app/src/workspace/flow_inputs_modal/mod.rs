//! The values a flow's declared inputs take for one desktop run, asked for
//! after every other question a start has (which flow, which profile).
//!
//! Each input gets the control its type calls for, filled from its default.
//! The values are checked here by the engine's own rule before the run is
//! handed on, so a refusal names the field in the form instead of arriving
//! as a toast after it closed.

use std::path::PathBuf;

use daruda_flow::input::{InputDecl, InputType, InputValue, InputValues};
use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, SharedString, WeakEntity, Window,
};

use crate::surface::strings as s;
use crate::ui::select::{self, SelectOption, SelectState};
use crate::ui::{InputState, TextareaState, WindowExt as _};
use crate::workspace::Workspace;
use crate::workspace::flow_request::FlowSelection;
use daruda_store::project::LaneRef;

/// A start that has every answer but its input values.
#[derive(Clone)]
pub(in crate::workspace) struct PendingFlowRun {
    pub lane: LaneRef,
    pub path: PathBuf,
    pub profile: Option<String>,
    pub selection: FlowSelection,
}

/// The control one input is answered with.
enum Control {
    /// A string or a number, typed on one line.
    Text(Entity<InputState>),
    /// A string declared multiline: Enter inserts a newline.
    Prose(Entity<TextareaState>),
    Toggle(bool),
    Choice(Entity<SelectState>),
}

struct Field {
    decl: InputDecl,
    control: Control,
    /// Why the last submit refused this field's value.
    error: Option<SharedString>,
}

pub(in crate::workspace) struct FlowInputsModal {
    pending: PendingFlowRun,
    fields: Vec<Field>,
    workspace: WeakEntity<Workspace>,
    panel_focus_handle: FocusHandle,
}

impl FlowInputsModal {
    /// Ask for the values of `decls` before `pending` runs, and return the
    /// form that went up.
    pub(in crate::workspace) fn open(
        workspace: WeakEntity<Workspace>,
        pending: PendingFlowRun,
        decls: Vec<InputDecl>,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<FlowInputsModal> {
        let title = s::flow::inputs_title(crate::workspace::flow_paths::flow_label(&pending.path));
        crate::workspace::dialog_helpers::open_form_modal::<FlowInputsModal, _>(
            title,
            None,
            move |window, cx| {
                let fields = decls
                    .into_iter()
                    .map(|decl| Field {
                        control: control_for(&decl, window, cx),
                        decl,
                        error: None,
                    })
                    .collect();
                FlowInputsModal {
                    pending,
                    fields,
                    workspace,
                    panel_focus_handle: cx.focus_handle(),
                }
            },
            window,
            cx,
        )
    }

    fn set_toggle(&mut self, ix: usize, checked: bool, cx: &mut Context<Self>) {
        if let Some(Field {
            control: Control::Toggle(value),
            ..
        }) = self.fields.get_mut(ix)
        {
            *value = checked;
            cx.notify();
        }
    }

    /// The values as the form holds them. A blank field is passed on blank:
    /// the engine keeps it for a string and applies the default otherwise.
    fn values(&self, cx: &App) -> InputValues {
        self.fields
            .iter()
            .filter_map(|field| {
                let value = match &field.control {
                    Control::Text(state) => InputValue::Text(state.read(cx).value().to_string()),
                    Control::Prose(state) => InputValue::Text(state.read(cx).value().to_string()),
                    Control::Toggle(checked) => InputValue::Bool(*checked),
                    Control::Choice(state) => {
                        InputValue::Text(state.read(cx).selected_value()?.to_string())
                    }
                };
                Some((field.decl.name.clone(), value))
            })
            .collect()
    }

    /// Run with these values, or show beside each field why not.
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let decls: Vec<InputDecl> = self.fields.iter().map(|f| f.decl.clone()).collect();
        let values = self.values(cx);
        if let Err(issues) = daruda_flow::input::resolve(&decls, &values) {
            for field in &mut self.fields {
                field.error = issues
                    .iter()
                    .find(|issue| issue_input(&issue.kind) == Some(field.decl.name.as_str()))
                    .map(|issue| s::flow::issue(&issue.kind).into());
            }
            cx.notify();
            return;
        }
        let mut pending = self.pending.clone();
        pending.selection.inputs = Some(values);
        window.close_dialog(cx);
        if let Err(e) = self
            .workspace
            .update(cx, |ws, cx| ws.run_flow_with_inputs(pending, window, cx))
        {
            daruda_store::observability::log_writer::LogWriter::log(
                daruda_store::observability::error_report::ErrorReport::new(
                    "Flow inputs modal: workspace gone while starting the run",
                )
                .severity(daruda_store::observability::error_report::ErrorSeverity::Warning)
                .at(file!(), line!())
                .with_context("error", format!("{e}"))
                .dedup("flow.inputs_modal.run")
                .build(),
            );
        }
    }
}

/// The control `decl` is answered with, holding its default.
fn control_for(
    decl: &InputDecl,
    window: &mut Window,
    cx: &mut Context<FlowInputsModal>,
) -> Control {
    let default = decl.default.as_ref().and_then(|value| value.coerce(decl));
    match decl.kind {
        InputType::Boolean => Control::Toggle(matches!(default, Some(InputValue::Bool(true)))),
        InputType::Choice => {
            let options = decl
                .choices
                .iter()
                .cloned()
                .map(SelectOption::simple)
                .collect();
            let initial = default.map(|value| SharedString::from(value.as_text()));
            Control::Choice(
                cx.new(|cx| select::state_with_options(options, initial.as_ref(), window, cx)),
            )
        }
        InputType::String | InputType::Number => {
            let text = default.map(|value| value.as_text()).unwrap_or_default();
            let multiline = decl.multiline && decl.kind == InputType::String;
            if multiline {
                Control::Prose(cx.new(|cx| {
                    TextareaState::new(window, cx)
                        .auto_grow(3, 8)
                        .default_value(text)
                }))
            } else {
                Control::Text(cx.new(|cx| InputState::new(window, cx).default_value(text)))
            }
        }
    }
}

/// The input a value check is about, when it is about one.
fn issue_input(kind: &daruda_flow::error::ValidationKind) -> Option<&str> {
    use daruda_flow::error::ValidationKind as K;
    match kind {
        K::InputMissing { name } | K::InputNotDeclared { name } | K::InputValueInvalid { name } => {
            Some(name)
        }
        _ => None,
    }
}

impl Focusable for FlowInputsModal {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        // The first field, so typing starts there.
        match self.fields.first().map(|field| &field.control) {
            Some(Control::Text(state)) => state.focus_handle(cx),
            Some(Control::Prose(state)) => state.focus_handle(cx),
            Some(Control::Choice(state)) => state.focus_handle(cx),
            _ => self.panel_focus_handle.clone(),
        }
    }
}

mod render;

#[cfg(test)]
mod tests;
