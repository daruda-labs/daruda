//! Settings-local cards and rows, independent of modal form geometry.

use gpui::{
    App, Div, IntoElement, ParentElement as _, SharedString, Styled as _, div, prelude::*, px,
};

use super::{BoolSetting, SelectSetting, SettingsView, TextSetting, spec};
use crate::surface::strings as s;
use crate::ui::theme;

pub(super) fn page_stack() -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(theme::SETTINGS_GROUP_GAP))
        .min_w_0()
}

pub(super) fn card(title: impl Into<SharedString>, cx: &App) -> Div {
    let t = theme::current(cx);
    div()
        .flex()
        .flex_col()
        .min_w_0()
        .border_1()
        .border_color(t.border)
        .rounded(px(theme::RADIUS_SM))
        .bg(t.dock_bg)
        .child(
            div()
                .px(px(theme::SETTINGS_CARD_PAD))
                .py(px(theme::PAD_XL))
                .bg(t.button_widget_bg)
                .rounded_t(px(theme::RADIUS_SM))
                .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(t.text_primary)
                .child(title.into()),
        )
}

pub(super) fn row(
    label: impl Into<SharedString>,
    description: impl Into<SharedString>,
    control: impl IntoElement,
    cx: &App,
) -> Div {
    let t = theme::current(cx);
    let description = description.into();
    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(theme::SETTINGS_ROW_GAP))
        .mx(px(theme::SETTINGS_CARD_PAD))
        .py(px(theme::SETTINGS_ROW_PAD_Y))
        .min_h(px(theme::SETTINGS_ROW_MIN_H))
        .border_t_1()
        .border_color(t.border)
        .child(
            div()
                .flex_1()
                .min_w(px(theme::SETTINGS_LABEL_MIN_W))
                .flex()
                .flex_col()
                .gap(px(theme::PAD_XS))
                .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                .text_color(t.text_body)
                .child(label.into())
                .when(!description.is_empty(), |el| {
                    el.child(
                        div()
                            .text_size(px(theme::TAB_FONT_SIZE))
                            .text_color(t.text_muted)
                            .child(description),
                    )
                }),
        )
        .child(
            div()
                .w(px(theme::SETTINGS_CONTROL_W))
                .max_w_full()
                .flex()
                .justify_end()
                .child(control),
        )
}

/// A switch with its On/Off word beside it, the control every switch row uses.
pub(super) fn switch_with_state(switch: crate::ui::Button, checked: bool, cx: &App) -> Div {
    let state = if checked {
        s::settings_toggle_on()
    } else {
        s::settings_toggle_off()
    };
    div()
        .flex()
        .items_center()
        .gap(px(theme::PAD_SM))
        .child(switch)
        .child(
            div()
                .text_size(px(theme::TAB_FONT_SIZE))
                .text_color(theme::current(cx).text_muted)
                .child(state),
        )
}

pub(super) fn card_content(content: impl IntoElement) -> Div {
    div()
        .p(px(theme::SETTINGS_CARD_PAD))
        .min_w_0()
        .child(content)
}

impl SettingsView {
    pub(super) fn text_row(
        &self,
        setting: TextSetting,
        label: String,
        description: String,
        cx: &App,
    ) -> Div {
        let input = (spec::text_spec(setting).field)(self);
        row(
            label,
            description,
            div()
                .w(px(theme::SETTINGS_NUMBER_W))
                .child(crate::ui::input(input, cx, 0)),
            cx,
        )
    }

    /// [`Self::text_row`] for free text (a path, a command) rather than a
    /// number: the input takes the full control column.
    pub(super) fn text_row_wide(
        &self,
        setting: TextSetting,
        label: String,
        description: String,
        cx: &App,
    ) -> Div {
        let input = (spec::text_spec(setting).field)(self);
        row(
            label,
            description,
            div().w_full().child(crate::ui::input(input, cx, 0)),
            cx,
        )
    }

    /// A row whose button asks the host for something through `event`.
    pub(super) fn event_row(
        &self,
        id: &'static str,
        label: String,
        description: String,
        button_label: String,
        event: fn() -> super::SettingsEvent,
        cx: &gpui::Context<Self>,
    ) -> Div {
        row(
            label,
            description,
            super::settings_button(id, button_label)
                .tab_stop(true)
                .on_click(cx.listener(move |_, _, _, cx| cx.emit(event()))),
            cx,
        )
    }

    pub(super) fn select_row(
        &self,
        setting: SelectSetting,
        label: String,
        description: String,
        cx: &App,
    ) -> Div {
        let input = (spec::select_spec(setting).field)(self);
        let control = crate::ui::select::select(input, cx, 0).when(
            setting == SelectSetting::UiPreset && daruda_config::UI_THEME_PRESETS.len() <= 1,
            |el| el.disabled(true),
        );
        row(label, description, div().w_full().child(control), cx)
    }

    pub(super) fn switch_row(
        &self,
        setting: BoolSetting,
        label: String,
        description: String,
        cx: &gpui::Context<Self>,
    ) -> Div {
        let checked = (spec::bool_spec(setting).get)(self);
        let id = format!("settings-switch-{:?}", setting);
        let control = switch_with_state(
            crate::ui::switch(id, checked, cx)
                .tooltip(label.clone())
                .on_click(
                    cx.listener(move |this, _, _, cx| this.set_bool_setting(setting, !checked, cx)),
                ),
            checked,
            cx,
        );
        row(label, description, control, cx)
    }

    /// Rows that only apply while `parent` is on, indented under it. While
    /// it is off they are dimmed and headed by a note naming the parent.
    pub(super) fn dependent_rows(
        &self,
        parent: BoolSetting,
        parent_label: String,
        rows: impl IntoIterator<Item = Div>,
        cx: &App,
    ) -> Div {
        let on = (spec::bool_spec(parent).get)(self);
        crate::ui::dependent(on, cx)
            .when(!on, |el| {
                el.child(
                    div()
                        .mx(px(theme::SETTINGS_CARD_PAD))
                        .pt(px(theme::PAD_SM))
                        .text_size(px(theme::TAB_FONT_SIZE))
                        .text_color(theme::current(cx).text_muted)
                        .child(s::settings_dependent_off(&parent_label)),
                )
            })
            .children(rows)
    }

    /// A row whose control jumps to another Settings page.
    pub(super) fn link_row(
        &self,
        id: &'static str,
        label: String,
        description: String,
        button_label: String,
        target: daruda_config::BuiltinSection,
        cx: &gpui::Context<Self>,
    ) -> Div {
        row(
            label,
            description,
            super::settings_button(id, button_label)
                .tab_stop(true)
                .on_click(
                    cx.listener(move |this, _, window, cx| this.focus_section(target, window, cx)),
                ),
            cx,
        )
    }

    /// The low-traffic rows of `section`, folded behind one header. Closed
    /// until the user opens it; the open set lives for the Settings session.
    pub(super) fn advanced_card(
        &self,
        section: daruda_config::BuiltinSection,
        rows: Vec<Div>,
        cx: &gpui::Context<Self>,
    ) -> Div {
        let t = theme::current(cx);
        let open = self.advanced_open.contains(&section);
        let count = rows.len();
        let chevron = if open {
            crate::ui::icons::EXPAND_MORE
        } else {
            crate::ui::icons::CHEVRON_RIGHT
        };
        let header = div()
            .id(gpui::ElementId::Name(
                format!("settings-advanced-{}", section.slug()).into(),
            ))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::PAD_SM))
            .px(px(theme::SETTINGS_CARD_PAD))
            .py(px(theme::PAD_XL))
            .bg(t.button_widget_bg)
            .rounded(px(theme::RADIUS_SM))
            .cursor_pointer()
            .text_size(px(theme::MODAL_BODY_FONT_SIZE))
            .font_weight(gpui::FontWeight::MEDIUM)
            .text_color(t.text_primary)
            .child(crate::ui::icons::icon(chevron))
            .child(s::settings_card_advanced())
            .child(
                div()
                    .ml_auto()
                    .text_size(px(theme::TAB_FONT_SIZE))
                    .font_weight(gpui::FontWeight::NORMAL)
                    .text_color(t.text_muted)
                    .child(s::settings_advanced_count(count)),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_advanced(section, cx)));
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .border_1()
            .border_color(t.border)
            .rounded(px(theme::RADIUS_SM))
            .bg(t.dock_bg)
            .child(header)
            .when(open, |el| el.children(rows))
    }

    pub(super) fn toggle_advanced(
        &mut self,
        section: daruda_config::BuiltinSection,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.advanced_open.remove(&section) {
            self.advanced_open.insert(section);
        }
        cx.notify();
    }

    pub(super) fn set_bool_setting(
        &mut self,
        setting: BoolSetting,
        value: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.persist_bool_setting(setting, value, cx) {
            (spec::bool_spec(setting).set)(self, value);
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_width_leaves_room_for_two_columns_and_card_insets() {
        const {
            assert!(
                theme::SETTINGS_CONTENT_MAX_W
                    >= theme::SETTINGS_CARD_PAD * 2.
                        + theme::SETTINGS_LABEL_MIN_W
                        + theme::SETTINGS_ROW_GAP
                        + theme::SETTINGS_CONTROL_W
            );
            assert!(theme::SETTINGS_NUMBER_W < theme::SETTINGS_CONTROL_W);
        }
    }
}
