//! View tab strip rendered in the left dock's header slot.
//!
//! Three tabs (Worktrees / Git / Files) map to
//! `daruda_store::project::SidebarView`. Clicking a tab calls
//! `Workspace::set_sidebar_view` via the snapshot's `workspace`
//! weak-entity handle.

use daruda_store::project::SidebarView;
use gpui::{AnyElement, Context, IntoElement, prelude::*, px};

use crate::surface::strings;
use crate::ui::{tab, tab_bar};

use super::super::dock::Dock;
use super::super::dock_snap::LeftDockSnap;

/// All view entries shown in the strip, in visible order.
fn entries() -> [(SidebarView, &'static str); 3] {
    [
        (SidebarView::Worktrees, strings::SIDEBAR_TAB_WORKTREES),
        (SidebarView::GitChanges, strings::SIDEBAR_TAB_GIT),
        (SidebarView::Files, strings::SIDEBAR_TAB_FILES),
    ]
}

/// Map a tab strip index back to its `SidebarView`. Falls back to the
/// first entry on out-of-bounds — `TabBar::on_click` only emits indices
/// within the children we passed, so this is a defensive ceiling.
fn sidebar_view_by_index(ix: usize) -> SidebarView {
    entries()
        .get(ix)
        .map(|(v, _)| *v)
        .unwrap_or(SidebarView::Worktrees)
}

/// Render the view tab strip.
pub(in crate::workspace) fn render(snap: &LeftDockSnap, _cx: &mut Context<Dock>) -> AnyElement {
    let all = entries();
    let active_ix = all
        .iter()
        .position(|(v, _)| *v == snap.sidebar_view)
        .unwrap_or(0);
    let workspace = snap.workspace.clone();

    tab_bar("sidebar-view-tabs")
        .w_full()
        .gap(px(0.))
        .selected_index(active_ix)
        .children(all.iter().map(|(_, label)| tab(*label)))
        .on_click(move |ix, _window, cx| {
            let view = sidebar_view_by_index(*ix);
            if let Some(ws) = workspace.upgrade() {
                ws.update(cx, |ws, cx| ws.set_sidebar_view(view, cx));
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
            assert_eq!(sidebar_view_by_index(ix), *view, "mismatch at index {ix}");
        }
    }

    #[test]
    fn out_of_bounds_index_falls_back_to_first() {
        let all = entries();
        let oob = all.len() + 5;
        assert_eq!(sidebar_view_by_index(oob), all[0].0);
    }
}
