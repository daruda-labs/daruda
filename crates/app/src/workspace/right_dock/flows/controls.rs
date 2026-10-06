//! Page identity, worktree scope and counted tabs share the Tasks chrome.

use super::list::{DefinitionList, ORIGINS, RunList};
use crate::surface::strings as s;
use crate::ui::{
    ButtonVariants as _, Disableable as _, DropdownMenu as _, PopupMenuItem, button_bare,
    button_with_icon, icons, menu_builder, tab, tab_bar, theme,
};
use crate::workspace::{
    flow_browser::{FlowScope, FlowTab, RunFilter},
    flow_paths::FlowOrigin,
    layout::RightDockSnapshot,
};
use gpui::{AnyElement, App, FontWeight, IntoElement, div, prelude::*, px};

pub(in crate::workspace) fn header(
    snap: &RightDockSnapshot,
    close: AnyElement,
    cx: &App,
) -> AnyElement {
    let t = theme::current(cx);
    let target = snap
        .flow_browser
        .targets
        .iter()
        .find(|target| target.lane == snap.flow_lane);
    let workspace = snap.workspace.clone();
    let lane = snap.flow_lane;
    div()
        .flex()
        .items_center()
        .gap(px(theme::GAP_STANDARD))
        .px(px(theme::DOCK_PAGE_PAD))
        .py(px(theme::PAD_STANDARD))
        .child(
            div()
                .flex_none()
                .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.text_primary)
                .child(s::dock::right_tab_flows()),
        )
        .child(scope_picker(snap, cx))
        .child(div().flex_1())
        .child(
            button_with_icon("flow-new", s::flow::new_title(), icons::ADD)
                .primary()
                .flex_none()
                .tab_stop(true)
                .disabled(target.is_none())
                .tooltip(
                    target
                        .map(|target| s::flow::new_in_project(&target.project))
                        .unwrap_or_else(s::flow::scope_unavailable),
                )
                .on_click(move |_, window, cx| {
                    if let Some(ws) = workspace.upgrade() {
                        ws.update(cx, |ws, cx| ws.prompt_new_flow(lane, window, cx));
                    }
                }),
        )
        .child(close)
        .into_any_element()
}

fn scope_picker(snap: &RightDockSnapshot, cx: &App) -> impl IntoElement {
    let targets = snap.flow_browser.targets.clone();
    let selected = snap.flow_lane;
    let workspace = snap.workspace.clone();
    let label = targets
        .iter()
        .find(|target| target.lane == selected)
        .map(|target| {
            if target.current {
                s::flow::scope_current(&target.label)
            } else {
                target.label.clone()
            }
        })
        .unwrap_or_else(s::flow::scope_unavailable);
    button_bare("flow-scope")
        .debug_selector(|| "flow-scope".into())
        .tab_stop(true)
        .min_w_0()
        .tooltip(s::flow::scope_hint())
        .child(icons::icon(icons::FOLDER).text_color(theme::current(cx).text_muted))
        .child(
            div()
                .min_w_0()
                .max_w(px(theme::FLOW_SCOPE_MAX_W))
                .truncate()
                .child(label),
        )
        .child(icons::icon(icons::EXPAND_MORE))
        .dropdown_menu(menu_builder(move |menu, _, _| {
            targets.iter().fold(menu.scrollable(true), |menu, target| {
                let workspace = workspace.clone();
                let scope = if target.current {
                    FlowScope::Current
                } else {
                    FlowScope::Worktree(target.lane)
                };
                let label = if target.current {
                    s::flow::scope_current(&target.label)
                } else {
                    target.label.clone()
                };
                menu.item(
                    PopupMenuItem::new(label)
                        .checked(target.lane == selected)
                        .on_click(move |_, _, cx| {
                            if let Some(ws) = workspace.upgrade() {
                                ws.update(cx, |ws, cx| ws.set_flow_scope(scope, cx));
                            }
                        }),
                )
            })
        }))
}

pub(super) fn section_tabs(snap: &RightDockSnapshot, run_count: usize) -> impl IntoElement {
    let workspace = snap.workspace.clone();
    tab_bar("flow-section-tabs")
        .w_full()
        .selected_index(snap.flow_browser.state.tab.index())
        .child(
            tab(s::common::filter_count(
                s::flow::panel_flows_heading(),
                snap.flow_files.len(),
            ))
            .debug_selector(|| "flow-tab-definitions".into()),
        )
        .child(
            tab(s::common::filter_count(s::flow::runs_tab(), run_count))
                .debug_selector(|| "flow-tab-runs".into()),
        )
        .on_click(move |index, _, cx| {
            if let Some(ws) = workspace.upgrade() {
                ws.update(cx, |ws, cx| {
                    ws.set_flow_tab(
                        if *index == 0 {
                            FlowTab::Definitions
                        } else {
                            FlowTab::Runs
                        },
                        cx,
                    )
                });
            }
        })
}

pub(super) fn origin_label(origin: Option<FlowOrigin>) -> String {
    match origin {
        None => s::flow::all_sources(),
        Some(FlowOrigin::Repo) => s::flow::source_repo(),
        Some(FlowOrigin::Project) => s::flow::source_project(),
        Some(FlowOrigin::Global) => s::flow::source_global(),
    }
}

pub(super) fn run_label(filter: RunFilter) -> String {
    match filter {
        RunFilter::All => s::flow::all_runs(),
        RunFilter::Running => s::flow::status_running(),
        RunFilter::Asking => s::flow::filter_asking(),
        RunFilter::Done => s::flow::status_done(),
        RunFilter::Failed => s::flow::status_failed(),
        RunFilter::Resumable => s::flow::filter_resumable(),
        RunFilter::Canceled => s::flow::status_canceled(),
        RunFilter::Unknown => s::flow::status_unknown(),
    }
}

pub(super) fn origin_tabs(snap: &RightDockSnapshot, list: &DefinitionList<'_>) -> impl IntoElement {
    let workspace = snap.workspace.clone();
    tab_bar("flow-origin-tabs")
        .w_full()
        .menu(true)
        .selected_index(
            ORIGINS
                .iter()
                .position(|origin| *origin == snap.flow_browser.state.origin)
                .unwrap_or(0),
        )
        .children(ORIGINS.into_iter().zip(list.counts).enumerate().map(
            |(index, (origin, count))| {
                tab(s::common::filter_count(origin_label(origin), count))
                    .debug_selector(move || format!("flow-origin-{index}"))
            },
        ))
        .on_click(move |index, _, cx| {
            if let Some(origin) = ORIGINS.get(*index).copied()
                && let Some(ws) = workspace.upgrade()
            {
                ws.update(cx, |ws, cx| ws.set_flow_origin(origin, cx));
            }
        })
}

pub(super) fn status_tabs(snap: &RightDockSnapshot, list: &RunList<'_>) -> impl IntoElement {
    let workspace = snap.workspace.clone();
    tab_bar("flow-status-tabs")
        .w_full()
        .menu(true)
        .selected_index(
            RunFilter::ALL
                .iter()
                .position(|filter| *filter == snap.flow_browser.state.run_filter)
                .unwrap_or(0),
        )
        .children(RunFilter::ALL.into_iter().zip(list.counts).enumerate().map(
            |(index, (filter, count))| {
                tab(s::common::filter_count(run_label(filter), count))
                    .debug_selector(move || format!("flow-status-{index}"))
            },
        ))
        .on_click(move |index, _, cx| {
            if let Some(filter) = RunFilter::ALL.get(*index).copied()
                && let Some(ws) = workspace.upgrade()
            {
                ws.update(cx, |ws, cx| ws.set_flow_run_filter(filter, cx));
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sources_and_execution_statuses_have_distinct_labels() {
        assert_ne!(
            origin_label(Some(FlowOrigin::Repo)),
            origin_label(Some(FlowOrigin::Project))
        );
        assert_ne!(
            run_label(RunFilter::Failed),
            run_label(RunFilter::Resumable)
        );
    }
}
