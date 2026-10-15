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
