//! Display filtering over thinking, prose, and tool categories.

use daruda_acp::{ChatItem, ToolCallItem};

use super::tool_category::{ToolCategory, ToolCategorySet, classify_tool};

/// How the panel groups facets into labelled sections. Each axis renders one
/// heading, an optional parent toggle standing for the whole section, and the
/// rows [`FilterAxis::rows`] derives from [`FilterFacet::axis`] — so a facet
/// cannot be listed under a heading its own axis disagrees with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::workspace) enum FilterAxis {
    Kind,
    Tool,
}

impl FilterAxis {
    /// Render order of the panel's sections.
    pub(in crate::workspace) const ALL: [FilterAxis; 2] = [Self::Kind, Self::Tool];

    /// The section-wide toggle drawn above this axis's own rows, if it has one.
    /// Only Tool does: `Tools` gates every category at once, so its rows nest
    /// under it. Kind's facets are independent and sit flat under the heading.
    pub(in crate::workspace) fn parent(self) -> Option<FilterFacet> {
        match self {
            Self::Kind => None,
            Self::Tool => Some(FilterFacet::Tools),
        }
    }

    /// The rows listed under this heading, in [`FilterFacet::ALL`] order —
    /// every facet on the axis except the section's own parent toggle.
    pub(in crate::workspace) fn rows(self) -> Vec<FilterFacet> {
        FilterFacet::ALL
            .into_iter()
            .filter(|facet| facet.axis() == self && Some(*facet) != self.parent())
            .collect()
    }
}

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

    /// Which panel section lists this facet. `Tools` is on the Tool axis even
    /// though it names no single [`ToolCategory`]: it is that section's parent
    /// toggle, not a Kind row.
    pub(in crate::workspace) fn axis(self) -> FilterAxis {
        match self {
            Self::Thinking | Self::Prose => FilterAxis::Kind,
            Self::Tools
            | Self::ToolRead
            | Self::ToolEdit
            | Self::ToolSearch
            | Self::ToolRun
            | Self::ToolOther => FilterAxis::Tool,
        }
    }

    pub(in crate::workspace) fn category(self) -> Option<ToolCategory> {
        match self {
            Self::Thinking | Self::Prose | Self::Tools => None,
            Self::ToolRead => Some(ToolCategory::Read),
            Self::ToolEdit => Some(ToolCategory::Edit),
            Self::ToolSearch => Some(ToolCategory::Search),
            Self::ToolRun => Some(ToolCategory::Run),
            Self::ToolOther => Some(ToolCategory::Other),
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

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(in crate::workspace) enum ToolSelection {
    #[default]
    Excluded,
    All,
    Some(ToolCategorySet),
}

/// Which kinds of work the pane shows. Each field is a visibility, so a checked
/// box means "this is on screen" — the way a checkbox list reads.
///
/// Tool selection distinguishes no tools, all tools, and a partial category set.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::workspace) struct DisplayFilter {
    thinking: bool,
    prose: bool,
    tools: ToolSelection,
}

impl Default for DisplayFilter {
    /// A pane opens showing its whole transcript, so every kind starts visible.
    fn default() -> Self {
        Self {
            thinking: true,
            prose: true,
            tools: ToolSelection::All,
        }
    }
}

impl DisplayFilter {
    /// Read the pane's stored visible set. Unlike [`Self::default`], an empty
    /// list is a real value here — the pane the user unchecked entirely.
    pub(in crate::workspace) fn from_stored(tokens: &[String]) -> Self {
        Self::from_tokens(tokens.iter().map(String::as_str))
    }

    /// Read the superseded `display_filter` field, whose list named the same
    /// visible set — except that it wrote an empty list for "unfiltered", the
    /// one value the current reading gives the opposite meaning.
    pub(in crate::workspace) fn from_legacy_tokens(tokens: &[String]) -> Self {
        if tokens.is_empty() {
            return Self::default();
        }
        Self::from_stored(tokens)
    }

    /// Build from a stored visible set. Starts from nothing visible and adds
    /// what the list names, so the empty list is "nothing on screen" — the
    /// state [`Self::default`] is the opposite of.
    pub(in crate::workspace) fn from_tokens<'a>(tokens: impl IntoIterator<Item = &'a str>) -> Self {
        let mut filter = Self {
            thinking: false,
            prose: false,
            tools: ToolSelection::Excluded,
        };
        let mut tools = false;
        let mut categories = ToolCategorySet::default();
        for facet in tokens.into_iter().filter_map(FilterFacet::from_token) {
            match facet {
                FilterFacet::Thinking => filter.thinking = true,
                FilterFacet::Prose => filter.prose = true,
                FilterFacet::Tools => tools = true,
                _ => categories.insert(facet.category().expect("tool facet")),
            }
        }
        filter.tools = if categories.is_empty() {
            if tools {
                ToolSelection::All
            } else {
                ToolSelection::Excluded
            }
        } else if categories.is_all() {
            ToolSelection::All
        } else {
            ToolSelection::Some(categories)
        };
        filter
    }

    pub(in crate::workspace) fn tokens(self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.thinking {
            out.push(FilterFacet::Thinking.token());
        }
        if self.prose {
            out.push(FilterFacet::Prose.token());
        }
        match self.tools {
            ToolSelection::Excluded => {}
            ToolSelection::All => out.push(FilterFacet::Tools.token()),
            ToolSelection::Some(categories) => {
                out.push(FilterFacet::Tools.token());
                out.extend(
                    FilterFacet::ALL
                        .into_iter()
                        .filter(|facet| facet.category().is_some_and(|c| categories.contains(c)))
                        .map(FilterFacet::token),
                );
            }
        }
        out
    }

    pub(in crate::workspace) fn contains(self, facet: FilterFacet) -> bool {
        match facet {
            FilterFacet::Thinking => self.thinking,
            FilterFacet::Prose => self.prose,
            FilterFacet::Tools => !matches!(self.tools, ToolSelection::Excluded),
            _ => match self.tools {
                ToolSelection::Excluded => false,
                ToolSelection::All => true,
                ToolSelection::Some(categories) => categories.contains(facet.category().unwrap()),
            },
        }
    }

    pub(in crate::workspace) fn toggled(mut self, facet: FilterFacet) -> Self {
        match facet {
            FilterFacet::Thinking => self.thinking = !self.thinking,
            FilterFacet::Prose => self.prose = !self.prose,
            FilterFacet::Tools => {
                self.tools = match self.tools {
                    ToolSelection::Excluded => ToolSelection::All,
                    ToolSelection::All | ToolSelection::Some(_) => ToolSelection::Excluded,
                }
            }
            _ => {
                let category = facet.category().expect("tool facet");
                let mut categories = match self.tools {
                    ToolSelection::Excluded => ToolCategorySet::default(),
                    ToolSelection::All => ToolCategorySet::all(),
                    ToolSelection::Some(categories) => categories,
                };
                categories.toggle(category);
                self.tools = if categories.is_empty() {
                    ToolSelection::Excluded
                } else if categories.is_all() {
                    ToolSelection::All
                } else {
                    ToolSelection::Some(categories)
                };
            }
        }
        self
    }

    pub(in crate::workspace) fn with_all_tools(mut self, selected: bool) -> Self {
        self.tools = if selected {
            ToolSelection::All
        } else {
            ToolSelection::Excluded
        };
        self
    }

    /// Nothing is hidden — the pane is not narrowed at all.
    pub(in crate::workspace) fn shows_everything(self) -> bool {
        self == Self::default()
    }

    /// The facets the user unchecked, in panel order. What the chip names: it
    /// is the shorter list, and it is what the user did.
    pub(in crate::workspace) fn hidden(self) -> Vec<FilterFacet> {
        FilterFacet::ALL
            .into_iter()
            .filter(|facet| !self.contains(*facet))
            // `Tools` stands for its whole section, so it would restate the
            // categories under it that are already listed.
            .filter(|facet| *facet != FilterFacet::Tools)
            .collect()
    }

    pub(in crate::workspace) fn tool_selection(self) -> ToolSelection {
        self.tools
    }

    /// Prompts, permissions, and failures are never filtered.
    pub(in crate::workspace) fn matches(self, item: &ChatItem) -> bool {
        match item {
            ChatItem::UserText(_) | ChatItem::Permission(_) | ChatItem::Failure(_) => true,
            ChatItem::Thinking { .. } => self.thinking,
            ChatItem::AssistantText { .. } => self.prose,
            ChatItem::ToolCall(tc) => self.matches_tool(tc),
        }
    }

    pub(in crate::workspace) fn matches_tool(self, tc: &ToolCallItem) -> bool {
        match self.tools {
            ToolSelection::Excluded => false,
            ToolSelection::All => true,
            ToolSelection::Some(categories) => categories.contains(classify_tool(tc)),
        }
    }
}

#[cfg(test)]
mod tests;
