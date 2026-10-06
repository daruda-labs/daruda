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

use crate::model::CommandExit;

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
    fn daruda_parent_tool_id_beats_the_claude_one() {
        // In native subagent mode both are present, and the claude id names a
        // launch call that mode never emits.
        assert_eq!(
            DefaultAdapter.parent_tool_id(&meta(json!({
                "daruda": {"parentToolId": "subagent:child-1"},
                "claudeCode": {"parentToolUseId": "toolu_suppressed"},
            }))),
            Some("subagent:child-1".to_owned())
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
    fn default_reads_claude_tool_name() {
        // The Claude adapter surfaces its own tool name here (confirmed on real
        // wire traffic: a Bash tool_call carries `_meta.claudeCode.toolName`).
        assert_eq!(
            DefaultAdapter.tool_name(&meta(json!({"claudeCode": {"toolName": "Bash"}}))),
            Some("Bash".to_owned())
        );
    }

    #[test]
    fn default_tool_name_is_none_without_the_key() {
        assert_eq!(DefaultAdapter.tool_name(&None), None);
        assert_eq!(
            DefaultAdapter.tool_name(&meta(json!({"claudeCode": {"parentToolUseId": "p"}}))),
            None
        );
    }

    #[test]
    fn default_tool_name_treats_blank_as_absent() {
        // A blank name would render as a blank header label with no fallback to
        // the kind, breaking the "never blank" guarantee — treat empty and
        // whitespace-only alike as absent.
        assert_eq!(
            DefaultAdapter.tool_name(&meta(json!({"claudeCode": {"toolName": ""}}))),
            None
        );
        assert_eq!(
            DefaultAdapter.tool_name(&meta(json!({"claudeCode": {"toolName": "   "}}))),
            None
        );
    }

    #[test]
    fn codex_delegates_to_default_for_now() {
        let m = meta(json!({"claudeCode": {"parentToolUseId": "toolu_x", "toolName": "Bash"}}));
        assert_eq!(
            CodexAdapter.parent_tool_id(&m),
            DefaultAdapter.parent_tool_id(&m)
        );
        assert_eq!(CodexAdapter.tool_name(&m), DefaultAdapter.tool_name(&m));
    }

    #[test]
    fn adapter_for_known_and_unknown_ids_extract_parent_id() {
        // codex and unknown ids both currently behave like Default for this hook.
        let m = meta(json!({"claudeCode": {"parentToolUseId": "toolu_y"}}));
        assert_eq!(
            adapter_for(None, "codex-acp").parent_tool_id(&m),
            Some("toolu_y".to_owned())
        );
        assert_eq!(
            adapter_for(None, "some-custom-agent").parent_tool_id(&m),
            Some("toolu_y".to_owned())
        );
    }

    #[test]
    fn command_exit_reads_codex_raw_output_exit_code() {
        let raw = json!({"formatted_output": "ok", "exit_code": 1});
        assert_eq!(
            DefaultAdapter.command_exit(&Some(raw), &None),
            Some(CommandExit {
                code: Some(1),
                signal: None
            })
        );
    }

    #[test]
    fn command_exit_reads_claude_terminal_exit_meta() {
        let m = meta(json!({"terminal_exit": {"exit_code": 2, "signal": null}}));
        assert_eq!(
            DefaultAdapter.command_exit(&None, &m),
            Some(CommandExit {
                code: Some(2),
                signal: None
            })
        );
    }

    #[test]
    fn command_exit_signal_only_yields_no_code() {
        let m = meta(json!({"terminal_exit": {"signal": "SIGTERM"}}));
        assert_eq!(
            DefaultAdapter.command_exit(&None, &m),
            Some(CommandExit {
                code: None,
                signal: Some("SIGTERM".to_owned())
            })
        );
    }

    #[test]
    fn command_exit_is_none_when_both_channels_absent() {
        assert_eq!(DefaultAdapter.command_exit(&None, &None), None);
        // Neither `exit_code` nor `terminal_exit` present in either channel.
        let raw = json!({"formatted_output": "ok"});
        let m = meta(json!({"claudeCode": {"toolName": "Bash"}}));
        assert_eq!(DefaultAdapter.command_exit(&Some(raw), &m), None);
    }

    #[test]
    fn command_exit_zero_is_reported_as_some() {
        // Display judgment belongs to the renderer — the model carries a
        // reported zero exit code as-is, not as an absence.
        let raw = json!({"formatted_output": "", "exit_code": 0});
        assert_eq!(
            DefaultAdapter.command_exit(&Some(raw), &None),
            Some(CommandExit {
                code: Some(0),
                signal: None
            })
        );
    }

    #[test]
    fn command_exit_prefers_terminal_exit_meta_over_raw_output() {
        let raw = json!({"exit_code": 9});
        let m = meta(json!({"terminal_exit": {"exit_code": 2}}));
        assert_eq!(
            DefaultAdapter.command_exit(&Some(raw), &m),
            Some(CommandExit {
                code: Some(2),
                signal: None
            })
        );
    }

    #[test]
    fn command_exit_is_none_when_the_channel_reports_neither_field() {
        // `dist/tools.js` builds `terminal_exit` from `bashResult.return_code`,
        // which is `undefined` when the SDK omits it — `JSON.stringify` then
        // drops the key and ships `{terminal_id, signal: null}`. An all-`None`
        // exit reads as "absent" in the renderer, so the parser must say `None`
        // rather than hand back a value that overwrites a recorded exit.
        let m = meta(json!({"terminal_exit": {"terminal_id": "t1", "signal": null}}));
        assert_eq!(DefaultAdapter.command_exit(&None, &m), None);
        // Same for the codex channel with an explicitly null `exit_code`.
        assert_eq!(
            DefaultAdapter.command_exit(&Some(json!({"exit_code": null})), &None),
            None
        );
    }

    #[test]
    fn command_exit_falls_back_to_raw_output_when_terminal_exit_is_empty() {
        let raw = json!({"formatted_output": "boom\n", "exit_code": 3});
        let m = meta(json!({"terminal_exit": {"signal": null}}));
        assert_eq!(
            DefaultAdapter.command_exit(&Some(raw), &m),
            Some(CommandExit {
                code: Some(3),
                signal: None
            })
        );
    }

    #[test]
    fn sideband_output_reads_claude_terminal_output_data() {
        let m = meta(json!({"terminal_output": {"terminal_id": "toolu_1", "data": "hello\n"}}));
        assert_eq!(
            DefaultAdapter.sideband_output(&m),
            Some("hello\n".to_owned())
        );
    }

    #[test]
    fn sideband_output_is_none_without_the_key() {
        // Absent `_meta`, an unrelated `_meta`, and a reshaped payload (the key
        // present but `data` renamed) must all degrade to `None`, not panic —
        // the shape is derived from adapter source, not a stable contract.
        assert_eq!(DefaultAdapter.sideband_output(&None), None);
        assert_eq!(
            DefaultAdapter.sideband_output(&meta(json!({"claudeCode": {"toolName": "Bash"}}))),
            None
        );
        assert_eq!(
            DefaultAdapter.sideband_output(&meta(json!({"terminal_output": {"output": "x"}}))),
            None
        );
        assert_eq!(
            DefaultAdapter.sideband_output(&meta(json!({"terminal_output": "x"}))),
            None
        );
    }

    #[test]
    fn sideband_output_ignores_a_non_string_data() {
        assert_eq!(
            DefaultAdapter.sideband_output(&meta(json!({"terminal_output": {"data": 42}}))),
            None
        );
    }

    #[test]
    fn codex_sideband_output_delegates_to_default() {
        // codex sends no `terminal_output` meta, so the shared shape read
        // yields `None` for it without any sender-identity branch.
        let m = meta(json!({"terminal_exit": {"exit_code": 0}}));
        assert_eq!(CodexAdapter.sideband_output(&m), None);
        assert_eq!(
            CodexAdapter.sideband_output(&m),
            DefaultAdapter.sideband_output(&m)
        );
    }

    #[test]
    fn codex_command_exit_delegates_to_default() {
        let raw = json!({"exit_code": 1});
        assert_eq!(
            CodexAdapter.command_exit(&Some(raw.clone()), &None),
            DefaultAdapter.command_exit(&Some(raw), &None)
        );
    }
}

#[cfg(test)]
mod selection_tests {
    use super::*;

    #[test]
    fn the_program_the_agent_reports_picks_the_strategy() {
        assert_eq!(
            adapter_id_for(Some("@agentclientprotocol/codex-acp"), "codex-acp"),
            AdapterId::Codex
        );
        assert_eq!(
            adapter_id_for(Some("@agentclientprotocol/claude-agent-acp"), "claude-acp"),
            AdapterId::Default
        );
    }

    /// The failure this selection exists to close: a user who registers
    /// codex-acp under a catalog id of their own still gets the codex strategy.
    #[test]
    fn a_catalog_id_of_the_users_own_still_reaches_the_right_strategy() {
        assert_eq!(
            adapter_id_for(Some("@agentclientprotocol/codex-acp"), "my-codex"),
            AdapterId::Codex
        );
    }

    /// `agent_info` is optional in the protocol, so the catalog id remains the
    /// fallback — an agent that reports nothing behaves exactly as before.
    #[test]
    fn an_agent_that_reports_nothing_falls_back_to_the_catalog_id() {
        assert_eq!(adapter_id_for(None, "codex-acp"), AdapterId::Codex);
        assert_eq!(adapter_id_for(None, "codex"), AdapterId::Codex);
        assert_eq!(
            adapter_id_for(None, "some-custom-agent"),
            AdapterId::Default
        );
    }

    /// The program on the other end of the pipe is the authority; the catalog
    /// id is only daruda's label for it.
    #[test]
    fn the_reported_program_outranks_the_catalog_id() {
        assert_eq!(
            adapter_id_for(Some("@agentclientprotocol/claude-agent-acp"), "codex-acp"),
            AdapterId::Default
        );
    }

    /// The scope is a publishing detail, so an unscoped report resolves the same.
    #[test]
    fn the_package_scope_is_not_part_of_the_identity() {
        assert_eq!(
            adapter_id_for(Some("codex-acp"), "custom"),
            AdapterId::Codex
        );
    }

    /// An unrecognised program is not evidence against the catalog id — a fork
    /// or wrapper reporting its own name still honours how it was registered.
    #[test]
    fn an_unrecognised_program_leaves_the_catalog_id_to_decide() {
        assert_eq!(
            adapter_id_for(Some("@someone/my-codex-wrapper"), "codex-acp"),
            AdapterId::Codex
        );
    }
}

#[cfg(test)]
mod phase_tests {
    use super::*;
    use serde_json::json;

    fn meta(v: serde_json::Value) -> Option<Meta> {
        Some(v.as_object().unwrap().clone())
    }

    /// Wire-captured (`acp-wire-codex-acp.log`): codex labels every message
    /// chunk, and across 3005 captured chunks it used exactly these two values.
    #[test]
    fn codex_reads_the_two_phases_it_actually_sends() {
        assert_eq!(
            CodexAdapter.message_phase(&meta(json!({"codex": {"phase": "commentary"}}))),
            MessagePhase::Commentary
        );
        assert_eq!(
            CodexAdapter.message_phase(&meta(json!({"codex": {"phase": "final_answer"}}))),
            MessagePhase::Answer
        );
    }

    /// A phase daruda has no opinion about must read as an ordinary answer, so
    /// a codex release that adds one cannot make messages vanish from a pane.
    #[test]
    fn an_unknown_phase_is_an_ordinary_answer() {
        assert_eq!(
            CodexAdapter.message_phase(&meta(json!({"codex": {"phase": "something_new"}}))),
            MessagePhase::Answer
        );
        assert_eq!(CodexAdapter.message_phase(&None), MessagePhase::Answer);
    }

    /// The vendor namespace stays with the named adapter. Default absorbing it
    /// would make the strategy choice decorative and let two agents' keys
    /// accumulate in one place.
    #[test]
    fn default_does_not_read_the_codex_namespace() {
        assert_eq!(
            DefaultAdapter.message_phase(&meta(json!({"codex": {"phase": "commentary"}}))),
            MessagePhase::Answer
        );
    }
}
