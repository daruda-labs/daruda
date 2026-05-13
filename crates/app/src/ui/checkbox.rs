//! Checkbox factory — `xsmall()` auto-applied; the third argument
//! decides Tab participation.
//!
//! Pass an `isize` to slot the checkbox into the modal's Tab cycle at
//! that index (default `tab_stop` stays `true`). Pass `()` to exclude
//! the toggle from cycling — same effect as the button factory baking
//! `.tab_stop(false)` for footer Cancel / Save.
//!
//! ```ignore
//! use crate::ui::checkbox;
//! parent.child(checkbox("auto-enter", "Press Enter after sending", 2)); // cycles at index 2
//! parent.child(checkbox("read-only-toggle", "Read only", ()));         // skip cycle
//! ```

use gpui::ElementId;
use gpui_component::Sizable as _;
use gpui_component::text::Text;

pub use gpui_component::checkbox::Checkbox;

/// Tab-participation specifier for [`checkbox`]. Implemented for
/// `isize` (cycle at that index) and `()` (skip cycle).
pub trait CheckboxTabSpec {
    fn apply(self, cb: Checkbox) -> Checkbox;
}

impl CheckboxTabSpec for isize {
    fn apply(self, cb: Checkbox) -> Checkbox {
        cb.tab_index(self)
    }
}

impl CheckboxTabSpec for () {
    fn apply(self, cb: Checkbox) -> Checkbox {
        cb.tab_stop(false)
    }
}

pub fn checkbox<T: CheckboxTabSpec>(
    id: impl Into<ElementId>,
    label: impl Into<Text>,
    tab: T,
) -> Checkbox {
    let cb = Checkbox::new(id).xsmall().label(label);
    tab.apply(cb)
}
