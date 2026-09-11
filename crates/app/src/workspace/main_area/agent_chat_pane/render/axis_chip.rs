//! The overridden mark an Activity Bar chip carries.

use crate::surface::strings as s;

/// A chip's label, marked when this pane has taken the axis off the configured
/// default. Takes `PaneChoice::is_following` rather than the value: a pane on
/// `Chosen(default)` is overridden and must say so — and an axis whose value is
/// a pair of levels has no single `PaneChoice` to hand over.
pub(super) fn axis_chip_label(value_label: String, following: bool) -> String {
    if following {
        value_label
    } else {
        s::agent_chat_chip_overridden(&value_label)
    }
}

#[cfg(test)]
mod tests {
    use super::axis_chip_label;

    /// The mark tracks whether the axis follows config, not the value — the
    /// whole point of the reset affordance is that `Chosen(default)` is still
    /// an override.
    #[test]
    fn only_a_chosen_axis_is_marked() {
        let plain = axis_chip_label("Fold: Auto".to_string(), true);
        assert_eq!(plain, "Fold: Auto");
        let marked = axis_chip_label("Fold: Auto".to_string(), false);
        assert!(marked.starts_with("Fold: Auto"));
        assert_ne!(marked, plain);
    }
}
