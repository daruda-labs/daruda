use super::*;
use daruda_acp::{DiffView, PermissionItem, ToolKindView, ToolStatusView};

fn call(name: Option<&str>, kind: ToolKindView, status: ToolStatusView) -> ChatItem {
    ChatItem::ToolCall(ToolCallItem {
        id: "c1".into(),
        title: "t".into(),
        kind,
        tool_name: name.map(str::to_owned),
        status,
        diffs: Vec::new(),
        output: Vec::new(),
        raw_input: None,
        parent_tool_id: None,
        exit: None,
    })
}

fn with_diff(mut item: ChatItem) -> ChatItem {
    if let ChatItem::ToolCall(tc) = &mut item {
        tc.diffs.push(DiffView {
            path: std::path::PathBuf::from("/tmp/x.rs"),
            old_text: None,
            new_text: "fn main() {}".into(),
        });
    }
    item
}

fn asst() -> ChatItem {
    ChatItem::AssistantText {
        text: "hello".into(),
        streaming: false,
        message_id: None,
        phase: Default::default(),
    }
}

fn think() -> ChatItem {
    ChatItem::Thinking {
        text: "hmm".into(),
        streaming: false,
        message_id: None,
    }
}

fn filter(tokens: &[&str]) -> DisplayFilter {
    DisplayFilter::from_tokens(tokens.iter().copied())
}

// --- Scenario tests: what the user picked, and what they get ---

#[test]
fn picking_edits_hides_thinking_and_prose() {
    let f = filter(&["tool_edit"]);
    assert!(
        !f.matches(&think()),
        "a tool condition is not about thinking"
    );
    assert!(!f.matches(&asst()), "a tool condition is not about prose");
    assert!(f.matches(&call(
        Some("Write"),
        ToolKindView::Execute,
        ToolStatusView::Completed
    )));
    assert!(!f.matches(&call(
        Some("Read"),
        ToolKindView::Execute,
        ToolStatusView::Completed
    )));
}

#[test]
fn picking_thinking_hides_every_tool() {
    let f = filter(&["thinking"]);
    assert!(f.matches(&think()));
    assert!(!f.matches(&asst()));
    assert!(!f.matches(&call(None, ToolKindView::Read, ToolStatusView::Completed)));
}

#[test]
fn thinking_and_edits_show_exactly_those_two() {
    let f = filter(&["thinking", "tools", "tool_edit"]);
    assert!(f.matches(&think()));
    assert!(!f.matches(&asst()));
    assert!(f.matches(&call(
        Some("Edit"),
        ToolKindView::Execute,
        ToolStatusView::Completed
    )));
    assert!(!f.matches(&call(
        Some("Read"),
        ToolKindView::Execute,
        ToolStatusView::Completed
    )));
}

#[test]
fn a_condition_below_tools_puts_tools_in_scope() {
    let f = DisplayFilter::default().toggled(FilterFacet::ToolEdit);
    assert!(
        f.contains(FilterFacet::Tools),
        "the parent comes on with it"
    );
    assert!(f.contains(FilterFacet::ToolEdit));
}

#[test]
fn turning_tools_off_discards_the_conditions_below_it() {
    let f = filter(&["tools", "tool_edit"]);
    let off = f.toggled(FilterFacet::Tools);
    assert!(!off.contains(FilterFacet::ToolEdit));
    assert!(off.is_empty(), "nothing survives the parent going away");
}

#[test]
fn clearing_the_last_tool_category_excludes_tools() {
    let f = filter(&["tools", "tool_edit"]).toggled(FilterFacet::ToolEdit);
    assert!(!f.contains(FilterFacet::Tools));
    assert!(f.is_empty());
    assert!(f.tokens().is_empty());
}

#[test]
fn a_bare_tool_token_round_trips_as_the_normalised_pair() {
    let f = filter(&["tool_edit"]);
    assert_eq!(f.tokens(), vec!["tools", "tool_edit"]);
    assert_eq!(filter(&["tools", "tool_edit"]), f);
    assert_eq!(DisplayFilter::from_tokens(f.tokens()), f);
}

#[test]
fn an_unknown_token_beside_a_known_one_drops_only_itself() {
    let f = filter(&["tool_edit", "not_a_facet"]);
    assert_eq!(f.tokens(), vec!["tools", "tool_edit"]);
}

#[test]
fn an_empty_filter_shows_everything() {
    let f = DisplayFilter::default();
    assert!(f.is_empty());
    for item in [
        asst(),
        think(),
        call(
            Some("Bash"),
            ToolKindView::Execute,
            ToolStatusView::Completed,
        ),
        call(None, ToolKindView::Read, ToolStatusView::Failed),
        ChatItem::UserText("q".into()),
    ] {
        assert!(f.matches(&item), "{item:?} shows with no filter");
    }
}

#[test]
fn tool_kind_filters_ignore_tool_status() {
    let f = filter(&["tools", "tool_read"]);
    assert!(!f.matches(&asst()));
    assert!(!f.matches(&think()));
    for status in [
        ToolStatusView::Completed,
        ToolStatusView::Failed,
        ToolStatusView::InProgress,
    ] {
        assert!(
            f.matches(&call(Some("Read"), ToolKindView::Execute, status)),
            "{status:?}"
        );
    }
    assert!(!f.matches(&call(
        Some("Bash"),
        ToolKindView::Execute,
        ToolStatusView::Completed
    )));
}

#[test]
fn the_kind_axis_keeps_only_what_is_picked() {
    let f = filter(&["tools"]);
    assert!(!f.matches(&asst()));
    assert!(!f.matches(&think()));
    assert!(f.matches(&call(
        Some("Bash"),
        ToolKindView::Execute,
        ToolStatusView::Completed
    )));
}

#[test]
fn picking_several_kinds_unions_them() {
    let f = filter(&["prose", "tools"]);
    assert!(f.matches(&asst()));
    assert!(!f.matches(&think()));
    assert!(f.matches(&call(None, ToolKindView::Read, ToolStatusView::Completed)));
}

#[test]
fn a_named_tool_is_classified_by_its_name() {
    let f = filter(&["tool_read"]);
    assert!(f.matches(&call(
        Some("Read"),
        ToolKindView::Execute,
        ToolStatusView::Completed
    )));
    assert!(!f.matches(&call(
        Some("Bash"),
        ToolKindView::Execute,
        ToolStatusView::Completed
    )));
}

#[test]
fn a_tool_name_is_matched_case_insensitively() {
    let f = filter(&["tool_search"]);
    assert!(f.matches(&call(
        Some("GREP"),
        ToolKindView::Execute,
        ToolStatusView::Completed
    )));
}

#[test]
fn an_unnamed_tool_falls_back_to_its_kind() {
    let f = filter(&["tool_read"]);
    assert!(f.matches(&call(None, ToolKindView::Read, ToolStatusView::Completed)));
    assert!(!f.matches(&call(
        None,
        ToolKindView::Execute,
        ToolStatusView::Completed
    )));
}

#[test]
fn a_name_the_build_does_not_know_falls_back_to_its_kind() {
    // `Skill`, `WebSearch`, `Task`, `mcp__*` … are not in the name table,
    // so the ACP kind decides rather than being discarded for `ToolOther`.
    let skill = || {
        call(
            Some("Skill"),
            ToolKindView::Execute,
            ToolStatusView::Completed,
        )
    };
    assert!(filter(&["tool_run"]).matches(&skill()));
    assert!(!filter(&["tool_other"]).matches(&skill()));

    let web_search = call(
        Some("WebSearch"),
        ToolKindView::Search,
        ToolStatusView::Completed,
    );
    assert!(filter(&["tool_search"]).matches(&web_search));
}

#[test]
fn only_an_unclassifiable_name_and_kind_land_in_other() {
    let todo = call(
        Some("TodoWrite"),
        ToolKindView::Other,
        ToolStatusView::Completed,
    );
    assert!(filter(&["tool_other"]).matches(&todo));
    assert!(!filter(&["tool_edit"]).matches(&todo));
}

#[test]
fn a_known_name_still_outranks_the_kind() {
    // Claude reports every call as `Execute`; the name is what keeps a
    // `Read` out of the "Runs" facet.
    let read = call(
        Some("Read"),
        ToolKindView::Execute,
        ToolStatusView::Completed,
    );
    assert!(filter(&["tool_read"]).matches(&read));
    assert!(!filter(&["tool_run"]).matches(&read));
}

#[test]
fn a_call_carrying_diffs_counts_as_an_edit_whatever_it_was_called() {
    let f = filter(&["tool_edit"]);
    let bash = call(
        Some("Bash"),
        ToolKindView::Execute,
        ToolStatusView::Completed,
    );
    assert!(!f.matches(&bash));
    assert!(f.matches(&with_diff(bash)));
}

#[test]
fn a_shell_written_file_is_invisible_to_the_edit_facet() {
    let f = filter(&["tool_edit"]);
    assert!(!f.matches(&call(
        Some("Bash"),
        ToolKindView::Execute,
        ToolStatusView::Completed
    )));
}

#[test]
fn delete_and_move_are_edits() {
    let f = filter(&["tool_edit"]);
    for kind in [ToolKindView::Edit, ToolKindView::Delete, ToolKindView::Move] {
        assert!(
            f.matches(&call(None, kind, ToolStatusView::Completed)),
            "{kind:?}"
        );
    }
}

#[test]
fn a_prompt_a_permission_and_a_failure_survive_every_filter() {
    let f = filter(&["tool_edit"]);
    assert!(f.matches(&ChatItem::UserText("q".into())));
    assert!(f.matches(&ChatItem::Permission(PermissionItem {
        id: 0,
        tool_title: Some("Write /tmp/x".into()),
        raw_input_summary: None,
        options: Vec::new(),
        resolved: None,
    })));
    assert!(
        f.matches(&ChatItem::Failure(daruda_acp::AcpFailure::Unclassified {
            message: "boom".into(),
        }))
    );
}

#[test]
fn every_facet_round_trips_through_its_token() {
    for facet in FilterFacet::ALL {
        assert_eq!(FilterFacet::from_token(facet.token()), Some(facet));
        let selected = DisplayFilter::from_tokens([facet.token()]);
        assert!(selected.contains(facet));
    }
}

#[test]
fn an_unknown_token_is_dropped_rather_than_failing() {
    let f = filter(&["tools", "not_a_facet"]);
    assert_eq!(f.tokens(), vec!["tools"]);
}

#[test]
fn removed_status_tokens_are_ignored_for_persistence_compatibility() {
    assert!(filter(&["status_running", "status_ok", "status_failed"]).is_empty());
    assert_eq!(filter(&["tools", "status_failed"]).tokens(), vec!["tools"]);
}

#[test]
fn toggling_a_facet_twice_returns_the_empty_filter() {
    let f = DisplayFilter::default().toggled(FilterFacet::Tools);
    assert!(f.contains(FilterFacet::Tools));
    assert!(f.toggled(FilterFacet::Tools).is_empty());
}

#[test]
fn one_tool_category_is_one_visible_selection() {
    let f = DisplayFilter::default().toggled(FilterFacet::ToolEdit);
    assert_eq!(f.selections().len(), 1);
    assert_eq!(f.selections(), vec![FilterFacet::ToolEdit]);
    assert_eq!(f.tokens(), vec!["tools", "tool_edit"]);
}

#[test]
fn all_tools_and_partial_tools_are_distinct_states() {
    let all = filter(&["tools"]);
    assert_eq!(all.tool_selection(), ToolSelection::All);
    assert_eq!(all.selections(), vec![FilterFacet::Tools]);

    let without_edit = all.toggled(FilterFacet::ToolEdit);
    assert!(matches!(
        without_edit.tool_selection(),
        ToolSelection::Some(_)
    ));
    assert!(!without_edit.contains(FilterFacet::ToolEdit));
    assert_eq!(without_edit.selections().len(), ToolCategory::ALL.len() - 1);
}

#[test]
fn every_facet_lands_in_exactly_one_section() {
    let mut placed: Vec<FilterFacet> = Vec::new();
    for axis in FilterAxis::ALL {
        placed.extend(axis.parent());
        placed.extend(axis.rows());
    }
    for facet in FilterFacet::ALL {
        let hits = placed.iter().filter(|f| **f == facet).count();
        assert_eq!(hits, 1, "{facet:?} appears {hits} times in the panel");
    }
    assert_eq!(placed.len(), FilterFacet::ALL.len());
}

#[test]
fn a_section_only_lists_facets_that_claim_its_axis() {
    for axis in FilterAxis::ALL {
        for facet in axis.parent().into_iter().chain(axis.rows()) {
            assert_eq!(
                facet.axis(),
                axis,
                "{facet:?} is drawn under {axis:?} but claims {:?}",
                facet.axis()
            );
        }
    }
}

#[test]
fn only_the_tool_section_nests_under_a_parent_toggle() {
    assert_eq!(FilterAxis::Kind.parent(), None);
    assert_eq!(FilterAxis::Tool.parent(), Some(FilterFacet::Tools));
    // The parent stands for the whole section, so it must not also be a row.
    assert!(!FilterAxis::Tool.rows().contains(&FilterFacet::Tools));
    // Every nested row names one category; the parent names none.
    assert!(
        FilterAxis::Tool
            .rows()
            .iter()
            .all(|f| f.category().is_some())
    );
    assert!(
        FilterAxis::Kind
            .rows()
            .iter()
            .all(|f| f.category().is_none())
    );
}

#[test]
fn clearing_the_last_facet_widens_rather_than_empties() {
    let search_only = DisplayFilter::default().toggled(FilterFacet::ToolSearch);
    assert!(!search_only.is_empty());
    let cleared = search_only.toggled(FilterFacet::ToolSearch);
    assert!(
        cleared.is_empty(),
        "unchecking the last facet is the unfiltered state, not a selection of nothing"
    );
    // The half the name promises: the transcript widens rather than emptying.
    let reasoning = ChatItem::Thinking {
        text: "t".into(),
        streaming: false,
        message_id: None,
    };
    assert!(!search_only.matches(&reasoning), "narrowed to searches");
    assert!(
        cleared.matches(&reasoning),
        "cleared shows everything again"
    );
    // An axis still held keeps the narrowing, so the widening is confined to
    // the degenerate all-off case.
    let with_reasoning = search_only.toggled(FilterFacet::Thinking);
    assert!(!with_reasoning.toggled(FilterFacet::ToolSearch).is_empty());
}
