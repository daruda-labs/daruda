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
}
