//! Skills tab body — renders the project + personal skill scopes
//! pulled from `RightDockSnapshot::skills` (a snapshot of `Workspace::skills`).
//!
//! Layout:
//! ```text
//! ┌─ Skills ──────────────────────────────────── [⚙][+] ┐
//! │  [ Search skills… ]                                 │
//! │  ▾ PROJECT                                        1 │
//! │  ▤ pr-review                                        │
//! │    Review pull requests …                           │
//! │  ─────────────────────────────────────────────────  │
//! │  ▸ PLUGIN                       38 skills · 5 plugins│
//! ├─────────────────────────────────────────────────────┤
//! │  ▤ 39 skills available                              │
//! └─────────────────────────────────────────────────────┘
//! ```
//!
//! All static text comes from `surface::strings::SKILLS_*`; pixel +
//! colour values from `crate::ui::theme::SKILL_*`.

use crate::ui::theme;
use crate::ui::theme::DarudaTheme;
use gpui::{AnyElement, Context, IntoElement, MouseButton, SharedString, div, prelude::*, px};

use super::super::super::layout::Dock;
use super::super::super::layout::RightDockSnapshot;
use crate::agent::skills::{Skill, SkillScope, SkillsSnapshot};
use crate::surface::strings;
use crate::ui::Sizable as _;
use crate::workspace::Workspace;
use crate::workspace::right_dock::section::DockSection;
use crate::workspace::right_dock::section_view::{
    ScopeSection, SectionFold, library_row, panel_footer,
};

/// Render the Skills tab body.
pub(in crate::workspace) fn render(snap: &RightDockSnapshot, cx: &mut Context<Dock>) -> AnyElement {
    let skills = &snap.skills;
    let workspace = snap.workspace.clone();
    let t = theme::current(cx).clone();

    // Plugin install / uninstall has moved to Settings → Plugin; the
    // panel only surfaces *installed* plugins now. Filter early so
    // empty-state copy and grouping both see a coherent slice.
    let installed_plugin_skills: Vec<Skill> = skills
        .plugin
        .iter()
        .filter(|s| is_installed(s))
        .cloned()
        .collect();

    // Apply the search filter to each scope before it is rendered.
    // A skill matches when its name or its
    // frontmatter description contains the query (case-insensitive).
    let query = snap.skill_search_query.trim().to_ascii_lowercase();
    let project = filter_skills(&skills.project, &query);
    let personal = filter_skills(&skills.personal, &query);
    let plugin = filter_skills(&installed_plugin_skills, &query);

    let any_match = !project.is_empty() || !personal.is_empty() || !plugin.is_empty();
    let searching = !query.is_empty();

    let mut col = crate::workspace::right_dock::right_panel_body()
        .child(header_row(workspace.clone(), cx))
        .child(search_row(snap, cx));

    if searching && !any_match {
        col = col.child(search_empty_hint(snap.skill_search_query.clone(), &t));
        return col.into_any_element();
    }

    let ctx = ScopeCtx {
        state: skills,
        workspace,
        plugin_expanded: &snap.skill_plugin_expanded,
        searching,
    };
    // While searching, render only the scopes that actually have a
    // match. Empty scopes get hidden entirely (no "No project skills"
    // hint), since that hint is misleading mid-search — the cause is
    // the active query, not an empty disk state. Search pins every
    // section and plugin group it shows open, so no match is folded away.
    let scopes = [
        (
            DockSection::SkillsProject,
            strings::skills_project(),
            SkillScope::Project,
            &project,
            skills.project_root.is_some(),
        ),
        (
            DockSection::SkillsPersonal,
            strings::skills_personal(),
            SkillScope::Personal,
            &personal,
            true,
        ),
        (
            DockSection::SkillsPlugins,
            strings::skills_plugin(),
            SkillScope::Plugin,
            &plugin,
            true,
        ),
    ];
    let mut divided = false;
    for (section, label, scope, list, enabled) in scopes {
        if searching && list.is_empty() {
            continue;
        }
        let fold = if searching {
            SectionFold::Fixed
        } else {
            SectionFold::toggleable(snap.sections.is_open(section))
        };
        let header = ScopeSection {
            section,
            label: label.into(),
            count: Some(scope_count(scope, list)),
            fold,
            divided,
        };
        let body = fold
            .is_open()
            .then(|| scope_body(scope, list, enabled, &ctx, &t, cx));
        col = col.child(header.render(body, &ctx.workspace, cx));
        divided = true;
    }
    col.into_any_element()
}

/// Every skill the panel can list, before the search filter.
pub(in crate::workspace) fn footer(snap: &RightDockSnapshot, cx: &gpui::App) -> AnyElement {
    let skills = &snap.skills;
    let installed = skills.plugin.iter().filter(|s| is_installed(s)).count();
    let total = skills.project.len() + skills.personal.len() + installed;
    panel_footer(
        crate::ui::icons::SKILL,
        strings::skills_footer_available(total),
        cx,
    )
}

/// Only installed plugins are listed; the footer counts the same set.
fn is_installed(skill: &Skill) -> bool {
    matches!(
        skill.plugin_availability,
        Some(crate::agent::skills::plugins::PluginAvailability::Installed)
    )
}

/// Substring filter on `name` + frontmatter `description`. Empty
/// query short-circuits to the original slice (cloned) so the caller
/// never branches on `query.is_empty()` itself.
fn filter_skills(skills: &[Skill], query_lower: &str) -> Vec<Skill> {
    if query_lower.is_empty() {
        return skills.to_vec();
    }
    skills
        .iter()
        .filter(|s| {
            let name = s.name.to_ascii_lowercase();
            if name.contains(query_lower) {
                return true;
            }
            if let Some(desc) = s.frontmatter.description.as_deref()
                && desc.to_ascii_lowercase().contains(query_lower)
            {
                return true;
            }
            false
        })
        .cloned()
        .collect()
}

/// Search input row. Wraps `RightDockSnapshot::skill_search_input` in a
/// relative container so the in-field `✕` button can sit absolutely on
/// the trailing edge. The icon only renders while the query is
/// non-empty — the row collapses back to a plain input at rest.
fn search_row(snap: &RightDockSnapshot, cx: &gpui::App) -> impl IntoElement {
    let has_query = !snap.skill_search_query.trim().is_empty();
    let workspace = snap.workspace.clone();
    div()
        .relative()
        .flex()
        .w_full()
        .child(crate::ui::input(&snap.skill_search_input, cx, ()))
        .when(has_query, |row| {
            row.child(
                crate::ui::button_icon("skill-search-clear", crate::ui::icons::CLOSE, cx)
                    .tooltip(strings::common_search_clear())
                    .absolute()
                    .right(px(theme::PAD_XS))
                    .top_0()
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                        // The mouse-down lands on the absolute overlay,
                        // not on the Input, so propagation stop is
                        // belt-and-braces — the input doesn't observe
                        // this event anyway.
                        cx.stop_propagation();
                        if let Some(ws) = workspace.upgrade() {
                            ws.update(cx, |ws, cx| ws.clear_skill_search(window, cx));
                        }
                    }),
            )
        })
}

/// Body shown when a non-empty search yields zero matches across every
/// scope. Text-only — the in-field `✕` already provides one-click
/// recovery, so a second affordance here would be redundant.
fn search_empty_hint(query: String, t: &DarudaTheme) -> impl IntoElement {
    let display_query = SharedString::from(format!("\"{}\"", query.trim()));
    div()
        .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
        .text_color(t.text_subtle)
        .child(SharedString::from(format!(
            "{}{}.",
            strings::skills_search_empty_prefix(),
            display_query
        )))
}

fn header_row(workspace: gpui::WeakEntity<Workspace>, cx: &gpui::App) -> impl IntoElement {
    crate::ui::SectionHeader::new(strings::right_panel_tab_skills())
        .prominent()
        .truncate_label(true)
        .actions(
            div()
                .flex()
                .flex_row()
                .gap(px(theme::SKILL_HEADER_GAP))
                .child(manage_plugins_button(cx))
                .child(new_skill_button(workspace, cx)),
        )
}

/// Right-hand `[Manage…]` button on the Skills tab header. Dispatches
/// `OpenSettings(BuiltinSection::Plugin)` so the user lands on the
/// install / uninstall page in the Settings window. The Skills tab
/// itself stays read-only — see Settings → Plugin for the CRUD UI.
fn manage_plugins_button(cx: &gpui::App) -> impl IntoElement {
    use crate::workspace::OpenSettings;
    crate::ui::button_icon(
        "plugin-manage-open-settings",
        crate::ui::icons::SETTINGS,
        cx,
    )
    .tooltip(strings::skills_manage_plugins_button())
    .on_click(|_, window, cx| {
        window.dispatch_action(
            Box::new(OpenSettings(daruda_config::BuiltinSection::Plugin)),
            cx,
        );
    })
}

fn new_skill_button(workspace: gpui::WeakEntity<Workspace>, cx: &gpui::App) -> impl IntoElement {
    crate::ui::button_icon("skills-new", crate::ui::icons::ADD, cx)
        .tooltip(strings::skills_new_button())
        .on_click(move |_, window, cx| {
            if let Some(ws) = workspace.upgrade() {
                ws.update(cx, |ws, cx| ws.open_create_skill(window, cx));
            }
        })
}

/// Inputs every scope body shares.
struct ScopeCtx<'a> {
    state: &'a SkillsSnapshot,
    workspace: gpui::WeakEntity<Workspace>,
    plugin_expanded: &'a std::collections::HashSet<String>,
    /// A search opens every plugin group it lists, and pins it open.
    searching: bool,
}

/// Plugin skills are spread across several plugins, so that scope's count
/// carries both numbers.
fn scope_count(scope: SkillScope, skills: &[Skill]) -> SharedString {
    if matches!(scope, SkillScope::Plugin) {
        strings::skills_count_chip_with_plugins(skills.len(), count_unique_plugins(skills)).into()
    } else {
        skills.len().to_string().into()
    }
}

fn scope_body(
    scope: SkillScope,
    skills: &[Skill],
    enabled: bool,
    ctx: &ScopeCtx<'_>,
    t: &DarudaTheme,
    cx: &gpui::App,
) -> AnyElement {
    if !enabled {
        // Project scope without a project root — explain why.
        return empty_hint(strings::skills_no_project_hint(), t);
    }
    if skills.is_empty() {
        // Empty at rest — show only the text hint. Inline action
        // buttons are intentionally absent: the panel header already
        // carries `[+ New skill]` and `[Manage…]`, and surfacing the
        // same action again as an inline chip muddies the empty
        // state. A search never lands here: the caller drops the scope.
        let msg = match scope {
            SkillScope::Project => strings::skills_empty_project(),
            SkillScope::Personal => strings::skills_empty_personal(),
            SkillScope::Plugin => strings::skills_empty_plugin(),
        };
        return empty_hint(msg, t);
    }
    if matches!(scope, SkillScope::Plugin) {
        // Plugin scope groups by owning plugin id under disclosure rows.
        return plugin_groups(skills, ctx, t, cx).into_any_element();
    }
    let mut col = div().flex().flex_col();
    for s in skills {
        let overrides =
            matches!(scope, SkillScope::Project) && ctx.state.project_overrides_personal(&s.name);
        col = col.child(skill_row(s, overrides, ctx.workspace.clone(), t, cx));
    }
    col.into_any_element()
}

/// Quiet one-line explanation for a scope with nothing to list.
fn empty_hint(msg: impl Into<SharedString>, t: &DarudaTheme) -> AnyElement {
    div()
        .pb(px(theme::DOCK_SECTION_HEADER_PAD_Y))
        .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
        .text_color(t.text_subtle)
        .child(msg.into())
        .into_any_element()
}

/// One plugin's worth of skills: a disclosure row over indented skill rows.
struct PluginGroup<'a> {
    /// Local plugin name without the `@<marketplace>` suffix —
    /// matches what the user types into Claude Code.
    plugin_local: String,
    /// Fully-qualified id (`<plugin>@<marketplace>`) — used as the
    /// expanded-set key.
    plugin_id: String,
    /// All skills sharing this plugin id, sorted by display name.
    skills: Vec<&'a Skill>,
}

/// Bucket plugin-scope skills by `plugin_id`. Groups (and skills
/// inside each group) come out in a stable lowercase-name order.
fn group_plugin_skills(skills: &[Skill]) -> Vec<PluginGroup<'_>> {
    use std::collections::BTreeMap;
    let mut buckets: BTreeMap<String, PluginGroup<'_>> = BTreeMap::new();
    for s in skills {
        let id = s.plugin_id.clone().unwrap_or_default();
        let local = id
            .split_once('@')
            .map(|(l, _)| l.to_string())
            .unwrap_or_else(|| id.clone());
        let entry = buckets.entry(id.clone()).or_insert_with(|| PluginGroup {
            plugin_local: local,
            plugin_id: id.clone(),
            skills: Vec::new(),
        });
        entry.skills.push(s);
    }
    let mut out: Vec<PluginGroup<'_>> = buckets.into_values().collect();
    for group in &mut out {
        group.skills.sort_by(|a, b| {
            a.name
                .to_ascii_lowercase()
                .cmp(&b.name.to_ascii_lowercase())
        });
    }
    out.sort_by(|a, b| {
        a.plugin_local
            .to_ascii_lowercase()
            .cmp(&b.plugin_local.to_ascii_lowercase())
    });
    out
}

/// One disclosure row per plugin over its skill rows. Rows only toggle;
/// each skill's name button (set in `skill_row`) still invokes it, so a
/// click inside an open group is unambiguous.
fn plugin_groups(
    skills: &[Skill],
    ctx: &ScopeCtx<'_>,
    t: &DarudaTheme,
    cx: &gpui::App,
) -> impl IntoElement {
    let mut col = div().flex().flex_col();
    for group in group_plugin_skills(skills) {
        let is_open = ctx.searching || ctx.plugin_expanded.contains(&group.plugin_id);
        let plugin_id = group.plugin_id.clone();
        let ws = ctx.workspace.clone();
        // Keyed by plugin id: search reorders groups, and a positional id
        // would hand one plugin's hover / press state to another.
        let row_id = SharedString::from(format!("skill-plugin-group-{}", group.plugin_id));
        let chevron_id = SharedString::from(format!("skill-plugin-chevron-{}", group.plugin_id));
        col = col.child(
            div()
                .id(row_id)
                .flex()
                .flex_row()
                .items_center()
                .gap(px(theme::LANE_LABEL_GAP))
                .py(px(theme::SKILL_PLUGIN_GROUP_PAD_Y))
                .rounded(px(theme::SKILL_ROW_RADIUS))
                .child(
                    crate::ui::disclosure(chevron_id, is_open)
                        .size(theme::DOCK_SECTION_CHEVRON_SIZE)
                        .color(t.text_muted),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_ellipsis()
                        .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
                        .text_color(t.text_body)
                        .child(SharedString::from(group.plugin_local.clone())),
                )
                .child(
                    div()
                        .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
                        .text_color(t.text_muted)
                        .child(SharedString::from(group.skills.len().to_string())),
                )
                .when(!ctx.searching, |row| {
                    row.cursor_pointer()
                        .hover(|d| d.bg(t.skill_row_hover_bg))
                        .on_click(move |_, _window, cx| {
                            if let Some(ws) = ws.upgrade() {
                                ws.update(cx, |ws, cx| {
                                    ws.toggle_skill_plugin_expanded(plugin_id.clone(), cx)
                                });
                            }
                        })
                }),
        );
        if is_open {
            for s in &group.skills {
                col = col.child(skill_row(s, false, ctx.workspace.clone(), t, cx));
            }
        }
    }
    col
}

fn skill_row(
    s: &Skill,
    overrides_personal: bool,
    workspace: gpui::WeakEntity<Workspace>,
    t: &DarudaTheme,
    cx: &gpui::App,
) -> AnyElement {
    use crate::ui::{ButtonVariants as _, button, button_delete_glyph, button_icon};

    let dir = s.dir.clone();
    let scope = s.scope;
    let meta_color = t.text_muted;
    let row_hover_bg = t.skill_row_hover_bg;
    let actions_bg = t.skill_row_hover_bg;

    // Plugin rows render under a per-plugin group that already
    // shows `<plugin>` in the header, so strip the namespace prefix
    // here and lean on indentation to communicate hierarchy.
    let display_name = if matches!(scope, SkillScope::Plugin) {
        s.name
            .split_once(':')
            .map(|(_plugin, name)| name.to_string())
            .unwrap_or_else(|| s.name.clone())
    } else {
        s.name.clone()
    };

    let description_full = s
        .frontmatter
        .description
        .clone()
        .or_else(|| Some(s.body_preview.clone()))
        .unwrap_or_default();
    let description_truncated =
        truncate_with_ellipsis(&description_full, SKILL_DESCRIPTION_MAX_CHARS);

    // Stable ids for the name button + description span so GPUI's
    // hover / tooltip plumbing can attach state slots. The skill name
    // is unique within its scope, so `scope-name` is collision-free
    // across the entire panel.
    let name_btn_id = SharedString::from(format!("skill-name-{}-{}", scope.slug(), s.name));
    let desc_id = SharedString::from(format!("skill-desc-{}-{}", scope.slug(), s.name));
    let row_id = SharedString::from(format!("skill-{}-{}", scope.slug(), s.name));

    // Invocation remains a button; the secondary line gets the full row width.
    let skill_for_invoke = s.clone();
    let workspace_for_invoke = workspace.clone();
    let name_button = button(name_btn_id, SharedString::from(display_name))
        .xsmall()
        .ghost()
        .p(px(0.))
        .max_w_full()
        .tooltip(s.name.clone())
        .on_click({
            let ws = workspace_for_invoke.clone();
            let sk = skill_for_invoke.clone();
            move |_: &gpui::ClickEvent, window, cx| {
                if let Some(ws) = ws.upgrade() {
                    let skill = sk.clone();
                    ws.update(cx, |ws: &mut Workspace, cx| {
                        ws.open_skill_invocation_modal(&skill, window, cx);
                    });
                }
            }
        });

    // Description span — single-line truncation, full text revealed
    // via a hover tooltip. The tooltip mounts only when the row is
    // hovered, so plumbing the full string is cheap.
    let description_span = (!description_truncated.is_empty()).then(|| {
        let full = description_full.clone();
        div()
            .id(desc_id)
            .w_full()
            .min_w_0()
            .text_ellipsis()
            .text_size(px(theme::FONT_SIZE_SM))
            .text_color(meta_color)
            .child(SharedString::from(description_truncated))
            .tooltip(crate::ui::tooltip::text(SharedString::from(full)))
    });

    // Actions — edit / delete for writable scopes, view for plugin scope.
    // Overlay actions preserve the text column's resting width.
    let workspace_for_actions = workspace.clone();
    let actions: AnyElement = if scope.is_writable() {
        let dir_edit = dir.clone();
        let dir_delete = dir.clone();
        let ws_edit = workspace_for_actions.clone();
        let ws_delete = workspace_for_actions.clone();
        div()
            .absolute()
            .right(px(theme::SKILL_ROW_PAD_X))
            .top_0()
            .bottom_0()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::GAP_SM))
            .bg(actions_bg)
            .pl(px(theme::SKILL_ROW_PAD_X))
            .invisible()
            .group_hover("skill-row", |s| s.visible())
            .child(
                button_icon(
                    SharedString::from(format!("skill-edit-{}-{}", scope.slug(), s.name)),
                    crate::ui::icons::EDIT,
                    cx,
                )
                .tooltip(strings::skills_button_edit())
                .debug_selector(|| "skill-edit".into())
                .on_click(move |_: &gpui::ClickEvent, window, cx| {
                    if let Some(ws) = ws_edit.upgrade() {
                        let dir = dir_edit.clone();
                        ws.update(cx, |ws, cx| ws.open_edit_skill(dir, window, cx));
                    }
                }),
            )
            .child(
                button_delete_glyph(
                    SharedString::from(format!("skill-delete-{}-{}", scope.slug(), s.name)),
                    cx,
                )
                .tooltip(strings::skills_button_delete())
                .debug_selector(|| "skill-delete".into())
                .on_click(move |_: &gpui::ClickEvent, window, cx| {
                    if let Some(ws) = ws_delete.upgrade() {
                        let dir = dir_delete.clone();
                        ws.update(cx, |ws, cx| {
                            ws.open_delete_skill_confirm(scope, dir, window, cx)
                        });
                    }
                }),
            )
            .into_any_element()
    } else {
        let dir_view = s.dir.clone();
        let ws_view = workspace_for_actions.clone();
        div()
            .absolute()
            .right(px(theme::SKILL_ROW_PAD_X))
            .top_0()
            .bottom_0()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::GAP_SM))
            .bg(actions_bg)
            .pl(px(theme::SKILL_ROW_PAD_X))
            .invisible()
            .group_hover("skill-row", |s| s.visible())
            .child(
                button_icon(
                    SharedString::from(format!("skill-view-{}-{}", scope.slug(), s.name)),
                    crate::ui::icons::VISIBILITY,
                    cx,
                )
                .tooltip(strings::skills_button_view())
                .debug_selector(|| "skill-view".into())
                .on_click(move |_: &gpui::ClickEvent, window, cx| {
                    if let Some(ws) = ws_view.upgrade() {
                        let dir = dir_view.clone();
                        ws.update(cx, |ws: &mut Workspace, cx| {
                            ws.open_skill_in_file_viewer(dir, window, cx)
                        });
                    }
                }),
            )
            .into_any_element()
    };

    // Plugin rows live under a per-plugin group — indent them so
    // the hierarchy reads at a glance. Project / Personal rows stay
    // flush with the section header.
    let row_pad_left = if matches!(scope, SkillScope::Plugin) {
        px(theme::SKILL_PLUGIN_INDENT)
    } else {
        px(theme::SKILL_ROW_PAD_X)
    };

    let name = div()
        .flex()
        .items_center()
        .w_full()
        .min_w_0()
        .gap(px(theme::SKILL_HEADER_GAP))
        .child(name_button)
        .when(overrides_personal, |c| {
            c.child(neutral_chip(strings::skills_overrides_personal(), t))
        });
    library_row(
        crate::ui::icons::SKILL,
        name,
        description_span.map(IntoElement::into_any_element),
        cx,
    )
    .id(row_id)
    .group("skill-row")
    .relative()
    .overflow_hidden()
    .pl(row_pad_left)
    .pr(px(theme::SKILL_ROW_PAD_X))
    .rounded(px(theme::SKILL_ROW_RADIUS))
    .hover(move |s| s.bg(row_hover_bg))
    .child(actions)
    .into_any_element()
}

/// Character budget for the skill row's description line. Chosen to
/// match the right-panel's default width at the standard font size
/// without leaving room for the cursor / scrollbar gutter — tune if
/// `RIGHT_PANEL_BODY_FONT_SIZE` changes meaningfully.
const SKILL_DESCRIPTION_MAX_CHARS: usize = 80;

/// Truncate `s` to at most `max_chars` Unicode characters and append
/// `…` when truncation actually happens. `s.chars().count()` is O(n)
/// but the inputs here are short (panel-row descriptions), so the
/// cost is negligible compared to the layout pass.
fn truncate_with_ellipsis(s: &str, max_chars: usize) -> String {
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn neutral_chip(label: impl Into<SharedString>, t: &DarudaTheme) -> impl IntoElement {
    div()
        .flex_none()
        .px(px(theme::SKILL_BADGE_PAD_X))
        .py(px(theme::SKILL_BADGE_PAD_Y))
        .rounded(px(theme::SKILL_BADGE_RADIUS))
        .bg(t.skill_aux_chip_bg)
        .text_size(px(theme::SKILL_BADGE_FONT_SIZE))
        .text_color(t.text_body)
        .child(label.into())
}

/// Count of distinct `plugin_id` values across `skills`. Used by the
/// Plugin scope header chip to surface `"N skills · M plugins"`.
fn count_unique_plugins(skills: &[Skill]) -> usize {
    let mut ids = std::collections::BTreeSet::new();
    for s in skills {
        if let Some(id) = &s.plugin_id {
            ids.insert(id.clone());
        }
    }
    ids.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_actions(cx: &mut gpui::TestAppContext, scope: SkillScope, selectors: &[&'static str]) {
        let plugin = matches!(scope, SkillScope::Plugin);
        let skill = Skill {
            name: "sample".into(),
            dir: std::path::PathBuf::from("sample-skill"),
            scope,
            frontmatter: Default::default(),
            body_preview: "A sample skill".into(),
            aux_file_count: 0,
            modified_at: std::time::SystemTime::UNIX_EPOCH,
            plugin_id: plugin.then(|| "sample-plugin".into()),
            plugin_availability: plugin
                .then_some(crate::agent::skills::plugins::PluginAvailability::Installed),
        };
        crate::workspace::right_dock::row_tests::assert_hover_targets_fit(
            cx,
            selectors,
            move |workspace, cx| skill_row(&skill, false, workspace, theme::current(cx), cx),
        );
    }

    #[gpui::test]
    fn writable_hover_actions_fit_the_skill_row(cx: &mut gpui::TestAppContext) {
        check_actions(cx, SkillScope::Personal, &["skill-edit", "skill-delete"]);
    }

    #[gpui::test]
    fn view_action_fits_the_plugin_row(cx: &mut gpui::TestAppContext) {
        check_actions(cx, SkillScope::Plugin, &["skill-view"]);
    }
}
