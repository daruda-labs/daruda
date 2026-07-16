//! Radio factory with `xsmall()` and daruda Tab-cycle policy.
//!
//! Matches the other input wrappers: pass `isize` to include it in the modal
//! cycle or `()` to exclude it (CLAUDE.md "Tab navigation in modals"). Upstream
//! `Radio` has no group manager; callers set exactly one `.checked(true)`.

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
    let r = Radio::new(id).small().label(label);
    tab.apply(r)
}
