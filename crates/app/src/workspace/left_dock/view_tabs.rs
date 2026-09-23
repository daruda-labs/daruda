//! ViewSwitcher for the left dock.
//!
//! Three tabs (Lanes / Git / Files) map to
//! `daruda_store::project::LeftDockView`; clicking one calls
//! `Workspace::set_left_dock_view` via the snapshot's weak handle.

use daruda_store::project::LeftDockView;
use gpui::{AnyElement, Context, IntoElement, div, prelude::*};

use crate::surface::strings;
use crate::ui::{Icon, IconName, dock_tab, dock_tab_bar, icons};

use super::super::layout::Dock;
use super::super::layout::LeftDockSnapshot;

/// All view entries shown in the strip, in visible order.
fn entries() -> Vec<(LeftDockView, gpui::SharedString)> {
    vec![
        (LeftDockView::Lanes, strings::sidebar_tab_worktrees().into()),
        (LeftDockView::GitChanges, strings::sidebar_tab_git().into()),
        (LeftDockView::Files, strings::sidebar_tab_files().into()),
    ]
}

/// Map a tab strip index back to its `LeftDockView`. Falls back to the
/// first entry on out-of-bounds (defensive; `TabBar::on_click` only emits
/// in-range indices).
fn view_by_index(ix: usize) -> LeftDockView {
    entries()
        .get(ix)
        .map(|(v, _)| *v)
        .unwrap_or(LeftDockView::Lanes)
}

fn view_icon(view: LeftDockView) -> Icon {
    match view {
        LeftDockView::Lanes => Icon::new(IconName::FolderClosed),
        LeftDockView::GitChanges => icons::icon(icons::DIFFERENCE),
        LeftDockView::Files => Icon::new(IconName::File),
    }
}

/// Render the ViewSwitcher tab strip for the left dock.
pub(in crate::workspace) fn render(snap: &LeftDockSnapshot, cx: &mut Context<Dock>) -> AnyElement {
    let all = entries();
    let active_ix = all
        .iter()
        .position(|(v, _)| *v == snap.left_dock_view)
        .unwrap_or(0);
    let workspace = snap.workspace.clone();

    let tabs = dock_tab_bar("left-dock-view-switcher")
        .selected_index(active_ix)
        .children(
            all.into_iter()
                .map(|(view, label)| dock_tab(view_icon(view), label)),
        )
        .on_click(move |ix, _window, cx| {
            let view = view_by_index(*ix);
            if let Some(ws) = workspace.upgrade() {
                ws.update(cx, |ws, cx| ws.set_left_dock_view(view, cx));
            }
        })
        .into_any_element();
    div()
        .flex()
        .flex_col()
        .flex_none()
        .child(crate::workspace::pages::render::navigation(snap, cx))
        .child(tabs)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_round_trips_through_entries() {
        let all = entries();
        for (ix, (view, _label)) in all.iter().enumerate() {
            assert_eq!(view_by_index(ix), *view, "mismatch at index {ix}");
        }
    }

    #[test]
    fn out_of_bounds_index_falls_back_to_first() {
        let all = entries();
        let oob = all.len() + 5;
        assert_eq!(view_by_index(oob), all[0].0);
    }
}
