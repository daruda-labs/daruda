//! Lifecycle rules, with historical overrides behind an advanced disclosure.

use std::rc::Rc;

use gpui::{
    AnyElement, App, Div, IntoElement, SharedString, Window, div, prelude::*, px, relative,
};

use super::state::FoldEditorState;
use super::{
    ResetSpec, TextRoles, aux_icon, aux_line, editor_column, fixed_band, reset_footer,
    scroll_content, scroll_region,
};
use crate::surface::strings as s;
use crate::transcript::fold_mode::{FoldBlock, FoldMode, FoldPreset, TurnPosition};
use crate::transcript::tool_category::ToolCategory;
use crate::ui::{
    ButtonCustomVariant, ButtonVariants as _, Disableable as _, DropdownMenu as _, Icon, IconName,
    PopupMenuItem, Selectable as _, button_bare, button_group, checkbox, icons, theme,
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

/// The table's row order: what a response shows first reads first. Separate
/// from [`FoldBlock::ALL`], whose order is the storage and token order.
const EDITOR_ORDER: [FoldBlock; FoldBlock::ALL.len()] = [
    FoldBlock::Response,
    FoldBlock::Assistant,
    FoldBlock::ToolGroup,
    FoldBlock::Tool,
    FoldBlock::Subagent,
    FoldBlock::ThinkingGroup,
    FoldBlock::Thinking,
    FoldBlock::Diff,
    FoldBlock::RawInput,
];

pub(crate) fn fold_editor(
    mode: FoldMode,
    state: FoldEditorState,
    id_prefix: &str,
    font_size: f32,
    actions: FoldEditorActions,
    cx: &App,
) -> AnyElement {
    let text = TextRoles::from_base(font_size);
    let on_history = actions.on_change.clone();
    let history_mode = mode.with_collapse_history(!mode.collapse_history());
    let advanced = state.history_rules_open();
    let on_history_rules = actions.on_history_rules.clone();

    let mut body = scroll_content()
        .child(preset_group(mode, state, id_prefix, text, &actions, cx))
        .child(
            div()
                .mt(px(theme::TRANSCRIPT_EDITOR_SECTION_GAP))
                .child(rule_headings(text, cx)),
        )
        .children(rule_rows(
            mode,
            TurnPosition::Last,
            state,
            id_prefix,
            text,
            &actions,
            cx,
        ))
        .child(separated_row(cx).child(disclosure(
            SharedString::from(format!("{id_prefix}-fold-advanced")),
            s::agent_chat::fold_editor_history_rules(),
            advanced,
            text,
            None,
            Rc::new(move |app| on_history_rules(app)),
        )));
    if advanced {
        body = body
            .child(div().mt(px(theme::GAP_LG)).child(rule_headings(text, cx)))
            .children(rule_rows(
                mode,
                TurnPosition::Past,
                state,
                id_prefix,
                text,
                &actions,
                cx,
            ));
    }

    let t = theme::current(cx);
    editor_column(text)
        .child(scroll_region(
            format!("{id_prefix}-fold-rules-scroll"),
            body,
        ))
        .child(
            fixed_band(cx)
                .py(px(theme::TRANSCRIPT_EDITOR_BAND_PAD_Y))
                .gap(px(theme::GAP_LG))
                .child(aux_line(
                    icons::HISTORY,
                    t.text_muted,
                    s::agent_chat::fold_editor_history(),
                    t.text_muted,
                    text,
                ))
                .child(
                    checkbox(
                        SharedString::from(format!("{id_prefix}-fold-history")),
                        s::agent_chat::fold_editor_collapse_history(),
                        0,
                    )
                    .text_size(px(text.body))
                    .checked(mode.collapse_history())
                    .on_click(move |_, window, app| on_history(history_mode, window, app)),
                ),
        )
        .child(reset_footer(
            SharedString::from(format!("{id_prefix}-fold-reset")),
            actions.reset,
            text,
            cx,
        ))
        .into_any_element()
}

fn rule_rows(
    mode: FoldMode,
    turn: TurnPosition,
    state: FoldEditorState,
    id: &str,
    text: TextRoles,
    actions: &FoldEditorActions,
    cx: &App,
) -> Vec<AnyElement> {
    let mut rows = Vec::new();
    for block in EDITOR_ORDER {
        let tools_open = state.tools_open(turn);
        let label = if block == FoldBlock::Tool {
            let on_tools = actions.on_tools.clone();
            disclosure(
                SharedString::from(format!("{id}-fold-tools-{}", turn.token())),
                block_label(block),
                tools_open,
                text,
                Some(s::agent_chat::fold_editor_tool_categories()),
                Rc::new(move |app| on_tools(turn, app)),
            )
        } else {
            row_label(block_label(block), false, text, cx).into_any_element()
        };
        rows.push(block_rule_row(
            mode, turn, block, label, id, text, actions, cx,
        ));
        if block == FoldBlock::Tool && tools_open {
            rows.extend(
                ToolCategory::ALL
                    .into_iter()
                    .filter(|c| c.folds_as_a_tool_card())
                    .map(|c| tool_rule_row(mode, turn, c, id, text, actions, cx)),
            );
        }
    }
    rows
}

/// One line of the table: a label track and two lifecycle tracks. Headings,
/// block rows, category rows and the history rows all go through it, so they
/// cannot drift off one grid.
fn table_row(label: impl IntoElement, phases: [AnyElement; 2]) -> Div {
    div()
        .w_full()
        .flex()
        .items_center()
        .child(
            div()
                .w(relative(theme::TRANSCRIPT_EDITOR_LABEL_TRACK))
                .min_w_0()
                .child(label),
        )
        .children(phases.into_iter().map(|phase| {
            div()
                .w(relative(theme::TRANSCRIPT_EDITOR_PHASE_TRACK))
                .min_w_0()
                .pl(px(theme::TRANSCRIPT_EDITOR_PHASE_GUTTER))
                .child(phase)
        }))
}

/// A row of the rule list: the baseline pitch with a hairline above it.
fn separated_row(cx: &App) -> Div {
    div()
        .w_full()
        .min_h(px(theme::TRANSCRIPT_EDITOR_ROW_MIN_H))
        .flex()
        .items_center()
        .border_t_1()
        .border_color(theme::current(cx).border)
}

fn rule_headings(text: TextRoles, cx: &App) -> impl IntoElement {
    let t = theme::current(cx);
    let phase = |icon: &'static str, tone, label: String| {
        div()
            .flex()
            .items_center()
            .justify_center()
            .gap(px(theme::GAP_STANDARD))
            .text_color(t.text_muted)
            .child(aux_icon(icon, tone))
            .child(SharedString::from(label))
            .into_any_element()
    };
    table_row(
        div()
            .text_color(t.text_body)
            .child(SharedString::from(s::agent_chat::fold_editor_rules())),
        [
            // The glyphs carry the lifecycle by shape; the tones only echo it.
            phase(
                icons::REFRESH,
                t.banner_warning_text,
                s::agent_chat::fold_editor_during(),
            ),
            phase(
                icons::CHECK_CIRCLE,
                t.banner_success_text,
                s::agent_chat::fold_editor_after(),
            ),
        ],
    )
    .pb(px(theme::GAP_LG))
    .text_size(px(text.aux))
}

fn row_label(label: String, nested: bool, text: TextRoles, cx: &App) -> Div {
    let t = theme::current(cx);
    div()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .text_size(px(text.body))
        .text_color(if nested { t.text_muted } else { t.text_body })
        .when(nested, |label| {
            label.pl(px(theme::TRANSCRIPT_EDITOR_NEST_INDENT))
        })
        .child(SharedString::from(label))
}

/// A label that opens or shuts what sits under it. Its own control, so a press
/// on it never reaches the phase controls beside it.
fn disclosure(
    id: SharedString,
    label: String,
    open: bool,
    text: TextRoles,
    tooltip: Option<String>,
    on_toggle: Rc<dyn Fn(&mut App)>,
) -> AnyElement {
    button_bare(id)
        .ghost()
        .tab_stop(true)
        .justify_start()
        .h_auto()
        .min_h(px(theme::TRANSCRIPT_EDITOR_CELL_MIN_H))
        .px(px(0.))
        .max_w_full()
        .child(Icon::new(if open {
            IconName::ChevronDown
        } else {
            IconName::ChevronRight
        }))
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(text.body))
                .child(SharedString::from(label)),
        )
        .when_some(tooltip, |button, tip| {
            button.tooltip(SharedString::from(tip))
        })
        .on_click(move |_, _, app| on_toggle(app))
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
    text: TextRoles,
    actions: &FoldEditorActions,
    cx: &App,
) -> impl IntoElement + use<> {
    let on_preset = actions.on_preset.clone();
    button_group(SharedString::from(format!("{id_prefix}-fold-presets")), cx)
        .w_full()
        .children(PresetSegment::ALL.into_iter().map(|segment| {
            button_bare(SharedString::from(format!(
                "{id_prefix}-fold-preset-{}",
                segment.token()
            )))
            .flex_1()
            .h_auto()
            .min_h(px(theme::TRANSCRIPT_EDITOR_PRESET_MIN_H))
            .child(
                div()
                    .text_size(px(text.body))
                    .child(SharedString::from(segment.label())),
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

#[allow(clippy::too_many_arguments)]
fn block_rule_row(
    mode: FoldMode,
    turn: TurnPosition,
    block: FoldBlock,
    label: AnyElement,
    id_prefix: &str,
    text: TextRoles,
    actions: &FoldEditorActions,
    cx: &App,
) -> AnyElement {
    let on_change = actions.on_change.clone();
    rule_row(
        label,
        SharedString::from(format!(
            "{id_prefix}-fold-rule-{}-{}",
            turn.token(),
            block.token()
        )),
        [
            mode.phase(turn, block, true),
            mode.phase(turn, block, false),
        ],
        text,
        Rc::new(move |active, expanded, window, app| {
            on_change(mode.with_phase(turn, block, active, expanded), window, app)
        }),
        cx,
    )
}

fn tool_rule_row(
    mode: FoldMode,
    turn: TurnPosition,
    category: ToolCategory,
    id_prefix: &str,
    text: TextRoles,
    actions: &FoldEditorActions,
    cx: &App,
) -> AnyElement {
    let on_change = actions.on_change.clone();
    rule_row(
        row_label(tool_category_label(category), true, text, cx).into_any_element(),
        SharedString::from(format!(
            "{id_prefix}-fold-tool-rule-{}-{}",
            turn.token(),
            category.token()
        )),
        [
            Some(mode.tool_rule(turn, category).is_expanded(true)),
            Some(mode.tool_rule(turn, category).is_expanded(false)),
        ],
        text,
        Rc::new(move |active, expanded, window, app| {
            on_change(
                mode.with_tool_phase(turn, category, active, expanded),
                window,
                app,
            )
        }),
        cx,
    )
}

type PhaseEdit = Rc<dyn Fn(bool, bool, &mut Window, &mut App)>;
/// One cell's pick: the value chosen for that cell's own phase.
type PhasePick = Rc<dyn Fn(bool, &mut Window, &mut App)>;

fn rule_row(
    label: AnyElement,
    id: SharedString,
    current: [Option<bool>; 2],
    text: TextRoles,
    on_change: PhaseEdit,
    cx: &App,
) -> AnyElement {
    let [during, after] = [true, false].map(|active| {
        let value = if active { current[0] } else { current[1] };
        phase_control(
            SharedString::from(format!("{id}-{active}")),
            value,
            text,
            {
                let on_change = on_change.clone();
                let pick: PhasePick =
                    Rc::new(move |expanded, window, app| on_change(active, expanded, window, app));
                pick
            },
            cx,
        )
    });
    separated_row(cx)
        .child(table_row(label, [during, after]))
        .into_any_element()
}

/// One lifecycle cell: the value on the left, the menu caret on the right.
///
/// Expanded is lifted a rung — fill, edge and label all step up — so the two
/// values differ by more than hue. A mixed tool row stays on the resting rung.
fn phase_control(
    id: SharedString,
    value: Option<bool>,
    text: TextRoles,
    on_pick: PhasePick,
    cx: &App,
) -> AnyElement {
    let t = theme::current(cx);
    let lifted = value == Some(true);
    let variant = ButtonCustomVariant::new(cx)
        .color(if lifted {
            t.overlay_prominent
        } else {
            t.modal_input_bg
        })
        .border(if lifted { t.text_subtle } else { t.border })
        .foreground(if lifted { t.text_primary } else { t.text_muted })
        .hover(t.button_widget_bg_hover)
        .active(t.overlay_active);
    let label = match value {
        Some(value) => expansion_label(value),
        None => s::agent_chat::fold_editor_mixed(),
    };
    button_bare(id)
        .custom(variant)
        .tab_stop(true)
        .w_full()
        .h_auto()
        .min_h(px(theme::TRANSCRIPT_EDITOR_CELL_MIN_H))
        .px(px(theme::TRANSCRIPT_EDITOR_CELL_PAD_X))
        // A column whose items stretch makes the vendored label row span the
        // control, so the value can sit left and the caret right.
        .flex_col()
        .items_stretch()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .text_size(px(text.aux))
                .child(SharedString::from(label)),
        )
        .child(Icon::new(IconName::ChevronDown))
        .dropdown_menu(crate::ui::stacked_menu_builder(move |menu, _, _| {
            [true, false].into_iter().fold(menu, |menu, expanded| {
                let on_pick = on_pick.clone();
                menu.item(
                    PopupMenuItem::new(expansion_label(expanded))
                        .checked(value == Some(expanded))
                        .on_click(move |_, window, app| on_pick(expanded, window, app)),
                )
            })
        }))
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

    /// The display order is the table's alone: every block appears once, and
    /// the storage order it departs from is untouched.
    #[test]
    fn the_table_lists_every_block_exactly_once() {
        for block in FoldBlock::ALL {
            assert_eq!(
                EDITOR_ORDER.iter().filter(|&&b| b == block).count(),
                1,
                "{block:?}"
            );
        }
    }

    /// The tracks fill the row exactly, so headings and rows share one grid.
    #[test]
    fn the_table_tracks_fill_the_row() {
        let total =
            theme::TRANSCRIPT_EDITOR_LABEL_TRACK + 2.0 * theme::TRANSCRIPT_EDITOR_PHASE_TRACK;
        assert!((total - 1.0).abs() < 1e-6, "{total}");
        const {
            assert!(theme::TRANSCRIPT_EDITOR_ROW_MIN_H >= theme::TRANSCRIPT_EDITOR_CELL_MIN_H);
        }
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
