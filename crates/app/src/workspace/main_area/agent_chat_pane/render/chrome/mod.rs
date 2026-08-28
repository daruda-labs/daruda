//! Pane chrome around the conversation — three parts that share a pane and
//! nothing else, so each owns a file:
//!
//! - [`activity_bar`] — the pane's own toolbar: who the session is, how much
//!   context it has left, and how the transcript is displayed.
//! - [`status_banner`] — the connection's state, from provisioning a runtime to
//!   a failure that offers a way back.
//! - [`working_indicator`] — the turn's live progress, projected into the
//!   conversation flow rather than the chrome.

mod activity_bar;
mod status_banner;
mod working_indicator;

pub(super) use activity_bar::{ActivityBarProps, activity_bar};
pub(super) use status_banner::status_banner;
pub(super) use working_indicator::{pulse_dots, working_indicator};
