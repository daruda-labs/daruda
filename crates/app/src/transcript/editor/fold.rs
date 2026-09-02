//! The fold-rule editor: a preset strip, a turn-column switch, and one rule row
//! per foldable block.

use std::rc::Rc;

use gpui::{AnyElement, App, IntoElement, SharedString, Window, div, prelude::*, px};

use super::state::FoldEditorState;
use super::{ResetSpec, fixed_region, panel_heading, reset_footer, scroll_region};
use crate::surface::strings as s;
use crate::transcript::fold_mode::{BlockRule, FoldBlock, FoldMode, FoldPreset, TurnPosition};
use crate::transcript::tool_category::ToolCategory;
use crate::ui::theme;
use crate::ui::{Disableable as _, Divider, Selectable as _, button, button_group, tab, tab_bar};

const RULES: [BlockRule; 3] = [
    BlockRule::Builtin,
    BlockRule::Expanded,
    BlockRule::Collapsed,
];

pub(crate) type FoldRuleEdit = Rc<dyn Fn(FoldMode, &mut Window, &mut App)>;
pub(crate) type FoldPresetPress = Rc<dyn Fn(Option<FoldPreset>, &mut Window, &mut App)>;
pub(crate) type FoldTurnPress = Rc<dyn Fn(TurnPosition, &mut App)>;

/// What the editor does with a click. Each host binds these to its own store;
/// the editor itself holds no state and writes nothing.
pub(crate) struct FoldEditorActions {
    pub on_change: FoldRuleEdit,
    /// A press on the preset strip, as the segment itself — resolving which
    /// matrix that segment stands for is the host's, because the host is what
    /// holds the [`FoldEditorState`] the `Custom` segment points into.
    pub on_preset: FoldPresetPress,
    pub on_turn: FoldTurnPress,
    pub reset: Option<ResetSpec>,
}

/// The value text for a mode — what a chip or a settings row shows without
/// opening the editor. Shared so the two readings cannot diverge.
pub(crate) fn mode_value(mode: FoldMode) -> String {
    match mode.preset() {
        Some(preset) => preset_label(preset),
        None => s::agent_chat_fold_mode_custom(),
    }
}

pub(crate) fn fold_editor(
    mode: FoldMode,
    state: FoldEditorState,
    id_prefix: &str,
    font_size: f32,
    actions: FoldEditorActions,
    cx: &App,
) -> AnyElement {
    let turn = state.turn();
    let mut rows = Vec::new();
    for block in FoldBlock::ALL {
        rows.push(block_rule_row(mode, turn, block, id_prefix, &actions, cx));
        if block == FoldBlock::Tool {
            rows.extend(
                ToolCategory::ALL
                    .into_iter()
                    .map(|category| tool_rule_row(mode, turn, category, id_prefix, &actions, cx)),
            );
        }
    }

    div()
        .flex_1()
        .min_h(px(0.))
        .overflow_hidden()
        .flex()
        .flex_col()
        .gap(px(theme::GAP_LG))
        .text_size(px(font_size))
        .child(
            fixed_region()
                .gap(px(theme::GAP_LG))
                .child(panel_heading(s::agent_chat_fold_editor_presets(), cx))
                .child(preset_group(mode, state, id_prefix, &actions, cx))
                // Separates "set a value" from "switch what is being edited":
                // the strip above is a choice, the tabs below are a view swap.
                .child(Divider::horizontal())
                .child(turn_tabs(turn, id_prefix, &actions)),
        )
        .child(
            scroll_region(SharedString::from(format!("{id_prefix}-fold-rules-scroll")))
                .children(rows),
        )
        .child(reset_footer(
            SharedString::from(format!("{id_prefix}-fold-reset")),
            s::agent_chat_fold_editor_reset_default(),
            actions.reset,
        ))
        .into_any_element()
}

/// The strip's segments: the three presets plus the state a hand-edited matrix
/// lands in. `Custom` is not a preset — it re-selects the matrix the user last
/// edited, so choosing a preset does not throw that work away.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PresetSegment {
    Preset(FoldPreset),
    Custom,
}

impl PresetSegment {
    /// The length is derived from [`FoldPreset::ALL`], so a new preset fails to
    /// compile here rather than silently dropping off the strip.
    const ALL: [PresetSegment; FoldPreset::ALL.len() + 1] = [
        Self::Preset(FoldPreset::Auto),
        Self::Preset(FoldPreset::Summary),
        Self::Preset(FoldPreset::Expanded),
        Self::Custom,
    ];

    fn preset(self) -> Option<FoldPreset> {
        match self {
            Self::Preset(preset) => Some(preset),
            Self::Custom => None,
        }
    }

    fn token(self) -> &'static str {
        match self {
            Self::Preset(preset) => preset_token(preset),
            Self::Custom => "custom",
        }
    }

    fn label(self) -> String {
        match self {
            Self::Preset(preset) => preset_label(preset),
            Self::Custom => s::agent_chat_fold_mode_custom(),
        }
    }

    /// A hand-edited matrix matches no preset, which is exactly the state
    /// `Custom` names.
    fn is_selected(self, mode: FoldMode) -> bool {
        mode.preset() == self.preset()
    }

    fn is_enabled(self, mode: FoldMode, state: FoldEditorState) -> bool {
        match self {
            Self::Preset(_) => true,
            Self::Custom => mode.preset().is_none() || state.custom().is_some(),
        }
    }
}

fn preset_group(
    mode: FoldMode,
    state: FoldEditorState,
    id_prefix: &str,
    actions: &FoldEditorActions,
    cx: &App,
) -> impl IntoElement + use<> {
    let on_preset = actions.on_preset.clone();
    button_group(SharedString::from(format!("{id_prefix}-fold-presets")), cx)
        .children(PresetSegment::ALL.into_iter().map(|segment| {
            button(
                SharedString::from(format!("{id_prefix}-fold-preset-{}", segment.token())),
                segment.label(),
            )
            .selected(segment.is_selected(mode))
            .disabled(!segment.is_enabled(mode, state))
        }))
        .on_click(move |indices, window, app| {
            let Some(segment) = indices.first().and_then(|&ix| PresetSegment::ALL.get(ix)) else {
                return;
            };
            on_preset(segment.preset(), window, app);
        })
}

/// Which matrix column the rows below edit — a view switch, not a value, so it
/// reads as tabs rather than as a third segmented strip in the same popover.
fn turn_tabs(
    current: TurnPosition,
    id_prefix: &str,
    actions: &FoldEditorActions,
) -> impl IntoElement + use<> {
    let on_turn = actions.on_turn.clone();
    let active_ix = TurnPosition::ALL
        .iter()
        .position(|turn| *turn == current)
        .unwrap_or(0);
    tab_bar(SharedString::from(format!("{id_prefix}-fold-turns")))
        .w_full()
        .gap(px(0.))
        .selected_index(active_ix)
        .children(
            TurnPosition::ALL
                .into_iter()
                .map(|turn| tab(SharedString::from(turn_label(turn)))),
        )
        .on_click(move |ix, _window, app| {
            let Some(&turn) = TurnPosition::ALL.get(*ix) else {
                return;
            };
            on_turn(turn, app);
        })
}

fn block_rule_row(
    mode: FoldMode,
    turn: TurnPosition,
    block: FoldBlock,
    id_prefix: &str,
    actions: &FoldEditorActions,
    cx: &App,
) -> AnyElement {
    let on_change = actions.on_change.clone();
    rule_row(
        block_label(block),
        false,
        SharedString::from(format!(
            "{id_prefix}-fold-rule-{}-{}",
            turn.token(),
            block.token()
        )),
        mode.rule(turn, block),
        cx,
        move |rule, window, app| on_change(mode.with_rule(turn, block, rule), window, app),
    )
}

fn tool_rule_row(
    mode: FoldMode,
    turn: TurnPosition,
    category: ToolCategory,
    id_prefix: &str,
    actions: &FoldEditorActions,
    cx: &App,
) -> AnyElement {
    let on_change = actions.on_change.clone();
    rule_row(
        tool_category_label(category),
        true,
        SharedString::from(format!(
            "{id_prefix}-fold-tool-rule-{}-{}",
            turn.token(),
            category.token()
        )),
        mode.tool_rule(turn, category),
        cx,
        move |rule, window, app| on_change(mode.with_tool_rule(turn, category, rule), window, app),
    )
}

fn rule_row(
    label: String,
    nested: bool,
    id: SharedString,
    current: BlockRule,
    cx: &App,
    on_change: impl Fn(BlockRule, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let button_id_prefix = id.clone();
    div()
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::GAP_LG))
        .when(nested, |row| {
            row.pl(px(theme::TRANSCRIPT_EDITOR_NEST_INDENT))
        })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(SharedString::from(label)),
        )
        .child(
            button_group(id, cx)
                .children(RULES.into_iter().map(|rule| {
                    button(
                        SharedString::from(format!("{button_id_prefix}-{}", rule_token(rule))),
                        rule_label(rule),
                    )
                    .selected(rule == current)
                }))
                .on_click(move |indices, window, app| {
                    if let Some(&ix) = indices.first() {
                        on_change(RULES[ix], window, app);
                    }
                }),
        )
        .into_any_element()
}

fn preset_token(preset: FoldPreset) -> &'static str {
    match preset {
        FoldPreset::Auto => "auto",
        FoldPreset::Summary => "summary",
        FoldPreset::Expanded => "expanded",
    }
}

fn rule_token(rule: BlockRule) -> &'static str {
    rule.token().unwrap_or("builtin")
}

fn preset_label(preset: FoldPreset) -> String {
    match preset {
        FoldPreset::Auto => s::agent_chat_fold_mode_auto(),
        FoldPreset::Summary => s::agent_chat_fold_mode_summary(),
        FoldPreset::Expanded => s::agent_chat_fold_mode_expanded(),
    }
}

fn turn_label(turn: TurnPosition) -> String {
    match turn {
        TurnPosition::Past => s::agent_chat_fold_editor_earlier_turns(),
        TurnPosition::Last => s::agent_chat_fold_editor_recent_turn(),
    }
}

fn rule_label(rule: BlockRule) -> String {
    match rule {
        BlockRule::Builtin => s::agent_chat_fold_editor_rule_builtin(),
        BlockRule::Expanded => s::agent_chat_fold_editor_rule_expanded(),
        BlockRule::Collapsed => s::agent_chat_fold_editor_rule_collapsed(),
    }
}

fn block_label(block: FoldBlock) -> String {
    match block {
        FoldBlock::Response => s::agent_chat_fold_block_response(),
        FoldBlock::ToolGroup => s::agent_chat_fold_block_tool_group(),
        FoldBlock::Tool => s::agent_chat_fold_block_tool(),
        FoldBlock::Subagent => s::agent_chat_fold_block_subagent(),
        FoldBlock::Thinking => s::agent_chat_fold_block_thinking(),
        FoldBlock::ThinkingGroup => s::agent_chat_fold_block_thinking_group(),
        FoldBlock::Assistant => s::agent_chat_fold_block_assistant(),
        FoldBlock::Diff => s::agent_chat_fold_block_diff(),
        FoldBlock::RawInput => s::agent_chat_fold_block_raw_input(),
    }
}

fn tool_category_label(category: ToolCategory) -> String {
    match category {
        ToolCategory::Read => s::agent_chat_filter_tool_read(),
        ToolCategory::Edit => s::agent_chat_filter_tool_edit(),
        ToolCategory::Search => s::agent_chat_filter_tool_search(),
        ToolCategory::Run => s::agent_chat_filter_tool_run(),
        ToolCategory::Other => s::agent_chat_filter_tool_other(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_editor_option_has_a_label() {
        for segment in PresetSegment::ALL {
            assert!(!segment.label().is_empty(), "{segment:?}");
            assert!(!segment.token().is_empty(), "{segment:?}");
        }
        for block in FoldBlock::ALL {
            assert!(!block_label(block).is_empty(), "{block:?}");
        }
        for category in ToolCategory::ALL {
            assert!(!tool_category_label(category).is_empty(), "{category:?}");
        }
        for rule in RULES {
            assert!(!rule_label(rule).is_empty(), "{rule:?}");
        }
        for turn in TurnPosition::ALL {
            assert!(!turn_label(turn).is_empty(), "{turn:?}");
        }
    }

    /// Exactly one segment is selected at a time, and a hand-edited matrix
    /// selects `Custom` rather than leaving the strip blank.
    #[test]
    fn one_segment_is_selected_for_every_mode() {
        let matrix = FoldPreset::Summary.mode().with_rule(
            TurnPosition::Past,
            FoldBlock::Thinking,
            BlockRule::Collapsed,
        );
        for mode in FoldPreset::ALL
            .map(FoldPreset::mode)
            .into_iter()
            .chain([matrix])
        {
            let selected: Vec<_> = PresetSegment::ALL
                .into_iter()
                .filter(|segment| segment.is_selected(mode))
                .collect();
            assert_eq!(selected.len(), 1, "{mode:?}");
        }
        assert_eq!(
            PresetSegment::ALL
                .into_iter()
                .find(|s| s.is_selected(matrix)),
            Some(PresetSegment::Custom)
        );
    }

    /// `Custom` is offered only when it has somewhere to go — the current
    /// matrix, or a remembered one.
    #[test]
    fn custom_is_disabled_until_something_is_edited() {
        let fresh = FoldEditorState::default();
        assert!(!PresetSegment::Custom.is_enabled(FoldPreset::Auto.mode(), fresh));
        let mut edited = FoldEditorState::default();
        let matrix = FoldPreset::Summary.mode().with_rule(
            TurnPosition::Last,
            FoldBlock::Diff,
            BlockRule::Expanded,
        );
        edited.remember(FoldPreset::Auto.mode(), matrix);
        assert!(PresetSegment::Custom.is_enabled(FoldPreset::Auto.mode(), edited));
        assert!(
            PresetSegment::Custom.is_enabled(matrix, fresh),
            "already on one"
        );
    }

    #[test]
    fn the_value_text_names_the_preset_or_says_custom() {
        assert_eq!(
            mode_value(FoldPreset::Auto.mode()),
            s::agent_chat_fold_mode_auto()
        );
        let matrix = FoldPreset::Summary.mode().with_rule(
            TurnPosition::Past,
            FoldBlock::Thinking,
            BlockRule::Collapsed,
        );
        assert_eq!(mode_value(matrix), s::agent_chat_fold_mode_custom());
    }
}
