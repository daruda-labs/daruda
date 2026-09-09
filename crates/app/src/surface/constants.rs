//! App identity — values that persist across themes, localisations,
//! and keyboard remaps. A rename changes a single constant here.

/// Display name used in the menu bar and window chrome.
pub const APP_NAME: &str = "Daruda";

/// Lowercase identity daruda presents to other programs — the MCP
/// `serverInfo.name` and the stem of the per-run server name an agent's
/// config sees. Distinct from [`APP_NAME`]: this one travels over a wire and
/// is matched on, so it must not follow display capitalisation.
pub const AGENT_FACING_NAME: &str = "daruda";
