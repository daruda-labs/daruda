# daruda

**daruda is an Agentic Development Environment (ADE) for running multiple AI
coding agents side by side.**

Open ACP-powered agent chats for Claude Code, Codex, Gemini, and other
configured agents. Run agents in separate git worktrees to keep parallel
changes in separate working directories and on independent branches. Review
tool calls, approve permission prompts, inspect diffs, and keep a terminal
nearby when a shell is still the right tool.

daruda is built on [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui)
and [ghostty_vt](https://ghostty.org). macOS 12+ is the primary supported
desktop target today.

---

## Product Overview

- **Agent chat first** - supervise coding agents in first-class chat panes
  instead of a stack of terminal tabs. Pick model, reasoning effort, and
  permission mode inline.
- **Worktree isolation** - give independent tasks separate git worktrees, each
  with its own working directory and branch.
- **Live review surface** - tool calls render as cards, file edits show
  word-level diffs, and permission requests can be approved or rejected in-app.
- **Agentic flows** - define repeatable multi-step work as YAML flows with
  agent nodes, command gates, parallel execution, resume, and visual graph
  editing.
- **Workspace views** - browse worktrees, files, git changes, tasks, usage,
  skills, and MCP tools from the same window.
- **Terminal compatibility** - keep full terminal panes for shells, legacy CLI
  agents, macros, command history, and one-off TTY work.
- **Session restore** - tabs, panes, docks, worktree assignments, and workspace
  state persist across restarts.

## Core Concept: Worktrees

A **worktree** is the unit of work in daruda. It can be a git worktree with its
own directory, HEAD, and branch, or a plain project directory for non-git work.
Agent chats, terminal panes, file views, diffs, tasks, and persisted workspace
state are all attached to a worktree.

New chats open in the current worktree. Chats in the same worktree share files
and a branch. Create or select a separate worktree before starting an
independent task.

Use `Cmd+Ctrl+1-9` to jump between worktrees.

---

## Quick Start

### Requirements

- macOS 12 Monterey or later
- Rust 1.95+
- Xcode Command Line Tools
- Zig 0.14.1, installed by the bootstrap script below on macOS

### Run From Source

```bash
git clone --recurse-submodules https://github.com/daruda-labs/daruda
cd daruda

./scripts/bootstrap-zig.sh
cargo fetch && ./scripts/apply-gpui-patch.sh
cargo run -p daruda
```

For contributor setup, local checks, CI commands, Linux notes, and release
packaging, see [CONTRIBUTING.md](CONTRIBUTING.md).

---

## Status

| Area | Status |
|---|---|
| macOS runtime | Primary supported target, Apple Silicon and Intel |
| Linux | Builds and tests pass; GUI runtime still needs desktop verification |
| Windows | Not ported yet |
| ACP chat sessions | Shipped for configured ACP agents |
| Claude Code integration | Status, usage, skills, tools, and task launching shipped |
| Other agent integrations | Ongoing as each CLI exposes stable metadata |

## Roadmap

- Provider-specific status, usage, skills, and tools for more agents
- Linux desktop runtime verification
- Windows platform port
- Native app-identity notifications
- Developer ID code-signing, notarization, and Homebrew Cask distribution
- Kitty keyboard protocol, vi scrollback navigation, and image protocol support

---

## License

AGPL-3.0. See [LICENSE](LICENSE).

daruda vendors [Ghostty](https://ghostty.org) as a git submodule under
`vendor/ghostty`; third-party code remains under its respective licenses.
