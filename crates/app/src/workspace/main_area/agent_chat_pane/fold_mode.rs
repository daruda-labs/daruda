//! Fold defaults by turn position and block kind. Explicit user choices still
//! override these rules; tail and filter rows remain owned by their chips.

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

    fn token(self) -> &'static str {
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
    Step,
    ToolGroup,
    Tool,
    Subagent,
    Thinking,
    Assistant,
    Diff,
    RawInput,
}

impl FoldBlock {
    /// Blocks in menu and serialization order.
    pub(in crate::workspace) const ALL: [FoldBlock; 9] = [
        Self::Response,
        Self::Step,
        Self::ToolGroup,
        Self::Tool,
        Self::Subagent,
        Self::Thinking,
        Self::Assistant,
        Self::Diff,
        Self::RawInput,
    ];

    const fn index(self) -> usize {
        match self {
            Self::Response => 0,
            Self::Step => 1,
            Self::ToolGroup => 2,
            Self::Tool => 3,
            Self::Subagent => 4,
            Self::Thinking => 5,
            Self::Assistant => 6,
            Self::Diff => 7,
            Self::RawInput => 8,
        }
    }

    fn token(self) -> &'static str {
        match self {
            Self::Response => "response",
            Self::Step => "step",
            Self::ToolGroup => "tool_group",
            Self::Tool => "tool",
            Self::Subagent => "subagent",
            Self::Thinking => "thinking",
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
    fn token(self) -> Option<&'static str> {
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
                mode.set(TurnPosition::Last, FoldBlock::Step, BlockRule::Expanded);
            }
        }
        mode
    }
}

/// One [`BlockRule`] per turn position and block kind.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(in crate::workspace) struct FoldMode {
    rules: [[BlockRule; FoldBlock::ALL.len()]; TurnPosition::ALL.len()],
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
        }
    }

    pub(in crate::workspace) fn rule(self, turn: TurnPosition, block: FoldBlock) -> BlockRule {
        self.rules[turn.index()][block.index()]
    }

    fn set(&mut self, turn: TurnPosition, block: FoldBlock, rule: BlockRule) {
        self.rules[turn.index()][block.index()] = rule;
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
        }
        out
    }
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
    fn expanded_opens_past_responses_and_keeps_settled_steps_open() {
        let ex = FoldPreset::Expanded.mode();
        assert_eq!(
            ex.rule(TurnPosition::Past, FoldBlock::Response),
            BlockRule::Expanded
        );
        assert_eq!(
            ex.rule(TurnPosition::Last, FoldBlock::Response),
            BlockRule::Expanded
        );
        assert_eq!(
            ex.rule(TurnPosition::Last, FoldBlock::Step),
            BlockRule::Expanded
        );
        assert_eq!(
            ex.rule(TurnPosition::Past, FoldBlock::Step),
            BlockRule::Builtin
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
