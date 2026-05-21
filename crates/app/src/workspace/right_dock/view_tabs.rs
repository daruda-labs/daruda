//! ViewSwitcher for the right dock (RightDock).
//!
//! Four tabs (Usage / Skills / Tools / Tasks) map to
//! `daruda_store::project::RightDockView`. Clicking a tab dispatches
//! `set_right_dock_view` on `Workspace` via the snapshot's `workspace`
//! weak-entity handle. Mirrors `workspace/left_dock/view_tabs.rs`.

use daruda_store::project::RightDockView;
use gpui::{AnyElement, Context, IntoElement, prelude::*, px};

use crate::surface::strings;
use crate::ui::{tab, tab_bar};

use super::super::layout::Dock;
use super::super::layout::RightDockSnapshot;

/// All view entries shown in the strip, in visible order.
fn entries() -> Vec<(RightDockView, gpui::SharedString)> {
    vec![
        (RightDockView::Usage, strings::right_panel_tab_usage().into()),
        (RightDockView::Skills, strings::right_panel_tab_skills().into()),
        (RightDockView::Tools, strings::right_panel_tab_tools().into()),
        (RightDockView::Tasks, strings::right_panel_tab_tasks().into()),
    ]
}

/// Map a tab strip index back to its `RightDockView`. Falls back to
/// the first entry on out-of-bounds — `TabBar::on_click` only emits
/// indices within the children we passed, so this is a defensive
/// ceiling.
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
