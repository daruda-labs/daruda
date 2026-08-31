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
    let f = DisplayFilter::default()
        .with_section(FilterFacet::Tools, false)
        .toggled(FilterFacet::ToolEdit);
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
    assert!(
        off.tokens().is_empty(),
        "nothing survives the parent going away"
    );
    assert!(!off.matches(&call(
        Some("Edit"),
        ToolKindView::Edit,
        ToolStatusView::Completed
    )));
}

#[test]
fn clearing_the_last_tool_category_excludes_tools() {
    let f = filter(&["tools", "tool_edit"]).toggled(FilterFacet::ToolEdit);
    assert!(!f.contains(FilterFacet::Tools));
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
fn the_default_filter_shows_every_kind() {
    let f = DisplayFilter::default();
    assert!(f.shows_everything());
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

/// A facet this build no longer knows must not decide what the pane shows. The
/// old field's reader treated a list it could parse nothing out of as
/// unfiltered — the short-circuit that made it so is gone, so the reader has to
/// say it.
#[test]
fn a_legacy_list_of_only_removed_tokens_still_means_unfiltered() {
    let stored: Vec<String> = ["status_running", "status_ok", "status_failed"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    assert_eq!(
        DisplayFilter::from_legacy_tokens(&stored),
        DisplayFilter::default(),
        "a pane whose stored facets no longer exist opens showing everything"
    );
}

/// A removed token beside a live one drops only itself — the live one still
/// says what to show.
#[test]
fn a_removed_token_beside_a_live_one_drops_only_itself() {
    assert_eq!(filter(&["tools", "status_failed"]).tokens(), vec!["tools"]);
    assert_eq!(
        DisplayFilter::from_legacy_tokens(&["tools".to_owned(), "status_failed".to_owned()]),
        filter(&["tools"])
    );
}

#[test]
fn toggling_a_facet_twice_returns_the_default() {
    let f = DisplayFilter::default().toggled(FilterFacet::Tools);
    assert!(!f.contains(FilterFacet::Tools), "the first click hides it");
    assert_eq!(f.toggled(FilterFacet::Tools), DisplayFilter::default());
}

#[test]
fn one_tool_category_is_one_hidden_facet() {
    let f = DisplayFilter::default().toggled(FilterFacet::ToolEdit);
    assert_eq!(f.hidden(), vec![FilterFacet::ToolEdit]);
    assert_eq!(
        f.tokens(),
        vec![
            "thinking",
            "prose",
            "tools",
            "tool_read",
            "tool_search",
            "tool_run",
            "tool_other"
        ]
    );
}

#[test]
fn all_tools_and_partial_tools_are_distinct_states() {
    let all = filter(&["tools"]);
    assert_eq!(all.section_state(FilterFacet::Tools), SectionState::On);

    let without_edit = all.toggled(FilterFacet::ToolEdit);
    assert_eq!(
        without_edit.section_state(FilterFacet::Tools),
        SectionState::Partial
    );
    assert!(!without_edit.contains(FilterFacet::ToolEdit));
    // Exact, not `contains`: a `toggled` that dropped a second category would
    // pass the looser check while quietly widening what the chip reports.
    assert_eq!(
        without_edit.hidden(),
        vec![
            FilterFacet::Thinking,
            FilterFacet::Prose,
            FilterFacet::ToolEdit
        ],
        "this filter starts from tools-only, so the two Kind facets are hidden too"
    );
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
fn a_fresh_filter_shows_everything_with_every_box_checked() {
    let f = DisplayFilter::default();
    for facet in FilterFacet::ALL {
        assert!(f.contains(facet), "{facet:?} starts checked");
    }
    assert!(f.matches(&think()));
    assert!(f.matches(&asst()));
    assert!(f.matches(&call(
        Some("Edit"),
        ToolKindView::Edit,
        ToolStatusView::Completed
    )));
    assert!(f.shows_everything());
}

/// The point of the change: unchecking one kind hides that kind and nothing
/// else. Before, checking one kind hid every *other* kind.
#[test]
fn unchecking_a_kind_hides_that_kind_alone() {
    let f = DisplayFilter::default().toggled(FilterFacet::Thinking);
    assert!(!f.matches(&think()), "the unchecked kind is hidden");
    assert!(f.matches(&asst()), "replies are untouched");
    assert!(
        f.matches(&call(
            Some("Edit"),
            ToolKindView::Edit,
            ToolStatusView::Completed
        )),
        "tools are untouched"
    );
    assert!(!f.shows_everything());
}

#[test]
fn unchecking_a_tool_kind_leaves_the_other_tool_kinds_visible() {
    let f = DisplayFilter::default().toggled(FilterFacet::ToolEdit);
    assert!(!f.matches(&call(
        Some("Edit"),
        ToolKindView::Edit,
        ToolStatusView::Completed
    )));
    assert!(f.matches(&call(
        Some("Read"),
        ToolKindView::Read,
        ToolStatusView::Completed
    )));
    assert!(f.matches(&asst()));
}

/// Reachable, and worth reaching by mistake only once — the user can put every
/// box back. Prompts and permissions are never filtered, so the pane is not blank.
#[test]
fn unchecking_everything_leaves_prompts_and_permissions() {
    let mut f = DisplayFilter::default();
    for facet in FilterFacet::ALL {
        if f.contains(facet) {
            f = f.toggled(facet);
        }
    }
    assert!(!f.matches(&think()));
    assert!(!f.matches(&asst()));
    assert!(!f.matches(&call(
        Some("Edit"),
        ToolKindView::Edit,
        ToolStatusView::Completed
    )));
    assert!(f.matches(&ChatItem::UserText("q".into())));
}

/// The chip names what is missing, because that is what the user did and it is
/// the shorter list.
#[test]
fn the_chip_names_what_is_hidden() {
    assert!(DisplayFilter::default().hidden().is_empty());
    let f = DisplayFilter::default().toggled(FilterFacet::Thinking);
    assert_eq!(f.hidden(), vec![FilterFacet::Thinking]);
}

/// New storage records what the pane shows, so an all-unchecked pane restores
/// as one — the state the old format could not tell from "unfiltered".
#[test]
fn the_visible_set_round_trips_including_the_empty_one() {
    for f in [
        DisplayFilter::default(),
        DisplayFilter::default().toggled(FilterFacet::Thinking),
        DisplayFilter::default().with_section(FilterFacet::Tools, false),
    ] {
        assert_eq!(DisplayFilter::from_tokens(f.tokens()), f);
    }
    let nothing = FilterFacet::ALL
        .into_iter()
        .fold(DisplayFilter::default(), |f, facet| {
            if f.contains(facet) {
                f.toggled(facet)
            } else {
                f
            }
        });
    assert!(nothing.tokens().is_empty());
    assert_eq!(DisplayFilter::from_tokens(Vec::new()), nothing);
}

/// The old field listed what a pane showed, but wrote an empty list for
/// "unfiltered" — the one value whose meaning the new reading inverts.
#[test]
fn a_legacy_empty_list_meant_unfiltered_not_blank() {
    assert_eq!(
        DisplayFilter::from_legacy_tokens(&[]),
        DisplayFilter::default()
    );
    assert_eq!(
        DisplayFilter::from_legacy_tokens(&["prose".to_string()]),
        DisplayFilter::from_tokens(["prose"]),
        "a non-empty legacy list already said exactly what to show"
    );
}

/// A pane stored before the prose kinds existed named only the parent. Reading
/// that as "every kind" is what keeps such a pane showing all of its prose
/// instead of silently losing its preambles.
#[test]
fn a_list_naming_only_the_parent_shows_every_prose_kind() {
    let old = DisplayFilter::from_tokens(["thinking", "prose", "tools"]);
    assert_eq!(
        old,
        DisplayFilter::default(),
        "an old full list is unfiltered"
    );
    assert!(old.contains(FilterFacet::ProseAnswer));
    assert!(old.contains(FilterFacet::ProsePreamble));
}

#[test]
fn a_single_prose_kind_round_trips_through_storage() {
    for facet in [FilterFacet::ProseAnswer, FilterFacet::ProsePreamble] {
        let one = DisplayFilter::default().toggled(other_prose_facet(facet));
        assert!(one.contains(facet), "{facet:?} stayed on");
        assert!(!one.contains(other_prose_facet(facet)));
        let stored: Vec<String> = one.tokens().into_iter().map(str::to_owned).collect();
        assert_eq!(
            DisplayFilter::from_stored(&stored),
            one,
            "{facet:?} survived {stored:?}"
        );
    }
}

fn other_prose_facet(facet: FilterFacet) -> FilterFacet {
    match facet {
        FilterFacet::ProseAnswer => FilterFacet::ProsePreamble,
        _ => FilterFacet::ProseAnswer,
    }
}

#[test]
fn unchecking_both_prose_kinds_is_the_parent_being_off() {
    let none = DisplayFilter::default()
        .toggled(FilterFacet::ProseAnswer)
        .toggled(FilterFacet::ProsePreamble);
    assert!(!none.contains(FilterFacet::Prose), "the parent reads off");
    assert_eq!(none.section_state(FilterFacet::Prose), SectionState::Off);
    // And the chip names the parent rather than both rows under it.
    assert_eq!(none.hidden(), vec![FilterFacet::Prose]);
}

#[test]
fn the_prose_filter_routes_an_assistant_message_by_its_phase() {
    let answer = ChatItem::AssistantText {
        text: "done".into(),
        streaming: false,
        message_id: None,
        phase: MessagePhase::Answer,
    };
    let preamble = ChatItem::AssistantText {
        text: "now I will".into(),
        streaming: false,
        message_id: None,
        phase: MessagePhase::Commentary,
    };

    let answers_only = DisplayFilter::default().toggled(FilterFacet::ProsePreamble);
    assert!(answers_only.matches(&answer));
    assert!(!answers_only.matches(&preamble));

    let preambles_only = DisplayFilter::default().toggled(FilterFacet::ProseAnswer);
    assert!(!preambles_only.matches(&answer));
    assert!(preambles_only.matches(&preamble));

    // An agent that does not label phases emits only `Answer`, so a pane hiding
    // preambles there loses nothing.
    assert!(answers_only.matches(&answer));
}

/// The parent toggle sets the whole section in one move, and each parented axis
/// drives its own filter — the hazard the old tools-only helper warned about.
#[test]
fn a_parent_toggle_sets_only_its_own_section() {
    let off_prose = DisplayFilter::default().with_section(FilterFacet::Prose, false);
    assert_eq!(
        off_prose.section_state(FilterFacet::Prose),
        SectionState::Off
    );
    assert_eq!(
        off_prose.section_state(FilterFacet::Tools),
        SectionState::On
    );

    let off_tools = DisplayFilter::default().with_section(FilterFacet::Tools, false);
    assert_eq!(
        off_tools.section_state(FilterFacet::Prose),
        SectionState::On
    );
    assert_eq!(
        off_tools.section_state(FilterFacet::Tools),
        SectionState::Off
    );
}

#[test]
fn a_partly_on_section_reads_indeterminate() {
    let partial = DisplayFilter::default().toggled(FilterFacet::ProsePreamble);
    assert_eq!(
        partial.section_state(FilterFacet::Prose),
        SectionState::Partial
    );
    assert_eq!(partial.hidden(), vec![FilterFacet::ProsePreamble]);
}
