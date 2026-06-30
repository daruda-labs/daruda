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

- Repo: <https://github.com/agentclientprotocol/agent-client-protocol>
- crates.io: `agent-client-protocol` (pulls in `agent-client-protocol-schema`
  and `-derive` transitively). Declared in `Cargo.toml`.
- **Latest as of 2026-06-30**: crate `1.0.1`, schema `1.1.0` (the crate pins
  `agent-client-protocol-schema = "=1.1.0"`; the schema crate's own latest is
  `1.2.0`, and the JSON spec tag line is `schema-v1.17.0` — three separate
  version lines, do not conflate them).
- Keep current: the bare `"1.0"` in `Cargo.toml` is a default (caret) requirement
  — Cargo reads it as `>=1.0.0, <2.0.0` — so `cargo update -p agent-client-protocol`
  already picks up patch and minor releases; edit the version string only when a
  new major lands.

### 2. Claude ACP adapter (npm, spawned at runtime)

- Repo: <https://github.com/agentclientprotocol/claude-agent-acp>
- npm: `@agentclientprotocol/claude-agent-acp` — the adapter that wraps
  Anthropic's Claude Agent SDK and speaks ACP. Spawned via
  `AdapterCommand::default()` in `connection.rs` (`npx -y <pkg>@latest`).
- **Latest as of 2026-06-30**: `0.53.0` (uses ACP SDK `@agentclientprotocol/sdk`
  `1.0.0` — the same 1.0 line as our Rust crate, so they stay in sync).
- Keep current: the `@latest` tag in the `npx` command already pulls the newest
  adapter at spawn time. When pinning a version, raise it to the newest release.

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
