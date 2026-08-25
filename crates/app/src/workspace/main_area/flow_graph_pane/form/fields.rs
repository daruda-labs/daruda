//! One node's fields as plain values, and the conversions either way.
//!
//! GPUI-free on purpose: everything here is `NodeFile` in, values out, or the
//! reverse. That is what lets the parsing and formatting be tested without a
//! window — `mod.rs` next door needs one for every line it holds, because a
//! text box is an entity.
//!
//! What a *box* holds is `mod.rs`'s ([`super::NodeForm`]); this is what its
//! contents mean.

use std::time::Duration;

use daruda_flow::parse::PermissionPolicyFile;

/// One node's editable fields, as the form currently holds them.
///
/// The `id` a save has to act on is [`NodeForm::node`] — the *original* one —
/// because this `id` may be the new name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) struct NodeFields {
    pub id: String,
    pub deps: Vec<String>,
    pub timeout: TimeoutField,
    /// How many prompts one attempt may send. Absent is the engine's default.
    pub max_turns: TurnsField,
    /// Where the node runs, relative to the run's directory. Empty is the run's
    /// own directory — the engine refuses an absolute one or one that climbs out,
    /// and that refusal stays the engine's.
    pub cwd: Option<String>,
    pub agent: AgentFields,
    pub body: BodyFields,
}

/// The five axes a node may override. All empty means the node writes no
/// `agent:` at all and takes `defaults`.
///
/// `mode` is free text on purpose: a session mode is whatever the adapter
/// advertises, and a list hardcoded here would go stale the first time one
/// changes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::workspace) struct AgentFields {
    pub id: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub mode: Option<String>,
    pub permission: Option<PermissionPolicyFile>,
}

impl AgentFields {
    /// The file's shape, or nothing when the node overrides no axis.
    pub(in crate::workspace) fn to_override(&self) -> Option<daruda_flow::parse::AgentOverride> {
        if self.is_empty() {
            return None;
        }
        Some(daruda_flow::parse::AgentOverride {
            id: self.id.clone(),
            model: self.model.clone(),
            effort: self.effort.clone(),
            mode: self.mode.clone(),
            permission: self.permission,
        })
    }

    pub(super) fn is_empty(&self) -> bool {
        self.id.is_none()
            && self.model.is_none()
            && self.effort.is_none()
            && self.mode.is_none()
            && self.permission.is_none()
    }
}

/// A duration the person typed. Kept as three states rather than
/// `Option<Duration>` plus an error flag: text that is not a duration cannot
/// become a `FlowFile` at all, so it is the one thing the form itself has to
/// refuse — the engine never gets to see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum TimeoutField {
    /// The field is empty: the node names no timeout.
    Absent,
    Set(Duration),
    /// What was typed, which is not a duration.
    Unreadable(String),
}

/// What the node is, and the values for whichever kind that is.
///
/// A struct rather than an enum: the form keeps both kinds' boxes alive so
/// switching and switching back does not lose what was typed, and `kind` is what
/// decides which set is read. The file only ever holds one of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) struct BodyFields {
    pub kind: KindChoice,
    pub prompt: SourceField,
    pub output: String,
    pub run: String,
    pub on_fail: FailFields,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum KindChoice {
    Agent,
    Command,
}

/// What a node does when it fails. One type for both kinds, because the shapes
/// differ only in which of the two policies the file can hold — and the form
/// shows the one its kind allows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum FailFields {
    Halt,
    /// An agent node's retry.
    Retry {
        hint: SourceField,
        max_attempts: AttemptsField,
        wait: TimeoutField,
    },
    /// A gate's repair.
    Repair {
        fix: String,
        rerun: Vec<String>,
        max_attempts: AttemptsField,
        wait: TimeoutField,
    },
}

/// Prose, or the file that holds it. `prompt`/`prompt_file` and `hint`/`hint_file`
/// are the same either-or, so they are the same type — and naming both is what
/// the engine refuses (`ConflictingField`), which the shape makes impossible.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum SourceField {
    Inline(String),
    File(String),
}

/// A count the person typed. Three states for the same reason
/// [`TimeoutField`] has them: text that is not a number cannot become a `u32`,
/// and the engine never sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum AttemptsField {
    Set(u32),
    /// What was typed, which is not a count. Empty text lands here too: a retry
    /// has to say how many times.
    Unreadable(String),
}

/// How many prompts one attempt may send, as the box holds it.
///
/// `Absent` is a real answer — the engine has a default and a node that never
/// mentioned turns must not grow the key on a save — which is why this is
/// shaped like [`TimeoutField`] and not like [`AttemptsField`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum TurnsField {
    Absent,
    Set(u32),
    /// What was typed, which is not a count.
    Unreadable(String),
}

/// Why a save is refused before the file is touched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum Refusal {
    EmptyId,
    /// A name the engine could not use — it becomes a filename and it
    /// delimits `{{node.<id>.output}}`.
    InvalidId,
    /// The turn box holds something that is not a count.
    Turns(String),
    Timeout(String),
    Attempts(String),
    OutputRequired,
    RunRequired,
}

pub(super) fn fields_of(node: &daruda_flow::parse::NodeFile) -> NodeFields {
    NodeFields {
        id: node.id.clone().into_string(),
        deps: node.deps.iter().map(|d| d.as_str().to_string()).collect(),
        timeout: node.timeout.map_or(TimeoutField::Absent, TimeoutField::Set),
        max_turns: turns_of(node),
        cwd: node.cwd.as_ref().map(|p| p.display().to_string()),
        agent: agent_fields_of(node),
        body: body_fields_of(node),
    }
}

/// The node's body as the file holds it, with the other kind's boxes empty.
pub(super) fn body_fields_of(node: &daruda_flow::parse::NodeFile) -> BodyFields {
    use daruda_flow::parse::{NodeKindFile, PromptSource};
    match &node.kind {
        NodeKindFile::Agent {
            prompt,
            output,
            on_fail,
            ..
        } => BodyFields {
            kind: KindChoice::Agent,
            prompt: match prompt {
                PromptSource::Prompt(text) => SourceField::Inline(text.clone()),
                PromptSource::PromptFile(path) => SourceField::File(path.display().to_string()),
            },
            output: output.display().to_string(),
            run: String::new(),
            on_fail: retry_fields_of(on_fail),
        },
        NodeKindFile::Command { run, on_fail } => BodyFields {
            kind: KindChoice::Command,
            prompt: SourceField::Inline(String::new()),
            output: String::new(),
            run: run.clone(),
            on_fail: repair_fields_of(on_fail),
        },
    }
}

/// Which policy a kind allows.
pub(super) fn fail_kind_of(kind: KindChoice) -> FailKind {
    match kind {
        KindChoice::Agent => FailKind::Retry,
        KindChoice::Command => FailKind::Repair,
    }
}

/// A node's own `agent:`, or nothing. Only agent nodes can carry one — a command
/// node runs a shell line and has no agent of its own.
/// The turn cap as the file holds it. A command node has no turns to cap.
fn turns_of(node: &daruda_flow::parse::NodeFile) -> TurnsField {
    use daruda_flow::parse::NodeKindFile;
    match &node.kind {
        NodeKindFile::Agent { max_turns, .. } => {
            max_turns.map_or(TurnsField::Absent, TurnsField::Set)
        }
        NodeKindFile::Command { .. } => TurnsField::Absent,
    }
}

pub(super) fn agent_fields_of(node: &daruda_flow::parse::NodeFile) -> AgentFields {
    use daruda_flow::parse::NodeKindFile;
    let NodeKindFile::Agent {
        agent: Some(agent), ..
    } = &node.kind
    else {
        return AgentFields::default();
    };
    AgentFields {
        id: agent.id.clone(),
        model: agent.model.clone(),
        effort: agent.effort.clone(),
        mode: agent.mode.clone(),
        permission: agent.permission,
    }
}

/// Which policy a kind's `act` option means.
#[derive(Copy, Clone)]
pub(super) enum FailKind {
    Retry,
    Repair,
}

pub(super) fn retry_fields_of(policy: &daruda_flow::parse::AgentFailFile) -> FailFields {
    use daruda_flow::parse::{AgentFailFile, HintSource};
    match policy {
        AgentFailFile::Halt => FailFields::Halt,
        AgentFailFile::Retry {
            hint,
            max_attempts,
            wait,
        } => FailFields::Retry {
            hint: match hint {
                HintSource::Hint(text) => SourceField::Inline(text.clone()),
                HintSource::HintFile(path) => SourceField::File(path.display().to_string()),
            },
            max_attempts: AttemptsField::Set(*max_attempts),
            wait: wait.map_or(TimeoutField::Absent, TimeoutField::Set),
        },
    }
}

pub(super) fn repair_fields_of(policy: &daruda_flow::parse::GateFailFile) -> FailFields {
    use daruda_flow::parse::GateFailFile;
    match policy {
        GateFailFile::Halt => FailFields::Halt,
        GateFailFile::Repair {
            fix,
            rerun,
            max_attempts,
            wait,
        } => FailFields::Repair {
            fix: fix.clone(),
            rerun: rerun.iter().map(|r| r.as_str().to_string()).collect(),
            max_attempts: AttemptsField::Set(*max_attempts),
            wait: wait.map_or(TimeoutField::Absent, TimeoutField::Set),
        },
    }
}

pub(super) fn parse_attempts(text: &str) -> AttemptsField {
    match text.parse::<u32>() {
        Ok(n) => AttemptsField::Set(n),
        Err(_) => AttemptsField::Unreadable(text.to_string()),
    }
}

pub(super) fn attempts_text(field: &AttemptsField) -> String {
    match field {
        AttemptsField::Set(n) => n.to_string(),
        AttemptsField::Unreadable(text) => text.clone(),
    }
}

/// Empty text is an absent key, not an empty value: a `model: ""` would be a
/// model named nothing.
pub(super) fn some_if_filled(text: &str) -> Option<String> {
    (!text.is_empty()).then(|| text.to_string())
}

/// Comma-separated, because a dependency is a node id and ids have no commas
/// (`daruda_flow` validates them as plain names). Empty entries are dropped so
/// a trailing comma while typing is not a dependency on nothing.
pub(super) fn parse_deps(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

pub(super) fn parse_timeout(text: &str) -> TimeoutField {
    if text.is_empty() {
        return TimeoutField::Absent;
    }
    match humantime::parse_duration(text) {
        Ok(duration) => TimeoutField::Set(duration),
        Err(_) => TimeoutField::Unreadable(text.to_string()),
    }
}

/// The same spelling the engine writes with, so a value that is only read and
/// written back does not change the file.
pub(super) fn timeout_text(field: &TimeoutField) -> String {
    match field {
        TimeoutField::Absent => String::new(),
        TimeoutField::Set(duration) => humantime::format_duration(*duration).to_string(),
        TimeoutField::Unreadable(text) => text.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dependencies_are_read_as_a_comma_separated_list() {
        assert_eq!(
            parse_deps("design, review"),
            vec!["design".to_string(), "review".to_string()]
        );
        assert_eq!(parse_deps("  design  "), vec!["design".to_string()]);
        assert!(parse_deps("").is_empty());
        assert_eq!(
            parse_deps("design,"),
            vec!["design".to_string()],
            "a trailing comma while typing is not a dependency"
        );
    }

    /// A timeout only read and written back must not change the file, which is
    /// why the form spells it the way the engine does.
    #[test]
    fn a_timeout_round_trips_through_the_text_the_engine_writes() {
        let field = parse_timeout("1m 30s");
        assert_eq!(field, TimeoutField::Set(Duration::from_secs(90)));
        assert_eq!(timeout_text(&field), "1m 30s");
        assert_eq!(parse_timeout(""), TimeoutField::Absent);
    }

    #[test]
    fn text_that_is_not_a_duration_is_kept_as_what_was_typed() {
        assert_eq!(
            parse_timeout("soon"),
            TimeoutField::Unreadable("soon".to_string())
        );
    }
}
