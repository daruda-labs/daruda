//! Per-agent ACP behavior strategies.
//!
//! ACP is a standard, but adapters diverge in vendor-namespaced `_meta` and in
//! where they place data. Rather than branch the mapper on sender identity, a
//! [`DefaultAdapter`] absorbs the superset of conventions seen in the wild and
//! named adapters override only the hooks that genuinely differ. The host
//! selects one strategy per session via [`adapter_for`], keyed on the program
//! the agent reports at `initialize` — the catalog id is only daruda's label
//! for it, and serves as the fallback for an agent that reports nothing.
//!
//! GPUI-free: a trait plus plain unit structs, unit-tested in isolation.

use agent_client_protocol::schema::v1::Meta;
use serde_json::Value;

use crate::model::{CommandExit, RateLimitView, RateLimitWindow};

/// Agent-specific hooks the mapper consults while folding protocol traffic into
/// the render model. Deliberately thin — grown as per-agent divergences are
/// confirmed from captured wire logs.
pub trait AcpAdapter: Send + Sync {
    /// The parent tool call id stamped on a nested subagent's tool call, read
    /// from the agent's vendor `_meta`. `None` for a top-level call or for an
    /// agent that does not nest subagent tool calls.
    fn parent_tool_id(&self, meta: &Option<Meta>) -> Option<String>;

    /// The agent's own tool name (e.g. `Bash`, `Read`, `Grep`), read from the
    /// agent's vendor `_meta`. More specific than the normalized `ToolKindView`
    /// and in the vocabulary the user already knows from the agent's CLI, so the
    /// renderer prefers it as the header label. `None` for an agent that does not
    /// surface a tool name, in which case the renderer falls back to the kind.
    fn tool_name(&self, meta: &Option<Meta>) -> Option<String>;

    /// Exit status of a shell-execution tool call. ACP does not carry this in a
    /// standard place, so each adapter puts it in a different channel. `None` =
    /// nothing usable was reported (a non-shell tool, a shell tool still
    /// running, or a channel that carried neither a code nor a signal).
    fn command_exit(&self, raw_output: &Option<Value>, meta: &Option<Meta>) -> Option<CommandExit>;

    /// Shell output recovered from a side channel when the standard place holds
    /// none. claude-agent-acp, once the client advertises `_meta.terminal_output`,
    /// replaces a Bash result's content with a content-less terminal handle and
    /// ships the real bytes as `_meta.terminal_output.data`. `None` when this
    /// adapter has nothing there — codex's shell output already comes back
    /// through `raw_output`, so it never reaches this hook.
    ///
    /// The text is pipe-captured stdout+stderr, not markdown: the caller must
    /// render it verbatim (see [`crate::model::ToolOutputBlock::RawText`]).
    fn sideband_output(&self, meta: &Option<Meta>) -> Option<String>;

    /// What role a streamed agent message plays in its turn, read from the
    /// agent's vendor `_meta`. [`MessagePhase::Answer`] for an agent that does
    /// not label its messages — and for any label this build has no opinion
    /// about, so a new one can never make a message vanish from the transcript.
    fn message_phase(&self, meta: &Option<Meta>) -> MessagePhase;

    /// The account's rate-limit standing carried on a usage update's vendor
    /// `_meta`. `None` for an agent that reports none, or a report too
    /// malformed to say which limit it is about.
    fn rate_limit(&self, meta: &Option<Meta>) -> Option<RateLimitView>;
}

/// Which role a streamed agent message plays in its turn.
///
/// Only an adapter that labels its messages reports anything but `Answer`.
/// Wire-captured from codex-acp, which sends exactly `commentary` (a preamble
/// written before it starts working) and `final_answer`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum MessagePhase {
    /// The agent's reply — what every unlabelled message is.
    #[default]
    Answer,
    /// A preamble the agent wrote before the work it is about.
    Commentary,
}

/// Superset behavior for any agent without a dedicated strategy. Handles the one
/// nested-subagent `_meta` convention in the wild today — Claude's
/// `claudeCode.parentToolUseId` — which is harmlessly absent for other agents,
/// so routing every agent through it preserves the pre-strategy behavior.
pub struct DefaultAdapter;

impl AcpAdapter for DefaultAdapter {
    fn parent_tool_id(&self, meta: &Option<Meta>) -> Option<String> {
        let meta = meta.as_ref()?;
        // daruda's own namespace wins. In native subagent mode Claude still
        // stamps `claudeCode.parentToolUseId`, but that mode suppresses the
        // launch call it names — so following it would leave the child pointing
        // at a tool the transcript never carried, and a dangling parent renders
        // the child as a top-level row instead of inside its card.
        if let Some(id) = meta
            .get(crate::native_subagents::DARUDA_META_KEY)
            .and_then(|v| v.get(crate::native_subagents::PARENT_TOOL_ID_META_KEY))
            .and_then(Value::as_str)
        {
            return Some(id.to_owned());
        }
        meta.get("claudeCode")?
            .get("parentToolUseId")?
            .as_str()
            .map(str::to_owned)
    }

    fn tool_name(&self, meta: &Option<Meta>) -> Option<String> {
        meta.as_ref()?
            .get("claudeCode")?
            .get("toolName")?
            .as_str()
            .filter(|s| !s.trim().is_empty())
            .map(str::to_owned)
    }

    fn command_exit(&self, raw_output: &Option<Value>, meta: &Option<Meta>) -> Option<CommandExit> {
        // Claude's channel (`_meta.terminal_exit`) is checked first; only when
        // it yields nothing usable does codex's `raw_output.exit_code` apply.
        // Both conventions read through the same object-shaped parse below, so a
        // second adapter's own `signal` field (should one appear) is picked up
        // for free.
        if let Some(exit) = meta
            .as_ref()
            .and_then(|m| m.get(TERMINAL_EXIT_META_KEY))
            .and_then(command_exit_of)
        {
            return Some(exit);
        }
        let raw = raw_output.as_ref()?;
        // `raw_output` is a free-form channel (MCP results land here too), so an
        // `exit_code` alone does not mean a command ran. codex-acp's shell result
        // is the pair `{ formatted_output, exit_code }` — verified on the wire —
        // and requiring the sibling is what keeps a stray `exit_code` in some
        // other tool's result from badging a card. The tool *kind* cannot serve
        // here: codex labels by intent, so a failing `ls` arrives as `Read`.
        raw.get("formatted_output")?;
        raw.get("exit_code")?;
        command_exit_of(raw)
    }

    fn sideband_output(&self, meta: &Option<Meta>) -> Option<String> {
        // Read by shape, not by sender: a missing or renamed key simply yields
        // `None` (the caller then reports the drop) rather than a panic, so an
        // adapter that reshapes this `_meta` degrades to today's behavior.
        meta.as_ref()?
            .get(TERMINAL_OUTPUT_META_KEY)?
            .get("data")?
            .as_str()
            .map(str::to_owned)
    }

    fn message_phase(&self, _meta: &Option<Meta>) -> MessagePhase {
        // No vendor namespace is read here. Unlike the hooks above — which
        // predate a trustworthy strategy choice and so absorb the superset —
        // selection now keys on the program the agent reports at `initialize`,
        // so a namespaced key belongs to the adapter that owns the namespace.
        MessagePhase::Answer
    }

    fn rate_limit(&self, meta: &Option<Meta>) -> Option<RateLimitView> {
        // Claude's namespace, read here because claude-agent-acp runs under
        // this strategy; no other agent sends `_claude/` keys to misread.
        rate_limit_of(meta.as_ref()?.get(CLAUDE_RATE_LIMIT_META_KEY)?)
    }
}

/// `_meta` key under which claude-agent-acp forwards the SDK's
/// `SDKRateLimitInfo` on a `usage_update`.
pub const CLAUDE_RATE_LIMIT_META_KEY: &str = "_claude/rateLimit";

/// Parse an `SDKRateLimitInfo` object. A string `status` is required as the
/// shape check, though unread; any other field of the wrong type reads as
/// absent, so a reshaped optional field cannot hide the overage flag beside it.
fn rate_limit_of(v: &Value) -> Option<RateLimitView> {
    v.get("status")?.as_str()?;
    Some(RateLimitView {
        window: v
            .get("rateLimitType")
            .and_then(Value::as_str)
            .map(RateLimitWindow::from_wire),
        resets_at: v.get("resetsAt").and_then(Value::as_i64),
        is_using_overage: v
            .get("isUsingOverage")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

/// `_meta` key carrying a shell tool's captured output, and the same key the
/// client advertises at `initialize` to switch claude-agent-acp into this mode.
/// That one flag gates **both** this key and [`TERMINAL_EXIT_META_KEY`] — the
/// adapter emits neither without it. daruda does not advertise it yet; see
/// `crate::session::client_capabilities`.
pub const TERMINAL_OUTPUT_META_KEY: &str = "terminal_output";

/// `_meta` key carrying a shell tool's exit status. Emitted by claude-agent-acp
/// only inside the same `supportsTerminalOutput` branch as
/// [`TERMINAL_OUTPUT_META_KEY`], so the exit badge and the output sideband stand
/// or fall together on that one advertised flag.
pub const TERMINAL_EXIT_META_KEY: &str = "terminal_exit";

/// Parse a `{ exit_code, signal }`-shaped object into a [`CommandExit`]. Shared
/// by both `command_exit` channels since they use the same field names. `None`
/// when neither field is usable: an all-`None` exit is indistinguishable from
/// "nothing reported" downstream, and returning it would let an empty later
/// report overwrite a real exit recorded earlier.
fn command_exit_of(v: &Value) -> Option<CommandExit> {
    let code = v
        .get("exit_code")
        .and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok());
    let signal = v
        .get("signal")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned);
    (code.is_some() || signal.is_some()).then_some(CommandExit { code, signal })
}

/// codex-acp. Delegates every hook whose convention it shares with the wider
/// superset, and owns the `_meta.codex` namespace.
pub struct CodexAdapter;

impl AcpAdapter for CodexAdapter {
    fn parent_tool_id(&self, meta: &Option<Meta>) -> Option<String> {
        DefaultAdapter.parent_tool_id(meta)
    }

    fn tool_name(&self, meta: &Option<Meta>) -> Option<String> {
        DefaultAdapter.tool_name(meta)
    }

    fn command_exit(&self, raw_output: &Option<Value>, meta: &Option<Meta>) -> Option<CommandExit> {
        DefaultAdapter.command_exit(raw_output, meta)
    }

    fn sideband_output(&self, meta: &Option<Meta>) -> Option<String> {
        DefaultAdapter.sideband_output(meta)
    }

    fn message_phase(&self, meta: &Option<Meta>) -> MessagePhase {
        match meta
            .as_ref()
            .and_then(|m| m.get("codex"))
            .and_then(|c| c.get("phase"))
            .and_then(Value::as_str)
        {
            Some(CODEX_COMMENTARY_PHASE) => MessagePhase::Commentary,
            _ => MessagePhase::Answer,
        }
    }

    fn rate_limit(&self, _meta: &Option<Meta>) -> Option<RateLimitView> {
        // `_claude/rateLimit` is Claude's namespace; codex reports no limits.
        None
    }
}

/// codex-acp's label for a message it writes before the work it is about. Its
/// sibling value, `final_answer`, needs no constant — it is the default.
const CODEX_COMMENTARY_PHASE: &str = "commentary";

/// Which strategy a session runs under. Named so the *choice* is a pure
/// function that can be decided and tested apart from building the strategy.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AdapterId {
    Default,
    Codex,
}

/// Adapter programs daruda has a dialect opinion about, keyed by the package
/// name the program reports at `initialize` (`agent_info.name`). Listing
/// claude-agent-acp as `Default` is not redundant with the fallback: it is what
/// makes "a recognised program outranks the catalog id" a rule rather than a
/// codex-only special case.
const KNOWN_PROGRAMS: [(&str, AdapterId); 3] = [
    ("codex-acp", AdapterId::Codex),
    ("codex", AdapterId::Codex),
    ("claude-agent-acp", AdapterId::Default),
];

/// The package name without its `@scope/` prefix — the scope is a publishing
/// detail, not part of which dialect the program speaks.
fn unscoped(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

/// Pick the strategy for a session.
///
/// `program` is what the agent called itself at `initialize`; `catalog_id` is
/// daruda's own label for it. The program wins **when daruda recognises it** —
/// it is the thing actually speaking, while the catalog id is user-editable and
/// a custom id would otherwise silently drop the agent to [`DefaultAdapter`].
/// An unrecognised program is not evidence against the catalog id (a fork or
/// wrapper reports its own name), and `agent_info` is optional in the protocol,
/// so both cases fall back to the id — which is exactly the pre-existing
/// behavior.
pub fn adapter_id_for(program: Option<&str>, catalog_id: &str) -> AdapterId {
    if let Some(name) = program.map(unscoped)
        && let Some((_, id)) = KNOWN_PROGRAMS.iter().find(|(known, _)| *known == name)
    {
        return *id;
    }
    match catalog_id {
        "codex" | "codex-acp" => AdapterId::Codex,
        _ => AdapterId::Default,
    }
}

/// Build the strategy [`adapter_id_for`] picks.
pub fn adapter_for(program: Option<&str>, catalog_id: &str) -> Box<dyn AcpAdapter> {
    match adapter_id_for(program, catalog_id) {
        AdapterId::Codex => Box::new(CodexAdapter),
        AdapterId::Default => Box::new(DefaultAdapter),
    }
}

#[cfg(test)]
mod tests;
