//! Plugin section of the Settings window — master/detail panes,
//! skill rows, action buttons, and the supporting helpers.
//!
//! Every method this file adds to `SettingsWindow` uses
//! `pub(in crate::settings_window)` so `sections::mod` and the
//! `settings_window::render` dispatcher can call them, but no code
//! outside `settings_window` can.

use crate::ui::button;
use crate::ui::theme;

use super::super::{PluginSkillBodyState, PluginSkillView, SettingsWindow};
use crate::agent::skills::plugins::{PluginAvailability, PluginInstall};
use crate::agent::skills::{Skill, SkillInvocation};
use crate::surface::strings as s;
use daruda_store::observability::system_info::redact_home;
use gpui::{AnyElement, ClickEvent, IntoElement, SharedString, div, prelude::*, px};
use std::collections::{BTreeMap, HashSet};

/// One row's worth of plugin metadata for the Settings → Plugin page.
/// Built by [`group_plugins_for_settings`] which dedupes by
/// `plugin_id` so a multi-skill plugin shows as a single row.
struct PluginGroupForSettings {
    /// Fully qualified id (`<plugin>@<marketplace>`). Stable across
    /// reloads; used as the Install / Uninstall CLI argument.
    plugin_id: String,
    /// Local name (`<plugin>` part) — what we show on the row.
    plugin_local: String,
    /// `Installed` vs `Available`. Drives which group the row joins
    /// and which button (Install / Uninstall) renders.
    availability: Option<PluginAvailability>,
    /// How many skills this plugin contributes — surfaced as a `· N
    /// skills` suffix on the row.
    skill_count: usize,
}

/// Dedupe the flat plugin-scope skill list into one row per
/// `plugin_id`. Sorted by lowercase local name so the order is stable
/// across renders.
fn group_plugins_for_settings(skills: &[Skill]) -> Vec<PluginGroupForSettings> {
    use std::collections::BTreeMap;
    let mut buckets: BTreeMap<String, PluginGroupForSettings> = BTreeMap::new();
    for s in skills {
        let Some(id) = s.plugin_id.clone() else {
            continue;
        };
        let local = id
            .split_once('@')
            .map(|(l, _)| l.to_string())
            .unwrap_or_else(|| id.clone());
        let entry = buckets
            .entry(id.clone())
            .or_insert_with(|| PluginGroupForSettings {
                plugin_id: id,
                plugin_local: local,
                availability: s.plugin_availability,
                skill_count: 0,
            });
        entry.skill_count += 1;
    }
    let mut out: Vec<PluginGroupForSettings> = buckets.into_values().collect();
    out.sort_by(|a, b| {
        a.plugin_local
            .to_ascii_lowercase()
            .cmp(&b.plugin_local.to_ascii_lowercase())
    });
    out
}

fn plugin_subheading(label: impl Into<gpui::SharedString>, cx: &gpui::App) -> impl IntoElement {
    div()
        .text_size(px(theme::MODAL_BODY_FONT_SIZE))
        .text_color(theme::current(cx).text_primary)
        .child(label.into())
}

fn plugin_empty_hint(label: impl Into<gpui::SharedString>, cx: &gpui::App) -> impl IntoElement {
    div()
        .text_size(px(theme::MODAL_BODY_FONT_SIZE))
        .text_color(theme::current(cx).text_body)
        .child(label.into())
}

/// Two-column row used inside the detail pane: a fixed-width label on
/// the left, free-form value on the right. Keeps every line aligned
/// when rendered in a `flex_col` so multiple `detail_row`s read as a
/// table.
fn detail_row(
    label: impl Into<gpui::SharedString>,
    value: SharedString,
    cx: &gpui::App,
) -> impl IntoElement {
    let t = theme::current(cx);
    let label_color = t.text_body;
    let value_color = t.text_primary;
    div()
        .flex()
        .flex_row()
        .gap(px(theme::SKILL_HEADER_GAP))
        .child(
            div()
                .w(px(theme::SETTINGS_PLUGIN_LABEL_W))
                .flex_none()
                .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                .text_color(label_color)
                .child(label.into()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                .text_color(value_color)
                .child(value),
        )
}

/// Thin horizontal rule used between meta and skills sections inside
/// the right pane.
fn plugin_divider(cx: &gpui::App) -> impl IntoElement {
    div().h(px(1.0_f32)).w_full().bg(theme::current(cx).border)
}

/// Display string for the invocation header — the literal characters
/// the user types after `/`. For plugin-scope skills the scanner has
/// already baked the `<plugin>:<skill>` namespace into `skill.name`
/// (see `scan::scan_plugins`), so the raw field is the right value.
fn display_name_for_invocation(skill: &Skill) -> String {
    skill.name.clone()
}

/// User-facing label for the 4-state `SkillInvocation`. Read off the
/// `user_invocable` / `disable_model_invocation` frontmatter pair.
fn invocation_status_label(inv: SkillInvocation) -> String {
    match inv {
        SkillInvocation::Both => s::settings_plugin_skill_invocation_both(),
        SkillInvocation::UserOnly => s::settings_plugin_skill_invocation_user_only(),
        SkillInvocation::ModelOnly => s::settings_plugin_skill_invocation_model_only(),
        SkillInvocation::Disabled => s::settings_plugin_skill_invocation_disabled(),
    }
}

/// Read `~/.claude/plugins/installed_plugins.json` and return the
/// records keyed by `<plugin>@<marketplace>` for O(1) lookup from the
/// detail pane. Errors collapse to an empty map — the detail pane
/// then surfaces `—` for the missing fields, matching the policy of
/// the upstream loader.
fn read_plugin_installs_indexed() -> BTreeMap<String, PluginInstall> {
    let path = crate::agent::skills::plugins::installed_plugins_manifest();
    let installs = crate::agent::skills::plugins::read_installed_plugins(&path);
    let mut out = BTreeMap::new();
    for install in installs {
        out.insert(install.id.clone(), install);
    }
    out
}

impl SettingsWindow {
    pub(in crate::settings_window) fn render_plugin(
        &self,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        // Snapshot the skills Global once per render to keep all derived
        // views consistent: the master list, the detail header, the
        // skill table. Reading `cx.global::<...>()` repeatedly in the
        // same paint walk is fine, but a single clone keeps the helper
        // signatures GPUI-free.
        let plugin_skills: Vec<Skill> = cx
            .global::<crate::agent::skills::SkillsState>()
            .plugin
            .clone();
        let groups = group_plugins_for_settings(&plugin_skills);
        let installs = read_plugin_installs_indexed();
        let in_flight = self.plugin_ops_in_flight.clone();

        let mut header_col = div()
            .flex()
            .flex_col()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(Self::section_label(s::settings_section_plugin(), cx));
        if let Some(err) = self.plugin_last_error.clone() {
            header_col = header_col.child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(theme::ERROR)
                    .child(err),
            );
        }

        let master = self.plugin_master_pane(&groups, cx);
        let detail = self.plugin_detail_pane(&groups, &plugin_skills, &installs, &in_flight, cx);

        let split = div()
            .flex()
            .flex_row()
            .gap(px(theme::MODAL_PANEL_GAP))
            .child(
                div()
                    .w(px(theme::SETTINGS_PLUGIN_MASTER_W))
                    .flex_none()
                    .child(master),
            )
            .child(div().flex_1().min_w_0().child(detail));

        header_col.child(split).into_any_element()
    }

    /// Left pane — flat list of every plugin row, grouped by
    /// `Installed` vs `Available`. Clicking a row sets
    /// `plugin_selected` so the right pane refreshes; the same row
    /// renders highlighted on the next paint.
    fn plugin_master_pane(
        &self,
        groups: &[PluginGroupForSettings],
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement + use<> {
        let installed: Vec<&PluginGroupForSettings> = groups
            .iter()
            .filter(|g| matches!(g.availability, Some(PluginAvailability::Installed)))
            .collect();
        let available: Vec<&PluginGroupForSettings> = groups
            .iter()
            .filter(|g| matches!(g.availability, Some(PluginAvailability::Available)))
            .collect();

        let mut col = div().flex().flex_col().gap(px(theme::MODAL_PANEL_GAP));

        col = col.child(plugin_subheading(s::settings_plugin_installed_header(), cx));
        if installed.is_empty() {
            col = col.child(plugin_empty_hint(s::settings_plugin_none_installed(), cx));
        } else {
            for g in &installed {
                col = col.child(self.plugin_master_row(g, cx));
            }
        }

        col = col.child(plugin_subheading(s::settings_plugin_available_header(), cx));
        if available.is_empty() {
            col = col.child(plugin_empty_hint(s::settings_plugin_none_available(), cx));
        } else {
            for g in &available {
                col = col.child(self.plugin_master_row(g, cx));
            }
        }
        col
    }

    fn plugin_master_row(
        &self,
        group: &PluginGroupForSettings,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement + use<> {
        let is_selected = self
            .plugin_selected
            .as_deref()
            .is_some_and(|sel| sel == group.plugin_id);
        let count_text = SharedString::from(if group.skill_count == 1 {
            "1 skill".to_string()
        } else {
            format!("{} skills", group.skill_count)
        });
        let row_id = SharedString::from(format!("settings-plugin-master-{}", group.plugin_id));
        let plugin_id = group.plugin_id.clone();

        let t = theme::current(cx);
        let title_color = t.text_primary;
        let subtitle_color = t.text_body;
        let active_bg = t.overlay_prominent;
        let hover_bg = t.skill_row_hover_bg;

        let mut row = div()
            .id(row_id)
            .flex()
            .flex_col()
            .gap(px(theme::SKILL_ROW_GAP))
            .px(px(theme::SKILL_ROW_PAD_X))
            .py(px(theme::SKILL_ROW_PAD_Y))
            .rounded(px(theme::SKILL_ROW_RADIUS))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.plugin_selected = Some(plugin_id.clone());
                this.plugin_view_skill = None;
                cx.notify();
            }))
            .child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(title_color)
                    .child(SharedString::from(group.plugin_local.clone())),
            )
            .child(
                div()
                    .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                    .text_color(subtitle_color)
                    .child(count_text),
            );
        if is_selected {
            row = row.bg(active_bg);
        } else {
            row = row.hover(move |el| el.bg(hover_bg));
        }
        row
    }

    /// Right pane — three states:
    /// 1. No selection → placeholder hint.
    /// 2. `plugin_view_skill = Some(...)` → SKILL.md viewer.
    /// 3. Otherwise → marketplace info + skills table for the
    ///    selected plugin.
    fn plugin_detail_pane(
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
            return plugin_empty_hint(s::settings_plugin_detail_empty(), cx).into_any_element();
        };
        let Some(group) = groups.iter().find(|g| g.plugin_id == selected_id) else {
            // Selection went stale (plugin uninstalled while open).
            return plugin_empty_hint(s::settings_plugin_detail_empty(), cx).into_any_element();
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
        let header = div().flex().flex_col().gap(px(theme::SKILL_ROW_GAP)).child(
            div()
                .text_size(px(theme::MODAL_TITLE_FONT_SIZE))
                .text_color(title_color)
                .child(SharedString::from(group.plugin_id.clone())),
        );

        let availability_text = match group.availability {
            Some(PluginAvailability::Installed) => s::settings_plugin_detail_status_installed(),
            Some(PluginAvailability::Available) => s::settings_plugin_detail_status_available(),
            None => s::settings_plugin_detail_unknown(),
        };

        let version_text = install
            .map(|i| i.version.clone())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| s::settings_plugin_detail_unknown().to_string());
        let path_text = install
            .map(|i| redact_home(&i.install_path))
            .unwrap_or_else(|| s::settings_plugin_detail_unknown().to_string());
        let scope_text = install
            .map(|i| i.scope.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| s::settings_plugin_detail_unknown().to_string());

        let meta = div()
            .flex()
            .flex_col()
            .gap(px(theme::SKILL_ROW_GAP))
            .child(detail_row(
                s::settings_plugin_detail_marketplace(),
                SharedString::from(marketplace_id),
                cx,
            ))
            .child(detail_row(
                s::settings_plugin_detail_availability(),
                SharedString::from(availability_text),
                cx,
            ))
            .child(detail_row(
                s::settings_plugin_detail_version(),
                SharedString::from(version_text),
                cx,
            ))
            .child(detail_row(
                s::settings_plugin_detail_scope(),
                SharedString::from(scope_text),
                cx,
            ))
            .child(detail_row(
                s::settings_plugin_detail_path(),
                SharedString::from(path_text),
                cx,
            ));

        let action_row = self.plugin_action_row(group, in_flight, cx);

        let mut skills_col =
            div()
                .flex()
                .flex_col()
                .gap(px(theme::SKILL_ROW_GAP))
                .child(plugin_subheading(
                    s::settings_plugin_detail_skills_header(),
                    cx,
                ));
        if skills_for_plugin.is_empty() {
            skills_col =
                skills_col.child(plugin_empty_hint(s::settings_plugin_detail_no_skills(), cx));
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
        use crate::agent::skills::plugin_ops::PluginAction;
        use crate::ui::Disableable as _;

        let is_in_flight = in_flight.contains(&group.plugin_id);
        let (button_label, action) = match (group.availability, is_in_flight) {
            (Some(PluginAvailability::Installed), false) => (
                s::settings_plugin_uninstall(),
                Some(PluginAction::Uninstall),
            ),
            (Some(PluginAvailability::Installed), true) => {
                (s::settings_plugin_uninstalling(), None)
            }
            (Some(PluginAvailability::Available), false) => {
                (s::settings_plugin_install(), Some(PluginAction::Install))
            }
            (Some(PluginAvailability::Available), true) => (s::settings_plugin_installing(), None),
            (None, _) => (String::new(), None),
        };

        let button_id = SharedString::from(format!("settings-plugin-action-{}", group.plugin_id));
        let plugin_id = group.plugin_id.clone();
        let mut btn = button(button_id, button_label).disabled(action.is_none());
        if let Some(action) = action {
            let plugin_id_for_handler = plugin_id.clone();
            btn = btn.on_click(cx.listener(move |this, _: &ClickEvent, _window, cx| {
                this.run_plugin_op(plugin_id_for_handler.clone(), action, cx);
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
            .gap(px(theme::SKILL_ROW_GAP))
            .px(px(theme::SKILL_ROW_PAD_X))
            .py(px(theme::SKILL_ROW_PAD_Y))
            .rounded(px(theme::SKILL_ROW_RADIUS))
            .hover(move |el| el.bg(row_hover_bg))
            .child(header_line);

        if let Some(desc) = description {
            col = col.child(detail_row(s::settings_plugin_skill_description(), desc, cx));
        }
        if let Some(hint) = arg_hint {
            col = col.child(detail_row(
                s::settings_plugin_skill_argument_hint(),
                hint,
                cx,
            ));
        }
        if let Some(tools) = allowed_tools {
            col = col.child(detail_row(
                s::settings_plugin_skill_allowed_tools(),
                tools,
                cx,
            ));
        }
        if let Some(p) = paths {
            col = col.child(detail_row(s::settings_plugin_skill_paths(), p, cx));
        }
        if let Some(w) = when_to_use {
            col = col.child(detail_row(s::settings_plugin_skill_when_to_use(), w, cx));
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
        button(id, s::settings_plugin_skill_view()).on_click(cx.listener(
            move |this, _: &ClickEvent, _, cx| {
                this.open_plugin_skill_view(display_name.clone(), skill_md_path.clone(), cx);
            },
        ))
    }

    fn plugin_skill_view_pane(
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
                    s::settings_plugin_skill_back(),
                )
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
            s::settings_plugin_detail_path(),
            SharedString::from(redact_home(&view.skill_md_path)),
            cx,
        );

        let body_block: AnyElement = match &view.body {
            PluginSkillBodyState::Loading => div()
                .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                .text_color(secondary_color)
                .child(s::settings_plugin_skill_body_loading())
                .into_any_element(),
            PluginSkillBodyState::Error(msg) => div()
                .flex()
                .flex_col()
                .gap(px(theme::SKILL_ROW_GAP))
                .child(
                    div()
                        .text_size(px(theme::MODAL_BODY_FONT_SIZE))
                        .text_color(error_color)
                        .child(s::settings_plugin_skill_body_error()),
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
                .gap(px(theme::SKILL_ROW_GAP))
                .child(
                    div()
                        .text_size(px(theme::SKILL_BADGE_FONT_SIZE))
                        .text_color(section_header_color)
                        .child(s::settings_plugin_skill_body()),
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
    pub(in crate::settings_window) fn open_plugin_skill_view(
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

    /// Spawn `claude plugin install/uninstall <plugin_id>` on the
    /// background executor, then broadcast a watcher refresh to every
    /// open Workspace so the right-bar Skills tab reflects the new
    /// state.
    pub(in crate::settings_window) fn run_plugin_op(
        &mut self,
        plugin_id: String,
        action: crate::agent::skills::plugin_ops::PluginAction,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.plugin_ops_in_flight.insert(plugin_id.clone()) {
            // Duplicate click while a previous spawn is still running.
            return;
        }
        self.plugin_last_error = None;
        cx.notify();

        let plugin_id_for_task = plugin_id.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    crate::agent::skills::plugin_ops::run_plugin_action(
                        action,
                        &plugin_id_for_task,
                        crate::agent::skills::plugin_ops::PluginScope::User,
                    )
                })
                .await;
            // SILENT-OK: settings modal may close before async defer fires
            let _ = this.update(cx, |this, cx| {
                this.plugin_ops_in_flight.remove(&plugin_id);
                if let Err(e) = result {
                    let verb = match action {
                        crate::agent::skills::plugin_ops::PluginAction::Install => "install",
                        crate::agent::skills::plugin_ops::PluginAction::Uninstall => "uninstall",
                    };
                    this.plugin_last_error = Some(SharedString::from(format!(
                        "plugin {verb} {plugin_id}: {e}"
                    )));
                }
                // Update the Plugin scope of the `SkillsState` Global
                // directly: the FSEvent stream lags the CLI's atomic
                // `installed_plugins.json` write, and depending on
                // `WindowRegistry::for_each_workspace` would silently
                // do nothing when no Workspace is open (e.g. user
                // running Settings from the welcome window). Direct
                // mutation triggers `observe_global::<SkillsState>` on
                // every observer — every open Workspace's Skills tab
                // and this Settings page's render path both re-paint
                // off the same source of truth.
                use gpui::BorrowAppContext as _;
                let personal = crate::agent::skills::scan::skills_personal_dir();
                cx.update_global::<crate::agent::skills::SkillsState, _>(|state, _| {
                    state.reload_scope(crate::agent::skills::SkillScope::Plugin, None, &personal);
                });
                cx.notify();
            });
        })
        .detach();
    }
}
