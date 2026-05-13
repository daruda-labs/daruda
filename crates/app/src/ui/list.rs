//! `crate::ui::list` — wrapper over `gpui_component::list`.
//!
//! Replaces daruda's `Picker` (search + filter + pick) with the
//! `gpui_component` `List + ListDelegate` stack. Hosts pick one of
//! two paths:
//!
//! - **Filtered** — host owns a small `Vec<I: FilteredItem>` and wants
//!   substring search out of the box. Use [`FilteredDelegate`] +
//!   [`searchable_list_state`]. The delegate handles `perform_search`
//!   automatically and exposes [`FilteredDelegate::item_at`] so the
//!   host can resolve `ListEvent::Confirm(IndexPath)` back to the
//!   underlying item without re-implementing the filter.
//! - **Custom** — for async / fuzzy / multi-field search, hand-roll a
//!   `ListDelegate` directly (see `gpui_component::list::ListDelegate`).
//!   The [`list`] factory still applies `xsmall` for sizing parity.
//!
//! Event mapping vs. the legacy `Picker`:
//! - `PickerEvent::Selected(usize)` → `ListEvent::Confirm(IndexPath)`.
//!   `IndexPath::row` is the **post-filter** index; resolve to the
//!   real item via [`FilteredDelegate::item_at`].
//! - `PickerEvent::Cancelled` → `ListEvent::Cancel`.
//! - `ListEvent::Select(IndexPath)` is new — emitted on highlight
//!   change (arrow key / hover). Hosts that only care about commit can
//!   ignore this variant.

use std::sync::Arc;

use gpui::{App, Context, Entity, ParentElement as _, SharedString, Task, Window};
use gpui_component::Sizable as _;
use gpui_component::list::{ListDelegate, ListItem};

pub use gpui_component::IndexPath;
pub use gpui_component::list::{List, ListEvent, ListState as GpuiListState};

/// Item-shape contract for the built-in [`FilteredDelegate`]. A label
/// + case-insensitive substring matcher are enough to drive most
///   "search this small list" UIs.
pub trait FilteredItem: 'static + Clone {
    fn label(&self) -> SharedString;

    /// Default: case-insensitive substring match on the label. Override
    /// for fuzzy or multi-field matching.
    fn matches(&self, query: &str) -> bool {
        if query.is_empty() {
            return true;
        }
        self.label().to_lowercase().contains(&query.to_lowercase())
    }
}

/// A `ListDelegate` that holds an immutable item set + a filtered
/// index list. `perform_search` re-runs the filter on every query
/// change; `render_item` produces a default `ListItem` showing
/// `FilteredItem::label`. Selection state is tracked locally so
/// `set_selected_index` callbacks from the list survive across
/// re-filters (the host typically cares only about the latest
/// `Confirm` anyway).
pub struct FilteredDelegate<I: FilteredItem> {
    items: Arc<Vec<I>>,
    /// Indices into `items` whose `matches(query)` returned true, in
    /// display order.
    matched: Vec<usize>,
    selected: Option<IndexPath>,
}

impl<I: FilteredItem> FilteredDelegate<I> {
    pub fn new(items: Vec<I>) -> Self {
        let matched = (0..items.len()).collect();
        Self {
            items: Arc::new(items),
            matched,
            selected: None,
        }
    }

    /// Replace the items, reset the filter, drop the selection. Caller
    /// must trigger a re-render via `cx.notify()` on the parent state.
    pub fn set_items(&mut self, items: Vec<I>) {
        self.items = Arc::new(items);
        self.matched = (0..self.items.len()).collect();
        self.selected = None;
    }

    /// Resolve the visible (post-filter) [`IndexPath`] back to the
    /// concrete item. Returns `None` when `ix` is out of range or
    /// section ≠ 0 (this delegate has only one section).
    pub fn item_at(&self, ix: IndexPath) -> Option<&I> {
        if ix.section != 0 {
            return None;
        }
        let &orig = self.matched.get(ix.row)?;
        self.items.get(orig)
    }
}

impl<I: FilteredItem> ListDelegate for FilteredDelegate<I> {
    type Item = ListItem;

    fn perform_search(
        &mut self,
        query: &str,
        _: &mut Window,
        _cx: &mut Context<GpuiListState<Self>>,
    ) -> Task<()> {
        self.matched = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| it.matches(query))
            .map(|(i, _)| i)
            .collect();
        // Drop a stale selection that fell out of range.
        if let Some(sel) = self.selected
            && sel.row >= self.matched.len()
        {
            self.selected = None;
        }
        Task::ready(())
    }

    fn items_count(&self, _section: usize, _cx: &App) -> usize {
        self.matched.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _: &mut Window,
        _cx: &mut Context<GpuiListState<Self>>,
    ) -> Option<Self::Item> {
        let item = self.item_at(ix)?;
        let selected = self.selected == Some(ix);
        Some(
            ListItem::new(("kit-list-item", ix.row))
                .selected(selected)
                .child(item.label()),
        )
    }

    fn set_selected_index(
        &mut self,
        ix: Option<IndexPath>,
        _: &mut Window,
        _cx: &mut Context<GpuiListState<Self>>,
    ) {
        self.selected = ix;
    }
}

/// State alias — what callers store in their entity field.
pub type FilteredListState<I> = GpuiListState<FilteredDelegate<I>>;

/// Construct a searchable [`FilteredListState`] over `items`. The
/// query input renders at the top of the list — the wrapper enables
/// `searchable` so the host doesn't have to remember to flip it.
pub fn searchable_list_state<I: FilteredItem>(
    items: Vec<I>,
    window: &mut Window,
    cx: &mut Context<FilteredListState<I>>,
) -> FilteredListState<I> {
    GpuiListState::new(FilteredDelegate::new(items), window, cx).searchable(true)
}

/// Build a render-time [`List`] element from any `ListState`, sized
/// `xsmall`. Caller chains `.scrollbar_visible(...)` /
/// `.search_placeholder(...)` etc. as needed.
pub fn list<D: ListDelegate>(state: &Entity<GpuiListState<D>>) -> List<D> {
    List::new(state).xsmall()
}
