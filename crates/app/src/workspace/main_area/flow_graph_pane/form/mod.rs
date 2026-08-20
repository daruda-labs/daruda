//! The selected node's fields, as text boxes and as plain values.
//!
//! Built from the **file's** node (`daruda_flow::parse::NodeFile`) rather than
//! from the resolved model the graph draws: what is saved goes back through
//! `FlowFile`, so what is shown has to be the same shape. The resolved model has
//! already merged `defaults` in, and showing that would put values into the file
//! that the file deliberately does not repeat.
//!
//! Nothing here knows YAML. A save hands [`NodeFields`] to
//! `Workspace::save_node_form`, which turns it into a `FlowFile` mutation and
//! lets `flow_edit` work out the text edits.
//!
//! Four files, because there are four jobs: what a field's contents *mean*
//! ([`fields`], GPUI-free), the boxes holding them (here — every one is an
//! entity, so every line here needs a window), the column that draws them
//! ([`render`]), and the mapping onto a `FlowFile` ([`apply`]) — which is where
//! a rename's and a deletion's reference sweeps live, since both are that
//! mapping and not the view.

pub(in crate::workspace) mod apply;
pub(in crate::workspace) mod fields;
pub(in crate::workspace) mod notes;
mod render;

pub(super) use render::{render, render_empty, render_many, render_no_nodes};
// Re-exported so every caller keeps saying `form::NodeFields`: which file a
// value's declaration sits in is not their business.
pub(in crate::workspace) use fields::*;

use gpui::{App, AppContext as _, Context, Entity, Window};

use crate::ui::InputState;
use daruda_flow::NodeId;
use daruda_flow::parse::PermissionPolicyFile;

/// The boxes behind [`AgentFields`].
pub(in crate::workspace) struct AgentStates {
    pub id: Entity<InputState>,
    pub model: Entity<InputState>,
    pub effort: Entity<InputState>,
    pub mode: Entity<InputState>,
    pub permission: Entity<crate::ui::select::SelectState>,
}

/// The boxes behind a [`SourceField`]: both alive at once, so switching to the
/// file and back does not lose the prose that was typed.
pub(in crate::workspace) struct SourceStates {
    /// Which of the two: [`SOURCE_INLINE`] or [`SOURCE_FILE`].
    pub choice: Entity<crate::ui::select::SelectState>,
    pub inline: Entity<InputState>,
    pub file: Entity<InputState>,
}

/// The source select's two values.
const SOURCE_INLINE: &str = "inline";
const SOURCE_FILE: &str = "file";

struct Body {
    /// [`KIND_AGENT`] or [`KIND_COMMAND`].
    kind: Entity<crate::ui::select::SelectState>,
    prompt: SourceStates,
    output: Entity<InputState>,
    run: Entity<InputState>,
    /// One set of boxes for both policies — which one is written depends on the
    /// kind, and `read_fail` is told which by the caller.
    on_fail: FailStates,
}

/// The kind select's two values.
const KIND_AGENT: &str = "agent";
const KIND_COMMAND: &str = "command";

/// The boxes behind [`FailFields`]. All of them exist whichever policy is
/// selected — switching to `retry` and back must not lose what was typed, and
/// the select is what decides which are read.
pub(in crate::workspace) struct FailStates {
    /// Which policy: [`FAIL_HALT`] or [`FAIL_ACT`].
    pub policy: Entity<crate::ui::select::SelectState>,
    /// A retry's hint — prose, or the file that holds it.
    pub hint: SourceStates,
    /// A repair's fix prompt.
    pub fix: Entity<InputState>,
    pub rerun: Entity<InputState>,
    pub max_attempts: Entity<InputState>,
    pub wait: Entity<InputState>,
}

/// The select's two values. `act` rather than `retry`/`repair` because the same
/// select serves both kinds and the label is what differs.
const FAIL_HALT: &str = "halt";
const FAIL_ACT: &str = "act";

pub(in crate::workspace) struct NodeForm {
    /// The node this form was built for, by the id the **file** holds. The
    /// rename case is why this is kept separately from the `id` field.
    pub(in crate::workspace) node: NodeId,
    /// What it was built from — what dirty compares against.
    initial: NodeFields,
    id: Entity<InputState>,
    deps: Entity<InputState>,
    timeout: Entity<InputState>,
    cwd: Entity<InputState>,
    agent: AgentStates,
    body: Body,
    /// Whether the override block is open. Closed by default — most nodes take
    /// `defaults` and have nothing here to look at.
    agent_open: bool,
    /// The same refusal, split out to the boxes it is about. Empty when nothing
    /// was refused, or when what was refused is about the flow rather than a box.
    notes: Vec<notes::FieldNote>,
    /// Why the last save did not reach the file, in the words whoever refused it
    /// used. Held here rather than toasted: with fifteen fields, "the flow would
    /// not load" is only useful next to the fields it is about.
    banner: Option<String>,
}

impl NodeForm {
    /// Build a form for `node` out of the file's own text. `None` when the text
    /// does not parse or holds no such node — both mean there is nothing to
    /// show, and the pane is already saying why.
    pub(super) fn build<T: 'static>(
        text: &str,
        node: &NodeId,
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Option<Self> {
        let file = daruda_flow::parse::parse_flow_file(text).ok()?;
        let found = file.nodes.iter().find(|n| &n.id == node)?;
        let initial = fields_of(found);
        let agent_open = !initial.agent.is_empty();

        let id = single_line(&initial.id, window, cx);
        let deps = single_line(&initial.deps.join(", "), window, cx);
        let timeout = single_line(&timeout_text(&initial.timeout), window, cx);
        let cwd = single_line(initial.cwd.as_deref().unwrap_or_default(), window, cx);
        let agent = AgentStates {
            id: single_line(initial.agent.id.as_deref().unwrap_or_default(), window, cx),
            model: single_line(
                initial.agent.model.as_deref().unwrap_or_default(),
                window,
                cx,
            ),
            effort: single_line(
                initial.agent.effort.as_deref().unwrap_or_default(),
                window,
                cx,
            ),
            mode: single_line(
                initial.agent.mode.as_deref().unwrap_or_default(),
                window,
                cx,
            ),
            permission: permission_select(initial.agent.permission, window, cx),
        };
        let body = Body {
            kind: kind_select(initial.body.kind, window, cx),
            prompt: source_states(&initial.body.prompt, window, cx),
            output: single_line(&initial.body.output, window, cx),
            run: single_line(&initial.body.run, window, cx),
            on_fail: fail_states(
                &initial.body.on_fail,
                fail_kind_of(initial.body.kind),
                window,
                cx,
            ),
        };
        Some(Self {
            node: node.clone(),
            initial,
            id,
            deps,
            timeout,
            cwd,
            agent,
            body,
            // Opened when the node already overrides something — there is
            // something to see — and closed when it does not.
            agent_open,
            banner: None,
            notes: Vec::new(),
        })
    }

    /// What the boxes say now.
    pub(in crate::workspace) fn fields(&self, cx: &App) -> NodeFields {
        NodeFields {
            id: read(&self.id, cx).trim().to_string(),
            deps: parse_deps(&read(&self.deps, cx)),
            timeout: parse_timeout(read(&self.timeout, cx).trim()),
            cwd: some_if_filled(read(&self.cwd, cx).trim()),
            agent: AgentFields {
                id: some_if_filled(read(&self.agent.id, cx).trim()),
                model: some_if_filled(read(&self.agent.model, cx).trim()),
                effort: some_if_filled(read(&self.agent.effort, cx).trim()),
                mode: some_if_filled(read(&self.agent.mode, cx).trim()),
                permission: selected_permission(&self.agent.permission, cx),
            },
            body: {
                let kind = selected_kind(&self.body.kind, cx);
                BodyFields {
                    kind,
                    prompt: read_source(&self.body.prompt, cx),
                    output: read(&self.body.output, cx).trim().to_string(),
                    run: read(&self.body.run, cx).trim().to_string(),
                    on_fail: read_fail(&self.body.on_fail, fail_kind_of(kind), cx),
                }
            },
        }
    }

    /// Is there anything to save? Compared against the values the form was built
    /// from rather than tracked as a flag: a person who types and then types the
    /// value back has changed nothing.
    pub(in crate::workspace) fn is_dirty(&self, cx: &App) -> bool {
        self.fields(cx) != self.initial
    }

    /// Why the form cannot be saved as it stands, if it cannot.
    ///
    /// Two things only, and both are things `daruda_flow::load` never gets to
    /// see: a node with no id cannot be written, and text that is not a duration
    /// cannot become one. Every other rule stays the engine's (D6).
    pub(in crate::workspace) fn refusal(&self, cx: &App) -> Option<Refusal> {
        let fields = self.fields(cx);
        if fields.id.is_empty() {
            return Some(Refusal::EmptyId);
        }
        if let TimeoutField::Unreadable(text) = fields.timeout {
            return Some(Refusal::Timeout(text));
        }
        // The acting policy's two numbers, for the same reason: neither can
        // become what the file needs, so the engine never sees them.
        // An agent has to write somewhere and a command has to run something.
        // Neither is the engine's rule — it loads an empty `output` and an empty
        // `run` without complaint — and neither is a node.
        match fields.body.kind {
            KindChoice::Agent if fields.body.output.is_empty() => {
                return Some(Refusal::OutputRequired);
            }
            KindChoice::Command if fields.body.run.is_empty() => {
                return Some(Refusal::RunRequired);
            }
            _ => {}
        }
        let on_fail = &fields.body.on_fail;
        match on_fail {
            FailFields::Halt => None,
            FailFields::Retry {
                max_attempts, wait, ..
            }
            | FailFields::Repair {
                max_attempts, wait, ..
            } => {
                if let AttemptsField::Unreadable(text) = max_attempts {
                    return Some(Refusal::Attempts(text.clone()));
                }
                if let TimeoutField::Unreadable(text) = wait {
                    return Some(Refusal::Timeout(text.clone()));
                }
                None
            }
        }
    }

    /// What to say about the last save, if it was refused.
    pub(in crate::workspace) fn banner(&self) -> Option<&str> {
        self.banner.as_deref()
    }

    /// Put a refusal on the form, or clear it. Cleared at the start of every
    /// save attempt, so what is shown is always about the latest one.
    pub(in crate::workspace) fn set_banner(&mut self, message: Option<String>) {
        self.banner = message;
    }

    /// The same refusal, pinned to the boxes it names.
    ///
    /// A note about the agent override opens that block: it is closed by default,
    /// and a capture found the pointer hidden behind it — which is the same as
    /// not pointing.
    pub(in crate::workspace) fn set_notes(&mut self, notes: Vec<notes::FieldNote>) {
        if notes
            .iter()
            .any(|note| note.field == notes::FormField::Agent)
        {
            self.agent_open = true;
        }
        self.notes = notes;
    }

    /// What to say under `field`, for a test that is about where a refusal
    /// lands rather than about how it looks.
    #[cfg(test)]
    pub(in crate::workspace) fn note_for_test(&self, field: notes::FormField) -> Option<&str> {
        self.note_for(field)
    }

    /// What to say under `field`, if anything.
    pub(super) fn note_for(&self, field: notes::FormField) -> Option<&str> {
        self.notes
            .iter()
            .find(|note| note.field == field)
            .map(|note| note.message.as_str())
    }

    pub(in crate::workspace) fn cwd_state(&self) -> &Entity<InputState> {
        &self.cwd
    }

    pub(in crate::workspace) fn agent_states(&self) -> &AgentStates {
        &self.agent
    }

    pub(in crate::workspace) fn agent_open(&self) -> bool {
        self.agent_open
    }

    pub(in crate::workspace) fn toggle_agent_open(&mut self) {
        self.agent_open = !self.agent_open;
    }

    pub(in crate::workspace) fn id_state(&self) -> &Entity<InputState> {
        &self.id
    }

    pub(in crate::workspace) fn deps_state(&self) -> &Entity<InputState> {
        &self.deps
    }

    pub(in crate::workspace) fn timeout_state(&self) -> &Entity<InputState> {
        &self.timeout
    }

    pub(in crate::workspace) fn body_states(&self, cx: &App) -> BodyStates<'_> {
        BodyStates {
            kind_select: &self.body.kind,
            kind: selected_kind(&self.body.kind, cx),
            prompt: &self.body.prompt,
            output: &self.body.output,
            run: &self.body.run,
            on_fail: &self.body.on_fail,
        }
    }
}

/// What the inspector has to draw: the kind select, and the boxes for the kind
/// it currently names.
pub(in crate::workspace) struct BodyStates<'a> {
    pub kind_select: &'a Entity<crate::ui::select::SelectState>,
    pub kind: KindChoice,
    pub prompt: &'a SourceStates,
    pub output: &'a Entity<InputState>,
    pub run: &'a Entity<InputState>,
    pub on_fail: &'a FailStates,
}

/// The kind select, holding what the node is now.
fn kind_select<T: 'static>(
    kind: KindChoice,
    window: &mut Window,
    cx: &mut Context<T>,
) -> Entity<crate::ui::select::SelectState> {
    use crate::ui::select::{SelectOption, state_with_options};
    let options = vec![
        SelectOption::new(KIND_AGENT, crate::surface::strings::flow_form_kind_agent()),
        SelectOption::new(
            KIND_COMMAND,
            crate::surface::strings::flow_form_kind_command(),
        ),
    ];
    let selected = gpui::SharedString::from(match kind {
        KindChoice::Agent => KIND_AGENT,
        KindChoice::Command => KIND_COMMAND,
    });
    cx.new(|cx| state_with_options(options, Some(&selected), window, cx))
}

fn selected_kind(state: &Entity<crate::ui::select::SelectState>, cx: &App) -> KindChoice {
    match state.read(cx).selected_value().map(|v| v.to_string()) {
        Some(v) if v == KIND_COMMAND => KindChoice::Command,
        _ => KindChoice::Agent,
    }
}

/// Boxes for both halves of an either-or, plus the select that says which one
/// the file names.
fn source_states<T: 'static>(
    field: &SourceField,
    window: &mut Window,
    cx: &mut Context<T>,
) -> SourceStates {
    use crate::ui::select::{SelectOption, state_with_options};
    let (inline, file, selected) = match field {
        SourceField::Inline(text) => (text.clone(), String::new(), SOURCE_INLINE),
        SourceField::File(path) => (String::new(), path.clone(), SOURCE_FILE),
    };
    let options = vec![
        SelectOption::new(
            SOURCE_INLINE,
            crate::surface::strings::flow_form_source_inline(),
        ),
        SelectOption::new(
            SOURCE_FILE,
            crate::surface::strings::flow_form_source_file(),
        ),
    ];
    let selected = gpui::SharedString::from(selected);
    SourceStates {
        choice: cx.new(|cx| state_with_options(options, Some(&selected), window, cx)),
        inline: multi_line(&inline, window, cx),
        file: single_line(&file, window, cx),
    }
}

/// Whichever half the select names. The other box keeps what was typed in it,
/// and the file never sees both.
fn read_source(states: &SourceStates, cx: &App) -> SourceField {
    if is_file_source(states, cx) {
        SourceField::File(read(&states.file, cx).trim().to_string())
    } else {
        SourceField::Inline(read(&states.inline, cx))
    }
}

/// Is the file half selected?
pub(super) fn is_file_source(states: &SourceStates, cx: &App) -> bool {
    states
        .choice
        .read(cx)
        .selected_value()
        .is_some_and(|v| v.as_ref() == SOURCE_FILE)
}

/// Boxes for whichever policy the node holds, and empty ones for the other — so
/// switching the select and switching back does not lose what was typed.
fn fail_states<T: 'static>(
    fields: &FailFields,
    kind: FailKind,
    window: &mut Window,
    cx: &mut Context<T>,
) -> FailStates {
    use crate::ui::select::{SelectOption, state_with_options};
    let empty = SourceField::Inline(String::new());
    let (hint, fix, rerun, attempts, wait, selected) = match fields {
        FailFields::Halt => (
            &empty,
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            FAIL_HALT,
        ),
        FailFields::Retry {
            hint,
            max_attempts,
            wait,
        } => (
            hint,
            String::new(),
            String::new(),
            attempts_text(max_attempts),
            timeout_text(wait),
            FAIL_ACT,
        ),
        FailFields::Repair {
            fix,
            rerun,
            max_attempts,
            wait,
        } => (
            &empty,
            fix.clone(),
            rerun.join(", "),
            attempts_text(max_attempts),
            timeout_text(wait),
            FAIL_ACT,
        ),
    };
    let options = vec![
        SelectOption::new(FAIL_HALT, crate::surface::strings::flow_form_fail_halt()),
        SelectOption::new(
            FAIL_ACT,
            match kind {
                FailKind::Retry => crate::surface::strings::flow_form_fail_retry(),
                FailKind::Repair => crate::surface::strings::flow_form_fail_repair(),
            },
        ),
    ];
    let selected = gpui::SharedString::from(selected);
    FailStates {
        policy: cx.new(|cx| state_with_options(options, Some(&selected), window, cx)),
        hint: source_states(hint, window, cx),
        fix: multi_line(&fix, window, cx),
        rerun: single_line(&rerun, window, cx),
        max_attempts: single_line(&attempts, window, cx),
        wait: single_line(&wait, window, cx),
    }
}

/// Is the acting policy (retry / repair) the one selected? Read from the select
/// rather than mirrored, so there is one answer.
pub(super) fn acting(states: &FailStates, cx: &App) -> bool {
    states
        .policy
        .read(cx)
        .selected_value()
        .is_some_and(|v| v.as_ref() == FAIL_ACT)
}

fn read_fail(states: &FailStates, kind: FailKind, cx: &App) -> FailFields {
    if !acting(states, cx) {
        return FailFields::Halt;
    }
    let max_attempts = parse_attempts(read(&states.max_attempts, cx).trim());
    let wait = parse_timeout(read(&states.wait, cx).trim());
    match kind {
        FailKind::Retry => FailFields::Retry {
            hint: read_source(&states.hint, cx),
            max_attempts,
            wait,
        },
        FailKind::Repair => FailFields::Repair {
            fix: read(&states.fix, cx),
            rerun: parse_deps(&read(&states.rerun, cx)),
            max_attempts,
            wait,
        },
    }
}

/// The permission select's options: absent first, then the three the engine
/// knows. Values are the serde spellings, so the option a file already names
/// selects itself.
fn permission_select<T: 'static>(
    initial: Option<PermissionPolicyFile>,
    window: &mut Window,
    cx: &mut Context<T>,
) -> Entity<crate::ui::select::SelectState> {
    use crate::ui::select::{SelectOption, state_with_options};
    let options = vec![
        SelectOption::new(
            PERMISSION_ABSENT,
            crate::surface::strings::flow_form_absent(),
        ),
        SelectOption::new("deny", crate::surface::strings::flow_form_permission_deny()),
        SelectOption::new(
            "allow_once",
            crate::surface::strings::flow_form_permission_allow_once(),
        ),
        SelectOption::new("ask", crate::surface::strings::flow_form_permission_ask()),
    ];
    let initial = gpui::SharedString::from(permission_value(initial));
    cx.new(|cx| state_with_options(options, Some(&initial), window, cx))
}

/// What the select holds for "the node names no permission". The empty string
/// would be indistinguishable from "nothing selected yet".
const PERMISSION_ABSENT: &str = "-";

fn permission_value(policy: Option<PermissionPolicyFile>) -> &'static str {
    match policy {
        None => PERMISSION_ABSENT,
        Some(PermissionPolicyFile::Deny) => "deny",
        Some(PermissionPolicyFile::AllowOnce) => "allow_once",
        Some(PermissionPolicyFile::Ask) => "ask",
    }
}

fn selected_permission(
    state: &Entity<crate::ui::select::SelectState>,
    cx: &App,
) -> Option<PermissionPolicyFile> {
    match state.read(cx).selected_value().map(|v| v.to_string()) {
        Some(v) if v == "deny" => Some(PermissionPolicyFile::Deny),
        Some(v) if v == "allow_once" => Some(PermissionPolicyFile::AllowOnce),
        Some(v) if v == "ask" => Some(PermissionPolicyFile::Ask),
        _ => None,
    }
}

fn single_line<T: 'static>(
    value: &str,
    window: &mut Window,
    cx: &mut Context<T>,
) -> Entity<InputState> {
    let value = value.to_string();
    cx.new(|cx| InputState::new(window, cx).default_value(value))
}

/// A prompt box: prose, a fixed visible height, scrolling inside itself.
///
/// Through `make_markdown_prose_state` rather than a hand-rolled
/// `multi_line(true).rows(n)` — that combination renders one line high (seen in a
/// capture), and `auto_grow(rows, rows)` is what the app's other prose boxes use.
fn multi_line<T: 'static>(
    value: &str,
    window: &mut Window,
    cx: &mut Context<T>,
) -> Entity<InputState> {
    crate::ui::make_markdown_prose_state(
        value,
        "",
        crate::ui::theme::palette::FLOW_INSPECTOR_PROMPT_ROWS,
        window,
        cx,
    )
}

fn read(state: &Entity<InputState>, cx: &App) -> String {
    state.read(cx).value().to_string()
}
