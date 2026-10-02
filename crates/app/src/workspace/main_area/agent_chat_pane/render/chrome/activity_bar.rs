//! The pane's own toolbar: who the session is on the left, the context-window
//! meter and the transcript controls on the right.
//!
//! The right cluster has two control classes, and the distinction is load-
//! bearing: a **chip** carries a word and therefore a hairline, because the
//! meter beside it is static text in the same tone; an **icon button** carries
//! a glyph and stays bare, because a glyph is already unmistakably a control.
//! Boxing the glyphs too would add three frames for no information.
//!
//! One View trigger owns all three axes. Narrow panes omit its label; the
//! override pin and full tooltip remain, independently of the open-state fill.

use daruda_acp::UsageView;
use gpui::{
    Anchor, AnyElement, Context, Hsla, IntoElement, SharedString, Window, div, prelude::*, px,
};

use super::super::filter::display_filter_chip_label;
use super::super::fold_mode::fold_mode_chip_label;
use super::super::tail_window::{TailChoices, tail_window_chip_label};
use crate::surface::strings as s;
use crate::surface::timestamp;
use crate::transcript::display_filter::DisplayFilter;
use crate::transcript::editor::state::{FilterEditorState, FoldEditorState};
use crate::transcript::editor::{
    TextRoles, aux_icon, dismiss_press, panel_header, panel_root, tabs_band,
};
use crate::transcript::fold_mode::FoldMode;
use crate::ui::theme;
use crate::ui::theme::PaneSurfaceTokens;
use crate::ui::{
    Disableable as _, Icon, IconName, Popover, Selectable as _, Sizable as _,
    button_bare_on_surface, button_chip_on_surface, icons, tab_bar,
};
use crate::workspace::main_area::agent_chat_pane::pane_choice::PaneChoice;
use crate::workspace::main_area::agent_chat_pane::view::{
    ActivityOptionsTab, AgentChatView, ChatContentWidth,
};
use crate::workspace::main_area::pane_tree::PaneId;

const ICON_EXPAND: &str = "icons/ui/expand.svg";
const ICON_COMPRESS: &str = "icons/ui/compress.svg";
const ICON_WIDTH_WIDE: &str = "icons/ui/width-wide.svg";
const ICON_OVERRIDE: &str = "icons/ui/keep.svg";

/// Pane activity bar: resolved session title on the left, the context-window
/// meter and the icon controls on the right, with a bottom hairline against the
/// conversation body. Always rendered, so the reading-width toggle stays
/// reachable while the conversation is empty or still connecting. `title` is
/// already resolved by the caller (`activity_bar_title`, falling back to the
/// agent name).
pub(in crate::workspace::main_area::agent_chat_pane::render) struct ActivityBarProps<'a> {
    pub pane_id: PaneId,
    pub agent_id: &'a str,
    pub title: Option<&'a str>,
    pub last_active: Option<&'a str>,
    pub usage: Option<&'a UsageView>,
    pub has_items: bool,
    pub content_width: ChatContentWidth,
    pub tail: TailChoices,
    pub display_filter: PaneChoice<DisplayFilter>,
    pub fold_mode: PaneChoice<FoldMode>,
    /// The fold editor's own state — see `AgentChatView`.
    pub fold_editor: FoldEditorState,
    /// Which Visible-items sections are shut — see `AgentChatView`.
    pub filter_editor: FilterEditorState,
    pub activity_options_tab: ActivityOptionsTab,
    pub compact_options: bool,
    pub options_popover_open: bool,
    pub dim: f32,
}

pub(in crate::workspace::main_area::agent_chat_pane::render) fn activity_bar(
    props: ActivityBarProps<'_>,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let title: SharedString = props
        .title
        .map(|s| SharedString::from(s.to_string()))
        .unwrap_or_default();
    // A tooltip rather than inline text: low-frequency detail, and the bar is
    // width-constrained (the title ellipsizes). The agent reports it via
    // `SessionInfoUpdate`; `reconcile_activity` advances it on each settle.
    let last_active = props.last_active.map(last_active_tooltip);
    // Every colour on this bar comes from the pane's own surface, already
    // dimmed: the bar paints on `agent_chat_bg` (a mirror of the terminal
    // palette), which the UI theme's control colours know nothing about.
    let surface = PaneSurfaceTokens::agent_chat(cx).dimmed(props.dim);

    let expand = button_bare_on_surface(
        ("agent-chat-expand-all", props.pane_id as usize),
        &surface,
        cx,
    )
    .xsmall()
    .icon(Icon::empty().path(ICON_EXPAND))
    .tooltip(SharedString::from(s::agent_chat::expand_all()))
    .disabled(!props.has_items)
    .debug_selector(|| "agent-chat-expand-all".into())
    .on_click(cx.listener(move |this, _ev, window, cx| this.set_all_folds(true, window, cx)));
    let collapse = button_bare_on_surface(
        ("agent-chat-collapse-all", props.pane_id as usize),
        &surface,
        cx,
    )
    .xsmall()
    .icon(Icon::empty().path(ICON_COMPRESS))
    .tooltip(SharedString::from(s::agent_chat::collapse_all()))
    .disabled(!props.has_items)
    .on_click(cx.listener(move |this, _ev, window, cx| this.set_all_folds(false, window, cx)));
    let transcript_control = view_options_chip(&props, &surface, cx).into_any_element();
    let reading_selected = props.content_width.is_reading();
    let reading_tooltip = if reading_selected {
        s::agent_chat::reading_width_off()
    } else {
        s::agent_chat::reading_width_on()
    };
    let reading_width = button_bare_on_surface(
        ("agent-chat-reading-width", props.pane_id as usize),
        &surface,
        cx,
    )
    .xsmall()
    .icon(Icon::empty().path(ICON_WIDTH_WIDE))
    .tooltip(SharedString::from(reading_tooltip))
    .selected(reading_selected)
    .on_click(cx.listener(move |this, _ev, _window, cx| this.toggle_content_width(cx)));

    div()
        .flex_none()
        .w_full()
        .flex()
        .flex_row()
        .justify_between()
        .items_center()
        .gap(px(theme::AGENT_CHAT_MSG_GAP))
        .px(px(theme::AGENT_CHAT_PAD_X))
        .py(px(theme::AGENT_CHAT_PAD_Y))
        .border_b_1()
        // Background-derived hairline: the bar sits directly on the pane's
        // `agent_chat_bg` (mirrored terminal bg), where the fixed `t.border`
        // hairline is near-invisible. Matches the tool-card / code-block edges.
        .border_color(surface.border_tint)
        .child(
            div()
                .id(("agent-chat-title", props.pane_id as usize))
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .flex()
                .flex_row()
                .items_center()
                .gap(px(theme::AGENT_CHAT_HEADER_ICON_GAP))
                .child(agent_icon(props.agent_id, surface.foreground_muted))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_size(px(theme::agent_chat_font_size(cx)))
                        .text_color(surface.foreground)
                        .child(title),
                )
                .when_some(last_active, |el, tip| {
                    el.tooltip(crate::ui::tooltip::text(tip))
                }),
        )
        // Context-window meter (from `UsageUpdate`): current fill on the right,
        // detail + optional cost in the tooltip. Distinct from the cumulative
        // Usage tab.
        .when_some(props.usage.map(context_meter), |row, meter| {
            row.child(
                div()
                    .id(("agent-chat-context-meter", props.pane_id as usize))
                    .flex_none()
                    .text_size(px(theme::agent_chat_font_size(cx)))
                    .text_color(surface.foreground_muted)
                    .child(SharedString::from(meter.label))
                    .tooltip(crate::ui::tooltip::text(SharedString::from(meter.tooltip))),
            )
        })
        .child(
            div()
                .flex_shrink()
                .min_w_0()
                .max_w_full()
                .flex()
                .flex_row()
                .justify_end()
                .items_center()
                .gap(px(theme::AGENT_CHAT_BAR_CONTROL_GAP))
                .text_color(surface.foreground_muted)
                // These controls say how the pane is displayed, not that the user
                // is engaging with what is in it — so the press stops short of the
                // pane wrapper, which reads one as `focus_pane_on_click`.
                .on_mouse_down(gpui::MouseButton::Left, |_, _window, cx| {
                    cx.stop_propagation();
                })
                .child(transcript_control)
                .child(expand)
                .child(collapse)
                .child(reading_width),
        )
}

/// Whether every transcript axis still follows the configured default.
///
/// The trigger's pin reads the same `PaneChoice` values as the tooltip marks.
fn every_axis_follows_config(
    fold: PaneChoice<FoldMode>,
    filter: PaneChoice<DisplayFilter>,
    tail: TailChoices,
) -> bool {
    fold.is_following() && filter.is_following() && tail.is_following()
}

/// All three axis summaries, including which ones no longer follow config.
fn options_tooltip(
    fold: PaneChoice<FoldMode>,
    filter: PaneChoice<DisplayFilter>,
    tail: TailChoices,
) -> String {
    s::agent_chat::view_options_tooltip(
        fold_mode_chip_label(fold),
        display_filter_chip_label(filter),
        tail_window_chip_label(tail),
    )
}

fn view_options_chip(
    props: &ActivityBarProps<'_>,
    surface: &PaneSurfaceTokens,
    cx: &mut Context<AgentChatView>,
) -> impl IntoElement + use<> {
    let pane_id = props.pane_id;
    let mode = props.fold_mode;
    let fold_editor = props.fold_editor;
    let filter_editor = props.filter_editor;
    let filter = props.display_filter;
    let tail = props.tail;
    let active_tab = props.activity_options_tab;
    let adjusted = !every_axis_follows_config(mode, filter, tail);
    let tooltip = options_tooltip(mode, filter, tail);
    let view = cx.entity().downgrade();
    Popover::new(SharedString::from(format!(
        "agent-chat-view-options-popover-{pane_id}"
    )))
    .default_open(props.options_popover_open)
    .anchor(Anchor::TopRight)
    // The panel owns its insets band by band; see `panel_root`.
    .p_0()
    .trigger(
        (if props.compact_options {
            button_bare_on_surface(("agent-chat-view-options", pane_id as usize), surface, cx)
        } else {
            button_chip_on_surface(
                ("agent-chat-view-options", pane_id as usize),
                s::agent_chat::view_options_label(),
                surface,
                cx,
            )
            .child(Icon::new(IconName::ChevronDown))
        })
        .xsmall()
        .tab_stop(true)
        .icon(Icon::new(IconName::Settings2))
        .when(adjusted, |button| {
            button.child(Icon::empty().path(ICON_OVERRIDE))
        })
        .tooltip(SharedString::from(tooltip)),
    )
    .content(move |state, window, cx| {
        activity_options_panel(
            &view,
            pane_id,
            OptionsPanelValues {
                mode,
                fold_editor,
                filter,
                filter_editor,
                tail,
            },
            active_tab,
            state,
            window,
            cx,
        )
    })
}

/// What the panel's three editors read, as one copy taken at render time.
struct OptionsPanelValues {
    mode: PaneChoice<FoldMode>,
    fold_editor: FoldEditorState,
    filter: PaneChoice<DisplayFilter>,
    filter_editor: FilterEditorState,
    tail: TailChoices,
}

fn activity_options_panel(
    view: &gpui::WeakEntity<AgentChatView>,
    pane_id: PaneId,
    values: OptionsPanelValues,
    active_tab: ActivityOptionsTab,
    state: &crate::ui::PopoverState,
    window: &Window,
    cx: &mut Context<crate::ui::PopoverState>,
) -> AnyElement {
    let panel = match active_tab {
        ActivityOptionsTab::Fold => super::super::fold_mode::fold_mode_panel(
            view,
            values.mode,
            values.fold_editor,
            pane_id,
            cx,
        ),
        ActivityOptionsTab::Filter => super::super::filter::filter_panel(
            view,
            values.filter,
            values.filter_editor,
            pane_id,
            cx,
        ),
        ActivityOptionsTab::RecentSteps => {
            super::super::tail_window::tail_window_panel(view, values.tail, pane_id, cx)
        }
    };
    let text = TextRoles::from_base(theme::agent_chat_font_size(cx));
    let close = dismiss_press(window, cx);
    panel_root(window, state)
        .child(panel_header(
            SharedString::from(format!("agent-chat-view-options-close-{pane_id}")),
            s::agent_chat::view_options_title(),
            text,
            close,
            cx,
        ))
        .child(tabs_band(
            activity_options_tabs(view, active_tab, pane_id, cx),
            cx,
        ))
        .child(panel)
        .into_any_element()
}

fn activity_options_tabs(
    view: &gpui::WeakEntity<AgentChatView>,
    active: ActivityOptionsTab,
    pane_id: PaneId,
    cx: &gpui::App,
) -> impl IntoElement + use<> {
    let view = view.clone();
    let tab_icon_color = theme::current(cx).text_muted;
    let text = TextRoles::from_base(theme::agent_chat_font_size(cx));
    tab_bar(SharedString::from(format!(
        "agent-chat-view-options-tabs-{pane_id}"
    )))
    // The factory's `text_xs` would hold the labels at 12px while the rest of
    // the panel follows the configured size.
    .text_size(px(text.body))
    .selected_index(
        ActivityOptionsTab::ALL
            .iter()
            .position(|tab| *tab == active)
            .unwrap_or(0),
    )
    .children(ActivityOptionsTab::ALL.into_iter().map(|tab| {
        // A prefix, not `Tab::icon`, which makes an icon-only tab.
        crate::ui::tab(SharedString::from(activity_options_label(tab)))
            .prefix(aux_icon(activity_options_icon(tab), tab_icon_color))
    }))
    .on_click(move |ix, _window, app| {
        if let Some(view) = view.upgrade() {
            view.update(app, |v, cx| {
                v.set_activity_options_tab(ActivityOptionsTab::ALL[*ix], cx)
            });
        }
    })
}

fn activity_options_icon(tab: ActivityOptionsTab) -> &'static str {
    match tab {
        ActivityOptionsTab::Fold => icons::UNFOLD_LESS,
        ActivityOptionsTab::Filter => icons::VISIBILITY,
        ActivityOptionsTab::RecentSteps => icons::HISTORY,
    }
}

fn activity_options_label(tab: ActivityOptionsTab) -> String {
    match tab {
        ActivityOptionsTab::Fold => s::agent_chat::view_options_fold(),
        ActivityOptionsTab::Filter => s::agent_chat::view_options_filter(),
        ActivityOptionsTab::RecentSteps => s::agent_chat::recent_steps_label(),
    }
}

fn agent_icon(agent_id: &str, color: Hsla) -> AnyElement {
    crate::ui::agent_icon(
        crate::agent::icons::icon_for_agent(agent_id),
        px(theme::AGENT_CHAT_HEADER_ICON_SIZE),
        color,
    )
}

/// The context meter's two pieces of copy, derived from one `UsageUpdate`.
struct ContextMeter {
    label: String,
    tooltip: String,
}

/// Derive the context-meter copy. `checked_div` keeps a size-0 window — seen
/// before the first real `UsageUpdate` — from dividing by zero.
fn context_meter(u: &UsageView) -> ContextMeter {
    let used = format_token_count(u.used);
    let size = format_token_count(u.size);
    let percent = u
        .used
        .saturating_mul(100)
        .checked_div(u.size)
        .map(|p| p.min(100) as u8)
        .unwrap_or(0);
    match &u.cost {
        Some(c) => {
            let amount = format!("{:.2}", c.amount);
            ContextMeter {
                label: s::agent_chat::context_meter_with_cost(&used, &size, &amount, &c.currency),
                tooltip: s::agent_chat::context_tooltip_with_cost(
                    &used,
                    &size,
                    percent,
                    &amount,
                    &c.currency,
                ),
            }
        }
        None => ContextMeter {
            label: s::agent_chat::context_meter(&used, &size),
            tooltip: s::agent_chat::context_tooltip(&used, &size, percent),
        },
    }
}

/// The title's tooltip: when this session was last active, in the machine's
/// local zone. A timestamp we cannot parse shows verbatim — the protocol
/// promises ISO 8601, which is wider than the RFC 3339 subset we read, so the
/// agent's own wording beats an empty tooltip.
fn last_active_tooltip(iso: &str) -> SharedString {
    let when = timestamp::local_datetime(iso).unwrap_or_else(|| iso.to_owned());
    SharedString::from(s::agent_chat::last_active_tooltip(&when))
}

/// Compact token count for the context meter: exact below 1000, whole
/// thousands with `k` below a million, then millions to one decimal with a
/// trailing `.0` dropped (53_000 → `53k`, 1_000_000 → `1M`, 1_500_000 →
/// `1.5M`). Precision loss is irrelevant for a context-window gauge.
fn format_token_count(n: u64) -> String {
    let thousands = n.saturating_add(500) / 1000;
    if n < 1000 {
        n.to_string()
    } else if thousands < 1000 {
        format!("{thousands}k")
    } else {
        // Branch on the rounded thousands so 999_600 reads `1M`, not `1000k`.
        let tenths = n.saturating_add(50_000) / 100_000;
        match tenths % 10 {
            0 => format!("{}M", tenths / 10),
            frac => format!("{}.{frac}M", tenths / 10),
        }
    }
}

#[cfg(test)]
mod tests;
