//! Per-account runtime isolation for Claude. Pure/GPUI-free. Split by
//! responsibility:
//! - [`layout`] — config-dir paths + scoped-Keychain service naming.
//! - [`credentials`] — config-dir-scoped credential read + Keychain delete.
//! - [`sweep`] — startup orphan-config-dir sweep.
//! - [`mcp_mirror`] — shared-`mcpServers` JSON mirroring into an account dir.
//! - [`login`] / [`identity`] — headless login subprocess + `oauthAccount`
//!   identity parsing.
//! - [`recipe`] / [`claude`] / [`codex`] — the [`AccountRecipe`] auth-domain
//!   strategy ([`ClaudeRecipe`] wraps the functions above behind one
//!   interface; [`CodexRecipe`] does the same for a `CODEX_HOME`).
//!
//! Re-exports every public item flat under `daruda_agent::accounts::*` so
//! callers don't need to know which submodule each lives in.

pub mod claude;
pub mod codex;
pub mod credentials;
pub mod identity;
pub mod layout;
pub mod login;
pub mod mcp_mirror;
pub mod recipe;
pub mod sweep;

pub use claude::ClaudeRecipe;
pub use codex::CodexRecipe;
pub use credentials::{
    AccountError, PlanInfo, delete_scoped_credentials, read_scoped_credentials,
    read_system_credentials,
};
pub use identity::{AccountIdentity, read_account_identity};
pub use layout::{account_config_dir, accounts_root, scoped_keychain_service};
pub use login::{
    LoginCompletion, LoginError, LoginOutcome, LoginProcess, LoginProcessHandle, WaitPolicy,
    is_oauth_denied, spawn_login,
};
pub use mcp_mirror::mirror_shared_mcp_servers;
pub use recipe::{AccountRecipe, recipe_for};
pub use sweep::sweep_orphan_dirs;
