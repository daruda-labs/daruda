//! The right-click menu of the Flows page's graph. The graph is a page's
//! detail, not a pane: it sits in no split and closes through the page, so
//! its menu carries only the graph's own rows, not a pane's.

use gpui::{Context, Pixels, Point, Window};

use crate::surface::strings as s;
use crate::workspace::Workspace;
use crate::workspace::pages::flows::detail::FlowDetailId;

use super::adapter::build_popup_menu;
use super::spec::{Activate, ItemState, MenuEntry, disabled_item, item, normalize_entries};

/// What the graph had selected when the menu opened.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct GraphSelection {
    /// Exactly one node — deleting needs one.
    node: bool,
    /// A line, drawn in the accent so what would be removed is visible.
    line: bool,
}

fn compose(id: FlowDetailId, selection: GraphSelection) -> Vec<MenuEntry> {
    let mut entries = vec![item(
        s::flow::add_node(),
        ItemState::Enabled,
        Activate::Op(Box::new(move |ws, window, cx| {
            ws.add_node_in_graph(id, window, cx);
        })),
    )];
    // Disabled rather than absent, so the row does not appear and disappear
    // under the pointer.
    entries.push(if selection.node {
        item(
            s::flow::delete_node(),
            ItemState::Enabled,
            Activate::Op(Box::new(move |ws, window, cx| {
                ws.delete_node_in_graph(id, window, cx);
            })),
        )
    } else {
        disabled_item(s::flow::delete_node(), None)
    });
    // Acts on the selected line, like its neighbour acts on the selected node
    // — not on whatever the right-click was over. Asks nothing: a line is one
    // drag to redraw, and the file is the undo stack.
    entries.push(if selection.line {
        item(
            s::flow::remove_connection(),
            ItemState::Enabled,
            Activate::Op(Box::new(move |ws, _window, cx| {
                ws.disconnect_selected_edge_in_graph(id, cx);
            })),
        )
    } else {
        disabled_item(s::flow::remove_connection(), None)
    });
    entries.push(MenuEntry::Separator);
    entries.push(item(
        s::ctx::reload_flow_graph(),
        ItemState::Enabled,
        Activate::Op(Box::new(move |ws, window, cx| {
            ws.reload_flow_graph(id, window, cx);
        })),
    ));
    normalize_entries(entries)
}

impl Workspace {
    /// Open graph `id`'s menu at `position`. A stale id opens nothing.
    pub(in crate::workspace) fn open_flow_graph_menu_at(
        &mut self,
        id: FlowDetailId,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((_, view)) = self.flow_graph(id) else {
            return;
        };
        let selection = {
            let view = view.read(cx);
            GraphSelection {
                node: view.selected_node(cx).is_some(),
                line: view.has_selected_edge(cx),
            }
        };
        let menu = build_popup_menu(
            compose(id, selection),
            None,
            cx.entity().downgrade(),
            window,
            cx,
        );
        self.open_context_menu(position, menu, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::main_area::pane_menu::spec::MenuItemSpec;

    const ID: FlowDetailId = FlowDetailId(1);

    fn find<'a>(entries: &'a [MenuEntry], label: &str) -> Option<&'a MenuItemSpec> {
        entries.iter().find_map(|entry| match entry {
            MenuEntry::Item(spec) if spec.label().as_ref() == label => Some(spec),
            _ => None,
        })
    }

    /// The graph's own rows and nothing a pane would add: it has no split to
    /// make and closes through the page.
    #[test]
    fn the_menu_holds_the_graphs_rows_only() {
        let entries = compose(ID, GraphSelection::default());
        assert!(find(&entries, &s::ctx::reload_flow_graph()).is_some());
        for forbidden in [
            s::common::close_tab(),
            s::menu::copy(),
            s::common::btn_stop(),
        ] {
            assert!(
                find(&entries, &forbidden).is_none(),
                "{forbidden} belongs to a pane"
            );
        }
    }

    /// Removing a line acts on the selected one, so the row is only live when
    /// there is one — and present either way. Same rule as "Delete Node".
    #[test]
    fn removing_a_connection_needs_a_selected_line() {
        let without = compose(ID, GraphSelection::default());
        let with = compose(
            ID,
            GraphSelection {
                node: false,
                line: true,
            },
        );
        assert!(
            find(&without, &s::flow::remove_connection())
                .expect("the row is present with nothing selected")
                .is_disabled()
        );
        assert!(
            !find(&with, &s::flow::remove_connection())
                .expect("and with a line selected")
                .is_disabled()
        );
    }

    /// The two gates are independent: a line is not a node, nor the reverse.
    #[test]
    fn a_selected_line_is_not_a_selected_node() {
        let line_only = compose(
            ID,
            GraphSelection {
                node: false,
                line: true,
            },
        );
        assert!(
            find(&line_only, &s::flow::delete_node())
                .expect("present")
                .is_disabled()
        );
        let node_only = compose(
            ID,
            GraphSelection {
                node: true,
                line: false,
            },
        );
        assert!(
            find(&node_only, &s::flow::remove_connection())
                .expect("present")
                .is_disabled()
        );
    }
}
