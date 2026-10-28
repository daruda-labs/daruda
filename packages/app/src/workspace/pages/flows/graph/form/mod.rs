//! Widgets for the selected node. InspectorModel owns input and draft lifetime.
//!
//! `binding` projects named raw values onto widgets and reads user input back;
//! `render` builds the inspector column. The GPUI-free `apply` module retains
//! the existing mapping from editable values to the flow file.

pub(in crate::workspace) mod apply;
mod binding;
pub(in crate::workspace) mod catalog;
mod choice;
mod editor;
pub(in crate::workspace) use super::inspector::fields;
mod file_picker;
mod links;
pub(in crate::workspace) mod notes;
pub(in crate::workspace) mod prompt_edit;
mod render;
#[cfg(feature = "screenshot")]
pub(in crate::workspace) mod screenshot;

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

pub(in crate::workspace) struct FormBinding {
    /// The node this form was built for, by the id the **file** holds. The
    /// rename case is why this is kept separately from the `id` field.
    pub(in crate::workspace) node: NodeId,
    applying: std::rc::Rc<std::cell::Cell<usize>>,
    id: Entity<InputState>,
    deps: Entity<InputState>,
    pub(super) dep_picker: Entity<crate::ui::select::SelectState>,
    pub(super) result_picker: Entity<crate::ui::select::SelectState>,
    timeout: Entity<InputState>,
    max_turns: Entity<InputState>,
    cwd: Entity<InputState>,
    agent: AgentStates,
    pub(super) agent_choices: Vec<Entity<choice::Choice>>,
    pub(super) timeout_choice: Entity<choice::Choice>,
    pub(super) turns_choice: Entity<choice::Choice>,
    inherited: daruda_flow::parse::AgentOverride,
    pub(super) settings_open: bool,
    /// Result targets focused since the form was built. Until then the caret
    /// is the one `set_value` left at the start, not one the person placed.
    placed_carets: Vec<gpui::EntityId>,
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

impl FormBinding {
    /// Construct widgets from the draft's cached baseline and its standing issues.
    pub(super) fn build<T: 'static>(
        draft: &super::inspector::NodeDraft,
        issues: &[daruda_flow::error::ValidationIssue],
        window: &mut Window,
        cx: &mut Context<T>,
    ) -> Self {
        let node = draft.node();
        let initial = draft.initial_fields();
        let notes = notes::notes_for(issues, node);
        // Same rule `set_notes` follows: a note about the override block is no
        // note at all while the block is closed, which is how it opens by
        // default only when there is something to see.
        let agent_open = !initial.agent.is_empty()
            || notes
                .iter()
                .any(|note| note.field == notes::FormField::Agent);

        let id = single_line(&initial.id, window, cx);
        let deps = single_line(&initial.deps.join(", "), window, cx);
        let timeout = single_line(&timeout_text(&initial.timeout), window, cx);
        let max_turns = single_line(&turns_text(&initial.max_turns), window, cx);
        let cwd = single_line(initial.cwd.as_deref().unwrap_or_default(), window, cx);
        // What an unstated axis resolves to, read off the file's own `defaults`.
        let inherited = draft.inherited_agent();
        let agent = AgentStates {
            id: single_line_inheriting(
                initial.agent.id.as_deref().unwrap_or_default(),
                inherited.id.as_deref(),
                window,
                cx,
            ),
            model: single_line_inheriting(
                initial.agent.model.as_deref().unwrap_or_default(),
                inherited.model.as_deref(),
                window,
                cx,
            ),
            effort: single_line_inheriting(
                initial.agent.effort.as_deref().unwrap_or_default(),
                inherited.effort.as_deref(),
                window,
                cx,
            ),
            mode: single_line_inheriting(
                initial.agent.mode.as_deref().unwrap_or_default(),
                inherited.mode.as_deref(),
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
        let agent_choices = [&agent.id, &agent.model, &agent.mode]
            .iter()
            .enumerate()
            .map(|(index, input)| {
                let input = (*input).clone();
                cx.new(|cx| {
                    choice::Choice::new(
                        input,
                        vec![catalog::inherited_option(None)],
                        // The id picker sits above the overrides toggle (6);
                        // model and mode sit below it.
                        [4, 7, 9][index],
                        window,
                        cx,
                    )
                })
            })
            .collect();
        let timeout_choice = suggested(&timeout, &["5m", "10m", "30m", "1h"], 20, window, cx);
        let turns_choice = suggested(&max_turns, &["1", "3", "5", "10"], 22, window, cx);
        let mut form = Self {
            node: node.clone(),
            applying: Default::default(),
            id,
            deps,
            dep_picker: cx.new(|cx| {
                crate::ui::select::state_with_options(vec![], None, window, cx).searchable(true)
            }),
            result_picker: cx.new(|cx| {
                crate::ui::select::state_with_options(vec![], None, window, cx).searchable(true)
            }),
            timeout,
            max_turns,
            cwd,
            agent,
            agent_choices,
            timeout_choice,
            turns_choice,
            inherited,
            settings_open: false,
            placed_carets: Vec::new(),
            body,
            // Opened when the node already overrides something — there is
            // something to see — and closed when it does not.
            agent_open,
            banner: None,
            notes,
        };
        form.set_notes(form.notes.clone());
        form
    }

    pub(super) fn refresh_choices(
        &mut self,
        catalog: &catalog::Catalog,
        window: &mut Window,
        cx: &mut Context<super::FlowGraphView>,
    ) {
        let id = some_if_filled(read(&self.agent.id, cx).trim())
            .or_else(|| self.inherited.id.clone())
            .unwrap_or_default();
        let options = [
            catalog.agents(self.inherited.id.as_deref()),
            catalog.axis(&id, true, self.inherited.model.as_deref()),
            catalog.axis(&id, false, self.inherited.mode.as_deref()),
        ];
        for (choice, options) in self.agent_choices.iter().zip(options) {
            choice.update(cx, |choice, cx| choice.set_options(options, window, cx));
        }
    }

    #[cfg(test)]
    pub(in crate::workspace) fn agent_picker(
        &self,
        axis: usize,
        cx: &App,
    ) -> Entity<crate::ui::select::SelectState> {
        self.agent_choices[axis].read(cx).select.clone()
    }

    #[cfg(test)]
    pub(in crate::workspace) fn timeout_picker(
        &self,
        cx: &App,
    ) -> Entity<crate::ui::select::SelectState> {
        self.timeout_choice.read(cx).select.clone()
    }

    /// Values currently displayed by this binding.
    pub(in crate::workspace) fn fields(&self, cx: &App) -> NodeFields {
        self.read(cx).effective().into_fields()
    }

    pub(in crate::workspace) fn refusal(&self, cx: &App) -> Option<Refusal> {
        super::inspector::fields::refusal_of(&self.fields(cx))
    }

    /// What to say about the last save, if it was refused.
    pub(in crate::workspace) fn banner(&self) -> Option<&str> {
        self.banner.as_deref()
    }

    /// Put a refusal on the form, or clear it. Cleared at the start of every
    /// save attempt, so what is shown is always about the latest one.
    pub(super) fn set_banner(&mut self, message: Option<String>) {
        self.banner = message;
    }

    /// The same refusal, pinned to the boxes it names.
    ///
    /// A note about the agent override opens that block: it is closed by default,
    /// and a capture found the pointer hidden behind it — which is the same as
    /// not pointing.
    pub(super) fn set_notes(&mut self, notes: Vec<notes::FieldNote>) {
        if notes
            .iter()
            .any(|note| note.field == notes::FormField::Agent)
        {
            self.agent_open = true;
        }
        if notes.iter().any(|note| {
            matches!(
                note.field,
                notes::FormField::Cwd | notes::FormField::Fix | notes::FormField::Rerun
            )
        }) {
            self.settings_open = true;
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

    pub(super) fn toggle_agent_open(&mut self) {
        self.agent_open = !self.agent_open;
    }

    pub(in crate::workspace) fn id_state(&self) -> &Entity<InputState> {
        &self.id
    }

    #[cfg(test)]
    pub(in crate::workspace) fn deps_state(&self) -> &Entity<InputState> {
        &self.deps
    }

    #[cfg(test)]
    pub(in crate::workspace) fn timeout_state(&self) -> &Entity<InputState> {
        &self.timeout
    }

    #[cfg(test)]
    pub(in crate::workspace) fn turns_state(&self) -> &Entity<InputState> {
        &self.max_turns
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
        SelectOption::new(KIND_AGENT, crate::surface::strings::flow::form_kind_agent()),
        SelectOption::new(
            KIND_COMMAND,
            crate::surface::strings::flow::form_kind_command(),
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
            crate::surface::strings::flow::form_source_inline(),
        ),
        SelectOption::new(
            SOURCE_FILE,
            crate::surface::strings::flow::form_source_file(),
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
        SelectOption::new(FAIL_HALT, crate::surface::strings::flow::form_fail_halt()),
        SelectOption::new(
            FAIL_ACT,
            match kind {
                FailKind::Retry => crate::surface::strings::flow::form_fail_retry(),
                FailKind::Repair => crate::surface::strings::flow::form_fail_repair(),
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
            crate::surface::strings::flow::form_absent(),
        ),
        SelectOption::new(
            "deny",
            crate::surface::strings::flow::form_permission_deny(),
        ),
        SelectOption::new(
            "allow_once",
            crate::surface::strings::flow::form_permission_allow_once(),
        ),
        SelectOption::new("ask", crate::surface::strings::flow::form_permission_ask()),
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

/// The same box, with what an empty one means written in it.
///
/// The five agent axes are overrides: empty is not "nothing", it is "whatever
/// `defaults` says". The card shows the resolved value and the box showed a
/// blank, so the two disagreed on screen about the same node — this is the box
/// saying what the card already says.
/// The turn cap as the box shows it. Empty is absent, which is the engine's
/// default and the only thing a node that never mentioned turns may write back.
fn turns_text(field: &TurnsField) -> String {
    match field {
        TurnsField::Absent => String::new(),
        TurnsField::Set(n) => n.to_string(),
        TurnsField::Unreadable(text) => text.clone(),
    }
}

fn single_line_inheriting<T: 'static>(
    value: &str,
    inherited: Option<&str>,
    window: &mut Window,
    cx: &mut Context<T>,
) -> Entity<InputState> {
    let value = value.to_string();
    let hint = inherited.map(crate::surface::strings::flow::form_inherited);
    cx.new(|cx| {
        let state = InputState::new(window, cx).default_value(value);
        match hint {
            Some(hint) => state.placeholder(hint),
            None => state,
        }
    })
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

fn suggested<T: 'static>(
    input: &Entity<InputState>,
    values: &[&str],
    tab: isize,
    window: &mut Window,
    cx: &mut Context<T>,
) -> Entity<choice::Choice> {
    let mut options = vec![catalog::inherited_option(None)];
    options.extend(
        values
            .iter()
            .map(|value| crate::ui::select::SelectOption::simple((*value).to_string())),
    );
    let input = input.clone();
    cx.new(|cx| choice::Choice::new(input, options, tab, window, cx))
}
