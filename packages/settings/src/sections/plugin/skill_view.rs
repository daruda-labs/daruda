//! The in-Settings SKILL.md viewer for one plugin skill and its async load.

use super::{detail_row, plugin_divider};
use crate::strings as s;
use crate::{PluginSkillBodyState, PluginSkillView, SettingsView, settings_button as button};
use daruda_store::observability::system_info::redact_home;
use daruda_ui::theme;
use gpui::{AnyElement, ClickEvent, IntoElement, SharedString, div, prelude::*, px};

impl SettingsView {
    pub(super) fn plugin_skill_view_pane(
        &self,
        view: PluginSkillView,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let t = theme::current(cx);
        let title_color = t.text_primary;
        let secondary_color = t.text_body;
        let error_color = theme::ERROR;
        let section_header_color = t.text_muted;
        let input_bg = t.modal_input_bg;
        let input_border = t.border;
        let body_text_color = t.text_primary;

        let header = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::SKILL_HEADER_GAP))
            .child(
                button(
                    "settings-plugin-skill-back",
                    s::settings::plugin_skill_back(),
                )
                .child(daruda_ui::icons::icon(daruda_ui::icons::BACK))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.plugin_view_skill = None;
                    cx.notify();
                })),
            )
            .child(
                div()
                    .text_size(px(theme::MODAL_TITLE_FONT_SIZE))
                    .text_color(title_color)
                    .child(SharedString::from(format!("/{}", view.display_name))),
            );

        let path_row = detail_row(
            s::settings::plugin_detail_path(),
            SharedString::from(redact_home(&view.skill_md_path)),
            cx,
        );

        let body_block: AnyElement = match &view.body {
            PluginSkillBodyState::Loading => div()
                .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                .text_color(secondary_color)
                .child(s::settings::plugin_skill_body_loading())
                .into_any_element(),
            PluginSkillBodyState::Error(msg) => div()
                .flex()
                .flex_col()
                .gap(px(theme::LIST_ROW_GAP))
                .child(
                    div()
                        .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                        .text_color(error_color)
                        .child(s::settings::plugin_skill_body_error()),
                )
                .child(
                    div()
                        .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                        .text_color(secondary_color)
                        .child(msg.clone()),
                )
                .into_any_element(),
            PluginSkillBodyState::Loaded(body) => div()
                .flex()
                .flex_col()
                .gap(px(theme::LIST_ROW_GAP))
                .child(
                    div()
                        .text_size(px(theme::SKILL_BADGE_FONT_SIZE))
                        .text_color(section_header_color)
                        .child(s::settings::plugin_skill_body()),
                )
                .child(
                    div()
                        .px(px(theme::MODAL_INPUT_PAD))
                        .py(px(theme::MODAL_INPUT_PAD))
                        .rounded(px(theme::MODAL_BUTTON_RADIUS))
                        .bg(input_bg)
                        .border_1()
                        .border_color(input_border)
                        .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                        .text_color(body_text_color)
                        .whitespace_normal()
                        .child(body.clone()),
                )
                .into_any_element(),
        };

        div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(header)
            .child(path_row)
            .child(plugin_divider(cx))
            .child(body_block)
            .into_any_element()
    }

    /// Open the SKILL.md viewer for the given plugin skill. Sets the
    /// `plugin_view_skill` state to `Loading` and spawns a background
    /// disk read; once the read settles, the spawn updates the state
    /// to `Loaded(body)` or `Error(msg)` and re-renders.
    pub(crate) fn open_plugin_skill_view(
        &mut self,
        display_name: String,
        skill_md_path: std::path::PathBuf,
        cx: &mut gpui::Context<Self>,
    ) {
        self.plugin_view_skill = Some(PluginSkillView {
            display_name,
            skill_md_path: skill_md_path.clone(),
            body: PluginSkillBodyState::Loading,
        });
        cx.notify();

        let path_for_task = skill_md_path.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { std::fs::read_to_string(&path_for_task) })
                .await;
            // SILENT-OK: settings modal may close before async defer fires
            let _ = this.update(cx, |this, cx| {
                if let Some(view) = this.plugin_view_skill.as_mut() {
                    // Verify the response is still for the file the user
                    // is looking at — otherwise a slow read for skill A
                    // would clobber a freshly opened skill B.
                    if view.skill_md_path == skill_md_path {
                        view.body = match result {
                            Ok(body) => PluginSkillBodyState::Loaded(SharedString::from(body)),
                            Err(e) => {
                                PluginSkillBodyState::Error(SharedString::from(e.to_string()))
                            }
                        };
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }
}
