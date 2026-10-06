//! Flow definitions and execution history, scoped independently of the terminal.

use crate::ui::theme;
use crate::workspace::{flow_browser::FlowTab, layout::RightDockSnapshot};
use gpui::{AnyElement, IntoElement, prelude::*, px};

mod controls;
mod files;
mod list;
mod live;
mod past;
mod rows;
mod toolbar;

pub(in crate::workspace) use controls::header;

pub(in crate::workspace) fn render(snap: &RightDockSnapshot, cx: &gpui::App) -> AnyElement {
    let state = &snap.flow_browser.state;
    let runs = list::RunList::project(
        &snap.flows,
        snap.flow_history.as_ref(),
        state.run_filter,
        &snap.flow_browser.query,
    );
    let mut body = super::right_panel_body()
        .text_size(px(theme::RIGHT_PANEL_BODY_FONT_SIZE))
        .child(controls::section_tabs(snap, runs.total));
    if runs.waiting > 0 {
        body = body.child(toolbar::attention(snap, runs.waiting, cx));
    }
    match state.tab {
        FlowTab::Definitions => {
            let files = list::DefinitionList::project(
                &snap.flow_files,
                state.origin,
                &snap.flow_browser.query,
            );
            body = body
                .child(controls::origin_tabs(snap, &files))
                .child(toolbar::search(snap, cx))
                .child(toolbar::results(snap, files.visible.len(), files.total, cx));
            body = body.child(if files.visible.is_empty() {
                toolbar::empty(snap, files.total, cx)
            } else {
                rows::definitions(snap, &files, cx)
            });
        }
        FlowTab::Runs => {
            body = body
                .child(controls::status_tabs(snap, &runs))
                .child(toolbar::search(snap, cx))
                .child(toolbar::results(snap, runs.visible.len(), runs.total, cx));
            body = body.child(if runs.visible.is_empty() {
                toolbar::empty(snap, runs.total, cx)
            } else {
                rows::runs(snap, &runs, cx)
            });
            if runs.total > 0 {
                body = body.child(past::retention_note(cx));
            }
        }
    }
    body.into_any_element()
}

#[cfg(test)]
mod tests {
    #[test]
    fn definitions_and_runs_are_separate_views() {
        assert_ne!(super::FlowTab::Definitions, super::FlowTab::Runs);
    }
}
