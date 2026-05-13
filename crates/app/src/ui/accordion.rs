//! Wrapper over `gpui_component::accordion`.
//!
//! Single-call factory plus `pub use` of `Accordion` / `AccordionItem`
//! so app code can store these in struct fields without bypassing
//! `crate::ui::*`. See `crates/app/src/ui/CLAUDE.md`.

use gpui::ElementId;
use gpui_component::Sizable as _;

pub use gpui_component::accordion::{Accordion, AccordionItem};

/// Construct an `Accordion` sized for daruda's right-panel groupings.
///
/// **Deliberate exception to the project's "xsmall everywhere" rule:**
/// accordion headers carry the section name plus a count chip and need
/// enough vertical room to read comfortably without feeling cramped.
/// `small` strikes that balance. If a future call site needs the
/// tighter `xsmall`, override with `accordion(id).xsmall()` at the
/// call site rather than changing the default.
pub fn accordion(id: impl Into<ElementId>) -> Accordion {
    Accordion::new(id).small()
}
