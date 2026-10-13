//! Skills-specific render helpers. Each modal owns its own input
//! subscriptions and footer inline now that the `form_modal` trait
//! plumbing has been retired in favour of GPUI's tab system +
//! Dialog's built-in Confirm / Cancel actions.

pub(super) use crate::ui::form_label as field_label;
