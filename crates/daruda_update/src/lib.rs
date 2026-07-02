//! Self-update logic for the daruda macOS app.
//!
//! This crate is GPUI-free: it holds pure release-parsing logic (Task 1),
//! plus (in later tasks) networking to fetch release metadata and download
//! assets, and the DMG-based install/bundle-swap flow.

pub mod check;
pub mod install;
pub mod release;

pub use check::{check_latest, download_dmg};
pub use install::{install_dmg, relaunch};
pub use release::{ReleaseInfo, parse_release};

/// Errors that can occur anywhere in the update flow: checking for a new
/// release, downloading its DMG asset, mounting it, and installing the
/// update by swapping the running app bundle.
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("HTTP request failed: {0}")]
    Http(String),
    #[error("failed to parse release metadata: {0}")]
    Parse(String),
    #[error("release has no .dmg asset")]
    NoDmgAsset,
    #[error("I/O error: {0}")]
    Io(String),
    #[error("failed to mount disk image: {0}")]
    Mount(String),
    #[error("failed to install update: {0}")]
    Sync(String),
    #[error("refusing to download from untrusted host: {0}")]
    UntrustedHost(String),
}
