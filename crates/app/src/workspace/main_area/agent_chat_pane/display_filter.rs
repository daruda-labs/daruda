//! Display filtering over thinking, prose, and tool categories.

use daruda_acp::{ChatItem, ToolCallItem};

use super::tool_category::{ToolCategory, ToolCategorySet, classify_tool};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::workspace) enum FilterAxis {
    Kind,
    Tool,
}

impl FilterAxis {
    #[cfg(test)]
    pub(in crate::workspace) const ALL: [FilterAxis; 2] = [Self::Kind, Self::Tool];
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

/// What the pane is narrowed to. Empty means show everything.
///
/// Tool selection distinguishes no tools, all tools, and a partial category set.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(in crate::workspace) struct DisplayFilter {
    thinking: bool,
    prose: bool,
    tools: ToolSelection,
}

impl DisplayFilter {
    pub(in crate::workspace) fn from_tokens<'a>(tokens: impl IntoIterator<Item = &'a str>) -> Self {
        let mut filter = Self::default();
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

    pub(in crate::workspace) fn is_empty(self) -> bool {
        !self.thinking && !self.prose && matches!(self.tools, ToolSelection::Excluded)
    }

    pub(in crate::workspace) fn selections(self) -> Vec<FilterFacet> {
        let mut out = Vec::new();
        if self.thinking {
            out.push(FilterFacet::Thinking);
        }
        if self.prose {
            out.push(FilterFacet::Prose);
        }
        match self.tools {
            ToolSelection::Excluded => {}
            ToolSelection::All => out.push(FilterFacet::Tools),
            ToolSelection::Some(categories) => out.extend(
                FilterFacet::ALL
                    .into_iter()
                    .filter(|facet| facet.category().is_some_and(|c| categories.contains(c))),
            ),
        }
        out
    }

    pub(in crate::workspace) fn tool_selection(self) -> ToolSelection {
        self.tools
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
        self.is_empty()
            || match self.tools {
                ToolSelection::Excluded => false,
                ToolSelection::All => true,
                ToolSelection::Some(categories) => categories.contains(classify_tool(tc)),
            }
    }
}

#[cfg(test)]
mod tests;
