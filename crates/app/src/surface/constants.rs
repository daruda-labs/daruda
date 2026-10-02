//! App identity — values that persist across themes, localisations,
//! and keyboard remaps. A rename changes a single constant here.

/// Display name used in the menu bar and window chrome.
pub const APP_NAME: &str = "Daruda";

/// Lowercase identity daruda presents to other programs — the MCP
/// `serverInfo.name` and the stem of the per-run server name an agent's
/// config sees. Distinct from [`APP_NAME`]: this one travels over a wire and
/// is matched on, so it must not follow display capitalisation.
pub const AGENT_FACING_NAME: &str = "daruda";

/// Help-menu targets.
pub const URL_GITHUB_REPO: &str = "https://github.com/daruda-ai/daruda";
pub const URL_REPORT_ISSUE: &str = "https://github.com/daruda-ai/daruda/issues/new";
pub const URL_HELP: &str = "https://github.com/daruda-ai/daruda#readme";

/// Version line under the Landing title. Read from the crate version so it
/// cannot drift — this surface is reachable from every project-less launch,
/// not just a first run.
pub const WELCOME_VERSION: &str = concat!("v", env!("CARGO_PKG_VERSION"));
