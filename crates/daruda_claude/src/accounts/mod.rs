//! Per-account runtime isolation for Claude. Pure/GPUI-free. Split by
//! responsibility:
//! - [`layout`] — config-dir paths + scoped-Keychain service naming.
//! - [`credentials`] — config-dir-scoped credential read + Keychain delete.
//! - [`sweep`] — startup orphan-config-dir sweep.
//! - [`mcp_mirror`] — shared-`mcpServers` JSON mirroring into an account dir.
//! - [`login`] / [`identity`] — headless login subprocess + `oauthAccount`
//!   identity parsing.
//!
//! Re-exports every public item flat under `daruda_claude::accounts::*` so
//! callers don't need to know which submodule each lives in.

pub mod credentials;
pub mod identity;
pub mod layout;
pub mod login;
pub mod mcp_mirror;
pub mod sweep;

pub use credentials::{AccountError, delete_scoped_credentials, read_scoped_credentials};
pub use identity::{AccountIdentity, read_account_identity};
pub use layout::{account_config_dir, accounts_root, scoped_keychain_service};
pub use login::{
    LoginError, LoginOutcome, LoginProcess, LoginProcessHandle, is_oauth_denied, spawn_login,
};
pub use mcp_mirror::mirror_shared_mcp_servers;
pub use sweep::sweep_orphan_dirs;
