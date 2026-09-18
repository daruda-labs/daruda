//! Stable tool categories shared by filtering, step summaries, and folding.

use daruda_acp::{ToolCallItem, ToolKindView};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ToolCategory {
    Read,
    Edit,
    /// A removal. Split from [`Self::Edit`] because it is the one file change
    /// that cannot be read back from what replaced it.
    Delete,
    Search,
    Run,
    /// A call that left the machine — ACP's own `fetch` kind.
    Fetch,
    /// A tool an MCP server provided, named `mcp__<server>__<tool>` on the
    /// wire. Keyed on that spelling because the kind cannot say it: every
    /// adapter reports these as `other`. The same prefix is read in
    /// `daruda_agent::jsonl::permissions`.
    Mcp,
    /// A delegated agent. Sits beside the file-work categories rather than
    /// under [`Self::Other`]: the card holds a whole run of someone else's
    /// work, which is the one thing a reader scans a turn for.
    Agent,
    Other,
}

impl ToolCategory {
    /// [`Self::Other`] stays last: it is the catch-all, so a new category
    /// takes the slot before it rather than displacing the tail.
    pub(crate) const ALL: [Self; 9] = [
        Self::Read,
        Self::Edit,
        Self::Delete,
        Self::Search,
        Self::Run,
        Self::Fetch,
        Self::Mcp,
        Self::Agent,
        Self::Other,
    ];

    pub(crate) const fn index(self) -> usize {
        match self {
            Self::Read => 0,
            Self::Edit => 1,
            Self::Delete => 2,
            Self::Search => 3,
            Self::Run => 4,
            Self::Fetch => 5,
            Self::Mcp => 6,
            Self::Agent => 7,
            Self::Other => 8,
        }
    }

    pub(crate) const fn token(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Edit => "edit",
            Self::Delete => "delete",
            Self::Search => "search",
            Self::Run => "run",
            Self::Fetch => "fetch",
            Self::Mcp => "mcp",
            Self::Agent => "agent",
            Self::Other => "other",
        }
    }

    pub(crate) fn from_token(token: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|category| category.token() == token)
    }

    pub(crate) const fn bit(self) -> u16 {
        1 << self.index()
    }
}

/// Compact set used by the filter's partial Tool selection.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) struct ToolCategorySet(u16);

impl ToolCategorySet {
    const ALL_BITS: u16 = (1 << ToolCategory::ALL.len()) - 1;

    pub(crate) fn all() -> Self {
        Self(Self::ALL_BITS)
    }

    pub(crate) fn contains(self, category: ToolCategory) -> bool {
        self.0 & category.bit() != 0
    }

    pub(crate) fn insert(&mut self, category: ToolCategory) {
        self.0 |= category.bit();
    }

    pub(crate) fn toggle(&mut self, category: ToolCategory) {
        self.0 ^= category.bit();
    }

    pub(crate) fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub(crate) fn is_all(self) -> bool {
        self.0 == Self::ALL_BITS
    }
}

const TOOL_NAME_CATEGORIES: [(&str, ToolCategory); 12] = [
    ("read", ToolCategory::Read),
    ("notebookread", ToolCategory::Read),
    ("edit", ToolCategory::Edit),
    ("multiedit", ToolCategory::Edit),
    ("write", ToolCategory::Edit),
    ("notebookedit", ToolCategory::Edit),
    ("grep", ToolCategory::Search),
    ("glob", ToolCategory::Search),
    ("search", ToolCategory::Search),
    ("bash", ToolCategory::Run),
    ("bashoutput", ToolCategory::Run),
    ("killshell", ToolCategory::Run),
];

/// How every agent spells a tool an MCP server provided. Also read by
/// `daruda_agent::jsonl::permissions`, which matches allow-patterns on it.
const MCP_TOOL_PREFIX: &str = "mcp__";

/// Whether `name` is an MCP server's tool.
pub(crate) fn is_mcp_tool_name(name: &str) -> bool {
    name.starts_with(MCP_TOOL_PREFIX)
}

fn category_for_name(name: &str) -> Option<ToolCategory> {
    TOOL_NAME_CATEGORIES
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(name))
        .map(|(_, category)| *category)
}

fn category_for_kind(kind: ToolKindView) -> ToolCategory {
    match kind {
        ToolKindView::Read => ToolCategory::Read,
        // A move rewrites where a file lives, which is a change to read back;
        // a delete leaves nothing to read, so it answers for itself.
        ToolKindView::Edit | ToolKindView::Move => ToolCategory::Edit,
        ToolKindView::Delete => ToolCategory::Delete,
        ToolKindView::Search => ToolCategory::Search,
        ToolKindView::Execute => ToolCategory::Run,
        ToolKindView::Fetch => ToolCategory::Fetch,
        ToolKindView::Think | ToolKindView::SwitchMode | ToolKindView::Other => ToolCategory::Other,
    }
}

/// Resolve one category: a launch first, then diffs, then the MCP prefix, then
/// tool name, then ACP kind. The launch and the prefix come before the kind
/// because nothing below can answer them — a spawned agent arrives as `Think`
/// like any reasoning tool, and every adapter reports an MCP tool as `other`.
/// A reported diff still wins over the prefix: it is evidence of what the call
/// did, which outranks where it came from.
pub(crate) fn classify_tool(tc: &ToolCallItem) -> ToolCategory {
    if tc.is_subagent_launch() {
        return ToolCategory::Agent;
    }
    if !tc.diffs.is_empty() {
        return ToolCategory::Edit;
    }
    if tc.tool_name.as_deref().is_some_and(is_mcp_tool_name) {
        return ToolCategory::Mcp;
    }
    tc.tool_name
        .as_deref()
        .and_then(category_for_name)
        .unwrap_or_else(|| category_for_kind(tc.kind))
}

/// What a set of tool calls did, by category, most-numerous first.
///
/// Two callers with two ranges: a *group* bar passes its own adjacent calls,
/// filter-aware, because expanding it is what puts those rows on screen; a
/// *turn* bar passes the run's top-level calls, filter-blind, because it
/// summarizes rather than discloses. Either way the members are mixed in
/// practice — measured at up to four categories in one group — so this returns
/// the whole tally rather than picking a representative.
///
/// Ties break by [`ToolCategory::index`] so a header does not reorder itself
/// between renders of the same group.
pub(crate) fn tally_categories<'a>(
    calls: impl IntoIterator<Item = &'a ToolCallItem>,
) -> Vec<(ToolCategory, usize)> {
    let mut counts = [0usize; ToolCategory::ALL.len()];
    for tc in calls {
        counts[classify_tool(tc).index()] += 1;
    }
    let mut tally: Vec<(ToolCategory, usize)> = ToolCategory::ALL
        .into_iter()
        .filter(|c| counts[c.index()] > 0)
        .map(|c| (c, counts[c.index()]))
        .collect();
    tally.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.index().cmp(&b.0.index())));
    tally
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_acp::{DiffView, ToolStatusView};

    fn tool(name: Option<&str>, kind: ToolKindView) -> ToolCallItem {
        ToolCallItem {
            id: "t".into(),
            title: "Tool".into(),
            kind,
            tool_name: name.map(str::to_owned),
            status: ToolStatusView::Completed,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: None,
            locations: Vec::new(),
            parent_tool_id: None,
            exit: None,
        }
    }

    /// A launch carries the delegated agent, not a category of file work, and
    /// its `Think` kind says nothing about what the agent went on to do — so
    /// the launch itself is the category.
    #[test]
    fn a_subagent_launch_is_its_own_category() {
        let mut launch = tool(None, ToolKindView::Think);
        launch.raw_input = Some(serde_json::json!({ "subagent_type": "code-reviewer" }));
        assert!(launch.is_subagent_launch(), "the fixture must be a launch");
        assert_eq!(classify_tool(&launch).token(), "agent");
        assert_eq!(
            classify_tool(&tool(None, ToolKindView::Think)).token(),
            "other",
            "a plain think-kind call is still Other"
        );
    }

    /// Three kinds the catch-all used to swallow. Each already has its own ACP
    /// kind or a name the wire spells one way, so the bar can name it too.
    #[test]
    fn a_distinct_kind_or_an_mcp_name_is_its_own_category() {
        assert_eq!(
            classify_tool(&tool(Some("WebFetch"), ToolKindView::Fetch)).token(),
            "fetch"
        );
        assert_eq!(
            classify_tool(&tool(None, ToolKindView::Delete)).token(),
            "delete"
        );
        assert_eq!(
            classify_tool(&tool(
                Some("mcp__obsidian__obsidian_put_content"),
                ToolKindView::Other
            ))
            .token(),
            "mcp"
        );
        assert_eq!(
            classify_tool(&tool(None, ToolKindView::Move)).token(),
            "edit",
            "a move mutates a file rather than removing it, so it stays an edit"
        );
    }

    #[test]
    fn a_known_name_corrects_a_generic_kind() {
        assert_eq!(
            classify_tool(&tool(Some("Read"), ToolKindView::Execute)),
            ToolCategory::Read
        );
    }

    #[test]
    fn a_missing_or_unknown_name_falls_back_to_the_kind() {
        assert_eq!(
            classify_tool(&tool(None, ToolKindView::Search)),
            ToolCategory::Search
        );
        assert_eq!(
            classify_tool(&tool(Some("WebSearch"), ToolKindView::Search)),
            ToolCategory::Search
        );
    }

    #[test]
    fn a_diff_is_conclusive_edit_evidence() {
        let mut tc = tool(Some("Bash"), ToolKindView::Execute);
        tc.diffs.push(DiffView {
            path: "/tmp/x".into(),
            old_text: None,
            new_text: "changed".into(),
        });
        assert_eq!(classify_tool(&tc), ToolCategory::Edit);
    }

    #[test]
    fn a_tally_orders_by_count_then_by_category() {
        // The shape the wire logs actually produce: one adjacency group holding
        // several categories at once, up to four.
        let calls = [
            tool(Some("Bash"), ToolKindView::Execute),
            tool(Some("Read"), ToolKindView::Read),
            tool(Some("Bash"), ToolKindView::Execute),
            tool(Some("Grep"), ToolKindView::Search),
            tool(Some("Bash"), ToolKindView::Execute),
            tool(None, ToolKindView::Fetch),
        ];
        assert_eq!(
            tally_categories(calls.iter()),
            vec![
                (ToolCategory::Run, 3),
                (ToolCategory::Read, 1),
                (ToolCategory::Search, 1),
                (ToolCategory::Fetch, 1),
            ],
            "count descending, then declaration order so ties do not shuffle"
        );
    }

    #[test]
    fn a_single_category_tallies_as_one_entry() {
        let calls = [
            tool(Some("Read"), ToolKindView::Read),
            tool(Some("Read"), ToolKindView::Read),
        ];
        assert_eq!(
            tally_categories(calls.iter()),
            vec![(ToolCategory::Read, 2)]
        );
    }

    /// A segment of one is the common case in a mixed group, so every category
    /// needs a singular form — "1 files read" is the shape this guards against.
    #[test]
    fn every_category_has_both_a_singular_and_a_plural_form() {
        for locale in ["en", "ko"] {
            for category in ToolCategory::ALL {
                let one_key = format!("agent_chat.group_{}_one", category.token());
                let many_key = format!("agent_chat.group_{}", category.token());
                let one = rust_i18n::t!(&one_key, locale = locale);
                let many = rust_i18n::t!(&many_key, count = 3, locale = locale);
                assert!(
                    !one.contains("group_") && !many.contains("group_"),
                    "{locale}/{} is missing a form (t! echoes the key when unresolved)",
                    category.token()
                );
                assert_ne!(one, many, "{locale}/{}", category.token());
            }
        }
        // Pinned, because the defect this guards is a *wording* one: the plural
        // form printed for a count of one ("1 files read").
        for (category, expected) in [
            ("read", "1 file read"),
            ("edit", "1 file edited"),
            ("search", "1 search"),
            ("run", "1 command"),
            ("other", "1 other call"),
        ] {
            let key = format!("agent_chat.group_{category}_one");
            assert_eq!(rust_i18n::t!(&key, locale = "en"), expected);
        }
    }

    #[test]
    fn an_empty_group_tallies_to_nothing() {
        assert!(tally_categories(std::iter::empty()).is_empty());
    }
}
