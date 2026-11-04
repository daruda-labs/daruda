//! The flow a task runs as: which file its name means, and which of the
//! flow's inputs the task's prompt fills.
//!
//! Saving a task and starting it ask the same question, so they cannot
//! disagree about whether a task can run: a task the editor accepted is one
//! Start will run, unless the flow file changed in between.

use std::path::{Path, PathBuf};

use daruda_flow::input::{InputDecl, InputValue, InputValues};
use daruda_store::project::ProjectUuid;
use daruda_store::tasks::{TaskFlow, TaskRunIn};

use crate::workspace::Workspace;
use crate::workspace::flow_declared::{ProfileRefusal, declared_flow, profile_refusal};
use crate::workspace::flow_paths::{FlowSources, flow_label, names_flow};

/// Why a task's flow cannot run as the task describes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum TaskFlowProblem {
    /// No flow of that name in the lane the task starts from.
    NotFound,
    /// The flow declares profiles and the task names none.
    NeedsProfile,
    /// The task names a profile the flow does not declare.
    UnknownProfile,
    /// The task names an input the flow does not declare.
    InputNotDeclared,
    /// The task names no input, and the flow has no single one to take it.
    InputAmbiguous,
    /// The flow requires an input besides the prompt's, which a task has no
    /// value for.
    OtherInputsRequired,
}

/// Fixed English, for a control answer; the person gets [`problem_text`].
impl std::fmt::Display for TaskFlowProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotFound => "no flow of that name in the task's worktree",
            Self::NeedsProfile => "the flow declares profiles and the task names none",
            Self::UnknownProfile => "the flow does not declare the task's profile",
            Self::InputNotDeclared => "the flow does not declare the input the task names",
            Self::InputAmbiguous => {
                "the flow has no single input for the prompt; name the one it fills"
            }
            Self::OtherInputsRequired => {
                "the flow requires inputs besides the prompt, which a task cannot give"
            }
        })
    }
}

/// A task's flow, found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) struct TaskFlowTarget {
    pub path: PathBuf,
    /// The file name, as the run's source records it.
    pub label: String,
    pub profile: Option<String>,
    /// The input the prompt fills; `None` when the flow declares none.
    pub input: Option<String>,
    /// Every input the flow declares, as the check above read them, so the
    /// prompt is checked against the declarations it was placed by.
    pub decls: Vec<InputDecl>,
    /// The lane whose flows the name was looked up in.
    pub source_lane: PathBuf,
}

impl TaskFlowTarget {
    /// The values the run is given: the prompt, verbatim, as the one input.
    /// The task's own wrapping (`render_task_prompt`) is for an agent turn; a
    /// command node would receive it as a shell argument.
    pub(in crate::workspace) fn inputs(&self, prompt: &str) -> InputValues {
        self.input
            .iter()
            .map(|name| (name.clone(), InputValue::Text(prompt.to_owned())))
            .collect()
    }
}

impl Workspace {
    /// Find `flow` for a task of `project` that runs per `run_in`.
    ///
    /// A new worktree does not exist yet, so its flows are read from the lane
    /// it branches from (`base`, else the repository's main checkout). An
    /// uncommitted change to a repository flow there is not on the new
    /// branch, but the run carries the flow it was started with.
    pub(in crate::workspace) fn resolve_task_flow(
        &self,
        project: ProjectUuid,
        run_in: &TaskRunIn,
        base: Option<&Path>,
        flow: &TaskFlow,
    ) -> Result<TaskFlowTarget, TaskFlowProblem> {
        let sources = self
            .task_flow_sources(project, run_in, base)
            .ok_or(TaskFlowProblem::NotFound)?;
        let path = sources
            .list_flows()
            .into_iter()
            .find(|found| names_flow(&found.path, &flow.name))
            .map(|found| found.path)
            .ok_or(TaskFlowProblem::NotFound)?;
        let (profiles, decls) = declared_flow(&path);
        match profile_refusal(&profiles, flow.profile.as_deref()) {
            Some(ProfileRefusal::Needed) => return Err(TaskFlowProblem::NeedsProfile),
            Some(ProfileRefusal::Unknown) => return Err(TaskFlowProblem::UnknownProfile),
            None => {}
        }
        let input = prompt_input(&decls, flow.input.as_deref())?;
        Ok(TaskFlowTarget {
            label: flow_label(&path),
            path,
            profile: flow.profile.clone(),
            input,
            decls,
            source_lane: sources.lane,
        })
    }

    /// How an approval names `flow`, run as a task of `project` per
    /// `run_in`: the flow, its profile, and the permission modes its agents
    /// run under. A flow that cannot be found is named as the task names it.
    pub(in crate::workspace) fn task_flow_approval(
        &self,
        project: ProjectUuid,
        run_in: &TaskRunIn,
        base: Option<&Path>,
        flow: &TaskFlow,
    ) -> String {
        use crate::surface::strings::control as s;
        let Ok(target) = self.resolve_task_flow(project, run_in, base, flow) else {
            return s::approval_task_as_flow_unresolved(&flow.name);
        };
        let modes = flow_permission_modes(&target.path, target.profile.as_deref());
        // A flow of commands alone has no agent to give a mode to.
        let modes = if modes.is_empty() {
            s::approval_flow_no_agents()
        } else {
            s::approval_flow_modes(modes.join(", "))
        };
        match &target.profile {
            Some(profile) => s::approval_task_as_flow_profile(&target.label, profile, &modes),
            None => s::approval_task_as_flow(&target.label, &modes),
        }
    }

    /// Show the flow run `task_id` last started, on the Flows page, if it ran
    /// as a flow in a worktree that is still registered.
    pub(in crate::workspace) fn open_task_flow_run(
        &mut self,
        task_id: &str,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some((lane, dir)) = self.task_flow_run(task_id, cx) else {
            return;
        };
        self.open_run_detail(lane, &dir, window, cx);
    }

    /// The worktree and run directory of the flow run `task_id` last started.
    pub(in crate::workspace) fn task_flow_run(
        &self,
        task_id: &str,
        cx: &gpui::App,
    ) -> Option<(daruda_store::project::LaneRef, PathBuf)> {
        let run = cx
            .global::<crate::runs::GlobalTasks>()
            .get(task_id)?
            .execution
            .as_ref()?;
        let dir = crate::runs::task_run_dir(run)?;
        Some((self.lane_ref_at(&run.cwd)?, dir))
    }

    /// The flows a task of `project` can be set to run as where it runs per
    /// `run_in` — the same lane Save and Start look the name up in. A
    /// project with no repository to branch from offers its first lane's.
    pub(in crate::workspace) fn task_flow_choices(
        &self,
        project: Option<ProjectUuid>,
        run_in: &TaskRunIn,
        base: Option<&Path>,
    ) -> Vec<super::flow_paths::FoundFlow> {
        let Some(project) = project else {
            return Vec::new();
        };
        let first_lane = self
            .project_by_uuid(project)
            .and_then(|p| p.lanes.first())
            .map(|lane| lane.path.clone());
        let sources = self.task_flow_sources(project, run_in, base).or_else(|| {
            let path = first_lane?;
            self.task_flow_sources(project, &TaskRunIn::ExistingLane { path }, None)
        });
        sources.map(|s| s.list_flows()).unwrap_or_default()
    }

    /// The flows a task sees: those of the lane it runs in or branches from,
    /// with the project's and the person's own.
    fn task_flow_sources(
        &self,
        project: ProjectUuid,
        run_in: &TaskRunIn,
        base: Option<&Path>,
    ) -> Option<FlowSources> {
        let project_id = self.project_by_uuid(project)?.id;
        let lane = match run_in {
            TaskRunIn::ExistingLane { path } => path.clone(),
            TaskRunIn::NewWorktree => match base {
                Some(base) => base.to_path_buf(),
                None => self.git_repo_root_for_project(project_id)?,
            },
        };
        Some(FlowSources {
            lane,
            project: self
                .project_for(project_id)
                .map(|p| super::flow_paths::project_flows_dir(&self.data_dir, &p.root)),
            global: super::flow_paths::global_flows_dir(&self.data_dir),
        })
    }
}

/// The permission modes the flow's agents run under with `profile`, each
/// once, in node order. A new flow defaults to `bypassPermissions`, so an
/// approval says it rather than leave it to be found out.
pub(in crate::workspace) fn flow_permission_modes(
    path: &Path,
    profile: Option<&str>,
) -> Vec<String> {
    let Some(loaded) = std::fs::read_to_string(path)
        .ok()
        .and_then(|text| daruda_flow::load(&text, profile).ok())
    else {
        return Vec::new();
    };
    let mut modes: Vec<String> = Vec::new();
    for node in &loaded.flow().nodes {
        if let daruda_flow::model::NodeKind::Agent(body) = &node.kind {
            let mode = body.agent.mode.clone().unwrap_or_else(|| "default".into());
            if !modes.contains(&mode) {
                modes.push(mode);
            }
        }
    }
    modes
}

/// Why a task's flow cannot run, in the person's words.
pub(in crate::workspace) fn problem_text(problem: TaskFlowProblem) -> String {
    use crate::surface::strings::task as t;
    match problem {
        TaskFlowProblem::NotFound => t::error_start_flow_missing(),
        TaskFlowProblem::NeedsProfile => t::flow_needs_profile(),
        TaskFlowProblem::UnknownProfile => t::flow_unknown_profile(),
        TaskFlowProblem::InputNotDeclared => t::flow_input_not_declared(),
        TaskFlowProblem::InputAmbiguous => t::flow_input_ambiguous(),
        TaskFlowProblem::OtherInputsRequired => t::flow_other_inputs_required(),
    }
}

/// Which input the prompt fills. Named, it must be declared; unnamed, it is
/// the one required input, or the only input. Either way no other input may
/// be required, since the task has nothing to give it.
fn prompt_input(
    decls: &[InputDecl],
    named: Option<&str>,
) -> Result<Option<String>, TaskFlowProblem> {
    let needs_value = |decl: &&InputDecl| decl.required && decl.default.is_none();
    let target = match named {
        Some(name) => decls
            .iter()
            .find(|decl| decl.name == name)
            .ok_or(TaskFlowProblem::InputNotDeclared)?,
        None if decls.is_empty() => return Ok(None),
        None => {
            let required: Vec<_> = decls.iter().filter(needs_value).collect();
            match (required.as_slice(), decls) {
                ([only], _) => *only,
                ([], [only]) => only,
                _ => return Err(TaskFlowProblem::InputAmbiguous),
            }
        }
    };
    if decls
        .iter()
        .filter(needs_value)
        .any(|decl| decl.name != target.name)
    {
        return Err(TaskFlowProblem::OtherInputsRequired);
    }
    Ok(Some(target.name.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decls(yaml: &str) -> Vec<InputDecl> {
        daruda_flow::parse::parse_flow_file(&format!("version: 1\ninputs:\n{yaml}nodes: []\n"))
            .unwrap()
            .inputs
    }

    #[test]
    fn an_unnamed_input_is_the_one_the_flow_requires() {
        let both = decls("  pr: { required: true }\n  focus: { default: x }\n");
        assert_eq!(prompt_input(&both, None), Ok(Some("pr".into())));
        let only = decls("  pr: {}\n");
        assert_eq!(prompt_input(&only, None), Ok(Some("pr".into())));
        assert_eq!(prompt_input(&[], None), Ok(None));
    }

    #[test]
    fn no_single_input_to_take_the_prompt_is_refused() {
        let two = decls("  pr: { required: true }\n  base: { required: true }\n");
        assert_eq!(
            prompt_input(&two, None),
            Err(TaskFlowProblem::InputAmbiguous)
        );
        let optional = decls("  pr: {}\n  base: {}\n");
        assert_eq!(
            prompt_input(&optional, None),
            Err(TaskFlowProblem::InputAmbiguous)
        );
    }

    #[test]
    fn a_named_input_must_be_declared_and_leave_nothing_else_required() {
        let two = decls("  pr: { required: true }\n  focus: {}\n");
        assert_eq!(
            prompt_input(&two, Some("focus")),
            Err(TaskFlowProblem::OtherInputsRequired)
        );
        assert_eq!(prompt_input(&two, Some("pr")), Ok(Some("pr".into())));
        assert_eq!(
            prompt_input(&two, Some("gone")),
            Err(TaskFlowProblem::InputNotDeclared)
        );
    }

    #[test]
    fn the_prompt_is_given_verbatim() {
        let target = TaskFlowTarget {
            path: PathBuf::from("/f/review.yaml"),
            label: "review.yaml".into(),
            profile: None,
            input: Some("pr".into()),
            decls: Vec::new(),
            source_lane: PathBuf::from("/f"),
        };
        assert_eq!(
            target.inputs("it's 'quoted'"),
            [("pr".to_string(), InputValue::Text("it's 'quoted'".into()))].into()
        );
    }
}
