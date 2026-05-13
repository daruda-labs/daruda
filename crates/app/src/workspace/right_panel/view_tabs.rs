//! Tab strip rendered in the right dock's header slot.
//!
//! Four tabs (Usage / Skills / Tools / Tasks) map to
//! `daruda_store::project::RightPanelView`. Clicking a tab dispatches
//! `set_right_panel_view` on `Workspace` via the snapshot's `workspace`
//! weak-entity handle. Mirrors `workspace/sidebar/view_tabs.rs`.

use daruda_store::project::RightPanelView;
use gpui::{AnyElement, Context, IntoElement, prelude::*, px};

use crate::surface::strings;
use crate::ui::{tab, tab_bar};

use super::super::dock::Dock;
use super::super::dock_snap::RightDockSnap;

/// All view entries shown in the strip, in visible order.
fn entries() -> [(RightPanelView, &'static str); 4] {
    [
        (RightPanelView::Usage, strings::RIGHT_PANEL_TAB_USAGE),
        (RightPanelView::Skills, strings::RIGHT_PANEL_TAB_SKILLS),
        (RightPanelView::Tools, strings::RIGHT_PANEL_TAB_TOOLS),
        (RightPanelView::Tasks, strings::RIGHT_PANEL_TAB_TASKS),
    ]
}

/// Map a tab strip index back to its `RightPanelView`. Falls back to
/// the first entry on out-of-bounds — `TabBar::on_click` only emits
/// indices within the children we passed, so this is a defensive
/// ceiling.
fn view_by_index(ix: usize) -> RightPanelView {
    entries()
        .get(ix)
        .map(|(v, _)| *v)
        .unwrap_or(RightPanelView::Usage)
}

/// Render the right-panel tab strip.
pub(in crate::workspace) fn render(snap: &RightDockSnap, _cx: &mut Context<Dock>) -> AnyElement {
    let all = entries();
    let active_ix = all
        .iter()
        .position(|(v, _)| *v == snap.right_panel_view)
        .unwrap_or(0);
    let workspace = snap.workspace.clone();

    tab_bar("right-dock-tabs")
        .w_full()
        .gap(px(0.))
        .selected_index(active_ix)
        .children(all.iter().map(|(_, label)| tab(*label)))
        .on_click(move |ix, _window, cx| {
            let view = view_by_index(*ix);
            if let Some(ws) = workspace.upgrade() {
                ws.update(cx, |ws, cx| ws.set_right_panel_view(view, cx));
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
