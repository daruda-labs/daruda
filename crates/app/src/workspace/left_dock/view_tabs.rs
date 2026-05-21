//! ViewSwitcher for the left dock (LeftDock).
//!
//! Three tabs (Lanes / Git / Files) map to
//! `daruda_store::project::LeftDockView`. Clicking a tab calls
//! `Workspace::set_left_dock_view` via the snapshot's `workspace`
//! weak-entity handle.

use daruda_store::project::LeftDockView;
use gpui::{AnyElement, Context, IntoElement, prelude::*, px};

use crate::surface::strings;
use crate::ui::{tab, tab_bar};

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
/// first entry on out-of-bounds — `TabBar::on_click` only emits indices
/// within the children we passed, so this is a defensive ceiling.
fn view_by_index(ix: usize) -> LeftDockView {
    entries()
        .get(ix)
        .map(|(v, _)| *v)
        .unwrap_or(LeftDockView::Lanes)
}

/// Render the ViewSwitcher tab strip for the left dock.
pub(in crate::workspace) fn render(snap: &LeftDockSnapshot, _cx: &mut Context<Dock>) -> AnyElement {
    let all = entries();
    let active_ix = all
        .iter()
        .position(|(v, _)| *v == snap.left_dock_view)
        .unwrap_or(0);
    let workspace = snap.workspace.clone();

    tab_bar("left-dock-view-switcher")
        .w_full()
        .gap(px(0.))
        .selected_index(active_ix)
        .children(all.into_iter().map(|(_, label)| tab(label)))
        .on_click(move |ix, _window, cx| {
            let view = view_by_index(*ix);
            if let Some(ws) = workspace.upgrade() {
                ws.update(cx, |ws, cx| ws.set_left_dock_view(view, cx));
            }
        })
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
