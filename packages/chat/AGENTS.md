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

- **Activity has one source** - see [Activity and Cancellation](#activity-and-cancellation).
- **Test seams return plain values.** `view/test_hooks.rs` is what the app's
  tests may drive, gated `cfg(any(test, feature = "test-support"))`; a hook
  never hands out the view's internals. Capture seams (`view/shot_ops.rs`,
  `shot_transcript.rs`) sit behind the `screenshot` feature.
- **Strings** — the view's own live in this package's `locales/`
  (`agent_chat:`, generated through `strings_gen`); host-side chat strings
  (composer chips, connect failures) stay in the app's `agent_chat:`.
- **Markdown** goes through `daruda_ui::markdown` (the vendored `TextView`),
  not the file viewer's renderer; see [shared Markdown behavior](../content/AGENTS.md#shared-markdown-behavior).
- **Transcript presentation** (fold matrix, display filter, tool categories)
  is `daruda_transcript`'s; this package applies it, Settings authors it.

## Activity and Cancellation

Every "is this pane working?" and "did it finish?" decision reads
`activity_state()`, `is_busy()`, or `activity_elapsed()` on `AgentChatView`.
The raw prompt `Turn` settles before trailing background subagents finish.

- Completion side effects (notification and backing-task done) fire only
  through `fire_activity_completion` at the busy-to-idle edge detected by
  `reconcile_activity`. Never fire directly from `AcpEvent::TurnEnded`
  or `AcpEvent::Error`; that can fire early or twice.
- `Turn` stays private to `src/view/` for prompt-queue sequencing. Tests
  use `set_turn_in_flight`, `set_turn_idle`, and `turn_is_idle`, gated by
  `#[cfg(test)]` or `test-support` for app tests.
- Stop (`cancel_turn`) settles locally and immediately, stashes `Stopped`,
  and moves to `Turn::AwaitingCancelAck`. `apply_event` swallows the one
  expected `cancelled` `TurnEnded` so it cannot be attributed to a new turn.
- Prompts entered during that cancel window buffer client-side; `Turn::can_dispatch`
  controls wire dispatch. The cancel window and in-flight turn are one value,
  not a separate bool.
- This contract relies on `daruda_acp::session/turn.rs` being strictly FIFO:
  one prompt produces one `TurnEnded`, in order; a hung turn blocks later ones.

Enforcement: [activity lint](../../scripts/lint-agent-activity.sh).
