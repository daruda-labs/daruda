//! ViewSwitcher for the right sidebar (RightSidebar).
//!
//! Four tabs (Usage / Skills / Tools / Tasks) map to
//! `daruda_store::project::RightSidebarView`. Clicking a tab dispatches
//! `set_right_sidebar_view` on `Workspace` via the snapshot's `workspace`
//! weak-entity handle. Mirrors `workspace/sidebar/view_tabs.rs`.

use daruda_store::project::RightSidebarView;
use gpui::{AnyElement, Context, IntoElement, prelude::*, px};

use crate::surface::strings;
use crate::ui::{tab, tab_bar};

use super::super::dock::Dock;
use super::super::dock_snap::RightSidebarSnapshot;

/// All view entries shown in the strip, in visible order.
fn entries() -> [(RightSidebarView, &'static str); 4] {
    [
        (RightSidebarView::Usage, strings::RIGHT_PANEL_TAB_USAGE),
        (RightSidebarView::Skills, strings::RIGHT_PANEL_TAB_SKILLS),
        (RightSidebarView::Tools, strings::RIGHT_PANEL_TAB_TOOLS),
        (RightSidebarView::Tasks, strings::RIGHT_PANEL_TAB_TASKS),
    ]
}

/// Map a tab strip index back to its `RightSidebarView`. Falls back to
/// the first entry on out-of-bounds — `TabBar::on_click` only emits
/// indices within the children we passed, so this is a defensive
/// ceiling.
fn view_by_index(ix: usize) -> RightSidebarView {
    entries()
        .get(ix)
        .map(|(v, _)| *v)
        .unwrap_or(RightSidebarView::Usage)
}

/// Render the ViewSwitcher tab strip for the right sidebar.
pub(in crate::workspace) fn render(
    snap: &RightSidebarSnapshot,
    _cx: &mut Context<Dock>,
) -> AnyElement {
    let all = entries();
    let active_ix = all
        .iter()
        .position(|(v, _)| *v == snap.right_sidebar_view)
        .unwrap_or(0);
    let workspace = snap.workspace.clone();

    tab_bar("right-sidebar-view-switcher")
        .w_full()
        .gap(px(0.))
        .selected_index(active_ix)
        .children(all.iter().map(|(_, label)| tab(*label)))
        .on_click(move |ix, _window, cx| {
            let view = view_by_index(*ix);
            if let Some(ws) = workspace.upgrade() {
                ws.update(cx, |ws, cx| ws.set_right_sidebar_view(view, cx));
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
