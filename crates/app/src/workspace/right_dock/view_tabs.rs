//! ViewSwitcher for the right dock.
//!
//! Four tabs (Usage / Skills / Tools / Tasks) map to
//! `daruda_store::project::RightDockView`. Clicking a tab dispatches
//! `set_right_dock_view` on `Workspace` via the snapshot's weak handle.

use daruda_store::project::RightDockView;
use gpui::{AnyElement, Context, IntoElement, prelude::*, px};

use crate::surface::strings;
use crate::ui::{tab, tab_bar};

use super::super::layout::Dock;
use super::super::layout::RightDockSnapshot;

/// All view entries shown in the strip, in visible order.
fn entries() -> Vec<(RightDockView, gpui::SharedString)> {
    vec![
        (
            RightDockView::Usage,
            strings::right_panel_tab_usage().into(),
        ),
        (
            RightDockView::Skills,
            strings::right_panel_tab_skills().into(),
        ),
        (
            RightDockView::Tools,
            strings::right_panel_tab_tools().into(),
        ),
        (
            RightDockView::Tasks,
            strings::right_panel_tab_tasks().into(),
        ),
        (
            RightDockView::Flows,
            strings::right_panel_tab_flows().into(),
        ),
    ]
}

/// Map a tab strip index back to its `RightDockView`, falling back to
/// the first entry on out-of-bounds (defensive).
fn view_by_index(ix: usize) -> RightDockView {
    entries()
        .get(ix)
        .map(|(v, _)| *v)
        .unwrap_or(RightDockView::Usage)
}

/// Render the ViewSwitcher tab strip for the right dock.
pub(in crate::workspace) fn render(
    snap: &RightDockSnapshot,
    _cx: &mut Context<Dock>,
) -> AnyElement {
    let all = entries();
    let active_ix = all
        .iter()
        .position(|(v, _)| *v == snap.right_dock_view)
        .unwrap_or(0);
    let workspace = snap.workspace.clone();

    tab_bar("right-dock-view-switcher")
        .w_full()
        .gap(px(0.))
        // Five labels do not fit this dock at its narrower widths — in
        // English "Flows" is already clipped at the default 250px and gone
        // entirely at the 220px minimum, with only its underline left. The
        // strip has no other overflow behaviour: it just cuts, so the active
        // tab can be the unreadable one. The menu keeps every tab reachable
        // whatever the width, and keeps doing so if a sixth is ever added.
        .menu(true)
        .selected_index(active_ix)
        .children(all.into_iter().map(|(_, label)| tab(label)))
        .on_click(move |ix, _window, cx| {
            let view = view_by_index(*ix);
            if let Some(ws) = workspace.upgrade() {
                ws.update(cx, |ws, cx| ws.set_right_dock_view(view, cx));
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
