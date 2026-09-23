//! Thin wrappers around platform APIs that GPUI does not
//! abstract. Keep each module narrow — one OS API per file — so the
//! `unsafe` surface stays auditable.

pub mod attention;
pub(crate) mod local_socket;
pub mod notifications;
pub mod presence;
pub(crate) mod window_controls;
