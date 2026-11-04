//! The Task editor's choice of a flow to run the task as: which flow, under
//! which profile, and which of its inputs the prompt fills.
//!
//! The flows offered are the ones of the lane the task runs in or branches
//! from — where Save and Start look the name up — so the list follows the
//! location pickers. The profile and input pickers are filled from the picked
//! flow's own declarations.

use std::path::{Path, PathBuf};

use daruda_store::tasks::{TaskAgentSurface, TaskFlow, TaskRunIn};
use gpui::{AppContext as _, Context, Entity, SharedString, Subscription, Window};

use super::TaskEditorId;
use super::state::{RunInChoice, TaskEditContent};
use crate::ui::select::{SelectOption, SelectState, state_with_options};
use crate::workspace::Workspace;
use crate::workspace::flow_declared::declared_flow;
use crate::workspace::flow_paths::{FoundFlow, flow_label, names_flow};

/// What drives the task: one agent, on the editor's surface, or a flow.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::workspace) enum RunAs {
    #[default]
    Agent,
    Flow,
}

/// One of the "Run on" choices, as a click names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum Runner {
    Agent(TaskAgentSurface),
    Flow,
}

/// The three pickers. Always built, so switching to Flow and back keeps the
/// picks; only read when the task runs as a flow.
pub(in crate::workspace) struct FlowChoice {
    pub(in crate::workspace) flow: Entity<SelectState>,
    pub(in crate::workspace) profile: Entity<SelectState>,
    pub(in crate::workspace) input: Entity<SelectState>,
    /// Whether the picked flow declares any, so the picker is worth showing.
    pub(in crate::workspace::pages::tasks) has_profiles: bool,
    pub(in crate::workspace::pages::tasks) has_inputs: bool,
}

/// The pickers' values, for Save and the dirty check. `""` is no pick; for
/// the input, the automatic choice.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::workspace) struct FlowChoiceValues {
    pub(in crate::workspace::pages::tasks) flow: String,
    pub(in crate::workspace::pages::tasks) profile: String,
    pub(in crate::workspace::pages::tasks) input: String,
}

impl FlowChoiceValues {
    /// The flow the task runs as; `None` until one is picked.
    pub(in crate::workspace) fn task_flow(&self) -> Option<TaskFlow> {
        let picked = |value: &str| (!value.is_empty()).then(|| value.to_owned());
        Some(TaskFlow {
            name: picked(&self.flow)?,
            profile: picked(&self.profile),
            input: picked(&self.input),
        })
    }
}

impl FlowChoice {
    pub(in crate::workspace) fn values(&self, cx: &gpui::App) -> FlowChoiceValues {
        FlowChoiceValues {
            flow: picked(&self.flow, cx),
            profile: picked(&self.profile, cx),
            input: picked(&self.input, cx),
        }
    }
}

fn picked(state: &Entity<SelectState>, cx: &gpui::App) -> String {
    state
        .read(cx)
        .selected_value()
        .map(ToString::to_string)
        .unwrap_or_default()
}

/// Where the editor's location pickers place the task, as Save reads them.
fn editor_place(te: &TaskEditContent, cx: &gpui::App) -> (TaskRunIn, Option<PathBuf>) {
    match te.run_in {
        RunInChoice::ExistingLane => (
            TaskRunIn::ExistingLane {
                path: PathBuf::from(te.lane_value(cx)),
            },
            None,
        ),
        RunInChoice::NewWorktree => {
            let base = picked(&te.base_select, cx);
            (
                TaskRunIn::NewWorktree,
                (!base.is_empty()).then(|| PathBuf::from(base)),
            )
        }
    }
}

/// The options for a list of flows, each by its file name.
fn flow_options(flows: &[FoundFlow]) -> Vec<SelectOption> {
    flows
        .iter()
        .map(|found| SelectOption::simple(flow_label(&found.path)))
        .collect()
}

/// The flow `name` means among `flows`, as Start would find it.
fn named<'a>(flows: &'a [FoundFlow], name: &str) -> Option<&'a FoundFlow> {
    flows.iter().find(|found| names_flow(&found.path, name))
}

/// What the profile and input pickers offer for the flow at `path`.
struct Declared {
    profiles: Vec<SelectOption>,
    inputs: Vec<SelectOption>,
}

fn declared_options(path: Option<&Path>) -> Declared {
    let (profiles, decls) = path.map(declared_flow).unwrap_or_default();
    let mut inputs = Vec::with_capacity(decls.len() + 1);
    if !decls.is_empty() {
        inputs.push(SelectOption::new(
            "",
            crate::surface::strings::task::edit_flow_input_auto(),
        ));
    }
    inputs.extend(
        decls
            .into_iter()
            .map(|decl| SelectOption::simple(decl.name)),
    );
    Declared {
        profiles: profiles.into_iter().map(SelectOption::simple).collect(),
        inputs,
    }
}

impl Workspace {
    /// The pickers for an editor of a task in `project`, placed per
    /// `place`, showing `initial`.
    pub(super) fn build_flow_choice(
        &self,
        editor_id: TaskEditorId,
        project: Option<daruda_store::project::ProjectUuid>,
        place: (&TaskRunIn, Option<&Path>),
        initial: Option<&TaskFlow>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (FlowChoice, Vec<Subscription>) {
        let flows = self.task_flow_choices(project, place.0, place.1);
        let found = initial.and_then(|flow| named(&flows, &flow.name));
        let declared = declared_options(found.map(|found| found.path.as_path()));
        let (has_profiles, has_inputs) =
            (!declared.profiles.is_empty(), !declared.inputs.is_empty());
        // The saved name may be a stem; the picker offers file names.
        let flow_pick = found.map(|found| SharedString::from(flow_label(&found.path)));
        let profile_pick = initial
            .and_then(|flow| flow.profile.clone())
            .map(SharedString::from);
        let input_pick = Some(SharedString::from(
            initial
                .and_then(|flow| flow.input.clone())
                .unwrap_or_default(),
        ));
        let options = flow_options(&flows);
        let flow = cx.new(|cx| state_with_options(options, flow_pick.as_ref(), window, cx));
        let profile =
            cx.new(|cx| state_with_options(declared.profiles, profile_pick.as_ref(), window, cx));
        let input =
            cx.new(|cx| state_with_options(declared.inputs, input_pick.as_ref(), window, cx));
        let flow_sub = cx.subscribe_in(
            &flow,
            window,
            move |this, _, ev: &crate::ui::select::ConfirmEvent, window, cx| {
                if matches!(ev, crate::ui::select::SelectEvent::Confirm(_)) {
                    this.on_task_edit_flow_changed(editor_id, window, cx);
                }
            },
        );
        let notify = |state: &Entity<SelectState>, cx: &mut Context<Self>| {
            cx.subscribe_in(
                state,
                window,
                move |_, _, ev: &crate::ui::select::ConfirmEvent, _, cx| {
                    if matches!(ev, crate::ui::select::SelectEvent::Confirm(_)) {
                        cx.notify();
                    }
                },
            )
        };
        let subscriptions = vec![flow_sub, notify(&profile, cx), notify(&input, cx)];
        (
            FlowChoice {
                flow,
                profile,
                input,
                has_profiles,
                has_inputs,
            },
            subscriptions,
        )
    }

    /// The flows the editor's task can run as, where it is placed now.
    fn editor_flows(&self, editor_id: TaskEditorId, cx: &gpui::App) -> Vec<FoundFlow> {
        let Some(te) = self.task_editor(editor_id) else {
            return Vec::new();
        };
        let (run_in, base) = editor_place(te, cx);
        self.task_flow_choices(te.project(cx), &run_in, base.as_deref())
    }

    /// Another flow was picked: offer its profiles, the first one picked, and
    /// its inputs, the automatic choice picked.
    pub(super) fn on_task_edit_flow_changed(
        &mut self,
        editor_id: TaskEditorId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(te) = self.task_editor(editor_id) else {
            return;
        };
        let picked = te.flow_choice.values(cx).flow;
        let path = named(&self.editor_flows(editor_id, cx), &picked).map(|f| f.path.clone());
        self.refill_flow_declarations(editor_id, path.as_deref(), false, window, cx);
    }

    /// The task's project or location changed: offer the flows found there.
    /// A pick still offered stays, with its profile and input.
    pub(super) fn refill_task_flow_choices(
        &mut self,
        editor_id: TaskEditorId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(te) = self.task_editor(editor_id) else {
            return;
        };
        let state = te.flow_choice.flow.clone();
        let current = te.flow_choice.values(cx).flow;
        let flows = self.editor_flows(editor_id, cx);
        let kept = named(&flows, &current).map(|found| found.path.clone());
        let options = flow_options(&flows);
        state.update(cx, |state, cx| {
            state.set_items(options, window, cx);
            match &kept {
                Some(path) => state.set_selected_value(&flow_label(path).into(), window, cx),
                None => state.set_selected_index(None, window, cx),
            }
        });
        self.refill_flow_declarations(editor_id, kept.as_deref(), kept.is_some(), window, cx);
    }

    /// Fill the profile and input pickers from the flow at `path`. `keep`
    /// leaves a pick that is still offered; otherwise the first profile and
    /// the automatic input are picked.
    fn refill_flow_declarations(
        &mut self,
        editor_id: TaskEditorId,
        path: Option<&Path>,
        keep: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(te) = self.task_editor_mut(editor_id) else {
            return;
        };
        let declared = declared_options(path);
        te.flow_choice.has_profiles = !declared.profiles.is_empty();
        te.flow_choice.has_inputs = !declared.inputs.is_empty();
        let (profile, input) = (te.flow_choice.profile.clone(), te.flow_choice.input.clone());
        let offered = |options: &[SelectOption], value: String| {
            options
                .iter()
                .any(|option| option.value.as_ref() == value)
                .then(|| SharedString::from(value))
        };
        let profile_pick = keep
            .then(|| offered(&declared.profiles, picked(&profile, cx)))
            .flatten()
            .or_else(|| declared.profiles.first().map(|option| option.value.clone()));
        let input_pick = keep
            .then(|| offered(&declared.inputs, picked(&input, cx)))
            .flatten()
            .unwrap_or_default();
        profile.update(cx, |state, cx| {
            state.set_items(declared.profiles, window, cx);
            match &profile_pick {
                Some(value) => state.set_selected_value(value, window, cx),
                None => state.set_selected_index(None, window, cx),
            }
        });
        input.update(cx, |state, cx| {
            state.set_items(declared.inputs, window, cx);
            state.set_selected_value(&input_pick, window, cx);
        });
        cx.notify();
    }

    /// One "Run on" click: an agent surface, or a flow.
    pub(super) fn set_task_runner(
        &mut self,
        editor_id: TaskEditorId,
        runner: Runner,
        cx: &mut Context<Self>,
    ) {
        if let Some(te) = self.task_editor_mut(editor_id) {
            match runner {
                Runner::Agent(surface) => {
                    te.run_as = RunAs::Agent;
                    te.agent_surface = surface;
                }
                Runner::Flow => te.run_as = RunAs::Flow,
            }
        }
        cx.notify();
    }

    /// Pick where the task runs, and offer the flows found there.
    pub(super) fn choose_task_run_in(
        &mut self,
        editor_id: TaskEditorId,
        choice: RunInChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_task_run_in(editor_id, choice, cx);
        self.refill_task_flow_choices(editor_id, window, cx);
    }

    /// A location picker committed a pick: offer the flows found there.
    pub(super) fn on_task_edit_place_changed(
        &mut self,
        editor_id: TaskEditorId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.refill_task_flow_choices(editor_id, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::FlowChoiceValues;

    #[test]
    fn a_flow_is_only_set_once_one_is_picked() {
        assert_eq!(FlowChoiceValues::default().task_flow(), None);
        let picked = FlowChoiceValues {
            flow: "review.yaml".into(),
            profile: String::new(),
            input: "pr".into(),
        }
        .task_flow()
        .unwrap();
        assert_eq!(picked.name, "review.yaml");
        assert_eq!(picked.profile, None);
        assert_eq!(picked.input.as_deref(), Some("pr"));
    }
}
