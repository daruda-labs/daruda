//! Thin wrappers around macOS-only platform APIs that GPUI does not
//! abstract. Keep each module narrow — one OS API per file — so the
//! `unsafe` surface stays auditable.

pub mod attention;
pub mod notifications;
