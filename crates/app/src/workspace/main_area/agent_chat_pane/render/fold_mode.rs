//! Fold-mode chip and editable rule popover.

use gpui::{Anchor, AnyElement, App, Context, IntoElement, SharedString, div, prelude::*, px};

use super::options_panel::{fixed_region, panel_max_h, panel_root, scroll_region};
use crate::surface::strings as s;
use crate::ui::theme;
use crate::ui::theme::PaneSurfaceTokens;
use crate::ui::{
    ButtonVariants as _, Disableable as _, Popover, Selectable as _, Sizable as _, button,
    button_group, button_on_surface,
};
use crate::workspace::main_area::agent_chat_pane::fold_mode::{
    BlockRule, FoldBlock, FoldMode, FoldPreset, TurnPosition,
};
use crate::workspace::main_area::agent_chat_pane::tool_category::ToolCategory;
use crate::workspace::main_area::agent_chat_pane::view::AgentChatView;
use crate::workspace::main_area::pane_tree::PaneId;

const RULES: [BlockRule; 3] = [
    BlockRule::Builtin,
    BlockRule::Expanded,
    BlockRule::Collapsed,
];

/// Activity-bar chip for the pane's transcript fold rules.
pub(super) fn fold_mode_chip(
    pane_id: PaneId,
    mode: FoldMode,
    editor_turn: TurnPosition,
    default_open: bool,
    surface: &PaneSurfaceTokens,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let label = SharedString::from(s::agent_chat_fold_mode_chip(&mode_value(mode)));
    let view = cx.entity().downgrade();
    Popover::new(SharedString::from(format!(
        "agent-chat-fold-mode-popover-{pane_id}"
    )))
    .default_open(default_open)
    .anchor(Anchor::TopRight)
    .trigger(
        button_on_surface(
            ("agent-chat-fold-mode", pane_id as usize),
            label,
            surface,
            cx,
        )
        .xsmall()
        .tooltip(SharedString::from(s::agent_chat_fold_mode_tooltip())),
    )
    .content(move |_, window, cx| {
        panel_root(theme::AGENT_CHAT_RULES_PANEL_W, panel_max_h(window))
            .child(fold_mode_panel(&view, mode, editor_turn, pane_id, cx))
            .into_any_element()
    })
}

fn mode_value(mode: FoldMode) -> String {
    match mode.preset() {
        Some(preset) => preset_label(preset),
        None => s::agent_chat_fold_mode_custom(),
    }
}

pub(super) fn fold_mode_panel(
    view: &gpui::WeakEntity<AgentChatView>,
    mode: FoldMode,
    editor_turn: TurnPosition,
    pane_id: PaneId,
    cx: &mut Context<crate::ui::PopoverState>,
) -> AnyElement {
    let mut rows = Vec::new();
    for block in FoldBlock::ALL {
        rows.push(block_rule_row(view, mode, editor_turn, block, pane_id));
        if block == FoldBlock::Tool {
            rows.extend(
                ToolCategory::ALL
                    .into_iter()
                    .map(|category| tool_rule_row(view, mode, editor_turn, category, pane_id)),
            );
        }
    }

    let reset_view = view.clone();
    div()
        .flex_1()
        .min_h(px(0.))
        .overflow_hidden()
        .flex()
        .flex_col()
        .gap(px(theme::GAP_LG))
        .text_size(px(theme::agent_chat_font_size(cx)))
        .child(
            fixed_region()
                .gap(px(theme::GAP_LG))
                .child(panel_heading(s::agent_chat_fold_editor_presets(), cx))
                .child(preset_group(view, mode, pane_id))
                .child(turn_group(view, editor_turn, pane_id)),
        )
        .child(
            scroll_region(SharedString::from(format!(
                "agent-chat-fold-rules-scroll-{pane_id}"
            )))
            .children(rows),
        )
        .child(
            fixed_region().child(
                button(
                    SharedString::from(format!("agent-chat-fold-reset-{pane_id}")),
                    s::agent_chat_fold_editor_reset_auto(),
                )
                .ghost()
                .xsmall()
                .disabled(mode == FoldPreset::Auto.mode())
                .on_click(move |_, _window, app| {
                    if let Some(view) = reset_view.upgrade() {
                        view.update(app, |v, cx| v.set_fold_mode(FoldPreset::Auto.mode(), cx));
                    }
                }),
            ),
        )
        .into_any_element()
}

fn preset_group(
    view: &gpui::WeakEntity<AgentChatView>,
    mode: FoldMode,
    pane_id: PaneId,
) -> impl IntoElement + use<> {
    let view = view.clone();
    button_group(SharedString::from(format!(
        "agent-chat-fold-presets-{pane_id}"
    )))
    .children(FoldPreset::ALL.into_iter().map(|preset| {
        button(
            SharedString::from(format!(
                "agent-chat-fold-preset-{}-{pane_id}",
                preset_token(preset)
            )),
            preset_label(preset),
        )
        .selected(mode.preset() == Some(preset))
    }))
    .on_click(move |indices, _window, app| {
        let Some(&ix) = indices.first() else {
            return;
        };
        if let Some(view) = view.upgrade() {
            view.update(app, |v, cx| v.set_fold_mode(FoldPreset::ALL[ix].mode(), cx));
        }
    })
}

fn turn_group(
    view: &gpui::WeakEntity<AgentChatView>,
    current: TurnPosition,
    pane_id: PaneId,
) -> impl IntoElement + use<> {
    let view = view.clone();
    button_group(SharedString::from(format!(
        "agent-chat-fold-turns-{pane_id}"
    )))
    .children(TurnPosition::ALL.into_iter().map(|turn| {
        button(
            SharedString::from(format!("agent-chat-fold-turn-{}-{pane_id}", turn.token())),
            turn_label(turn),
        )
        .selected(turn == current)
    }))
    .on_click(move |indices, _window, app| {
        let Some(&ix) = indices.first() else {
            return;
        };
        if let Some(view) = view.upgrade() {
            view.update(app, |v, cx| {
                v.set_fold_editor_turn(TurnPosition::ALL[ix], cx)
            });
        }
    })
}

fn block_rule_row(
    view: &gpui::WeakEntity<AgentChatView>,
    mode: FoldMode,
    turn: TurnPosition,
    block: FoldBlock,
    pane_id: PaneId,
) -> AnyElement {
    let current = mode.rule(turn, block);
    let view = view.clone();
    rule_row(
        block_label(block),
        false,
        SharedString::from(format!(
            "agent-chat-fold-rule-{}-{}-{pane_id}",
            turn.token(),
            block.token()
        )),
        current,
        move |rule, app| {
            if let Some(view) = view.upgrade() {
                view.update(app, |v, cx| {
                    v.set_fold_mode(mode.with_rule(turn, block, rule), cx)
                });
            }
        },
    )
}

fn tool_rule_row(
    view: &gpui::WeakEntity<AgentChatView>,
    mode: FoldMode,
    turn: TurnPosition,
    category: ToolCategory,
    pane_id: PaneId,
) -> AnyElement {
    let current = mode.tool_rule(turn, category);
    let view = view.clone();
    rule_row(
        tool_category_label(category),
        true,
        SharedString::from(format!(
            "agent-chat-fold-tool-rule-{}-{}-{pane_id}",
            turn.token(),
            category.token()
        )),
        current,
        move |rule, app| {
            if let Some(view) = view.upgrade() {
                view.update(app, |v, cx| {
                    v.set_fold_mode(mode.with_tool_rule(turn, category, rule), cx)
                });
            }
        },
    )
}

fn rule_row(
    label: String,
    nested: bool,
    id: SharedString,
    current: BlockRule,
    on_change: impl Fn(BlockRule, &mut App) + 'static,
) -> AnyElement {
    let button_id_prefix = id.clone();
    div()
        .w_full()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::GAP_LG))
        .when(nested, |row| {
            row.pl(px(theme::AGENT_CHAT_OPTION_NEST_INDENT))
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
            button_group(id)
                .children(RULES.into_iter().map(|rule| {
                    button(
                        SharedString::from(format!("{button_id_prefix}-{}", rule_token(rule))),
                        rule_label(rule),
                    )
                    .selected(rule == current)
                }))
                .on_click(move |indices, _window, app| {
                    if let Some(&ix) = indices.first() {
                        on_change(RULES[ix], app);
                    }
                }),
        )
        .into_any_element()
}

fn panel_heading(label: String, cx: &App) -> impl IntoElement {
    div()
        .text_color(theme::current(cx).text_subtle)
        .child(SharedString::from(label))
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
        FoldBlock::Step => s::agent_chat_fold_block_step(),
        FoldBlock::ToolGroup => s::agent_chat_fold_block_tool_group(),
        FoldBlock::Tool => s::agent_chat_fold_block_tool(),
        FoldBlock::Subagent => s::agent_chat_fold_block_subagent(),
        FoldBlock::Thinking => s::agent_chat_fold_block_thinking(),
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
        for preset in FoldPreset::ALL {
            assert!(!preset_label(preset).is_empty(), "{preset:?}");
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
    }

    #[test]
    fn the_chip_names_the_preset_and_falls_back_to_custom() {
        assert_eq!(
            mode_value(FoldMode::default()),
            preset_label(FoldPreset::Auto)
        );
        assert_eq!(
            mode_value(FoldPreset::Summary.mode()),
            preset_label(FoldPreset::Summary)
        );
        let custom = FoldMode::from_tokens(["auto", "last.tool=expanded"]);
        assert_eq!(mode_value(custom), s::agent_chat_fold_mode_custom());
    }
}
