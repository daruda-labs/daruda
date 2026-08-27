//! Display filtering over what a chat pane shows: thinking, prose, and tools.
//! Tool kinds are conditions *inside* tools, not peers of them.

use daruda_acp::{ChatItem, ToolCallItem, ToolKindView};

/// How the menu groups facets into labelled sections. Purely presentational —
/// the nesting that decides matching is [`FacetSlot`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::workspace) enum FilterAxis {
    Kind,
    Tool,
}

impl FilterAxis {
    pub(in crate::workspace) const ALL: [FilterAxis; 2] = [Self::Kind, Self::Tool];
}

/// One selectable facet of the filter, as the menu and the persisted tokens
/// name it. Where it actually lives in the filter is [`FilterFacet::slot`].
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
}

impl FilterFacet {
    /// Every facet, grouped by axis in menu order.
    pub(in crate::workspace) const ALL: [FilterFacet; 8] = [
        Self::Thinking,
        Self::Prose,
        Self::Tools,
        Self::ToolRead,
        Self::ToolEdit,
        Self::ToolSearch,
        Self::ToolRun,
        Self::ToolOther,
    ];

    /// Which labelled menu section this facet is listed under.
    pub(in crate::workspace) fn axis(self) -> FilterAxis {
        match self {
            Self::Thinking | Self::Prose | Self::Tools => FilterAxis::Kind,
            Self::ToolRead
            | Self::ToolEdit
            | Self::ToolSearch
            | Self::ToolRun
            | Self::ToolOther => FilterAxis::Tool,
        }
    }

    /// Where this facet is stored in a [`DisplayFilter`]. Tool kinds name a bit
    /// inside [`ToolSelector`], which only exists when tools are in scope.
    fn slot(self) -> FacetSlot {
        match self {
            Self::Thinking => FacetSlot::Thinking,
            Self::Prose => FacetSlot::Prose,
            Self::Tools => FacetSlot::Tools,
            Self::ToolRead => FacetSlot::ToolKind(1 << 0),
            Self::ToolEdit => FacetSlot::ToolKind(1 << 1),
            Self::ToolSearch => FacetSlot::ToolKind(1 << 2),
            Self::ToolRun => FacetSlot::ToolKind(1 << 3),
            Self::ToolOther => FacetSlot::ToolKind(1 << 4),
        }
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
        }
    }

    pub(in crate::workspace) fn from_token(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.token() == token)
    }
}

/// The place a [`FilterFacet`] occupies in the filter's shape.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FacetSlot {
    Thinking,
    Prose,
    Tools,
    /// A bit in [`ToolSelector::kinds`].
    ToolKind(u8),
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

/// `None` for a name the table does not know — the caller falls back to the
/// ACP kind rather than dumping every unlisted tool into `ToolOther`.
fn facet_for_name(name: &str) -> Option<FilterFacet> {
    TOOL_NAME_FACETS
        .iter()
        .find(|(known, _)| known.eq_ignore_ascii_case(name))
        .map(|(_, facet)| *facet)
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

/// An empty kind selection is unconstrained and keeps every tool kind.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
struct ToolSelector {
    kinds: u8,
}

impl ToolSelector {
    fn has_kind(self, facet: FilterFacet) -> bool {
        matches!(facet.slot(), FacetSlot::ToolKind(bit) if self.kinds & bit != 0)
    }

    /// Classify on the first signal that resolves: a *recognised* agent tool
    /// name (Claude reports generic ACP kinds, so the name wins where it is
    /// known), else the kind (Codex omits names, and an unlisted name like
    /// `WebSearch` still carries a usable kind), else diffs, which imply edits.
    /// Shell writes without diffs cannot be classified as edits reliably.
    fn kind_allows(self, tc: &ToolCallItem) -> bool {
        if self.kinds == 0 {
            return true;
        }
        let facet = tc
            .tool_name
            .as_deref()
            .and_then(facet_for_name)
            .unwrap_or_else(|| facet_for_kind(tc.kind));
        self.has_kind(facet) || (self.has_kind(FilterFacet::ToolEdit) && !tc.diffs.is_empty())
    }

    fn selected_count(self) -> usize {
        self.kinds.count_ones() as usize
    }
}

/// What the pane is narrowed to. Every choice empty = show everything.
///
/// Toggling `thinking` or `prose` flips that bool. Toggling `tools` flips
/// `Some`/`None`, and turning it off **discards** the conditions below it.
/// Toggling a tool kind brings `tools` into scope if it was not, then flips that
/// bit; clearing the last such bit leaves `Some(empty)`, which
/// reads as "all tools" — the user is still looking at tools, and the menu
/// still shows `tools` checked. Clearing entirely goes through `tools` itself
/// or [`DisplayFilter::default`].
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(in crate::workspace) struct DisplayFilter {
    thinking: bool,
    prose: bool,
    /// `Some` = tools are in scope; the subordinate conditions live only inside.
    tools: Option<ToolSelector>,
}

impl DisplayFilter {
    /// Parse tokens, ignoring unknown facets for forward compatibility. A bare
    /// subordinate token pulls `tools` in, so `["tool_edit"]` and
    /// `["tools", "tool_edit"]` parse to the same value.
    pub(in crate::workspace) fn from_tokens<'a>(tokens: impl IntoIterator<Item = &'a str>) -> Self {
        tokens
            .into_iter()
            .filter_map(FilterFacet::from_token)
            .fold(Self::default(), Self::with)
    }

    /// Select `facet` regardless of its current state, unlike [`Self::toggled`].
    fn with(mut self, facet: FilterFacet) -> Self {
        match facet.slot() {
            FacetSlot::Thinking => self.thinking = true,
            FacetSlot::Prose => self.prose = true,
            FacetSlot::Tools => {
                self.tools.get_or_insert_default();
            }
            FacetSlot::ToolKind(bit) => self.tools.get_or_insert_default().kinds |= bit,
        }
        self
    }

    /// Normalised: a selected subordinate always emits `tools` ahead of itself.
    pub(in crate::workspace) fn tokens(self) -> Vec<&'static str> {
        FilterFacet::ALL
            .into_iter()
            .filter(|f| self.contains(*f))
            .map(FilterFacet::token)
            .collect()
    }

    pub(in crate::workspace) fn contains(self, facet: FilterFacet) -> bool {
        match facet.slot() {
            FacetSlot::Thinking => self.thinking,
            FacetSlot::Prose => self.prose,
            FacetSlot::Tools => self.tools.is_some(),
            FacetSlot::ToolKind(_) => self.tools.is_some_and(|t| t.has_kind(facet)),
        }
    }

    pub(in crate::workspace) fn toggled(mut self, facet: FilterFacet) -> Self {
        match facet.slot() {
            FacetSlot::Thinking => self.thinking = !self.thinking,
            FacetSlot::Prose => self.prose = !self.prose,
            FacetSlot::Tools => {
                self.tools = self.tools.xor(Some(ToolSelector::default()));
            }
            FacetSlot::ToolKind(bit) => self.tools.get_or_insert_default().kinds ^= bit,
        }
        self
    }

    pub(in crate::workspace) fn is_empty(self) -> bool {
        !self.thinking && !self.prose && self.tools.is_none()
    }

    pub(in crate::workspace) fn selected_count(self) -> usize {
        usize::from(self.thinking)
            + usize::from(self.prose)
            + self.tools.map_or(0, |t| 1 + t.selected_count())
    }

    /// Prompts, permissions, and failures are never filtered.
    pub(in crate::workspace) fn matches(self, item: &ChatItem) -> bool {
        if self.is_empty() {
            return true;
        }
        match item {
            ChatItem::UserText(_) | ChatItem::Permission(_) | ChatItem::Failure(_) => true,
            ChatItem::Thinking { .. } => self.thinking,
            ChatItem::AssistantText { .. } => self.prose,
            ChatItem::ToolCall(tc) => self.matches_tool(tc),
        }
    }

    pub(in crate::workspace) fn matches_tool(self, tc: &ToolCallItem) -> bool {
        self.is_empty() || self.tools.is_some_and(|t| t.kind_allows(tc))
    }
}

#[cfg(test)]
mod tests;
