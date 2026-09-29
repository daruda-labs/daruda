//! The built-in presets no entry uses yet: a switch for each one daruda can
//! run, an install page for each one it cannot.

use crate::surface::strings as s;
use crate::ui::theme;
use daruda_config::{AgentPreset, PresetLaunchability};
use gpui::{AnyElement, ClickEvent, IntoElement, SharedString, div, prelude::*, px};

use super::super::super::{SettingsView, settings_button as button};
use super::groups::PresetGroups;

impl SettingsView {
    /// Search, then the available and needs-install lists. A list the query
    /// empties still shows its header, so narrowing never hides where an
    /// agent would be.
    pub(super) fn render_preset_lists(
        &self,
        presets: PresetGroups,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(Self::section_label(s::settings_agent_group_available(), cx))
            .child(crate::ui::input(&self.agent_catalog_search, cx, 0));
        for preset in presets.available {
            body = body.child(Self::render_available_preset(preset, cx));
        }
        body = body.child(Self::section_label(
            s::settings_agent_group_needs_install(),
            cx,
        ));
        for preset in presets.needs_install {
            body = body.child(Self::render_needs_install_preset(preset, cx));
        }
        body.into_any_element()
    }

    /// A runnable preset: switching it on adds an entry for it.
    fn render_available_preset(preset: AgentPreset, cx: &mut gpui::Context<Self>) -> AnyElement {
        let id = preset.id;
        let switch = crate::ui::switch(
            SharedString::from(format!("settings-agent-available-{id}")),
            false,
            cx,
        )
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.enable_agent_preset(id, window, cx);
        }));
        Self::preset_row(preset, switch, cx)
    }

    /// A preset that ships binaries daruda cannot launch: its install page.
    fn render_needs_install_preset(
        preset: AgentPreset,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let PresetLaunchability::NeedsManualInstall { install_url } = preset.launchability else {
            return div().into_any_element();
        };
        let install = button(
            SharedString::from(format!("settings-agent-install-{}", preset.id)),
            s::settings_agent_preset_install_page(),
        )
        .on_click(cx.listener(move |_this, _: &ClickEvent, _window, cx| {
            cx.open_url(install_url);
        }));
        Self::preset_row(preset, install, cx)
    }

    /// Icon, name and id on the left, `action` on the right.
    fn preset_row(
        preset: AgentPreset,
        action: impl IntoElement,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let t = theme::current(cx);
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::MODAL_FOOTER_GAP))
            .child(crate::ui::agent_icon(
                crate::agent::icons::icon_for_agent(preset.id),
                px(theme::SETTINGS_AGENT_CARD_ICON_SIZE),
                t.text_body,
            ))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(theme::MODAL_FOOTER_GAP))
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .child(div().text_color(t.text_primary).child(preset.name))
                    .child(div().text_color(t.text_muted).truncate().child(preset.id)),
            )
            .child(action)
            .into_any_element()
    }
}
