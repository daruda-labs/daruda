use crate::surface::strings as s;

/// Reusing a finished output instead of paying for it again, and taking that
/// back. Both name the nodes for the reason the delete question does: a count
/// does not tell you which ones a marquee caught.
pub(crate) fn pin_tooltip(nodes: &[daruda_flow::NodeId]) -> String {
    rust_i18n::t!("flow.pin_tooltip", nodes = node_list(nodes)).into_owned()
}

pub(crate) fn unpin_tooltip(nodes: &[daruda_flow::NodeId]) -> String {
    rust_i18n::t!("flow.unpin_tooltip", nodes = node_list(nodes)).into_owned()
}

/// A pin that could not be honoured: no finished run holds that node's output,
/// so it runs. Said rather than dropped — the pin was there to save the money
/// this run is about to spend.
pub(crate) fn pin_unavailable(nodes: &[daruda_flow::NodeId]) -> String {
    rust_i18n::t!("flow.pin_unavailable", nodes = node_list(nodes)).into_owned()
}

/// Node ids as one phrase. One place, so two lists cannot be punctuated
/// differently.
fn node_list(nodes: &[daruda_flow::NodeId]) -> String {
    nodes
        .iter()
        .map(daruda_flow::NodeId::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The same question for a marquee selection. Names them rather than counting
/// only: "3 nodes" does not tell you which three you caught in the drag.
pub(crate) fn delete_nodes_confirm_body(
    nodes: &[daruda_flow::NodeId],
    dependents: usize,
) -> String {
    let list = node_list(nodes);
    if dependents == 0 {
        rust_i18n::t!(
            "flow.delete_nodes_confirm_body",
            nodes = list,
            n = nodes.len().to_string()
        )
        .into_owned()
    } else {
        std::borrow::Cow::<str>::Owned(s::flow::delete_nodes_confirm_body_deps(
            nodes.len(),
            list,
            dependents,
        ))
        .into_owned()
    }
}

pub(crate) fn delete_node_confirm_body(node: &str, dependents: usize) -> String {
    if dependents == 0 {
        rust_i18n::t!("flow.delete_node_confirm_body", node = node).into_owned()
    } else {
        std::borrow::Cow::<str>::Owned(s::flow::delete_node_confirm_body_deps(node, dependents))
            .into_owned()
    }
}

/// A past run's start time, decoded from its id. Local time, since the
/// question it answers is "was that the one I ran before lunch"; no year,
/// since the list it labels only keeps recent runs.
pub(crate) fn run_started_at(at: chrono::DateTime<chrono::Utc>) -> String {
    crate::surface::timestamp::local_month_day_time(at)
}

/// How a finished run ended. `Running` and `Crashed` are not markers the
/// engine writes — they are read from the lock, which is why a run that
/// died with the app can be told from one that is still going.
pub(crate) fn run_status(status: daruda_flow::marker::RunStatus) -> String {
    use daruda_flow::marker::RunStatus;
    match status {
        RunStatus::Done => std::borrow::Cow::<str>::Owned(s::flow::status_done()),
        RunStatus::Failed => std::borrow::Cow::<str>::Owned(s::flow::status_failed()),
        RunStatus::Canceled => std::borrow::Cow::<str>::Owned(s::flow::status_canceled()),
        RunStatus::Running => std::borrow::Cow::<str>::Owned(s::flow::status_running()),
        RunStatus::Crashed => std::borrow::Cow::<str>::Owned(s::flow::status_crashed()),
        RunStatus::Stalled => std::borrow::Cow::<str>::Owned(s::flow::status_stalled()),
        RunStatus::Unknown => std::borrow::Cow::<str>::Owned(s::flow::status_unknown()),
    }
    .into_owned()
}

/// Which ceiling stopped the run. A `Debug` rendering of the limit would
/// put a Rust identifier in front of a user.
pub(crate) fn budget_exhausted(limit: daruda_flow::schedule::BudgetLimit) -> String {
    use daruda_flow::schedule::BudgetLimit as L;
    match limit {
        L::WallClock => std::borrow::Cow::<str>::Owned(s::flow::budget_wall_clock()),
        L::NodeRuns => std::borrow::Cow::<str>::Owned(s::flow::budget_node_runs()),
        L::Cost => std::borrow::Cow::<str>::Owned(s::flow::budget_cost()),
    }
    .into_owned()
}

/// One line of a validation report. A node-level problem names the node;
/// a whole-graph one (a cycle) has no node to name.
/// Every validation problem as its own line, node named where there is one.
///
/// `FlowError::Validate`'s own `Display` is a count — "1 validation problem(s)" —
/// so anything that shows a person why a flow does not load has to walk the
/// issues itself. One helper, so the graph pane and a refused edit say the same
/// thing about the same file.
pub(crate) fn issue_lines(issues: &[daruda_flow::error::ValidationIssue]) -> Vec<String> {
    issues
        .iter()
        .map(|issue| {
            s::flow::issue_line(
                issue.node.as_ref().map(|n| n.as_str()),
                &s::flow::issue(&issue.kind),
            )
        })
        .collect()
}

pub(crate) fn issue_line(node: Option<&str>, text: &str) -> String {
    match node {
        Some(node) => std::borrow::Cow::<str>::Owned(s::flow::issue_line_at_node(node, text)),
        None => rust_i18n::t!("flow.issue_line", text => text),
    }
    .into_owned()
}

/// User-facing wording for one validation problem.
///
/// The match lives here rather than at the call site because the whole
/// point of the split is that `ValidationIssue.message` is developer
/// detail and never reaches a user — putting the branch anywhere else
/// invites someone to reach for `.message` instead. Exhaustive on
/// purpose: a new `ValidationKind` stops compiling until it has wording.
pub(crate) fn issue(kind: &daruda_flow::error::ValidationKind) -> String {
    use daruda_flow::error::ValidationKind as K;
    match kind {
        K::MissingAgent => std::borrow::Cow::<str>::Owned(s::flow::issue_missing_agent()),
        K::AgentIdWithoutMode => {
            std::borrow::Cow::<str>::Owned(s::flow::issue_agent_id_without_mode())
        }
        K::AskWithoutMode => std::borrow::Cow::<str>::Owned(s::flow::issue_ask_without_mode()),
        K::UnknownProfile { name } => {
            std::borrow::Cow::<str>::Owned(s::flow::issue_unknown_profile(name))
        }
        K::ReservedProfileName => {
            std::borrow::Cow::<str>::Owned(s::flow::issue_reserved_profile_name())
        }
        K::CwdEscapesRunCwd => std::borrow::Cow::<str>::Owned(s::flow::issue_cwd_escapes()),
        K::CwdMissing { path } => std::borrow::Cow::<str>::Owned(s::flow::issue_cwd_missing(path)),
        K::UnknownVersion(version) => {
            std::borrow::Cow::<str>::Owned(s::flow::issue_unknown_version(version))
        }
        K::UnreachableOutputRef { referenced } => {
            std::borrow::Cow::<str>::Owned(s::flow::issue_unreachable_output_ref(referenced))
        }
        K::UnknownField { field } => {
            std::borrow::Cow::<str>::Owned(s::flow::issue_unknown_field(field))
        }
        K::ConflictingField { field, wins } => {
            std::borrow::Cow::<str>::Owned(s::flow::issue_conflicting_field(field, wins))
        }
        K::DuplicateOutput => std::borrow::Cow::<str>::Owned(s::flow::issue_duplicate_output()),
        K::OutputEscapesRunDir => {
            std::borrow::Cow::<str>::Owned(s::flow::issue_output_escapes_run_dir())
        }
        K::RerunNotAnAncestor { root } => {
            std::borrow::Cow::<str>::Owned(s::flow::issue_rerun_not_an_ancestor(root))
        }
        K::RepairWithoutFailureContext => {
            std::borrow::Cow::<str>::Owned(s::flow::issue_repair_without_failure_context())
        }
        K::RepairWithoutAgent => {
            std::borrow::Cow::<str>::Owned(s::flow::issue_repair_without_agent())
        }
        K::ReservedNodeId => std::borrow::Cow::<str>::Owned(s::flow::issue_reserved_node_id()),
        K::InvalidNodeId => std::borrow::Cow::<str>::Owned(s::flow::issue_invalid_node_id()),
        K::OutputInReservedDir { reserved } => {
            std::borrow::Cow::<str>::Owned(s::flow::issue_output_in_reserved_dir(reserved))
        }
        K::DuplicateId => std::borrow::Cow::<str>::Owned(s::flow::issue_duplicate_id()),
        K::UnknownDep { dep } => std::borrow::Cow::<str>::Owned(s::flow::issue_unknown_dep(dep)),
        K::Cycle => std::borrow::Cow::<str>::Owned(s::flow::issue_cycle()),
        K::UnknownAgent { id } => std::borrow::Cow::<str>::Owned(s::flow::issue_unknown_agent(id)),
        K::NobodyToAsk => std::borrow::Cow::<str>::Owned(s::flow::issue_nobody_to_ask()),
        K::MissingPromptFile { field, path } => std::borrow::Cow::<str>::Owned(
            s::flow::issue_missing_prompt_file(field, path.display()),
        ),
        K::RelativeRequestPath { field } => {
            std::borrow::Cow::<str>::Owned(s::flow::issue_relative_request_path(field))
        }
        K::PromptFileOutsideFlowDir { field, path } => std::borrow::Cow::<str>::Owned(
            s::flow::issue_prompt_file_outside_flow_dir(field, path.display()),
        ),
        K::EmptyPrompt => std::borrow::Cow::<str>::Owned(s::flow::issue_empty_prompt()),
        K::UnknownPin { node } => {
            std::borrow::Cow::<str>::Owned(s::flow::issue_unknown_pin(node.as_str()))
        }
        K::PinnedSourceMissing { node, path } => std::borrow::Cow::<str>::Owned(
            s::flow::issue_pinned_source_missing(node.as_str(), path),
        ),
        K::UnknownUntil { node } => {
            std::borrow::Cow::<str>::Owned(s::flow::issue_unknown_until(node.as_str()))
        }
        K::UnsupportedSchemaKeyword { keyword } => {
            std::borrow::Cow::<str>::Owned(s::flow::issue_unsupported_schema_keyword(keyword))
        }
        K::ContinueUntilWithoutObjectSchema => {
            std::borrow::Cow::<str>::Owned(s::flow::issue_continue_until_needs_object())
        }
        K::ContinueUntilFieldNotDeclared { field } => {
            std::borrow::Cow::<str>::Owned(s::flow::issue_continue_until_undeclared(field))
        }
        K::ContinueUntilFieldNotRequired { field } => {
            std::borrow::Cow::<str>::Owned(s::flow::issue_continue_until_optional(field))
        }
        K::ContinueUntilValueNotAllowed { field } => {
            std::borrow::Cow::<str>::Owned(s::flow::issue_continue_until_value_forbidden(field))
        }
        K::MaxTurnsIsZero => std::borrow::Cow::<str>::Owned(s::flow::issue_max_turns_zero()),
        K::PinnedNodeInRerun { node, gate } => std::borrow::Cow::<str>::Owned(
            s::flow::issue_pinned_node_in_rerun(gate.as_str(), node.as_str()),
        ),
        K::SchemaKeywordOnWrongKind { keyword, kind } => std::borrow::Cow::<str>::Owned(
            s::flow::issue_schema_keyword_on_wrong_kind(keyword, kind),
        ),
    }
    .into_owned()
}
