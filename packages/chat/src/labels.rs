//! Chat wording the host shows too — a Telegram relay, an attachment refusal —
//! so the two say it in the same words. The `test-support` ones exist for the
//! host's tests to compare against, not for the host to show.

use crate::strings as s;

/// One category segment of a tool-group header, e.g. "3 files read".
pub fn group_category(category: &str, count: usize) -> String {
    s::agent_chat::group_category(category, count)
}

/// Why an attachment was refused for being over the per-prompt limits.
pub fn attachment_limit() -> String {
    s::agent_chat::attachment_limit()
}

/// What a diff preview says when it had to leave `n` lines out.
pub fn diff_fallback_truncated(n: usize) -> String {
    s::agent_chat::diff_fallback_truncated(n)
}

/// A session whose transport closed under it.
#[cfg(any(test, feature = "test-support"))]
pub fn transport_closed() -> String {
    s::agent_chat::transport_closed()
}

/// An axis value that comes from the agent's defaults.
#[cfg(any(test, feature = "test-support"))]
pub fn source_following_agent() -> String {
    s::agent_chat::source_following_agent()
}

/// An axis value this chat chose for itself.
#[cfg(any(test, feature = "test-support"))]
pub fn source_this_chat() -> String {
    s::agent_chat::source_this_chat()
}
