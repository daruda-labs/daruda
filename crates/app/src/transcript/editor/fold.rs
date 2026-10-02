//! Lifecycle rules, with historical overrides behind an advanced disclosure.

use std::rc::Rc;

use gpui::{AnyElement, App, IntoElement, SharedString, Window, div, prelude::*, px};

use super::state::FoldEditorState;
use super::{ResetSpec, fixed_region, panel_heading, reset_footer, scroll_region};
use crate::surface::strings as s;
use crate::transcript::fold_mode::{FoldBlock, FoldMode, FoldPreset, TurnPosition};
use crate::transcript::tool_category::ToolCategory;
use crate::ui::theme;
use crate::ui::{
    ButtonVariants as _, Disableable as _, Divider, DropdownMenu as _, IconName, PopupMenuItem,
    Selectable as _, Sizable as _, button, button_group, checkbox,
};

pub(crate) type FoldRuleEdit = Rc<dyn Fn(FoldMode, &mut Window, &mut App)>;
pub(crate) type FoldPresetPress = Rc<dyn Fn(Option<FoldPreset>, &mut Window, &mut App)>;
pub(crate) type FoldToolsPress = Rc<dyn Fn(TurnPosition, &mut App)>;

/// What the editor does with a click. Each host binds these to its own store;
/// the editor itself holds no state and writes nothing.
pub(crate) struct FoldEditorActions {
    pub on_change: FoldRuleEdit,
    /// A press on the preset strip, as the segment itself — resolving which
    /// matrix that segment stands for is the host's, because the host is what
    /// holds the [`FoldEditorState`] the `Custom` segment points into.
    pub on_preset: FoldPresetPress,
    pub on_history_rules: Rc<dyn Fn(&mut App)>,
    /// Opens or shuts one section's tool categories.
    pub on_tools: FoldToolsPress,
    pub reset: Option<ResetSpec>,
}

/// The value text for a mode — what a chip or a settings row shows without
/// opening the editor. Shared so the two readings cannot diverge.
pub(crate) fn mode_value(mode: FoldMode) -> String {
    match mode.preset() {
        Some(preset) => preset_label(preset),
        None => s::agent_chat::fold_mode_custom(),
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
    let on_history = actions.on_change.clone();
    let history_mode = mode.with_collapse_history(!mode.collapse_history());
    let advanced = state.history_rules_open();
    let on_history_rules = actions.on_history_rules.clone();
    let mut rows = rule_rows(mode, TurnPosition::Last, state, id_prefix, &actions);
    rows.push(
        button(
            SharedString::from(format!("{id_prefix}-fold-advanced")),
            s::agent_chat::fold_editor_history_rules(),
        )
        .ghost()
        .xsmall()
        .tab_stop(true)
        .justify_start()
        .icon(if advanced {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        })
        .on_click(move |_, _, app| on_history_rules(app))
        .into_any_element(),
    );
    if advanced {
        rows.push(rule_headings(cx).into_any_element());
        rows.extend(rule_rows(
            mode,
            TurnPosition::Past,
            state,
            id_prefix,
            &actions,
        ));
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
                .child(preset_group(mode, state, id_prefix, &actions, cx))
                .child(rule_headings(cx)),
        )
        .child(
            scroll_region(SharedString::from(format!("{id_prefix}-fold-rules-scroll")))
                .children(rows),
        )
        .child(
            fixed_region()
                .child(Divider::horizontal())
                .child(panel_heading(s::agent_chat::fold_editor_history(), cx))
                .child(
                    checkbox(
                        SharedString::from(format!("{id_prefix}-fold-history")),
                        s::agent_chat::fold_editor_collapse_history(),
                        0,
                    )
                    .checked(mode.collapse_history())
                    .on_click(move |_, window, app| on_history(history_mode, window, app)),
                ),
        )
        .child(reset_footer(
            SharedString::from(format!("{id_prefix}-fold-reset")),
            actions.reset,
        ))
        .into_any_element()
}

fn rule_rows(
    mode: FoldMode,
    turn: TurnPosition,
    state: FoldEditorState,
    id: &str,
    actions: &FoldEditorActions,
) -> Vec<AnyElement> {
    let mut rows = Vec::new();
    for block in FoldBlock::ALL {
        rows.push(block_rule_row(mode, turn, block, id, actions));
        if block == FoldBlock::Tool {
            let on_tools = actions.on_tools.clone();
            rows.push(
                button(
                    SharedString::from(format!("{id}-fold-tools-{}", turn.token())),
                    s::agent_chat::fold_editor_tool_categories(),
                )
                .ghost()
                .xsmall()
                .tab_stop(true)
                .justify_start()
                .icon(if state.tools_open(turn) {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .on_click(move |_, _, app| on_tools(turn, app))
                .into_any_element(),
            );
            if state.tools_open(turn) {
                rows.extend(
                    ToolCategory::ALL
                        .into_iter()
                        .filter(|c| c.folds_as_a_tool_card())
                        .map(|c| tool_rule_row(mode, turn, c, id, actions)),
                );
            }
        }
    }
    rows
}

fn rule_headings(cx: &App) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(theme::GAP_SM))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(panel_heading(s::agent_chat::fold_editor_rules(), cx)),
        )
        .children(
            [
                s::agent_chat::fold_editor_during(),
                s::agent_chat::fold_editor_after(),
            ]
            .into_iter()
            .map(|label| {
                div()
                    .flex_none()
                    .w(px(theme::TRANSCRIPT_EDITOR_RULE_COLUMN_W))
                    .text_center()
                    .child(panel_heading(label, cx))
            }),
        )
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
            Self::Custom => s::agent_chat::fold_mode_custom(),
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
            .tab_stop(true)
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

fn block_rule_row(
    mode: FoldMode,
    turn: TurnPosition,
    block: FoldBlock,
    id_prefix: &str,
    actions: &FoldEditorActions,
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
        [
            mode.phase(turn, block, true),
            mode.phase(turn, block, false),
        ],
        Rc::new(move |active, expanded, window, app| {
            on_change(mode.with_phase(turn, block, active, expanded), window, app)
        }),
    )
}

fn tool_rule_row(
    mode: FoldMode,
    turn: TurnPosition,
    category: ToolCategory,
    id_prefix: &str,
    actions: &FoldEditorActions,
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
        [
            Some(mode.tool_rule(turn, category).is_expanded(true)),
            Some(mode.tool_rule(turn, category).is_expanded(false)),
        ],
        Rc::new(move |active, expanded, window, app| {
            on_change(
                mode.with_tool_phase(turn, category, active, expanded),
                window,
                app,
            )
        }),
    )
}

type PhaseEdit = Rc<dyn Fn(bool, bool, &mut Window, &mut App)>;

fn rule_row(
    label: String,
    nested: bool,
    id: SharedString,
    current: [Option<bool>; 2],
    on_change: PhaseEdit,
) -> AnyElement {
    div()
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::GAP_SM))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .when(nested, |label| {
                    label.pl(px(theme::TRANSCRIPT_EDITOR_NEST_INDENT))
                })
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(SharedString::from(label)),
        )
        .children(
            [true, false]
                .into_iter()
                .zip(current)
                .map(|(active, value)| {
                    let on_change = on_change.clone();
                    let label = match value {
                        Some(value) => expansion_label(value),
                        None => s::agent_chat::fold_editor_mixed(),
                    };
                    button(SharedString::from(format!("{id}-{active}")), label)
                        .outline()
                        .xsmall()
                        .tab_stop(true)
                        .w(px(theme::TRANSCRIPT_EDITOR_RULE_COLUMN_W))
                        .child(crate::ui::Icon::new(IconName::ChevronDown))
                        .dropdown_menu(crate::ui::menu_builder(move |menu, _, _| {
                            [true, false].into_iter().fold(menu, |menu, expanded| {
                                let on_change = on_change.clone();
                                menu.item(
                                    PopupMenuItem::new(expansion_label(expanded))
                                        .checked(value == Some(expanded))
                                        .on_click(move |_, window, app| {
                                            on_change(active, expanded, window, app)
                                        }),
                                )
                            })
                        }))
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

fn preset_label(preset: FoldPreset) -> String {
    match preset {
        FoldPreset::Auto => s::agent_chat::fold_mode_auto(),
        FoldPreset::Summary => s::agent_chat::fold_mode_summary(),
        FoldPreset::Expanded => s::agent_chat::fold_mode_expanded(),
    }
}

fn expansion_label(expanded: bool) -> String {
    if expanded {
        s::agent_chat::fold_editor_rule_expanded()
    } else {
        s::agent_chat::fold_editor_rule_collapsed()
    }
}

fn block_label(block: FoldBlock) -> String {
    match block {
        FoldBlock::Response => s::agent_chat::fold_block_response(),
        FoldBlock::ToolGroup => s::agent_chat::fold_block_tool_group(),
        FoldBlock::Tool => s::agent_chat::fold_block_tool(),
        FoldBlock::Subagent => s::agent_chat::fold_block_subagent(),
        FoldBlock::Thinking => s::agent_chat::fold_block_thinking(),
        FoldBlock::ThinkingGroup => s::agent_chat::fold_block_thinking_group(),
        FoldBlock::Assistant => s::agent_chat::fold_block_assistant(),
        FoldBlock::Diff => s::agent_chat::fold_block_diff(),
        FoldBlock::RawInput => s::agent_chat::fold_block_raw_input(),
    }
}

fn tool_category_label(category: ToolCategory) -> String {
    match category {
        ToolCategory::Read => s::agent_chat::filter_tool_read(),
        ToolCategory::Edit => s::agent_chat::filter_tool_edit(),
        ToolCategory::Delete => s::agent_chat::filter_tool_delete(),
        ToolCategory::Search => s::agent_chat::filter_tool_search(),
        ToolCategory::Run => s::agent_chat::filter_tool_run(),
        ToolCategory::Fetch => s::agent_chat::filter_tool_fetch(),
        ToolCategory::Mcp => s::agent_chat::filter_tool_mcp(),
        ToolCategory::Agent => s::agent_chat::filter_tool_agent(),
        ToolCategory::Other => s::agent_chat::filter_tool_other(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::fold_mode::BlockRule;

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
        for expanded in [true, false] {
            assert!(!expansion_label(expanded).is_empty());
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
            BlockRule::Collapsed,
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
            s::agent_chat::fold_mode_auto()
        );
        let matrix = FoldPreset::Summary.mode().with_rule(
            TurnPosition::Past,
            FoldBlock::Thinking,
            BlockRule::Collapsed,
        );
        assert_eq!(mode_value(matrix), s::agent_chat::fold_mode_custom());
    }
}
