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
- A hook must not hold a `Committer` handle, including through a clone of a
  store that wraps one. The thread exits when the last handle drops, and a
  handle inside the hook it owns is a reference cycle: it never exits.

## Fresh State Comes Only From Replay

Construct state only through `RunLog::open`, `RunLog::read_only`, or
`RunLog::read_history`. The last two replay without writing an `opened` record,
repairing a torn tail, or pruning generations, so recovery can preserve every
file while exposing its state; `read_history` also returns the records.
Do not seed a machine's state from another file, a cache, or a default after
a failed open: the log is the single source, and a second source drifts from it.

File names (`log.<g>.jsonl`, `snapshot.<g>.json`) belong to `FileBackend`.
Other crates never parse a log's lines or build its paths: they read through
`read_history`, ask `FileBackend::holds_log` whether one exists, and take
`FileBackend::log_path` only to show or edit the bytes in a test.

## Readers Without the Lock

`read_only` and `read_history` may run while another process writes. They
apply the same rules as open, so the one tolerance a reader gets is the same
torn last line: a live writer may be mid-append, and that line is simply not
returned (`History::torn`). Anything else open would refuse, a reader gets as
an error too; it must not fall back to a prefix of the log.

## Strict Replay

- Only a torn **last** line (no trailing newline, or unparseable) is
  tolerated; open cuts it before writing anything.
- Any other unreadable line, an event the machine cannot deserialize, or a
  seq gap is `OpenError::Corrupt { line }`. Never skip a line.
- A record or snapshot with a newer `v` is `OpenError::TooNew`, and the log
  is left byte-for-byte untouched. Never overwrite a newer version.
- A broken newest snapshot falls back to an older generation; with no older
  generation left, open fails instead of starting empty. A generation is only
  dropped after its snapshot and log were checked for newer versions: what a
  newer build wrote is refused, never treated as broken and dropped.

## Schema Versions

`v` covers only the envelope. A machine's events and state carry
`Machine::SCHEMA`, recorded in every `opened` record and snapshot and checked
before any event or state is parsed, so an older build refuses a newer log as
`OpenError::TooNew { what: Versioned::Schema, .. }` instead of `Corrupt`.

- Bump the machine's `SCHEMA` in the same change that adds, renames, or
  reshapes an event or state variant or field, even an optional one.
- Never lower it or reuse a number. Reading an older schema must keep working.

## Single Writer

The caller must hold the external lock that guarantees one writer per log
before calling `open`: `desktop.lock` for profile logs, `RunLock` for flow
runs, the per-bot remote lock for the inbox. This crate does no locking.

## Durability

- `FileBackend` takes a directory the caller resolved through daruda's
  storage policy; it never resolves paths itself.
- Successful snapshot replacement and generation selection guarantee durable
  publication under the final names, including existing paths left by a failed
  publication. Unix flushes files and directory ancestors; Windows flushes the
  final writable file handles, including their metadata. `sync_dir` remains a
  no-op on Windows and is only an optimisation for old-generation cleanup.
- Publication failures refuse the open or close the writer before another
  event can be acknowledged. Recovery publishes the chosen generation before
  deleting older ones. A lost old-generation deletion is harmless; a successful
  commit must survive recovery, including after compaction.
- Compaction writes the next generation's snapshot, switches to its empty
  log, then drops the old generation. Once the new snapshot may be on disk,
  a failure closes the log.

## Tests

Machines in other crates use the `test-support` harness from their tests:
`assert_machine_laws` for the machine contract and `enumerate_crashes` for
every crash point. A new machine must pass both.
