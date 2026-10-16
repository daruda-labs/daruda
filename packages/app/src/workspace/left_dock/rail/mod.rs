//! Collapsed navigation with one group fold level and typed worktree/session choices.

use crate::ui::Sizable as _;
mod ops;
mod projects;

use projects::project;

use crate::{
    surface::strings as s,
    ui::{self, theme},
    workspace::{
        layout::{Dock, LeftDockSnapshot},
        pages::Page,
    },
};
use daruda_store::project::LeftDockView;
use gpui::{AnyElement, Context, IntoElement, Window, div, prelude::*, px};

pub(super) fn project_initial(name: &str) -> String {
    use unicode_segmentation::UnicodeSegmentation as _;
    name.trim()
        .graphemes(true)
        .next()
        .unwrap_or("?")
        .to_uppercase()
}

pub(in crate::workspace) fn render(
    snap: &LeftDockSnapshot,
    _window: &mut Window,
    cx: &mut Context<Dock>,
) -> AnyElement {
    let t = theme::current(cx).clone();
    let mut nav = div()
        .flex()
        .flex_col()
        .flex_none()
        .items_center()
        .py(px(theme::PAD_XS));
    for page in [Page::Tasks, Page::Flows] {
        let ws = snap.workspace.clone();
        nav = nav.child(
            ui::button_icon_sized(page.icon(), page.icon(), snap.metrics, cx)
                .tab_stop(true)
                .tooltip(page.label())
                .on_click(move |_, window, cx| {
                    if let Some(ws) = ws.upgrade() {
                        ws.update(cx, |ws, cx| ws.open_page(page, window, cx));
                    }
                }),
        );
    }
    for (view, icon, label) in [
        (
            LeftDockView::Lanes,
            ui::icons::FOLDER,
            s::dock::sidebar_tab_worktrees(),
        ),
        (
            LeftDockView::GitChanges,
            ui::icons::DIFFERENCE,
            s::dock::sidebar_tab_git(),
        ),
        (
            LeftDockView::Files,
            ui::icons::SKILL,
            s::dock::sidebar_tab_files(),
        ),
    ] {
        let ws = snap.workspace.clone();
        nav = nav.child(
            ui::button_icon_sized(icon, icon, snap.metrics, cx)
                .tab_stop(true)
                .tooltip(label)
                .on_click(move |_, window, cx| {
                    if let Some(ws) = ws.upgrade() {
                        ws.update(cx, |ws, cx| ws.reveal_left_dock_view(view, window, cx));
                    }
                }),
        );
    }
    let mut roots: Vec<_> = snap
        .groups
        .iter()
        .map(|g| (g.tab_order, Some(g.id), None))
        .collect();
    roots.extend(
        snap.projects
            .iter()
            .filter(|p| p.group_id.is_none())
            .map(|p| (p.tab_order, None, Some(p))),
    );
    roots.sort_by_key(|r| r.0);
    let mut tree = div()
        .id("rail-projects")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll();
    for (_, group, standalone) in roots {
        if let Some(project_snap) = standalone {
            tree = tree.child(project(project_snap, snap, cx));
        }
        if let Some(id) = group {
            let g = snap.groups.iter().find(|g| g.id == id).unwrap();
            let ws = snap.workspace.clone();
            let key_ws = ws.clone();
            let icon = if g.is_collapsed {
                ui::icons::CHEVRON_RIGHT
            } else {
                ui::icons::EXPAND_MORE
            };
            tree = tree.child(
                div()
                    .id(("rail-group", id as usize))
                    .debug_selector(move || format!("rail-group-{id}"))
                    .focusable()
                    .tab_stop(true)
                    .cursor_pointer()
                    .mt(px(theme::GAP_LG))
                    .h(px(snap.metrics.target(theme::CONTROL_TARGET_SIZE)))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(theme::GAP_XS))
                    .text_size(px(snap.metrics.text(theme::FONT_SIZE_XS)))
                    .text_color(t.text_muted)
                    .hover(|el| el.bg(t.lane_row_hover_bg))
                    .child(
                        div()
                            .min_w_0()
                            .text_ellipsis()
                            .child(group_abbreviation(&g.name)),
                    )
                    .child(
                        ui::icons::icon(icon)
                            .with_size(px(snap.metrics.icon(theme::CONTROL_ICON_SIZE))),
                    )
                    .tooltip(ui::tooltip::text(g.name.clone()))
                    .on_key_down(move |event, window, cx| {
                        if let Some(ws) = key_ws.upgrade() {
                            ws.update(cx, |ws, cx| {
                                ws.on_rail_key(ops::RailItem::Group(id), event, window, cx)
                            });
                        }
                    })
                    .on_click(move |_, _, cx| {
                        if let Some(ws) = ws.upgrade() {
                            ws.update(cx, |ws, cx| ws.toggle_group_collapse(id, cx));
                        }
                    }),
            );
            if !g.is_collapsed {
                for p in snap.projects.iter().filter(|p| p.group_id == Some(id)) {
                    tree = tree.child(project(p, snap, cx));
                }
            }
        }
    }
    div()
        .h_full()
        .w_full()
        .flex()
        .flex_col()
        .bg(t.dock_bg)
        .border_r_1()
        .border_color(t.border)
        .child(nav)
        .child(tree)
        .child(
            div()
                .flex_none()
                .flex()
                .justify_center()
                .p(px(theme::PAD_XS))
                .border_t_1()
                .border_color(t.border)
                .child(
                    ui::button_icon_sized("rail-settings", ui::icons::SETTINGS, snap.metrics, cx)
                        .tab_stop(true)
                        .tooltip(s::dock::settings())
                        .on_click(|_, window, cx| {
                            window.dispatch_action(
                                Box::new(crate::workspace::OpenSettings(
                                    daruda_config::BuiltinSection::default(),
                                )),
                                cx,
                            )
                        }),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests;

pub(super) fn lane_indicator(
    status: Option<daruda_agent::SessionStatus>,
    metrics: theme::UiMetrics,
    cx: &gpui::App,
) -> AnyElement {
    if status.is_some() {
        return super::projects::agent_badges::agent_status_cell(
            status,
            metrics,
            metrics.icon(theme::STATUS_INDICATOR_CELL_WIDTH),
            cx,
        );
    }
    div()
        .flex_none()
        .w(px(metrics.icon(theme::STATUS_INDICATOR_CELL_WIDTH)))
        .flex()
        .items_center()
        .justify_center()
        .text_color(theme::current(cx).text_muted)
        .child(
            ui::icons::icon(ui::icons::RADIO_OFF)
                .with_size(px(metrics.icon(theme::CONTROL_ICON_SIZE))),
        )
        .into_any_element()
}

fn group_abbreviation(name: &str) -> String {
    use unicode_segmentation::UnicodeSegmentation as _;
    let words: Vec<_> = name.split_whitespace().collect();
    let label = if words.len() > 1 {
        words
            .iter()
            .filter_map(|w| w.graphemes(true).next())
            .take(3)
            .collect::<String>()
    } else {
        name.trim()
            .graphemes(true)
            .take(if name.is_ascii() { 3 } else { 2 })
            .collect::<String>()
    };
    if label.is_empty() {
        return "?".into();
    }
    let upper = label.to_uppercase();
    upper
        .graphemes(true)
        .take(if upper.is_ascii() { 3 } else { 2 })
        .collect()
}
