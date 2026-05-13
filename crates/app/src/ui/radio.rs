//! Radio factory — `xsmall()` auto-applied; the third argument decides
//! Tab participation.
//!
//! Same shape as [`crate::ui::checkbox`]: pass an `isize` to slot the
//! radio into the modal's Tab cycle at that index, or `()` to exclude
//! it (mouse-only). daruda doesn't ship Radio call sites yet — this
//! wrapper exists so future radio groups go through the same
//! `tab_index` policy as the other input wrappers (see CLAUDE.md
//! "Tab navigation in modals").
//!
//! ```ignore
//! use crate::ui::radio;
//! parent.child(radio("level-low", "Low", 0).checked(level == Level::Low));
//! parent.child(radio("level-high", "High", 1).checked(level == Level::High));
//! ```
//!
//! `gpui_component::Radio` doesn't bundle a group manager — pick at
//! most one as `.checked(true)` based on the active value and wire
//! each `.on_click(...)` to set that value. See
//! `gpui_component::RadioGroup` if you'd rather opt into the upstream
//! group widget directly.

use gpui::ElementId;
use gpui_component::Sizable as _;
use gpui_component::text::Text;

pub use gpui_component::radio::Radio;

/// Tab-participation specifier for [`radio`]. Mirrors the
/// `CheckboxTabSpec` pattern (`isize` cycles, `()` skips).
pub trait RadioTabSpec {
    fn apply(self, r: Radio) -> Radio;
}

impl RadioTabSpec for isize {
    fn apply(self, r: Radio) -> Radio {
        r.tab_index(self)
    }
}

impl RadioTabSpec for () {
    fn apply(self, r: Radio) -> Radio {
        r.tab_stop(false)
    }
}

pub fn radio<T: RadioTabSpec>(id: impl Into<ElementId>, label: impl Into<Text>, tab: T) -> Radio {
    let r = Radio::new(id).xsmall().label(label);
    tab.apply(r)
}
