//! Skills tab body — renders the project + personal skill scopes
//! pulled from `RightSidebarSnapshot::skills` (a snapshot of `Workspace::skills`).
//!
//! Layout:
//! ```text
//! ┌─ Skills ─────────────────────────── [+ New skill] ┐
//! │  PROJECT  (.claude/skills · daruda)               │
//! │  ┌─ pr-review ───────────────────────────────┐    │
//! │  │  Review pull requests …      🤖 user+model│    │
//! │  │  allowed-tools: Read · Bash · …  📎 2     │    │
//! │  └────────────────────────────────────────────┘   │
//! │  PERSONAL  (~/.claude/skills)                     │
//! │  …                                                │
//! └───────────────────────────────────────────────────┘
//! ```
//!
//! All static text comes from `surface::strings::SKILLS_*`; pixel +
//! colour values from `crate::ui::theme::SKILL_*`.

use crate::ui::theme;
use crate::ui::theme::DarudaTheme;
use gpui::{AnyElement, Context, IntoElement, MouseButton, SharedString, div, prelude::*, px};

use super::super::super::layout::Dock;
use super::super::super::layout::RightSidebarSnapshot;
use crate::agent::skills::{Skill, SkillScope, SkillsSnapshot};
use crate::surface::strings;
use crate::ui::Divider;
use crate::workspace::Workspace;

/// Render the Skills tab body.
pub(in crate::workspace) fn render(
    snap: &RightSidebarSnapshot,
    cx: &mut Context<Dock>,
) -> AnyElement {
    let skills = &snap.skills;
    let workspace = snap.workspace.clone();
    let t = theme::current(cx).clone();

    // Plugin install / uninstall has moved to Settings → Plugin; the
    // panel only surfaces *installed* plugins now. Filter early so
    // empty-state copy and grouping both see a coherent slice.
    let installed_plugin_skills: Vec<Skill> = skills
        .plugin
        .iter()
        .filter(|s| {
            matches!(
                s.plugin_availability,
                Some(crate::agent::skills::plugins::PluginAvailability::Installed)
            )
        })
        .cloned()
        .collect();

    // Apply the search filter to each scope before passing to
    // `scope_section`. A skill matches when its name or its
    // frontmatter description contains the query (case-insensitive).
    let query = snap.skill_search_query.trim().to_ascii_lowercase();
    let project = filter_skills(&skills.project, &query);
    let personal = filter_skills(&skills.personal, &query);
    let plugin = filter_skills(&installed_plugin_skills, &query);

    let any_match = !project.is_empty() || !personal.is_empty() || !plugin.is_empty();
    let searching = !query.is_empty();

    let mut col = div()
        .flex()
        .flex_col()
        .px(px(theme::RIGHT_PANEL_PAD_X))
        .py(px(theme::RIGHT_PANEL_PAD_Y))
        .gap(px(theme::SKILL_SECTION_GAP))
        .child(header_row(workspace.clone(), &t))
        .child(search_row(snap, cx, &t));

    if searching && !any_match {
        col = col.child(search_empty_hint(snap.skill_search_query.clone(), &t));
        return col.into_any_element();
    }

    let plugin_expanded = &snap.skill_plugin_expanded;

    // While searching, render only the scopes that actually have a
    // match. Empty scopes get hidden entirely (no "No project skills"
    // hint), since that hint is misleading mid-search — the cause is
    // the active query, not an empty disk state.
    let project_section = scope_section(
        strings::SKILLS_PROJECT,
        SkillScope::Project,
        &project,
        skills,
        workspace.clone(),
        skills.project_root.is_some(),
        plugin_expanded,
        searching,
        &t,
    );
    let personal_section = scope_section(
        strings::SKILLS_PERSONAL,
        SkillScope::Personal,
        &personal,
        skills,
        workspace.clone(),
        true,
        plugin_expanded,
        searching,
        &t,
    );
    let plugin_section = scope_section(
        strings::SKILLS_PLUGIN,
        SkillScope::Plugin,
        &plugin,
        skills,
        workspace,
        true,
        plugin_expanded,
        searching,
        &t,
    );

    col.when_some(project_section, |c, sec| c.child(sec))
        .when(!searching, |c| c.child(Divider::horizontal()))
        .when_some(personal_section, |c, sec| c.child(sec))
        .when(!searching, |c| c.child(Divider::horizontal()))
        .when_some(plugin_section, |c, sec| c.child(sec))
        .into_any_element()
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

/// Search input row. Wraps `RightSidebarSnapshot::skill_search_input` in a
/// relative container so the in-field `✕` button can sit absolutely on
/// the trailing edge. The icon only renders while the query is
/// non-empty — the row collapses back to a plain input at rest.
fn search_row(snap: &RightSidebarSnapshot, cx: &gpui::App, t: &DarudaTheme) -> impl IntoElement {
    let has_query = !snap.skill_search_query.trim().is_empty();
    let chip_text = t.skill_aux_chip_text;
    let chip_hover_text = t.skill_name_text;
    let workspace = snap.workspace.clone();
    div()
        .relative()
        .flex()
        .w_full()
        .child(crate::ui::input(&snap.skill_search_input, cx, ()))
        .when(has_query, |row| {
            row.child(
                div()
                    .id("skill-search-clear")
                    .absolute()
                    .right(px(theme::SKILL_ROW_PAD_X))
                    .top_0()
                    .bottom_0()
                    .flex()
                    .items_center()
                    .px(px(theme::SKILL_BADGE_PAD_X))
                    .text_size(px(theme::SKILL_BADGE_FONT_SIZE))
                    .text_color(chip_text)
                    .cursor_pointer()
                    .hover(move |s| s.text_color(chip_hover_text))
                    .child(strings::SKILLS_SEARCH_CLEAR_ICON)
                    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                        // The mouse-down lands on the absolute overlay,
                        // not on the Input, so propagation stop is
                        // belt-and-braces — the input doesn't observe
                        // this event anyway.
                        cx.stop_propagation();
                        if let Some(ws) = workspace.upgrade() {
                            ws.update(cx, |ws: &mut Workspace, cx| {
                                ws.skill_search_input.update(cx, |input, cx_state| {
                                    input.set_value("".to_string(), window, cx_state);
                                });
                                cx.notify();
                            });
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
        .text_color(t.skill_empty_text)
        .child(SharedString::from(format!(
            "{}{}.",
            strings::SKILLS_SEARCH_EMPTY_PREFIX,
            display_query
        )))
}

fn header_row(workspace: gpui::WeakEntity<Workspace>, t: &DarudaTheme) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(theme::SKILL_HEADER_GAP))
        .child(
            div()
                .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
                .text_color(t.skill_section_header_text)
                .child(strings::RIGHT_PANEL_TAB_SKILLS),
        )
        .child(
            div()
                .flex()
                .flex_row()
                .gap(px(theme::SKILL_HEADER_GAP))
                .child(manage_plugins_button(t))
                .child(new_skill_button(workspace, t)),
        )
}

/// Right-hand `[Manage…]` button on the Skills tab header. Dispatches
/// `OpenSettings(BuiltinSection::Plugin)` so the user lands on the
/// install / uninstall page in the Settings window. The Skills tab
/// itself stays read-only — see Settings → Plugin for the CRUD UI.
fn manage_plugins_button(t: &DarudaTheme) -> impl IntoElement {
    use crate::workspace::OpenSettings;
    let chip_bg = t.skill_aux_chip_bg;
    let chip_text = t.skill_aux_chip_text;
    let hover_bg = t.skill_row_hover_bg;
    div()
        .flex()
        .flex_none()
        .id("plugin-manage-open-settings")
        .px(px(theme::SKILL_BADGE_PAD_X))
        .py(px(theme::SKILL_BADGE_PAD_Y))
        .rounded(px(theme::SKILL_BADGE_RADIUS))
        .bg(chip_bg)
        .text_size(px(theme::SKILL_BADGE_FONT_SIZE))
        .text_color(chip_text)
        .cursor_pointer()
        .hover(move |s| s.bg(hover_bg))
        .child(strings::SKILLS_MANAGE_PLUGINS_BUTTON)
        .on_mouse_down(MouseButton::Left, |_, window, cx| {
            window.dispatch_action(
                Box::new(OpenSettings(daruda_config::BuiltinSection::Plugin)),
                cx,
            );
        })
}

fn new_skill_button(workspace: gpui::WeakEntity<Workspace>, t: &DarudaTheme) -> impl IntoElement {
    let chip_bg = t.skill_badge_user_only_bg;
    let chip_text = t.skill_badge_user_only_text;
    let hover_bg = t.skill_row_hover_bg;
    div()
        .flex()
        .flex_none()
        .px(px(theme::SKILL_BADGE_PAD_X))
        .py(px(theme::SKILL_BADGE_PAD_Y))
        .rounded(px(theme::SKILL_BADGE_RADIUS))
        .bg(chip_bg)
        .text_size(px(theme::SKILL_BADGE_FONT_SIZE))
        .text_color(chip_text)
        .cursor_pointer()
        .hover(move |s| s.bg(hover_bg))
        .child(strings::SKILLS_NEW_BUTTON)
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            if let Some(ws) = workspace.upgrade() {
                ws.update(cx, |ws: &mut Workspace, cx| {
                    ws.open_create_skill_modal(None, window, cx)
                });
            }
        })
}

#[allow(clippy::too_many_arguments)]
fn scope_section(
    label: &'static str,
    scope: SkillScope,
    skills: &[Skill],
    state: &SkillsSnapshot,
    workspace: gpui::WeakEntity<Workspace>,
    enabled: bool,
    plugin_expanded: &std::collections::HashSet<String>,
    searching: bool,
    t: &DarudaTheme,
) -> Option<AnyElement> {
    // While searching, an empty scope means "nothing matches the
    // query in this scope". Hide the section entirely — the default
    // empty hint ("No project skills") would read as a misleading
    // absence-of-disk message in that context.
    if searching && skills.is_empty() {
        return None;
    }

    // Count chip on the section header — for Plugin scope this shows
    // both total skills and how many distinct plugins they're spread
    // across so the user gets a quick sense of catalogue size.
    let count_text: SharedString = if matches!(scope, SkillScope::Plugin) {
        let plugins = count_unique_plugins(skills);
        SharedString::from(format!("{} skills · {} plugins", skills.len(), plugins))
    } else {
        SharedString::from(format!("{}", skills.len()))
    };

    let mut col = div().flex().flex_col().gap(px(theme::SKILL_ROW_GAP)).child(
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::SKILL_HEADER_GAP))
            .text_size(px(theme::RIGHT_PANEL_LABEL_FONT_SIZE))
            .text_color(t.skill_section_header_text)
            .child(SharedString::from(label.to_string()))
            .child(neutral_chip(count_text, t)),
    );

    if !enabled {
        // Project scope without a project root — explain why.
        return Some(
            col.child(
                div()
                    .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
                    .text_color(t.skill_empty_text)
                    .child(strings::SKILLS_NO_PROJECT_HINT),
            )
            .into_any_element(),
        );
    }

    if skills.is_empty() {
        // Empty at rest — show only the text hint. Inline action
        // buttons are intentionally absent: the panel header already
        // carries `[+ New skill]` and `[Manage…]`, and surfacing the
        // same action again as an inline chip muddies the empty
        // state.
        let msg = match scope {
            SkillScope::Project => strings::SKILLS_EMPTY_PROJECT,
            SkillScope::Personal => strings::SKILLS_EMPTY_PERSONAL,
            SkillScope::Plugin => strings::SKILLS_EMPTY_PLUGIN,
        };
        return Some(
            col.child(
                div()
                    .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
                    .text_color(t.skill_empty_text)
                    .child(msg),
            )
            .into_any_element(),
        );
    }

    if matches!(scope, SkillScope::Plugin) {
        // Plugin scope groups by owning plugin id, rendered as an
        // Accordion so each plugin can be expanded / collapsed
        // independently. Header clicks no longer trigger invocation —
        // they only toggle the section open state. Individual skill
        // rows inside an open section keep their own click-to-invoke.
        col = col.child(plugin_accordion(skills, &workspace, plugin_expanded, t));
    } else {
        for s in skills {
            let overrides =
                matches!(scope, SkillScope::Project) && state.project_overrides_personal(&s.name);
            col = col.child(skill_row(s, overrides, workspace.clone(), t));
        }
    }
    Some(col.into_any_element())
}

/// One plugin's worth of skills, ready to be rendered as an
/// `AccordionItem` (title = plugin name + count chip; children =
/// indented skill rows).
struct PluginGroup<'a> {
    /// Local plugin name without the `@<marketplace>` suffix —
    /// matches what the user types into Claude Code.
    plugin_local: String,
    /// Fully-qualified id (`<plugin>@<marketplace>`) — used as the
    /// expanded-set key and as the accordion item id.
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

/// Build the per-plugin Accordion. Each `AccordionItem` corresponds
/// to one plugin id and houses its skill rows as children. Header
/// click toggles open / closed; the per-skill row's own
/// `on_mouse_down` (set in `skill_row`) still triggers invocation,
/// so clicking inside an open section is unambiguous.
fn plugin_accordion(
    skills: &[Skill],
    workspace: &gpui::WeakEntity<Workspace>,
    plugin_expanded: &std::collections::HashSet<String>,
    t: &DarudaTheme,
) -> impl IntoElement {
    use crate::ui::accordion::{AccordionItem, accordion};

    // Build the grouping once. The closure passed to
    // `Accordion::on_toggle_click` needs the plugin-id order to map
    // back from indices, so capture that vector alongside the items.
    let groups = group_plugin_skills(skills);
    let plugin_ids: Vec<String> = groups.iter().map(|g| g.plugin_id.clone()).collect();

    let mut acc = accordion("skills-plugin-groups")
        .multiple(true)
        .bordered(false);
    for group in &groups {
        let plugin_local = group.plugin_local.clone();
        let count = group.skills.len();
        let is_open = plugin_expanded.contains(&group.plugin_id);

        // Build the indented skill rows up front so the item can take
        // them as children (`ParentElement`). `skill_row` returns an
        // `AnyElement`, which matches Accordion's expectation.
        let mut item = AccordionItem::new()
            .title(plugin_title(&plugin_local, count, t))
            .open(is_open)
            .bordered(false);
        for s in &group.skills {
            item.extend(std::iter::once(
                skill_row(s, false, workspace.clone(), t).into_any_element(),
            ));
        }
        acc = acc.item(|_| item);
    }

    // The accordion-level callback fires whenever any item is
    // toggled. The argument is the full list of currently-open
    // indices, which we map back into the `plugin_id` set and hand
    // to `Workspace::set_skill_plugin_expanded` in one go.
    let ws_for_toggle = workspace.clone();
    acc.on_toggle_click(move |open_indices, _window, cx| {
        let Some(ws) = ws_for_toggle.upgrade() else {
            return;
        };
        let new_set: std::collections::HashSet<String> = open_indices
            .iter()
            .filter_map(|&ix| plugin_ids.get(ix).cloned())
            .collect();
        ws.update(cx, |ws: &mut Workspace, cx| {
            ws.set_skill_plugin_expanded(new_set, cx);
        });
    })
}

/// Title element rendered inside the accordion header for one plugin
/// — plugin name on the left, skill-count chip on the right.
fn plugin_title(plugin_local: &str, count: usize, t: &DarudaTheme) -> gpui::AnyElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::SKILL_HEADER_GAP))
        .child(
            div()
                .flex_1()
                .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
                .text_color(t.skill_name_text)
                .child(SharedString::from(plugin_local.to_string())),
        )
        .child(neutral_chip(SharedString::from(format!("{count}")), t))
        .into_any_element()
}

fn skill_row(
    s: &Skill,
    overrides_personal: bool,
    workspace: gpui::WeakEntity<Workspace>,
    t: &DarudaTheme,
) -> AnyElement {
    use crate::ui::{button, button_bare};

    let dir = s.dir.clone();
    let scope = s.scope;
    let meta_color = t.skill_meta_text;
    let row_hover_bg = t.skill_row_hover_bg;
    let actions_bg = t.skill_row_hover_bg;

    // Plugin rows render under a per-plugin accordion that already
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

    // Name button — primary affordance for "invoke this skill". Made
    // an actual button so click affordance is obvious; secondary
    // variant keeps the panel quiet.
    let skill_for_invoke = s.clone();
    let workspace_for_invoke = workspace.clone();
    // Outline variant keeps the button visually distinct from the
    // row's hover background — at default sizing the secondary fill
    // sat right on top of `SKILL_ROW_HOVER_BG` and read as flat. The
    // outline border draws a clean edge against whatever surface the
    // row is sitting on (hovered or not).
    let name_button = button(name_btn_id, SharedString::from(display_name))
        .outline()
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
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .whitespace_nowrap()
            .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
            .text_color(meta_color)
            .child(SharedString::from(description_truncated))
            .tooltip(crate::ui::tooltip::text(SharedString::from(full)))
    });

    // Actions — Edit / × for writable scopes, View for plugin scope.
    // Absolute-positioned overlay on the right so the row stays a
    // single visual line; on hover the actions slide in over the
    // tail of the description.
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
            .gap(px(theme::SKILL_HEADER_GAP))
            .bg(actions_bg)
            .pl(px(theme::SKILL_ROW_PAD_X))
            .invisible()
            .group_hover("skill-row", |s| s.visible())
            .child(
                button(
                    SharedString::from(format!("skill-edit-{}-{}", scope.slug(), s.name)),
                    strings::SKILLS_BUTTON_EDIT,
                )
                .outline()
                .on_click(move |_: &gpui::ClickEvent, window, cx| {
                    if let Some(ws) = ws_edit.upgrade() {
                        let dir = dir_edit.clone();
                        ws.update(cx, |ws: &mut Workspace, cx| {
                            ws.open_edit_skill_modal(dir, window, cx)
                        });
                    }
                }),
            )
            .child(
                button_bare(SharedString::from(format!(
                    "skill-delete-{}-{}",
                    scope.slug(),
                    s.name
                )))
                .label(strings::SKILLS_BUTTON_DELETE_ICON)
                .outline()
                .on_click(move |_: &gpui::ClickEvent, window, cx| {
                    if let Some(ws) = ws_delete.upgrade() {
                        let dir = dir_delete.clone();
                        ws.update(cx, |ws: &mut Workspace, cx| {
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
            .gap(px(theme::SKILL_HEADER_GAP))
            .bg(actions_bg)
            .pl(px(theme::SKILL_ROW_PAD_X))
            .invisible()
            .group_hover("skill-row", |s| s.visible())
            .child(
                button(
                    SharedString::from(format!("skill-view-{}-{}", scope.slug(), s.name)),
                    strings::SKILLS_BUTTON_VIEW,
                )
                .outline()
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

    // Plugin rows live under a per-plugin accordion — indent them so
    // the hierarchy reads at a glance. Project / Personal rows stay
    // flush with the section header.
    let row_pad_left = if matches!(scope, SkillScope::Plugin) {
        px(theme::SKILL_PLUGIN_INDENT)
    } else {
        px(theme::SKILL_ROW_PAD_X)
    };

    div()
        .id(row_id)
        .group("skill-row")
        .relative()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::SKILL_HEADER_GAP))
        .pl(row_pad_left)
        .pr(px(theme::SKILL_ROW_PAD_X))
        // Plugin-scope rows sit inside an accordion section, so trim
        // their vertical padding for a denser list look. Project /
        // Personal rows keep the standard pad since they're top-level.
        .py(if matches!(scope, SkillScope::Plugin) {
            px(theme::SKILL_PLUGIN_ROW_PAD_Y)
        } else {
            px(theme::SKILL_ROW_PAD_Y)
        })
        .rounded(px(theme::SKILL_ROW_RADIUS))
        .hover(move |s| s.bg(row_hover_bg))
        .child(name_button)
        .when(overrides_personal, |c| {
            c.child(neutral_chip(strings::SKILLS_OVERRIDES_PERSONAL, t))
        })
        .when_some(description_span, |c, span| c.child(span))
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
        .text_color(t.skill_aux_chip_text)
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
