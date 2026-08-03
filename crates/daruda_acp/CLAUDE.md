# daruda_acp

GPUI-free ACP (Agent Client Protocol) client core. Spawns an ACP agent adapter
as a subprocess, drives the `initialize → session/new → session/prompt` exchange
over stdio JSON-RPC, and bridges protocol traffic to the host (`app`) as
`AcpEvent`s. The host never touches protocol types — it consumes the render
model in `model.rs`.

**Official guide**: <https://agentclientprotocol.com/> — the canonical ACP
documentation. Consult it for protocol concepts, the exchange flow, and feature
specs (e.g. session config options) before changing protocol behavior.

## Upstream dependencies — keep on the latest version

This crate sits on **two independently versioned, actively developed** upstreams.
Both move fast and gate real features behind version bumps, so **always track the
latest** of each. Lagging behind silently loses capabilities (see the
config-options example below).

### 1. ACP protocol library (Rust crate)

**Three separate version lines live here — do not conflate them.** They come
from two different repos, and each release note names only its own line:

| Line | Where | Latest as of 2026-07-30 | daruda |
|---|---|---|---|
| SDK crate `agent-client-protocol` | <https://github.com/agentclientprotocol/rust-sdk> | `2.0.0` | `2.0.0` |
| Schema crate `agent-client-protocol-schema` | <https://github.com/agentclientprotocol/agent-client-protocol> (tags `vX.Y.Z`, "Rust Crate") | `1.6.0` | `1.5.0` |
| JSON spec | same repo (tags `schema-vX.Y.Z`) | `schema-v1.20.0` | `~v1.19.1` |

- The SDK **exact-pins** the schema crate (`= 1.5.0` at SDK 2.0.0), so the
  schema line cannot be advanced on its own — it moves only when the SDK
  releases against a newer one. No SDK release consumes schema `1.6.0` /
  `schema-v1.20.0` yet.
- Keep current: `"2.0"` in `Cargo.toml` is a caret requirement (`>=2.0.0,
  <3.0.0`), so `cargo update -p agent-client-protocol` picks up patch and minor
  releases; edit the version string only when a new major lands.

**Capabilities are opt-in, and silence costs features.** An agent may only use a
feature the client advertises at `initialize`. `session.configOptions.boolean`
(schema `1.5.0`, spec `schema-v1.18.0`) is the worked example: both shipping
adapters check it and *degrade a native boolean toggle to a two-value select*
when it is absent — so before daruda advertised it, Claude's "Fast mode" arrived
as a select and the boolean path was simply unreachable. `client_capabilities()`
in `session.rs` is the single place this is declared; advertise a capability only
once the host actually renders it.

**Implemented but not advertised: `_meta.terminal_output`.** A vendor-private,
non-standard claude-agent-acp flag. Setting it makes a Bash result *content-less*
(`content: [{type:"terminal"}]`) and moves the bytes to
`_meta.terminal_output.data`; unset, the adapter returns a fenced ```` ```console ````
block. It also gates `_meta.terminal_exit`, the exit badge's only source. The
mapper parses both (`adapter.rs` + `mapping.rs`, with the three-notification
sequence pinned by a test), but `client_capabilities()` deliberately withholds
the advertisement until a live wire capture confirms that sequence. The shape is
read from adapter source (`dist/acp-agent.js`, `dist/tools.js`, 0.62.0), not the
wire — **re-verify on every adapter bump** before flipping it on.

### 2. Claude ACP adapter (npm, spawned at runtime)

- Repo: <https://github.com/agentclientprotocol/claude-agent-acp>
- npm: `@agentclientprotocol/claude-agent-acp` — the adapter that wraps
  Anthropic's Claude Agent SDK and speaks ACP.
- **Latest as of 2026-06-30**: `0.53.0` (uses ACP SDK `@agentclientprotocol/sdk`
  `1.0.0` — the same 1.0 line as our Rust crate, so they stay in sync).
- **Where the production launch command lives**: the shipped app reads its agent
  launch command from config — `daruda_config::AgentDefinition::claude_default()`
  (`crates/daruda_config/src/agent.rs`) — and passes it to
  `connect_agent_session`. That is the **one place to edit** the `@latest` pin /
  npm package for the shipped app. This crate's own `AdapterCommand::default()` /
  `ADAPTER_NPM_PACKAGE` in `connection.rs` are used only by `daruda_acp`'s
  examples and tests — editing them does **not** change what the app launches.
- Keep current: the `@latest` tag in the command already pulls the newest adapter
  at spawn time. When pinning a version, raise it in
  `AgentDefinition::claude_default()`.

### Why latest matters — config options

ACP standardizes model / reasoning-effort (`thought_level`) / permission-mode
selection through **session config options** (agent-advertised, agent-first: the
client can only set what the agent advertises —
<https://agentclientprotocol.com/protocol/v1/session-config-options>). The newest
adapter (`0.53.0`) advertises all of them in the `session/new` response, and the
newest Rust schema (`1.1.0+`) models them as `SessionConfigOption` (category
`Mode` / `Model` / `ThoughtLevel`). An adapter that doesn't advertise
`configOptions`, or a schema too old to carry them, makes model and
reasoning-effort selection invisible to the client — so both sides must be current
for the feature to work end to end.

## Wire tap — reading a capture

Every raw JSON-RPC line is tapped to a file in debug builds (`wire_log.rs`; the
app points `DARUDA_ACP_WIRE_LOG` at the log dir in `bootstrap.rs`). It is the
first diagnostic for "did the adapter actually send that" questions.

Each agent gets a pair of files: `acp-wire-<agent>.log` is the protocol
skeleton (one valid-JSON line per wire line), and
`acp-wire-<agent>.payload.jsonl` holds the string bodies that were too fat to
inline. Payload text is ~93% of a raw capture, so a string over 512 bytes
becomes a `@@acp-payload:<id>:<bytes>@@` marker in the slim log — measured on a
real 7.6 MB capture, slim lands at 0.56 MB. Resolve a marker by id:

```bash
jq 'select(.id == 4821)' acp-wire-codex-acp.payload.jsonl
```

`DARUDA_ACP_WIRE_LOG_MAX_FIELD=0` restores unelided raw lines;
`DARUDA_ACP_WIRE_LOG_PAYLOADS=0` drops payloads instead of writing the sidecar.
Both files truncate on the first session of each process run.
