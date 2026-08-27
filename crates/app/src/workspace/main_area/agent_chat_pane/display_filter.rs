//! Display filtering across independent block-kind, tool, and status axes.
//! An axis with no selected facets is unconstrained.

use daruda_acp::{ChatItem, ToolCallItem, ToolKindView, ToolStatusView};

/// Which of the three independent axes a facet belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::workspace) enum FilterAxis {
    Kind,
    Tool,
    Status,
}

impl FilterAxis {
    pub(in crate::workspace) const ALL: [FilterAxis; 3] = [Self::Kind, Self::Tool, Self::Status];
}

/// One selectable facet of the filter. Selecting none of an axis's facets
/// leaves that axis unconstrained (see [`DisplayFilter::axis_allows`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::workspace) enum FilterFacet {
    Thinking,
    Prose,
    Tools,
    ToolRead,
    ToolEdit,
    ToolSearch,
    ToolRun,
    ToolOther,
    StatusRunning,
    StatusOk,
    StatusFailed,
}

impl FilterFacet {
    /// Every facet, grouped by axis in menu order.
    pub(in crate::workspace) const ALL: [FilterFacet; 11] = [
        Self::Thinking,
        Self::Prose,
        Self::Tools,
        Self::ToolRead,
        Self::ToolEdit,
        Self::ToolSearch,
        Self::ToolRun,
        Self::ToolOther,
        Self::StatusRunning,
        Self::StatusOk,
        Self::StatusFailed,
    ];

    pub(in crate::workspace) fn axis(self) -> FilterAxis {
        match self {
            Self::Thinking | Self::Prose | Self::Tools => FilterAxis::Kind,
            Self::ToolRead
            | Self::ToolEdit
            | Self::ToolSearch
            | Self::ToolRun
            | Self::ToolOther => FilterAxis::Tool,
            Self::StatusRunning | Self::StatusOk | Self::StatusFailed => FilterAxis::Status,
        }
    }

    fn bit(self) -> u16 {
        1 << self as u16
    }

    /// Stable config and persistence token.
    pub(in crate::workspace) fn token(self) -> &'static str {
        match self {
            Self::Thinking => "thinking",
            Self::Prose => "prose",
            Self::Tools => "tools",
            Self::ToolRead => "tool_read",
            Self::ToolEdit => "tool_edit",
            Self::ToolSearch => "tool_search",
            Self::ToolRun => "tool_run",
            Self::ToolOther => "tool_other",
            Self::StatusRunning => "status_running",
            Self::StatusOk => "status_ok",
            Self::StatusFailed => "status_failed",
        }
    }

    pub(in crate::workspace) fn from_token(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.token() == token)
    }
}

/// Agent tool names mapped to filter facets.
const TOOL_NAME_FACETS: [(&str, FilterFacet); 12] = [
    ("read", FilterFacet::ToolRead),
    ("notebookread", FilterFacet::ToolRead),
    ("edit", FilterFacet::ToolEdit),
    ("multiedit", FilterFacet::ToolEdit),
    ("write", FilterFacet::ToolEdit),
    ("notebookedit", FilterFacet::ToolEdit),
    ("grep", FilterFacet::ToolSearch),
    ("glob", FilterFacet::ToolSearch),
    ("search", FilterFacet::ToolSearch),
    ("bash", FilterFacet::ToolRun),
    ("bashoutput", FilterFacet::ToolRun),
    ("killshell", FilterFacet::ToolRun),
];

fn facet_for_name(name: &str) -> FilterFacet {
    TOOL_NAME_FACETS
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(name))
        .map(|(_, facet)| *facet)
        .unwrap_or(FilterFacet::ToolOther)
}

fn facet_for_kind(kind: ToolKindView) -> FilterFacet {
    match kind {
        ToolKindView::Read => FilterFacet::ToolRead,
        ToolKindView::Edit | ToolKindView::Delete | ToolKindView::Move => FilterFacet::ToolEdit,
        ToolKindView::Search => FilterFacet::ToolSearch,
        ToolKindView::Execute => FilterFacet::ToolRun,
        ToolKindView::Think
        | ToolKindView::Fetch
        | ToolKindView::SwitchMode
        | ToolKindView::Other => FilterFacet::ToolOther,
    }
}

fn facet_for_status(status: ToolStatusView) -> FilterFacet {
    match status {
        ToolStatusView::Pending | ToolStatusView::InProgress => FilterFacet::StatusRunning,
        ToolStatusView::Completed => FilterFacet::StatusOk,
        ToolStatusView::Failed | ToolStatusView::Cancelled => FilterFacet::StatusFailed,
    }
}

/// The set of facets a pane is currently narrowed to. Empty = show everything.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(in crate::workspace) struct DisplayFilter {
    facets: u16,
}

impl DisplayFilter {
    /// Parse tokens, ignoring unknown facets for forward compatibility.
    pub(in crate::workspace) fn from_tokens<'a>(tokens: impl IntoIterator<Item = &'a str>) -> Self {
        let facets = tokens
            .into_iter()
            .filter_map(FilterFacet::from_token)
            .fold(0u16, |acc, f| acc | f.bit());
        Self { facets }
    }

    pub(in crate::workspace) fn tokens(self) -> Vec<&'static str> {
        FilterFacet::ALL
            .into_iter()
            .filter(|f| self.contains(*f))
            .map(FilterFacet::token)
            .collect()
    }

    pub(in crate::workspace) fn contains(self, facet: FilterFacet) -> bool {
        self.facets & facet.bit() != 0
    }

    pub(in crate::workspace) fn toggled(self, facet: FilterFacet) -> Self {
        Self {
            facets: self.facets ^ facet.bit(),
        }
    }

    pub(in crate::workspace) fn is_empty(self) -> bool {
        self.facets == 0
    }

    pub(in crate::workspace) fn selected_count(self) -> usize {
        self.facets.count_ones() as usize
    }

    fn axis_mask(axis: FilterAxis) -> u16 {
        FilterFacet::ALL
            .into_iter()
            .filter(|f| f.axis() == axis)
            .fold(0u16, |acc, f| acc | f.bit())
    }

    /// An axis with no selected facets admits every facet on that axis.
    fn axis_allows(self, facet: FilterFacet) -> bool {
        self.facets & Self::axis_mask(facet.axis()) == 0 || self.contains(facet)
    }

    /// Prompts, permissions, and failures are never filtered.
    pub(in crate::workspace) fn matches(self, item: &ChatItem) -> bool {
        match item {
            ChatItem::UserText(_) | ChatItem::Permission(_) | ChatItem::Failure(_) => true,
            ChatItem::Thinking { .. } => self.axis_allows(FilterFacet::Thinking),
            ChatItem::AssistantText { .. } => self.axis_allows(FilterFacet::Prose),
            ChatItem::ToolCall(tc) => self.matches_tool(tc, tc.status),
        }
    }

    /// Match a tool using its subtree-aware status.
    pub(in crate::workspace) fn matches_tool(
        self,
        tc: &ToolCallItem,
        status: ToolStatusView,
    ) -> bool {
        self.axis_allows(FilterFacet::Tools)
            && self.tool_matches(tc)
            && self.axis_allows(facet_for_status(status))
    }

    /// Prefer the agent tool name because Claude reports generic ACP kinds;
    /// Codex omits names, so it falls back to the kind. Diffs also imply edits.
    /// Shell writes without diffs cannot be classified as edits reliably.
    fn tool_matches(self, tc: &ToolCallItem) -> bool {
        if self.facets & Self::axis_mask(FilterAxis::Tool) == 0 {
            return true;
        }
        let by_signal = match tc.tool_name.as_deref() {
            Some(name) => self.contains(facet_for_name(name)),
            None => self.contains(facet_for_kind(tc.kind)),
        };
        by_signal || (self.contains(FilterFacet::ToolEdit) && !tc.diffs.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_acp::{DiffView, PermissionItem};

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
    fn an_untouched_axis_stays_unconstrained() {
        let f = filter(&["tool_read"]);
        assert!(f.matches(&asst()));
        assert!(f.matches(&think()));
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
    fn a_name_the_build_does_not_know_lands_in_other() {
        let f = filter(&["tool_other"]);
        assert!(f.matches(&call(
            Some("Skill"),
            ToolKindView::Execute,
            ToolStatusView::Completed
        )));
        assert!(!filter(&["tool_run"]).matches(&call(
            Some("Skill"),
            ToolKindView::Execute,
            ToolStatusView::Completed
        )));
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
    fn the_status_axis_narrows_to_the_picked_outcomes() {
        let f = filter(&["status_failed"]);
        assert!(f.matches(&call(None, ToolKindView::Read, ToolStatusView::Failed)));
        assert!(f.matches(&call(None, ToolKindView::Read, ToolStatusView::Cancelled)));
        assert!(!f.matches(&call(None, ToolKindView::Read, ToolStatusView::Completed)));
        assert!(!f.matches(&call(None, ToolKindView::Read, ToolStatusView::InProgress)));
    }

    #[test]
    fn axes_intersect_rather_than_union() {
        let f = filter(&["tool_read", "status_failed"]);
        assert!(f.matches(&call(None, ToolKindView::Read, ToolStatusView::Failed)));
        assert!(!f.matches(&call(None, ToolKindView::Read, ToolStatusView::Completed)));
        assert!(!f.matches(&call(None, ToolKindView::Execute, ToolStatusView::Failed)));
    }

    #[test]
    fn a_prompt_a_permission_and_a_failure_survive_every_filter() {
        let f = filter(&["tool_edit", "status_failed"]);
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
        }
        let all = DisplayFilter::from_tokens(FilterFacet::ALL.into_iter().map(FilterFacet::token));
        assert_eq!(all.selected_count(), FilterFacet::ALL.len());
        assert_eq!(
            all.tokens(),
            FilterFacet::ALL
                .into_iter()
                .map(FilterFacet::token)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn an_unknown_token_is_dropped_rather_than_failing() {
        let f = filter(&["tools", "not_a_facet"]);
        assert_eq!(f.tokens(), vec!["tools"]);
    }

    #[test]
    fn toggling_a_facet_twice_returns_the_empty_filter() {
        let f = DisplayFilter::default().toggled(FilterFacet::Tools);
        assert!(f.contains(FilterFacet::Tools));
        assert!(f.toggled(FilterFacet::Tools).is_empty());
    }

    #[test]
    fn each_facet_owns_a_distinct_bit() {
        let mut seen = 0u16;
        for facet in FilterFacet::ALL {
            assert_eq!(seen & facet.bit(), 0, "{facet:?} collides");
            seen |= facet.bit();
        }
    }

    #[test]
    fn the_axis_masks_partition_every_facet() {
        let union = FilterAxis::ALL
            .into_iter()
            .fold(0u16, |acc, a| acc | DisplayFilter::axis_mask(a));
        let all = FilterFacet::ALL
            .into_iter()
            .fold(0u16, |acc, f| acc | f.bit());
        assert_eq!(union, all, "every facet belongs to exactly one axis");
    }
}
