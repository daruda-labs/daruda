//! Per-agent ACP behavior strategies.
//!
//! ACP is a standard, but adapters diverge in vendor-namespaced `_meta` and in
//! where they place data. Rather than branch the mapper on sender identity, a
//! [`DefaultAdapter`] absorbs the superset of conventions seen in the wild and
//! named adapters override only the hooks that genuinely differ. The host
//! selects one strategy per session via [`adapter_for`], keyed on the agent's
//! catalog id.
//!
//! GPUI-free: a trait plus plain unit structs, unit-tested in isolation.

use agent_client_protocol::schema::v1::Meta;

/// Agent-specific hooks the mapper consults while folding protocol traffic into
/// the render model. Deliberately thin — one hook today — and grown as per-agent
/// divergences are confirmed from captured wire logs.
pub trait AcpAdapter: Send + Sync {
    /// The parent tool call id stamped on a nested subagent's tool call, read
    /// from the agent's vendor `_meta`. `None` for a top-level call or for an
    /// agent that does not nest subagent tool calls.
    fn parent_tool_id(&self, meta: &Option<Meta>) -> Option<String>;
}

/// Superset behavior for any agent without a dedicated strategy. Handles the one
/// nested-subagent `_meta` convention in the wild today — Claude's
/// `claudeCode.parentToolUseId` — which is harmlessly absent for other agents,
/// so routing every agent through it preserves the pre-strategy behavior.
pub struct DefaultAdapter;

impl AcpAdapter for DefaultAdapter {
    fn parent_tool_id(&self, meta: &Option<Meta>) -> Option<String> {
        meta.as_ref()?
            .get("claudeCode")?
            .get("parentToolUseId")?
            .as_str()
            .map(str::to_owned)
    }
}

/// codex-acp. Currently identical to [`DefaultAdapter`]; kept as the explicit
/// home for codex-specific behavior once it is confirmed from captured wire
/// logs (the planned step after this seam lands).
pub struct CodexAdapter;

impl AcpAdapter for CodexAdapter {
    fn parent_tool_id(&self, meta: &Option<Meta>) -> Option<String> {
        DefaultAdapter.parent_tool_id(meta)
    }
}

/// Select the strategy for a catalog agent id. Unknown ids (custom agents)
/// resolve to [`DefaultAdapter`], which preserves the pre-strategy universal
/// behavior for anything not explicitly special-cased.
pub fn adapter_for(agent_id: &str) -> Box<dyn AcpAdapter> {
    match agent_id {
        "codex" | "codex-acp" => Box::new(CodexAdapter),
        _ => Box::new(DefaultAdapter),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn meta(v: serde_json::Value) -> Option<Meta> {
        Some(v.as_object().unwrap().clone())
    }

    #[test]
    fn default_reads_claude_parent_tool_id() {
        // The Claude adapter stamps the parent id here on a subagent's inner call.
        assert_eq!(
            DefaultAdapter.parent_tool_id(&meta(
                json!({"claudeCode": {"parentToolUseId": "toolu_parent"}})
            )),
            Some("toolu_parent".to_owned())
        );
    }

    #[test]
    fn default_parent_tool_id_is_none_without_the_key() {
        assert_eq!(DefaultAdapter.parent_tool_id(&None), None);
        assert_eq!(
            DefaultAdapter.parent_tool_id(&meta(json!({"claudeCode": {"toolName": "Bash"}}))),
            None
        );
    }

    #[test]
    fn codex_delegates_to_default_for_now() {
        let m = meta(json!({"claudeCode": {"parentToolUseId": "toolu_x"}}));
        assert_eq!(
            CodexAdapter.parent_tool_id(&m),
            DefaultAdapter.parent_tool_id(&m)
        );
    }

    #[test]
    fn adapter_for_known_and_unknown_ids_extract_parent_id() {
        // codex and unknown ids both currently behave like Default for this hook.
        let m = meta(json!({"claudeCode": {"parentToolUseId": "toolu_y"}}));
        assert_eq!(
            adapter_for("codex-acp").parent_tool_id(&m),
            Some("toolu_y".to_owned())
        );
        assert_eq!(
            adapter_for("some-custom-agent").parent_tool_id(&m),
            Some("toolu_y".to_owned())
        );
    }
}
