//! Self-update logic for the daruda app.
//!
//! This crate is GPUI-free: it holds release parsing, release metadata
//! fetching, package download, and the macOS DMG install/bundle-swap flow.
//! Which package a build looks for is [`release::asset_suffix`]'s answer.

pub mod check;
pub mod install;
pub mod release;

pub use check::{check_latest, download_asset};
pub use install::{install_dmg, relaunch};
pub use release::{ReleaseInfo, asset_suffix, parse_release};

/// Errors that can occur anywhere in the update flow: checking for a new
/// release, downloading this platform's package, mounting it, and installing
/// the update by swapping the running app bundle.
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("HTTP request failed: {0}")]
    Http(String),
    #[error("failed to parse release metadata: {0}")]
    Parse(String),
    #[error("release has no {0} asset")]
    NoAssetForPlatform(&'static str),
    #[error("no daruda package is published for {0}")]
    NoPackageForPlatform(&'static str),
    #[error("I/O error: {0}")]
    Io(String),
    #[error("failed to mount disk image: {0}")]
    Mount(String),
    #[error("failed to install update: {0}")]
    Sync(String),
    #[error("refusing to download from untrusted host: {0}")]
    UntrustedHost(String),
}
