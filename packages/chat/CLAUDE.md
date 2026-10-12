# daruda_chat

The agent chat pane: `AgentChatView`, its transcript rows and folds, and the
turn, activity and prompt-queue state behind them. The app keeps the host —
pane construction, the connection lifecycle, queue routing, the Telegram
relay — in `packages/app/src/workspace/main_area/agent_chat_host/`.

## The host boundary

The view knows nothing of `Workspace`, lanes or the agent catalog. Everything
crosses in one of four ways:

| Direction | Through |
|---|---|
| Host → view, at construction | `AgentChatView::new` arguments: `AgentIdentity` (catalog id, name, icon), `TranscriptDefaults`, syntax theme, `AgentDefaultWriter` |
| Host reads | `view/host_surface.rs` — one reader per fact; add one rather than widening a field |
| Host tells | `view/host_commands.rs` — each verb names the change |
| View → host | `AgentChatEvent` (`view/event.rs`) — errors, links, retry, reauth, prefs |

A host-specific behaviour is a callback the host installs (`AgentDefaultWriter`
writes "save as agent default" into the catalog), not a dependency.
`daruda_agent` is deliberately not a dependency: the lane vocabulary
(`SessionStatus`, `AgentOutcome`) is mapped from `status()` /
`activity_state()` / `TurnOutcome` in the host's `lane_status.rs`.

## Rules

- **Activity has one source** — AGENTS Pitfall 11. `Turn` stays private to
  `view/`; `scripts/lint-agent-activity.sh` enforces it.
- **Test seams return plain values.** `view/test_hooks.rs` is what the app's
  tests may drive, gated `cfg(any(test, feature = "test-support"))`; a hook
  never hands out the view's internals. Capture seams (`view/shot_ops.rs`,
  `shot_transcript.rs`) sit behind the `screenshot` feature.
- **Strings** — the view's own live in this package's `locales/`
  (`agent_chat:`, generated through `strings_gen`); host-side chat strings
  (composer chips, connect failures) stay in the app's `agent_chat:`.
- **Markdown** goes through `daruda_ui::markdown` (the vendored `TextView`),
  not the file viewer's renderer — see AGENTS "two rendering stacks".
- **Transcript presentation** (fold matrix, display filter, tool categories)
  is `daruda_transcript`'s; this package applies it, Settings authors it.
