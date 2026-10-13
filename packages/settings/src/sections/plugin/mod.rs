//! Plugin section of Settings — master/detail panes,
//! skill rows, action buttons, and the supporting helpers.
//!
//! The section's entry points on `SettingsView` are
//! `pub(crate)` so `sections::mod` and the
//! `settings::render` dispatcher can call them, but no code
//! outside `settings` can; the panes in child modules stay `pub(super)`.

mod detail;
mod skill_view;

use daruda_ui::theme;

use super::super::{SettingsEvent, SettingsView};
use crate::strings as s;
use daruda_agent::skills::plugins::{PluginAvailability, PluginInstall};
use daruda_agent::skills::{Skill, SkillInvocation};
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::{AnyElement, ClickEvent, IntoElement, SharedString, div, prelude::*, px};
use std::collections::BTreeMap;

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
        SkillInvocation::Both => s::settings::plugin_skill_invocation_both(),
        SkillInvocation::UserOnly => s::settings::plugin_skill_invocation_user_only(),
        SkillInvocation::ModelOnly => s::settings::plugin_skill_invocation_model_only(),
        SkillInvocation::Disabled => s::settings::plugin_skill_invocation_disabled(),
    }
}

/// Read `~/.claude/plugins/installed_plugins.json` and return the
/// records keyed by `<plugin>@<marketplace>` for O(1) lookup from the
/// detail pane. Errors collapse to an empty map — the detail pane
/// then surfaces `—` for the missing fields, matching the policy of
/// the upstream loader.
pub(crate) fn read_plugin_installs_indexed() -> BTreeMap<String, PluginInstall> {
    let path = daruda_agent::skills::plugins::installed_plugins_manifest();
    let installs = daruda_agent::skills::plugins::read_installed_plugins(&path);
    let mut out = BTreeMap::new();
    for install in installs {
        out.insert(install.id.clone(), install);
    }
    out
}

impl SettingsView {
    pub(crate) fn render_plugin(&self, cx: &mut gpui::Context<Self>) -> AnyElement {
        // One clone per render keeps the master list, the detail header and
        // the skill table on the same snapshot.
        let plugin_skills: Vec<Skill> = self.plugin_skills.clone();
        let groups = group_plugins_for_settings(&plugin_skills);
        let in_flight = self.plugin_ops_in_flight.clone();

        let header_col = div().flex().flex_col().gap(px(theme::SETTINGS_GROUP_GAP));
        let master = self.plugin_master_pane(&groups, cx);
        let detail = self.plugin_detail_pane(
            &groups,
            &plugin_skills,
            &self.plugin_installs,
            &in_flight,
            cx,
        );

        let split = div()
            .flex()
            .flex_row()
            .flex_wrap()
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

        col = col.child(plugin_subheading(
            s::settings::plugin_installed_header(),
            cx,
        ));
        if installed.is_empty() {
            col = col.child(plugin_empty_hint(s::settings::plugin_none_installed(), cx));
        } else {
            for g in &installed {
                col = col.child(self.plugin_master_row(g, cx));
            }
        }

        col = col.child(plugin_subheading(
            s::settings::plugin_available_header(),
            cx,
        ));
        if available.is_empty() {
            col = col.child(plugin_empty_hint(s::settings::plugin_none_available(), cx));
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
        let count_text = SharedString::from(s::settings::plugin_skill_count(group.skill_count));
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
            .gap(px(theme::LIST_ROW_GAP))
            .px(px(theme::LIST_ROW_PAD_X))
            .py(px(theme::LIST_ROW_PAD_Y))
            .rounded(px(theme::LIST_ROW_RADIUS))
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

    /// Spawn `claude plugin install/uninstall <plugin_id>` on the
    /// background executor, then broadcast a watcher refresh to every
    /// open Workspace so the right-bar Skills tab reflects the new
    /// state.
    pub(crate) fn run_plugin_op(
        &mut self,
        plugin_id: String,
        action: daruda_agent::skills::plugin_ops::PluginAction,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.plugin_ops_in_flight.insert(plugin_id.clone()) {
            // Duplicate click while a previous spawn is still running.
            return;
        }
        self.error = None;
        cx.notify();

        let plugin_id_for_task = plugin_id.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    daruda_agent::skills::plugin_ops::run_plugin_action(
                        action,
                        &plugin_id_for_task,
                        daruda_agent::skills::plugin_ops::PluginScope::User,
                    )
                })
                .await;
            // SILENT-OK: settings modal may close before async defer fires
            let _ = this.update(cx, |this, cx| {
                this.finish_plugin_op(&plugin_id, action, result, cx);
            });
        })
        .detach();
    }

    /// Fold one finished plugin op back into the window: drop its in-flight
    /// marker, report a failure, and republish the skills snapshot.
    ///
    /// Split from the spawn above so the outcome handling is reachable without
    /// running the `claude` CLI.
    pub(crate) fn finish_plugin_op(
        &mut self,
        plugin_id: &str,
        action: daruda_agent::skills::plugin_ops::PluginAction,
        result: Result<String, daruda_agent::skills::plugin_ops::PluginOpError>,
        cx: &mut gpui::Context<Self>,
    ) {
        self.plugin_ops_in_flight.remove(plugin_id);
        match result {
            // The banner reports the outcome of the last action, whichever it
            // was, so a success clears whatever the previous one left. Every
            // other action in this window behaves the same way.
            Ok(_) => self.error = None,
            Err(e) => {
                let text = match action {
                    daruda_agent::skills::plugin_ops::PluginAction::Install => {
                        s::settings::plugin_install_failed(plugin_id, e.to_string())
                    }
                    daruda_agent::skills::plugin_ops::PluginAction::Uninstall => {
                        s::settings::plugin_uninstall_failed(plugin_id, e.to_string())
                    }
                };
                self.report_section_error(
                    text,
                    ErrorReport::new("Plugin operation failed")
                        .severity(ErrorSeverity::Warning)
                        .from_error(&e)
                        .at(file!(), line!())
                        .with_context("plugin", plugin_id.to_owned())
                        .dedup("settings.plugin_op"),
                    cx,
                );
            }
        }
        // The host rescans for every window rather than waiting for the
        // file watcher, which lags the CLI's atomic manifest write.
        cx.emit(SettingsEvent::PluginsChanged);
        cx.notify();
    }
}
