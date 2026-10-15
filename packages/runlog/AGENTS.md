# daruda_runlog

An append-only, event-sourced run log. A domain `Machine` proposes events;
`RunLog` persists each one before it changes memory, so a crash at any point
reopens to a committed prefix. GPUI-free; depends only on `daruda_core`.

## Commit Order

`check` -> `assign` -> serialize -> append line + `\n` -> sync -> `apply` ->
seq advances. Nothing else may change machine state.

- `check` is pure and must also accept the event as `assign` filled it in;
  test builds re-check every replayed record.
- `assign` fills numbers the log owns (attempt, ask id) from current state,
  so concurrent proposers never pick the same one.
- `apply` is infallible and deterministic: replay runs it alone.
- A failed append or sync does not apply the event and closes the log;
  every later commit returns `CommitError::Closed`. In-memory state never
  runs ahead of disk.

## Committer

`Committer` owns a `RunLog` on its own thread and handles requests strictly
in arrival order. Per commit: `RunLog` commit (durable) -> `after_commit`
hook -> live subscribers -> the proposer's future resolves. So a proposer
that reads a hook-updated projection after its await sees its own commit,
and no subscriber sees a record the disk lacks. A rejection emits nothing.

- `subscribe(after_seq)` reads the catch-up and registers the live sender
  between two commits: no gap, no duplicate. Every record with a greater seq
  is delivered, `opened` records included (only in catch-ups, since a live
  committer never opens). Behind a compaction it gets the snapshot instead.
- After an `Io` error every later proposal is `Closed`; after a panic in the
  machine or the hook, `Poisoned` (the panicking event may be durable). Both
  end every live stream; `flush` and later requests answer at once.
- `flush(deadline)` waits until every request queued before it is answered.
- Dropping the last handle drains the queue, then joins the thread.
- Never block on the committer from inside the hook: `subscribe` refuses,
  `flush` times out.

## Fresh State Comes Only From Replay

Construct state only through `RunLog::open`. Do not seed a machine's state
from another file, a cache, or a default after a failed open: the log is the
single source, and a second source drifts from it.

## Strict Replay

- Only a torn **last** line (no trailing newline, or unparseable) is
  tolerated; open cuts it before writing anything.
- Any other unreadable line, an event the machine cannot deserialize, or a
  seq gap is `OpenError::Corrupt { line }`. Never skip a line.
- A record or snapshot with a newer `v` is `OpenError::TooNew`, and the log
  is left byte-for-byte untouched. Never overwrite a newer version.
- A broken newest snapshot falls back to an older generation; with no older
  generation left, open fails instead of starting empty.

## Single Writer

The caller must hold the external lock that guarantees one writer per log
before calling `open`: `desktop.lock` for profile logs, `RunLock` for flow
runs, the per-bot remote lock for the inbox. This crate does no locking.

## Durability

- `FileBackend` takes a directory the caller resolved through daruda's
  storage policy; it never resolves paths itself.
- Correctness never depends on a directory fsync. `sync_dir` is an
  optimisation (a no-op on Windows); the generation rules make any lost
  rename, create, or delete reopen to a consistent generation.
- Compaction writes the next generation's snapshot, switches to its empty
  log, then drops the old generation. Once the new snapshot may be on disk,
  a failure closes the log.

## Tests

Machines in other crates use the `test-support` harness from their tests:
`assert_machine_laws` for the machine contract and `enumerate_crashes` for
every crash point. A new machine must pass both.
