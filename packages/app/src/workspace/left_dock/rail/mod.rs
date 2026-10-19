//! Collapsed navigation with one group fold level and typed worktree/session choices.

use crate::ui::Sizable as _;
mod alias_modal;
mod groups;
mod lanes;
mod ops;
mod panes;
mod preferences;
mod projects;

pub(in crate::workspace) use panes::PaneSnapshot;

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

pub(in crate::workspace::left_dock) fn project_badge_label(
    name: &str,
    alias: Option<&str>,
) -> String {
    alias
        .map(str::trim)
        .filter(|alias| !alias.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| project_initial(name))
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
    for (index, (_, group, standalone)) in roots.into_iter().enumerate() {
        if let Some(project_snap) = standalone {
            tree = tree.child(
                div()
                    .when(index > 0, |element| element.mt(px(theme::GAP_LG)))
                    .child(project(project_snap, snap, cx)),
            );
        }
        if let Some(id) = group {
            let g = snap.groups.iter().find(|g| g.id == id).unwrap();
            tree = tree.child(groups::header(g, snap, cx));
            if !g.is_collapsed {
                tree = tree.child(
                    div().flex().flex_col().gap(px(theme::GAP_LG)).children(
                        snap.projects
                            .iter()
                            .filter(|project| project.group_id == Some(id))
                            .map(|project_snap| project(project_snap, snap, cx)),
                    ),
                );
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
    outcome: Option<daruda_agent::AgentOutcome>,
    metrics: theme::UiMetrics,
    cx: &gpui::App,
) -> AnyElement {
    if status.is_some() || outcome.is_some() {
        return super::projects::agent_badges::agent_status_cell(
            status,
            outcome,
            metrics,
            metrics.icon(theme::STATUS_INDICATOR_CELL_WIDTH),
            cx,
        );
    }
    neutral_indicator(ui::icons::RADIO_OFF, metrics, cx)
}

pub(in crate::workspace::left_dock) fn pane_indicator(
    pane: &PaneSnapshot,
    metrics: theme::UiMetrics,
    cx: &gpui::App,
) -> AnyElement {
    match pane.unconnected_icon() {
        Some(icon) => neutral_indicator(icon, metrics, cx),
        None => lane_indicator(pane.status, pane.outcome, metrics, cx),
    }
}

fn neutral_indicator(icon: &'static str, metrics: theme::UiMetrics, cx: &gpui::App) -> AnyElement {
    div()
        .flex_none()
        .w(px(metrics.icon(theme::STATUS_INDICATOR_CELL_WIDTH)))
        .flex()
        .items_center()
        .justify_center()
        .text_color(theme::current(cx).text_muted)
        .child(ui::icons::icon(icon).with_size(px(metrics.icon(theme::CONTROL_ICON_SIZE))))
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
