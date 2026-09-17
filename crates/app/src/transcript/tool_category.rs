//! Stable tool categories shared by filtering, step summaries, and folding.

use daruda_acp::{ToolCallItem, ToolKindView};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ToolCategory {
    Read,
    Edit,
    Search,
    Run,
    Other,
}

impl ToolCategory {
    pub(crate) const ALL: [Self; 5] =
        [Self::Read, Self::Edit, Self::Search, Self::Run, Self::Other];

    pub(crate) const fn index(self) -> usize {
        match self {
            Self::Read => 0,
            Self::Edit => 1,
            Self::Search => 2,
            Self::Run => 3,
            Self::Other => 4,
        }
    }

    pub(crate) const fn token(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Edit => "edit",
            Self::Search => "search",
            Self::Run => "run",
            Self::Other => "other",
        }
    }

    pub(crate) fn from_token(token: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|category| category.token() == token)
    }

    pub(crate) const fn bit(self) -> u8 {
        1 << self.index()
    }
}

/// Compact set used by the filter's partial Tool selection.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) struct ToolCategorySet(u8);

impl ToolCategorySet {
    const ALL_BITS: u8 = (1 << ToolCategory::ALL.len()) - 1;

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

fn category_for_name(name: &str) -> Option<ToolCategory> {
    TOOL_NAME_CATEGORIES
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(name))
        .map(|(_, category)| *category)
}

fn category_for_kind(kind: ToolKindView) -> ToolCategory {
    match kind {
        ToolKindView::Read => ToolCategory::Read,
        ToolKindView::Edit | ToolKindView::Delete | ToolKindView::Move => ToolCategory::Edit,
        ToolKindView::Search => ToolCategory::Search,
        ToolKindView::Execute => ToolCategory::Run,
        ToolKindView::Think
        | ToolKindView::Fetch
        | ToolKindView::SwitchMode
        | ToolKindView::Other => ToolCategory::Other,
    }
}

/// Resolve one category from diffs, then tool name, then ACP kind.
pub(crate) fn classify_tool(tc: &ToolCallItem) -> ToolCategory {
    if !tc.diffs.is_empty() {
        return ToolCategory::Edit;
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
                (ToolCategory::Other, 1),
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
            ("run", "1 command run"),
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
