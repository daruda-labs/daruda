//! Fold defaults by turn position and block kind. Explicit user choices still
//! override these rules; tail and filter rows remain owned by their chips.

use crate::transcript::tool_category::ToolCategory;

/// Whether a block belongs to the newest turn or history.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TurnPosition {
    Past,
    Last,
}

impl TurnPosition {
    pub(crate) const ALL: [TurnPosition; 2] = [Self::Past, Self::Last];

    const fn index(self) -> usize {
        match self {
            Self::Past => 0,
            Self::Last => 1,
        }
    }

    pub(crate) fn token(self) -> &'static str {
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
pub(crate) enum FoldBlock {
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
    pub(crate) const ALL: [FoldBlock; 9] = [
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

    pub(crate) fn token(self) -> &'static str {
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

/// How one matrix cell folds its block.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum BlockRule {
    Expanded,
    Collapsed,
    /// Open while the block is still being produced, folded once it settles.
    WhileRunning,
}

impl BlockRule {
    /// Rules in menu order.
    pub(crate) const ALL: [BlockRule; 3] = [Self::Expanded, Self::Collapsed, Self::WhileRunning];

    pub(crate) fn token(self) -> &'static str {
        match self {
            Self::Expanded => "expanded",
            Self::Collapsed => "collapsed",
            Self::WhileRunning => "running",
        }
    }

    fn from_token(token: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.token() == token)
    }

    /// Whether a block under this rule is open, given whether it still runs.
    pub(crate) fn is_expanded(self, active: bool) -> bool {
        match self {
            Self::Expanded => true,
            Self::Collapsed => false,
            Self::WhileRunning => active,
        }
    }
}

/// The rule every cell starts on — what the `summary` preset states.
const fn base_rule(block: FoldBlock) -> BlockRule {
    match block {
        FoldBlock::Response
        | FoldBlock::ToolGroup
        | FoldBlock::Thinking
        | FoldBlock::ThinkingGroup => BlockRule::WhileRunning,
        FoldBlock::Assistant | FoldBlock::Diff => BlockRule::Expanded,
        FoldBlock::Tool | FoldBlock::Subagent | FoldBlock::RawInput => BlockRule::Collapsed,
    }
}

/// Named fold-mode presets offered by the chip.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum FoldPreset {
    /// Auto reproduces the shipped default behavior exactly, so a pane that
    /// never picks a mode looks the same as one that picks this.
    #[default]
    Auto,
    Summary,
    Expanded,
}

impl FoldPreset {
    pub(crate) const ALL: [FoldPreset; 3] = [Self::Auto, Self::Summary, Self::Expanded];

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

    pub(crate) fn mode(self) -> FoldMode {
        let mut mode = FoldMode::base();
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
///
/// The [`FoldBlock::Tool`] row is not a cell of its own: a tool card always
/// folds by its category, so the row reads and writes every category at once.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct FoldMode {
    /// The `Tool` slot holds its base value and is never read.
    rules: [[BlockRule; FoldBlock::ALL.len()]; TurnPosition::ALL.len()],
    tool_rules: [[BlockRule; ToolCategory::ALL.len()]; TurnPosition::ALL.len()],
}

impl Default for FoldMode {
    fn default() -> Self {
        FoldPreset::default().mode()
    }
}

impl FoldMode {
    fn base() -> Self {
        Self {
            rules: [FoldBlock::ALL.map(base_rule); TurnPosition::ALL.len()],
            tool_rules: [[base_rule(FoldBlock::Tool); ToolCategory::ALL.len()];
                TurnPosition::ALL.len()],
        }
    }

    /// The categories the tool row stands for.
    fn tool_card_categories() -> impl Iterator<Item = ToolCategory> {
        ToolCategory::ALL
            .into_iter()
            .filter(|category| category.folds_as_a_tool_card())
    }

    /// The rule a block row states. `None` only for [`FoldBlock::Tool`] whose
    /// categories disagree, which no single rule describes.
    pub(crate) fn rule(self, turn: TurnPosition, block: FoldBlock) -> Option<BlockRule> {
        if block != FoldBlock::Tool {
            return Some(self.rules[turn.index()][block.index()]);
        }
        let mut rules = Self::tool_card_categories().map(|c| self.tool_rule(turn, c));
        let first = rules.next()?;
        rules.all(|rule| rule == first).then_some(first)
    }

    fn set(&mut self, turn: TurnPosition, block: FoldBlock, rule: BlockRule) {
        if block == FoldBlock::Tool {
            for category in Self::tool_card_categories() {
                self.tool_rules[turn.index()][category.index()] = rule;
            }
        } else {
            self.rules[turn.index()][block.index()] = rule;
        }
    }

    pub(crate) fn with_rule(
        mut self,
        turn: TurnPosition,
        block: FoldBlock,
        rule: BlockRule,
    ) -> Self {
        self.set(turn, block, rule);
        self
    }

    pub(crate) fn tool_rule(self, turn: TurnPosition, category: ToolCategory) -> BlockRule {
        self.tool_rules[turn.index()][category.index()]
    }

    /// A category outside the tool row folds by its own block, so a rule for
    /// it would never apply and is dropped.
    pub(crate) fn with_tool_rule(
        mut self,
        turn: TurnPosition,
        category: ToolCategory,
        rule: BlockRule,
    ) -> Self {
        if category.folds_as_a_tool_card() {
            self.tool_rules[turn.index()][category.index()] = rule;
        }
        self
    }

    /// The matching preset, or `None` for a custom matrix.
    pub(crate) fn preset(self) -> Option<FoldPreset> {
        FoldPreset::ALL.into_iter().find(|p| p.mode() == self)
    }

    /// Parse tokens left-to-right; a preset replaces the matrix and cell tokens
    /// override it. Within one preset's cells a category token narrows the
    /// `tool` row wherever it is written. Unknown tokens are ignored.
    pub(crate) fn from_tokens<'a>(tokens: impl IntoIterator<Item = &'a str>) -> Self {
        let mut mode = Self::default();
        let mut categories = Vec::new();
        for token in tokens {
            if let Some(preset) = FoldPreset::from_token(token) {
                mode = preset.mode();
                categories.clear();
            } else if let Some(cell) = parse_tool_cell(token) {
                categories.push(cell);
            } else if let Some((turn, block, rule)) = parse_cell(token) {
                mode.set(turn, block, rule);
            }
        }
        for (turn, category, rule) in categories {
            mode = mode.with_tool_rule(turn, category, rule);
        }
        mode
    }

    /// Serialize as a preset, or as `summary` plus the cells that differ from it.
    pub(crate) fn tokens(self) -> Vec<String> {
        if let Some(preset) = self.preset() {
            return vec![preset.token().to_owned()];
        }
        let base = Self::base();
        let mut out = vec![FoldPreset::Summary.token().to_owned()];
        for turn in TurnPosition::ALL {
            for block in FoldBlock::ALL {
                match self.rule(turn, block) {
                    Some(rule) if Some(rule) == base.rule(turn, block) => {}
                    Some(rule) => out.push(format!(
                        "{}.{}={}",
                        turn.token(),
                        block.token(),
                        rule.token()
                    )),
                    // A mixed tool row: each category that left the base.
                    None => {
                        for category in Self::tool_card_categories() {
                            let rule = self.tool_rule(turn, category);
                            if rule != base.tool_rule(turn, category) {
                                out.push(format!(
                                    "{}.tool.{}={}",
                                    turn.token(),
                                    category.token(),
                                    rule.token()
                                ));
                            }
                        }
                    }
                }
            }
        }
        out
    }
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
        let summary = FoldPreset::Summary.mode();
        for turn in TurnPosition::ALL {
            for block in FoldBlock::ALL {
                let expected = if (turn, block) == (TurnPosition::Last, FoldBlock::Response) {
                    Some(BlockRule::Expanded)
                } else {
                    summary.rule(turn, block)
                };
                assert_eq!(auto.rule(turn, block), expected, "{turn:?}/{block:?}");
            }
        }
    }

    #[test]
    fn summary_states_the_base_rule_for_every_cell() {
        let summary = FoldPreset::Summary.mode();
        for turn in TurnPosition::ALL {
            for block in FoldBlock::ALL {
                assert_eq!(
                    summary.rule(turn, block),
                    Some(base_rule(block)),
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
            Some(BlockRule::Expanded)
        );
        assert_eq!(
            ex.rule(TurnPosition::Last, FoldBlock::Response),
            Some(BlockRule::Expanded)
        );
        for block in [FoldBlock::ToolGroup, FoldBlock::ThinkingGroup] {
            assert_eq!(
                ex.rule(TurnPosition::Last, block),
                Some(BlockRule::Expanded)
            );
            assert_eq!(
                ex.rule(TurnPosition::Past, block),
                Some(BlockRule::WhileRunning)
            );
        }
    }

    #[test]
    fn while_running_follows_activity_and_the_others_ignore_it() {
        for active in [true, false] {
            assert!(BlockRule::Expanded.is_expanded(active));
            assert!(!BlockRule::Collapsed.is_expanded(active));
            assert_eq!(BlockRule::WhileRunning.is_expanded(active), active);
        }
    }

    #[test]
    fn the_tool_row_writes_every_tool_card_category() {
        let mode = FoldPreset::Summary.mode().with_rule(
            TurnPosition::Last,
            FoldBlock::Tool,
            BlockRule::WhileRunning,
        );
        for category in ToolCategory::ALL
            .into_iter()
            .filter(|c| c.folds_as_a_tool_card())
        {
            assert_eq!(
                mode.tool_rule(TurnPosition::Last, category),
                BlockRule::WhileRunning,
                "{category:?}"
            );
            assert_eq!(
                mode.tool_rule(TurnPosition::Past, category),
                BlockRule::Collapsed,
                "{category:?}"
            );
        }
        assert_eq!(
            mode.rule(TurnPosition::Last, FoldBlock::Tool),
            Some(BlockRule::WhileRunning)
        );
    }

    #[test]
    fn a_mixed_tool_row_states_no_single_rule() {
        let mode = FoldPreset::Summary.mode().with_tool_rule(
            TurnPosition::Last,
            ToolCategory::Edit,
            BlockRule::Expanded,
        );
        assert_eq!(mode.rule(TurnPosition::Last, FoldBlock::Tool), None);
        assert_eq!(
            mode.rule(TurnPosition::Past, FoldBlock::Tool),
            Some(BlockRule::Collapsed)
        );
    }

    #[test]
    fn a_rule_for_a_category_outside_the_tool_row_is_dropped() {
        let summary = FoldPreset::Summary.mode();
        let mode =
            summary.with_tool_rule(TurnPosition::Last, ToolCategory::Agent, BlockRule::Expanded);
        assert_eq!(mode, summary);
        assert_eq!(
            FoldMode::from_tokens(["summary", "last.tool.agent=expanded"]),
            summary
        );
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
        mode.set(
            TurnPosition::Past,
            FoldBlock::Assistant,
            BlockRule::WhileRunning,
        );
        assert_eq!(mode.preset(), None, "no preset covers this");
        assert_eq!(
            mode.tokens(),
            vec![
                "summary".to_owned(),
                "past.assistant=running".to_owned(),
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
            .with_tool_rule(
                TurnPosition::Past,
                ToolCategory::Run,
                BlockRule::WhileRunning,
            );
        assert_eq!(mode.preset(), None);
        assert_eq!(
            mode.tokens(),
            vec![
                "summary".to_owned(),
                "past.tool.run=running".to_owned(),
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
            Some(BlockRule::Expanded)
        );
        assert_eq!(
            mode.rule(TurnPosition::Last, FoldBlock::Tool),
            Some(BlockRule::Expanded)
        );
        assert_eq!(mode.preset(), None);
    }

    /// A category token narrows the `tool` row on either side of it, so a
    /// hand-ordered config cannot change meaning.
    #[test]
    fn a_category_token_narrows_the_tool_row_in_either_order() {
        let narrowed =
            FoldMode::from_tokens(["summary", "last.tool=expanded", "last.tool.read=collapsed"]);
        assert_eq!(
            narrowed.tool_rule(TurnPosition::Last, ToolCategory::Read),
            BlockRule::Collapsed
        );
        assert_eq!(
            narrowed.tool_rule(TurnPosition::Last, ToolCategory::Edit),
            BlockRule::Expanded
        );
        let reordered =
            FoldMode::from_tokens(["summary", "last.tool.read=collapsed", "last.tool=expanded"]);
        assert_eq!(reordered, narrowed);
        // A preset still discards every category written before it.
        let reset = FoldMode::from_tokens(["last.tool.read=expanded", "summary"]);
        assert_eq!(reset, FoldPreset::Summary.mode());
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
            "last.tool=running",
            "last.tool=builtin",
        ]);
        assert_eq!(
            mode.rule(TurnPosition::Last, FoldBlock::Tool),
            Some(BlockRule::WhileRunning)
        );
        assert_eq!(
            mode.tokens(),
            vec!["summary".to_owned(), "last.tool=running".to_owned()]
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
        for rule in BlockRule::ALL {
            assert_eq!(BlockRule::from_token(rule.token()), Some(rule));
        }
    }
}
