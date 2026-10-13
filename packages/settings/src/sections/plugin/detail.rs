//! The Plugin section's right pane: marketplace details, the install action,
//! and the selected plugin's skill rows.

use super::{
    PluginGroupForSettings, detail_row, display_name_for_invocation, invocation_status_label,
    plugin_divider, plugin_empty_hint, plugin_subheading,
};
use crate::strings as s;
use crate::{SettingsView, settings_button as button};
use daruda_agent::skills::Skill;
use daruda_agent::skills::plugins::{PluginAvailability, PluginInstall};
use daruda_store::observability::system_info::redact_home;
use daruda_ui::theme;
use gpui::{AnyElement, ClickEvent, IntoElement, SharedString, div, prelude::*, px};
use std::collections::{BTreeMap, HashSet};

impl SettingsView {
    /// Right pane — three states:
    /// 1. No selection → placeholder hint.
    /// 2. `plugin_view_skill = Some(...)` → SKILL.md viewer.
    /// 3. Otherwise → marketplace info + skills table for the
    ///    selected plugin.
    pub(super) fn plugin_detail_pane(
        &self,
        groups: &[PluginGroupForSettings],
        plugin_skills: &[Skill],
        installs: &BTreeMap<String, PluginInstall>,
        in_flight: &HashSet<String>,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        if let Some(view) = self.plugin_view_skill.clone() {
            return self.plugin_skill_view_pane(view, cx);
        }
        let Some(selected_id) = self.plugin_selected.clone() else {
            return plugin_empty_hint(s::settings::plugin_detail_empty(), cx).into_any_element();
        };
        let Some(group) = groups.iter().find(|g| g.plugin_id == selected_id) else {
            // Selection went stale (plugin uninstalled while open).
            return plugin_empty_hint(s::settings::plugin_detail_empty(), cx).into_any_element();
        };

        let install = installs.get(&group.plugin_id);
        let marketplace_id = group
            .plugin_id
            .split_once('@')
            .map(|(_, m)| m.to_string())
            .unwrap_or_default();

        let skills_for_plugin: Vec<&Skill> = plugin_skills
            .iter()
            .filter(|sk| sk.plugin_id.as_deref() == Some(group.plugin_id.as_str()))
            .collect();

        let title_color = theme::current(cx).text_primary;
        let header = div().flex().flex_col().gap(px(theme::LIST_ROW_GAP)).child(
            div()
                .text_size(px(theme::MODAL_TITLE_FONT_SIZE))
                .text_color(title_color)
                .child(SharedString::from(group.plugin_id.clone())),
        );

        let availability_text = match group.availability {
            Some(PluginAvailability::Installed) => s::settings::plugin_detail_status_installed(),
            Some(PluginAvailability::Available) => s::settings::plugin_detail_status_available(),
            None => s::settings::plugin_detail_unknown(),
        };

        let version_text = install
            .map(|i| i.version.clone())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| s::settings::plugin_detail_unknown().to_string());
        let path_text = install
            .map(|i| redact_home(&i.install_path))
            .unwrap_or_else(|| s::settings::plugin_detail_unknown().to_string());
        let scope_text = install
            .map(|i| i.scope.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| s::settings::plugin_detail_unknown().to_string());

        let meta = div()
            .flex()
            .flex_col()
            .gap(px(theme::LIST_ROW_GAP))
            .child(detail_row(
                s::settings::plugin_detail_marketplace(),
                SharedString::from(marketplace_id),
                cx,
            ))
            .child(detail_row(
                s::settings::plugin_detail_availability(),
                SharedString::from(availability_text),
                cx,
            ))
            .child(detail_row(
                s::settings::plugin_detail_version(),
                SharedString::from(version_text),
                cx,
            ))
            .child(detail_row(
                s::settings::plugin_detail_scope(),
                SharedString::from(scope_text),
                cx,
            ))
            .child(detail_row(
                s::settings::plugin_detail_path(),
                SharedString::from(path_text),
                cx,
            ));

        let action_row = self.plugin_action_row(group, in_flight, cx);

        let mut skills_col =
            div()
                .flex()
                .flex_col()
                .gap(px(theme::LIST_ROW_GAP))
                .child(plugin_subheading(
                    s::settings::plugin_detail_skills_header(),
                    cx,
                ));
        if skills_for_plugin.is_empty() {
            skills_col = skills_col.child(plugin_empty_hint(
                s::settings::plugin_detail_no_skills(),
                cx,
            ));
        } else {
            for sk in &skills_for_plugin {
                skills_col = skills_col.child(self.plugin_skill_row(sk, cx));
            }
        }

        div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(header)
            .child(meta)
            .child(action_row)
            .child(plugin_divider(cx))
            .child(skills_col)
            .into_any_element()
    }

    fn plugin_action_row(
        &self,
        group: &PluginGroupForSettings,
        in_flight: &HashSet<String>,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement + use<> {
        use daruda_agent::skills::plugin_ops::PluginAction;
        use daruda_ui::Disableable as _;

        let is_in_flight = in_flight.contains(&group.plugin_id);
        let (button_label, action) = match (group.availability, is_in_flight) {
            (Some(PluginAvailability::Installed), false) => (
                s::settings::plugin_uninstall(),
                Some(PluginAction::Uninstall),
            ),
            (Some(PluginAvailability::Installed), true) => {
                (s::settings::plugin_uninstalling(), None)
            }
            (Some(PluginAvailability::Available), false) => {
                (s::settings::plugin_install(), Some(PluginAction::Install))
            }
            (Some(PluginAvailability::Available), true) => (s::settings::plugin_installing(), None),
            (None, _) => (String::new(), None),
        };

        let button_id = SharedString::from(format!("settings-plugin-action-{}", group.plugin_id));
        let plugin_id = group.plugin_id.clone();
        let mut btn = button(button_id, button_label).disabled(action.is_none());
        if let Some(action) = action {
            let plugin_id_for_handler = plugin_id.clone();
            btn = btn.on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.request_plugin_op(plugin_id_for_handler.clone(), action, window, cx);
            }));
        }
        div().flex().flex_row().child(btn)
    }

    fn plugin_skill_row(
        &self,
        skill: &Skill,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement + use<> {
        let display_name = display_name_for_invocation(skill);
        let slash_cmd = SharedString::from(format!("/{display_name}"));
        let invocation_label =
            SharedString::from(invocation_status_label(skill.invocation()).to_string());
        let description = skill
            .frontmatter
            .description
            .clone()
            .filter(|d| !d.is_empty())
            .map(SharedString::from);
        let arg_hint = skill
            .frontmatter
            .argument_hint
            .clone()
            .filter(|d| !d.is_empty())
            .map(SharedString::from);
        let allowed_tools = skill
            .frontmatter
            .allowed_tools
            .clone()
            .filter(|d| !d.is_empty())
            .map(SharedString::from);
        let paths = skill
            .frontmatter
            .paths
            .clone()
            .filter(|d| !d.is_empty())
            .map(SharedString::from);
        let when_to_use = skill
            .frontmatter
            .when_to_use
            .clone()
            .filter(|d| !d.is_empty())
            .map(SharedString::from);

        let row_id = SharedString::from(format!(
            "settings-plugin-skill-row-{}-{}",
            skill.plugin_id.clone().unwrap_or_default(),
            skill.name
        ));

        let t = theme::current(cx);
        let name_color = t.text_primary;
        let meta_color = t.text_body;
        let row_hover_bg = t.skill_row_hover_bg;

        let header_line = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::SKILL_HEADER_GAP))
            .child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(name_color)
                    .child(slash_cmd),
            )
            .child(
                div()
                    .text_size(px(theme::SKILL_BADGE_FONT_SIZE))
                    .text_color(meta_color)
                    .child(invocation_label),
            )
            .child(div().flex_1())
            .child(self.plugin_skill_view_button(skill, cx));

        let mut col = div()
            .id(row_id)
            .flex()
            .flex_col()
            .gap(px(theme::LIST_ROW_GAP))
            .px(px(theme::LIST_ROW_PAD_X))
            .py(px(theme::LIST_ROW_PAD_Y))
            .rounded(px(theme::LIST_ROW_RADIUS))
            .hover(move |el| el.bg(row_hover_bg))
            .child(header_line);

        if let Some(desc) = description {
            col = col.child(detail_row(
                s::settings::plugin_skill_description(),
                desc,
                cx,
            ));
        }
        if let Some(hint) = arg_hint {
            col = col.child(detail_row(
                s::settings::plugin_skill_argument_hint(),
                hint,
                cx,
            ));
        }
        if let Some(tools) = allowed_tools {
            col = col.child(detail_row(
                s::settings::plugin_skill_allowed_tools(),
                tools,
                cx,
            ));
        }
        if let Some(p) = paths {
            col = col.child(detail_row(s::settings::plugin_skill_paths(), p, cx));
        }
        if let Some(w) = when_to_use {
            col = col.child(detail_row(s::settings::plugin_skill_when_to_use(), w, cx));
        }
        col
    }

    fn plugin_skill_view_button(
        &self,
        skill: &Skill,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement + use<> {
        let plugin_id = skill.plugin_id.clone().unwrap_or_default();
        let display_name = display_name_for_invocation(skill);
        let skill_md_path = skill.skill_md_path();
        let id = SharedString::from(format!(
            "settings-plugin-skill-view-{}-{}",
            plugin_id, skill.name
        ));
        // `plugin_id` is captured implicitly by the parent
        // `plugin_selected` field — the [View] button only fires from
        // the detail pane, which can only be entered through
        // master-row selection. Pass `display_name` + `skill_md_path`
        // to the spawn loader so they survive the async disk read.
        let _ = plugin_id;
        button(id, s::settings::plugin_skill_view()).on_click(cx.listener(
            move |this, _: &ClickEvent, _, cx| {
                this.open_plugin_skill_view(display_name.clone(), skill_md_path.clone(), cx);
            },
        ))
    }
}
