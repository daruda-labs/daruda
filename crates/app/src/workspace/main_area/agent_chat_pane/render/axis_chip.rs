//! The overridden mark an Activity Bar chip carries. The popover shell the
//! panels sit in is host-neutral and lives with the editors they hold, in
//! [`crate::transcript::editor`].

use crate::surface::strings as s;
use crate::workspace::main_area::agent_chat_pane::pane_choice::PaneChoice;

/// A chip's label, marked when this pane has taken the axis off the configured
/// default. Reads `PaneChoice` rather than comparing the value: a pane on
/// `Chosen(default)` is overridden and must say so.
pub(super) fn axis_chip_label<T: Copy>(value_label: String, choice: PaneChoice<T>) -> String {
    if choice.is_following() {
        value_label
    } else {
        s::agent_chat_chip_overridden(&value_label)
    }
}

#[cfg(test)]
mod tests {
    use super::axis_chip_label;
    use crate::workspace::main_area::agent_chat_pane::pane_choice::PaneChoice;

    /// The mark tracks the variant, not the value — the whole point of the
    /// reset affordance is that `Chosen(default)` is still an override.
    #[test]
    fn only_a_chosen_axis_is_marked() {
        let plain = axis_chip_label("Fold: Auto".to_string(), PaneChoice::Seeded(1u8));
        assert_eq!(plain, "Fold: Auto");
        let marked = axis_chip_label("Fold: Auto".to_string(), PaneChoice::Chosen(1u8));
        assert!(marked.starts_with("Fold: Auto"));
        assert_ne!(marked, plain);
    }
}
