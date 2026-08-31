//! Display filtering over thinking, prose, and tool categories.

use daruda_acp::{ChatItem, MessagePhase, ToolCallItem};

use super::tool_category::{ToolCategory, ToolCategorySet, classify_tool};

/// How the panel groups facets into labelled sections. Each axis renders one
/// heading, an optional parent toggle standing for the whole section, and the
/// rows [`FilterAxis::rows`] derives from [`FilterFacet::axis`] — so a facet
/// cannot be listed under a heading its own axis disagrees with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::workspace) enum FilterAxis {
    Kind,
    Reply,
    Tool,
}

impl FilterAxis {
    /// Render order of the panel's sections.
    pub(in crate::workspace) const ALL: [FilterAxis; 3] = [Self::Kind, Self::Reply, Self::Tool];

    /// The section-wide toggle drawn above this axis's own rows, if it has one.
    /// Reply and Tool each have one — `Prose` and `Tools` gate their whole
    /// section, so those rows nest under them. Kind's facet is independent and
    /// sits flat under the heading.
    pub(in crate::workspace) fn parent(self) -> Option<FilterFacet> {
        match self {
            Self::Kind => None,
            Self::Reply => Some(FilterFacet::Prose),
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
    ProseAnswer,
    ProsePreamble,
    Tools,
    ToolRead,
    ToolEdit,
    ToolSearch,
    ToolRun,
    ToolOther,
}

impl FilterFacet {
    pub(in crate::workspace) const ALL: [FilterFacet; 10] = [
        Self::Thinking,
        Self::Prose,
        Self::ProseAnswer,
        Self::ProsePreamble,
        Self::Tools,
        Self::ToolRead,
        Self::ToolEdit,
        Self::ToolSearch,
        Self::ToolRun,
        Self::ToolOther,
    ];

    /// Which panel section lists this facet. `Prose` and `Tools` sit on the
    /// axis they parent even though neither names a single kind under it: each
    /// is that section's parent toggle, not a row of its own.
    pub(in crate::workspace) fn axis(self) -> FilterAxis {
        match self {
            Self::Thinking => FilterAxis::Kind,
            Self::Prose | Self::ProseAnswer | Self::ProsePreamble => FilterAxis::Reply,
            Self::Tools
            | Self::ToolRead
            | Self::ToolEdit
            | Self::ToolSearch
            | Self::ToolRun
            | Self::ToolOther => FilterAxis::Tool,
        }
    }

    /// The prose kind this facet names, for the two rows under `Prose`.
    pub(in crate::workspace) fn prose_kind(self) -> Option<ProseKind> {
        match self {
            Self::ProseAnswer => Some(ProseKind::Answer),
            Self::ProsePreamble => Some(ProseKind::Preamble),
            _ => None,
        }
    }

    pub(in crate::workspace) fn category(self) -> Option<ToolCategory> {
        match self {
            Self::Thinking
            | Self::Prose
            | Self::ProseAnswer
            | Self::ProsePreamble
            | Self::Tools => None,
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
            Self::ProseAnswer => "prose_answer",
            Self::ProsePreamble => "prose_preamble",
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

/// The two kinds of agent prose. Codex labels them on the wire; an agent that
/// does not label its messages emits only [`Self::Answer`], so a filter aimed at
/// preambles is inert there rather than wrong.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::workspace) enum ProseKind {
    /// The reply itself — what every unlabelled message is.
    Answer,
    /// A preamble the agent wrote before the work it is about. Belongs to the
    /// step that follows it, which is why hiding it can empty a step out.
    Preamble,
}

impl ProseKind {
    fn of(phase: MessagePhase) -> Self {
        match phase {
            MessagePhase::Answer => Self::Answer,
            MessagePhase::Commentary => Self::Preamble,
        }
    }
}

/// Which prose the pane shows.
///
/// Mirrors [`ToolSelection`], and for the same reason: a stored list naming only
/// the parent (`prose`) means every kind under it, so a pane saved before these
/// kinds existed keeps showing all of its prose. There are exactly two kinds, so
/// a partial selection is one of them — [`Self::Only`] rather than a set that
/// could hold none and have to mean [`Self::Excluded`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::workspace) enum ProseSelection {
    Excluded,
    All,
    Only(ProseKind),
}

/// Whether a parented section is fully on, partly on, or off — what its
/// tri-state parent checkbox draws.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::workspace) enum SectionState {
    Off,
    Partial,
    On,
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
    prose: ProseSelection,
    tools: ToolSelection,
}

impl Default for DisplayFilter {
    /// A pane opens showing its whole transcript, so every kind starts visible.
    fn default() -> Self {
        Self {
            thinking: true,
            prose: ProseSelection::All,
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
    /// visible set — except for the lists it could parse no facet out of.
    ///
    /// Those meant "unfiltered", and not only when the list was empty: the old
    /// reader produced an all-off filter for a list of tokens it did not know,
    /// then short-circuited to "show everything" on seeing it was all off. The
    /// current reading has no such short-circuit, so a pane storing a facet this
    /// build has since removed would come back showing nothing. The condition is
    /// "no facet parsed", not "no token present".
    pub(in crate::workspace) fn from_legacy_tokens(tokens: &[String]) -> Self {
        if !tokens
            .iter()
            .any(|token| FilterFacet::from_token(token).is_some())
        {
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
            prose: ProseSelection::Excluded,
            tools: ToolSelection::Excluded,
        };
        let mut tools = false;
        let mut categories = ToolCategorySet::default();
        let mut prose = false;
        let (mut answer, mut preamble) = (false, false);
        for facet in tokens.into_iter().filter_map(FilterFacet::from_token) {
            match facet {
                FilterFacet::Thinking => filter.thinking = true,
                FilterFacet::Prose => prose = true,
                FilterFacet::ProseAnswer => answer = true,
                FilterFacet::ProsePreamble => preamble = true,
                FilterFacet::Tools => tools = true,
                _ => categories.insert(facet.category().expect("tool facet")),
            }
        }
        // Naming neither kind means the whole section, which is what a list
        // written before the kinds existed looks like.
        filter.prose = match (answer, preamble) {
            (false, false) if prose => ProseSelection::All,
            (false, false) => ProseSelection::Excluded,
            (true, true) => ProseSelection::All,
            (true, false) => ProseSelection::Only(ProseKind::Answer),
            (false, true) => ProseSelection::Only(ProseKind::Preamble),
        };
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
        match self.prose {
            ProseSelection::Excluded => {}
            ProseSelection::All => out.push(FilterFacet::Prose.token()),
            ProseSelection::Only(kind) => {
                out.push(FilterFacet::Prose.token());
                out.push(match kind {
                    ProseKind::Answer => FilterFacet::ProseAnswer.token(),
                    ProseKind::Preamble => FilterFacet::ProsePreamble.token(),
                });
            }
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
            FilterFacet::Prose => !matches!(self.prose, ProseSelection::Excluded),
            FilterFacet::ProseAnswer | FilterFacet::ProsePreamble => match self.prose {
                ProseSelection::Excluded => false,
                ProseSelection::All => true,
                ProseSelection::Only(kind) => Some(kind) == facet.prose_kind(),
            },
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
            FilterFacet::Prose => {
                self.prose = match self.prose {
                    ProseSelection::Excluded => ProseSelection::All,
                    ProseSelection::All | ProseSelection::Only(_) => ProseSelection::Excluded,
                }
            }
            FilterFacet::ProseAnswer | FilterFacet::ProsePreamble => {
                let kind = facet.prose_kind().expect("prose facet");
                let (answer, preamble) = (
                    self.contains(FilterFacet::ProseAnswer),
                    self.contains(FilterFacet::ProsePreamble),
                );
                let (answer, preamble) = match kind {
                    ProseKind::Answer => (!answer, preamble),
                    ProseKind::Preamble => (answer, !preamble),
                };
                self.prose = match (answer, preamble) {
                    (true, true) => ProseSelection::All,
                    (true, false) => ProseSelection::Only(ProseKind::Answer),
                    (false, true) => ProseSelection::Only(ProseKind::Preamble),
                    (false, false) => ProseSelection::Excluded,
                };
            }
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

    /// How full the section `parent` stands for is — what its tri-state
    /// checkbox draws. Keyed off the parent facet so a second parented axis
    /// cannot silently read another axis's selection.
    pub(in crate::workspace) fn section_state(self, parent: FilterFacet) -> SectionState {
        match parent {
            FilterFacet::Prose => match self.prose {
                ProseSelection::Excluded => SectionState::Off,
                ProseSelection::All => SectionState::On,
                ProseSelection::Only(_) => SectionState::Partial,
            },
            FilterFacet::Tools => match self.tools {
                ToolSelection::Excluded => SectionState::Off,
                ToolSelection::All => SectionState::On,
                ToolSelection::Some(_) => SectionState::Partial,
            },
            // Every other facet is a row, not a section: it is exactly as on as
            // its own checkbox.
            other => {
                if self.contains(other) {
                    SectionState::On
                } else {
                    SectionState::Off
                }
            }
        }
    }

    /// Turn a whole parented section on or off in one move.
    pub(in crate::workspace) fn with_section(mut self, parent: FilterFacet, on: bool) -> Self {
        match parent {
            FilterFacet::Prose => {
                self.prose = if on {
                    ProseSelection::All
                } else {
                    ProseSelection::Excluded
                }
            }
            FilterFacet::Tools => {
                self.tools = if on {
                    ToolSelection::All
                } else {
                    ToolSelection::Excluded
                }
            }
            other if self.contains(other) != on => return self.toggled(other),
            _ => {}
        }
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
            // A parent stands for its whole section, so exactly one of the two
            // speaks for it: the parent when the section is entirely off ("Tool
            // calls", not all five categories), the rows when it is partly on.
            .filter(|facet| match facet.axis().parent() {
                Some(parent) if *facet == parent => self.section_state(parent) == SectionState::Off,
                Some(parent) => self.section_state(parent) != SectionState::Off,
                None => true,
            })
            .collect()
    }

    /// Prompts, permissions, and failures are never filtered.
    pub(in crate::workspace) fn matches(self, item: &ChatItem) -> bool {
        match item {
            ChatItem::UserText(_) | ChatItem::Permission(_) | ChatItem::Failure(_) => true,
            ChatItem::Thinking { .. } => self.thinking,
            ChatItem::AssistantText { phase, .. } => match self.prose {
                ProseSelection::Excluded => false,
                ProseSelection::All => true,
                ProseSelection::Only(kind) => kind == ProseKind::of(*phase),
            },
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
