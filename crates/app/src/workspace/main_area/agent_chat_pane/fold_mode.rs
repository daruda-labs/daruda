//! Fold defaults by turn position and block kind. Explicit user choices still
//! override these rules; tail and filter rows remain owned by their chips.

use super::tool_category::ToolCategory;

/// Whether a block belongs to the newest turn or history.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::workspace) enum TurnPosition {
    Past,
    Last,
}

impl TurnPosition {
    pub(in crate::workspace) const ALL: [TurnPosition; 2] = [Self::Past, Self::Last];

    const fn index(self) -> usize {
        match self {
            Self::Past => 0,
            Self::Last => 1,
        }
    }

    pub(in crate::workspace) fn token(self) -> &'static str {
        match self {
            Self::Past => "past",
            Self::Last => "last",
        }
    }

    fn from_token(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.token() == token)
    }
}

/// Foldable block kinds controlled by a mode.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::workspace) enum FoldBlock {
    Response,
    ToolGroup,
    Tool,
    Subagent,
    Thinking,
    ThinkingGroup,
    Assistant,
    Diff,
    RawInput,
}

impl FoldBlock {
    /// Blocks in menu and serialization order.
    pub(in crate::workspace) const ALL: [FoldBlock; 9] = [
        Self::Response,
        Self::ToolGroup,
        Self::Tool,
        Self::Subagent,
        Self::Thinking,
        Self::ThinkingGroup,
        Self::Assistant,
        Self::Diff,
        Self::RawInput,
    ];

    const fn index(self) -> usize {
        match self {
            Self::Response => 0,
            Self::ToolGroup => 1,
            Self::Tool => 2,
            Self::Subagent => 3,
            Self::Thinking => 4,
            Self::ThinkingGroup => 5,
            Self::Assistant => 6,
            Self::Diff => 7,
            Self::RawInput => 8,
        }
    }

    pub(in crate::workspace) fn token(self) -> &'static str {
        match self {
            Self::Response => "response",
            Self::ToolGroup => "tool_group",
            Self::Tool => "tool",
            Self::Subagent => "subagent",
            Self::Thinking => "thinking",
            Self::ThinkingGroup => "thinking_group",
            Self::Assistant => "assistant",
            Self::Diff => "diff",
            Self::RawInput => "raw_input",
        }
    }

    fn from_token(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|b| b.token() == token)
    }
}

/// A mode's override for one matrix cell.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(in crate::workspace) enum BlockRule {
    #[default]
    Builtin,
    Expanded,
    Collapsed,
}

impl BlockRule {
    pub(in crate::workspace) fn token(self) -> Option<&'static str> {
        match self {
            Self::Builtin => None,
            Self::Expanded => Some("expanded"),
            Self::Collapsed => Some("collapsed"),
        }
    }

    fn from_token(token: &str) -> Option<Self> {
        [Self::Expanded, Self::Collapsed]
            .into_iter()
            .find(|r| r.token() == Some(token))
    }
}

/// Named fold-mode presets offered by the chip.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(in crate::workspace) enum FoldPreset {
    /// Auto reproduces the shipped default behavior exactly, so a pane that
    /// never picks a mode looks the same as one that picks this.
    #[default]
    Auto,
    Summary,
    Expanded,
}

impl FoldPreset {
    pub(in crate::workspace) const ALL: [FoldPreset; 3] =
        [Self::Auto, Self::Summary, Self::Expanded];

    fn token(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Summary => "summary",
            Self::Expanded => "expanded",
        }
    }

    fn from_token(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.token() == token)
    }

    pub(in crate::workspace) fn mode(self) -> FoldMode {
        let mut mode = FoldMode::neutral();
        match self {
            Self::Auto => mode.set(TurnPosition::Last, FoldBlock::Response, BlockRule::Expanded),
            Self::Summary => {}
            Self::Expanded => {
                mode.set(TurnPosition::Past, FoldBlock::Response, BlockRule::Expanded);
                mode.set(TurnPosition::Last, FoldBlock::Response, BlockRule::Expanded);
                // One level deeper on the newest turn: its groups open too.
                mode.set(
                    TurnPosition::Last,
                    FoldBlock::ToolGroup,
                    BlockRule::Expanded,
                );
                mode.set(
                    TurnPosition::Last,
                    FoldBlock::ThinkingGroup,
                    BlockRule::Expanded,
                );
            }
        }
        mode
    }
}

/// One [`BlockRule`] per turn position and block kind.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::workspace) struct FoldMode {
    rules: [[BlockRule; FoldBlock::ALL.len()]; TurnPosition::ALL.len()],
    tool_rules: [[BlockRule; ToolCategory::ALL.len()]; TurnPosition::ALL.len()],
}

impl Default for FoldMode {
    fn default() -> Self {
        FoldPreset::default().mode()
    }
}

impl FoldMode {
    fn neutral() -> Self {
        Self {
            rules: [[BlockRule::Builtin; FoldBlock::ALL.len()]; TurnPosition::ALL.len()],
            tool_rules: [[BlockRule::Builtin; ToolCategory::ALL.len()]; TurnPosition::ALL.len()],
        }
    }

    pub(in crate::workspace) fn rule(self, turn: TurnPosition, block: FoldBlock) -> BlockRule {
        self.rules[turn.index()][block.index()]
    }

    fn set(&mut self, turn: TurnPosition, block: FoldBlock, rule: BlockRule) {
        self.rules[turn.index()][block.index()] = rule;
    }

    pub(in crate::workspace) fn with_rule(
        mut self,
        turn: TurnPosition,
        block: FoldBlock,
        rule: BlockRule,
    ) -> Self {
        self.set(turn, block, rule);
        self
    }

    pub(in crate::workspace) fn tool_rule(
        self,
        turn: TurnPosition,
        category: ToolCategory,
    ) -> BlockRule {
        self.tool_rules[turn.index()][category.index()]
    }

    pub(in crate::workspace) fn with_tool_rule(
        mut self,
        turn: TurnPosition,
        category: ToolCategory,
        rule: BlockRule,
    ) -> Self {
        self.tool_rules[turn.index()][category.index()] = rule;
        self
    }

    /// The matching preset, or `None` for a custom matrix.
    pub(in crate::workspace) fn preset(self) -> Option<FoldPreset> {
        FoldPreset::ALL.into_iter().find(|p| p.mode() == self)
    }

    /// Parse tokens left-to-right; presets replace the matrix and cell tokens
    /// override it. Unknown tokens are ignored for forward compatibility.
    pub(in crate::workspace) fn from_tokens<'a>(tokens: impl IntoIterator<Item = &'a str>) -> Self {
        let mut mode = Self::default();
        for token in tokens {
            if let Some(preset) = FoldPreset::from_token(token) {
                mode = preset.mode();
            } else if let Some((turn, category, rule)) = parse_tool_cell(token) {
                mode.tool_rules[turn.index()][category.index()] = rule;
            } else if let Some((turn, block, rule)) = parse_cell(token) {
                mode.set(turn, block, rule);
            }
        }
        mode
    }

    /// Serialize as a preset or a neutral base plus cell overrides.
    pub(in crate::workspace) fn tokens(self) -> Vec<String> {
        if let Some(preset) = self.preset() {
            return vec![preset.token().to_owned()];
        }
        let mut out = vec![FoldPreset::Summary.token().to_owned()];
        for turn in TurnPosition::ALL {
            for block in FoldBlock::ALL {
                if let Some(rule) = self.rule(turn, block).token() {
                    out.push(format!("{}.{}={rule}", turn.token(), block.token()));
                }
            }
            for category in ToolCategory::ALL {
                if let Some(rule) = self.tool_rule(turn, category).token() {
                    out.push(format!("{}.tool.{}={rule}", turn.token(), category.token()));
                }
            }
        }
        out
    }
}

/// The preset tokens a stored `fold_mode` list can name, in menu order.
/// Re-exported as `crate::workspace::fold_preset_tokens` so the Settings agent
/// catalog offers exactly this vocabulary instead of a copy of it.
pub(crate) fn preset_tokens() -> [&'static str; FoldPreset::ALL.len()] {
    FoldPreset::ALL.map(FoldPreset::token)
}

fn parse_tool_cell(token: &str) -> Option<(TurnPosition, ToolCategory, BlockRule)> {
    let (cell, rule) = token.split_once('=')?;
    let mut parts = cell.split('.');
    let turn = TurnPosition::from_token(parts.next()?)?;
    if parts.next()? != FoldBlock::Tool.token() {
        return None;
    }
    let category = ToolCategory::from_token(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some((turn, category, BlockRule::from_token(rule)?))
}

fn parse_cell(token: &str) -> Option<(TurnPosition, FoldBlock, BlockRule)> {
    let (cell, rule) = token.split_once('=')?;
    let (turn, block) = cell.split_once('.')?;
    Some((
        TurnPosition::from_token(turn)?,
        FoldBlock::from_token(block)?,
        BlockRule::from_token(rule)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_mode_is_auto() {
        assert_eq!(FoldMode::default().preset(), Some(FoldPreset::Auto));
        assert_eq!(FoldMode::default(), FoldPreset::Auto.mode());
    }

    #[test]
    fn auto_pins_only_the_newest_response() {
        let auto = FoldPreset::Auto.mode();
        for turn in TurnPosition::ALL {
            for block in FoldBlock::ALL {
                let expected = if (turn, block) == (TurnPosition::Last, FoldBlock::Response) {
                    BlockRule::Expanded
                } else {
                    BlockRule::Builtin
                };
                assert_eq!(auto.rule(turn, block), expected, "{turn:?}/{block:?}");
            }
        }
    }

    #[test]
    fn summary_is_wholly_neutral() {
        let summary = FoldPreset::Summary.mode();
        for turn in TurnPosition::ALL {
            for block in FoldBlock::ALL {
                assert_eq!(
                    summary.rule(turn, block),
                    BlockRule::Builtin,
                    "{turn:?}/{block:?}"
                );
            }
        }
    }

    #[test]
    fn expanded_opens_past_responses_and_keeps_the_newest_groups_open() {
        let ex = FoldPreset::Expanded.mode();
        assert_eq!(
            ex.rule(TurnPosition::Past, FoldBlock::Response),
            BlockRule::Expanded
        );
        assert_eq!(
            ex.rule(TurnPosition::Last, FoldBlock::Response),
            BlockRule::Expanded
        );
        for block in [FoldBlock::ToolGroup, FoldBlock::ThinkingGroup] {
            assert_eq!(ex.rule(TurnPosition::Last, block), BlockRule::Expanded);
            assert_eq!(ex.rule(TurnPosition::Past, block), BlockRule::Builtin);
        }
    }

    #[test]
    fn the_two_axes_are_independent() {
        let auto = FoldPreset::Auto.mode();
        let summary = FoldPreset::Summary.mode();
        assert_eq!(
            auto.rule(TurnPosition::Past, FoldBlock::Response),
            summary.rule(TurnPosition::Past, FoldBlock::Response)
        );
        assert_ne!(
            auto.rule(TurnPosition::Last, FoldBlock::Response),
            summary.rule(TurnPosition::Last, FoldBlock::Response)
        );
    }

    #[test]
    fn every_preset_is_distinct() {
        for (i, a) in FoldPreset::ALL.into_iter().enumerate() {
            for b in FoldPreset::ALL.into_iter().skip(i + 1) {
                assert_ne!(a.mode(), b.mode(), "{a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn every_preset_round_trips_through_its_token() {
        for preset in FoldPreset::ALL {
            let mode = preset.mode();
            assert_eq!(mode.tokens(), vec![preset.token().to_owned()]);
            assert_eq!(
                FoldMode::from_tokens(mode.tokens().iter().map(String::as_str)),
                mode
            );
        }
    }

    #[test]
    fn a_custom_matrix_round_trips_cell_by_cell() {
        let mut mode = FoldPreset::Summary.mode();
        mode.set(TurnPosition::Last, FoldBlock::Tool, BlockRule::Expanded);
        mode.set(TurnPosition::Past, FoldBlock::Diff, BlockRule::Collapsed);
        assert_eq!(mode.preset(), None, "no preset covers this");
        assert_eq!(
            mode.tokens(),
            vec![
                "summary".to_owned(),
                "past.diff=collapsed".to_owned(),
                "last.tool=expanded".to_owned(),
            ]
        );
        assert_eq!(
            FoldMode::from_tokens(mode.tokens().iter().map(String::as_str)),
            mode
        );
    }

    #[test]
    fn tool_category_rules_round_trip_without_changing_other_cells() {
        let mode = FoldPreset::Auto
            .mode()
            .with_tool_rule(TurnPosition::Last, ToolCategory::Edit, BlockRule::Expanded)
            .with_tool_rule(TurnPosition::Past, ToolCategory::Run, BlockRule::Collapsed);
        assert_eq!(mode.preset(), None);
        assert_eq!(
            mode.tokens(),
            vec![
                "summary".to_owned(),
                "past.tool.run=collapsed".to_owned(),
                "last.response=expanded".to_owned(),
                "last.tool.edit=expanded".to_owned(),
            ]
        );
        assert_eq!(
            FoldMode::from_tokens(mode.tokens().iter().map(String::as_str)),
            mode
        );
    }

    #[test]
    fn a_cell_token_layers_on_the_preceding_preset() {
        let mode = FoldMode::from_tokens(["auto", "last.tool=expanded"]);
        assert_eq!(
            mode.rule(TurnPosition::Last, FoldBlock::Response),
            BlockRule::Expanded
        );
        assert_eq!(
            mode.rule(TurnPosition::Last, FoldBlock::Tool),
            BlockRule::Expanded
        );
        assert_eq!(mode.preset(), None);
    }

    #[test]
    fn a_later_preset_token_replaces_everything_before_it() {
        let mode = FoldMode::from_tokens(["auto", "last.tool=expanded", "summary"]);
        assert_eq!(mode, FoldPreset::Summary.mode());
    }

    #[test]
    fn no_tokens_means_the_shipped_default() {
        assert_eq!(FoldMode::from_tokens([]), FoldPreset::Auto.mode());
    }

    #[test]
    fn unknown_tokens_are_dropped_rather_than_failing() {
        let mode = FoldMode::from_tokens([
            "summary",
            "sideways.tool=expanded",
            "last.wormhole=expanded",
            "last.tool=sideways",
            "last.tool",
            "last.tool=collapsed",
        ]);
        assert_eq!(
            mode.rule(TurnPosition::Last, FoldBlock::Tool),
            BlockRule::Collapsed
        );
        assert_eq!(
            mode.tokens(),
            vec!["summary".to_owned(), "last.tool=collapsed".to_owned()]
        );
    }

    #[test]
    fn every_token_is_distinct_within_its_vocabulary() {
        for (i, a) in FoldBlock::ALL.into_iter().enumerate() {
            for b in FoldBlock::ALL.into_iter().skip(i + 1) {
                assert_ne!(a.token(), b.token(), "{a:?} vs {b:?}");
            }
            assert_eq!(FoldBlock::from_token(a.token()), Some(a));
        }
        for p in TurnPosition::ALL {
            assert_eq!(TurnPosition::from_token(p.token()), Some(p));
        }
        for p in FoldPreset::ALL {
            assert_eq!(FoldPreset::from_token(p.token()), Some(p));
        }
        assert_eq!(BlockRule::Builtin.token(), None);
    }
}
