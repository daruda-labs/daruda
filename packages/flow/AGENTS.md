# daruda_flow

The declarative flow engine: it parses, validates and runs a DAG of agent and
command nodes, and records each run on a run log so a crashed run resumes.
GPUI-free. Read [Run log instructions](../runlog/AGENTS.md) first; this file
adds the flow-specific contracts on top of it.

## Run Directory

A run lives in `<cwd>/.daruda/flow-runs/<run-id>/`:

| Path | What it is |
|---|---|
| `run.yaml` | The flow as resolved at start. A resume reads it, never the source file. |
| `logs/progress/` | The run log (`run_log/`). The record of the run; never compacted. File names belong to `daruda_runlog`; read it through `run_log::reader`. |
| `progress.jsonl` | An older build's journal. Read-only: imported once, into an empty log, on resume. |
| `logs/` (other files) | Evidence: transcripts, command logs, archived outputs. |
| `DONE`, `FAILED`, `CANCELED`, `STALLED` | Markers, a cache of the log's ending. Written after `RunEnded`; the log wins. |
| `run.md` | The human report, written before the marker. |

The run's lock lives outside the tree, under the host's lock root
(`lock::lock_dir_for`). Never move it inside: `git clean -fdx` would delete it.

## Ordering Contract

- **Intent before an irreversible effect.** Commit `AttemptStarted` before a
  session is paid for, `Invalidated` before outputs are archived, `FixStarted`
  before a fix runs, `AskRaised` before a host sees a question.
- **Files before the record naming them.** Sync an output, or the directories
  of a rename, off the run thread (`schedule/durable.rs`) before committing the
  pass, failure or cancel that refers to it.
- **Narrate only what is durable.** Hosts hear an event from the after-commit
  hook, never before its record is on disk.
- **Numbers come from `assign`.** Attempt, evidence and question numbers are
  assigned by the log when an event commits. Never keep an in-memory counter;
  concurrent nodes in a wave would pick the same number.

## Resume Order

Each step must give the same result if a crash interrupts it and it runs
again:

1. Before the lock, read two things and decide nothing about progress from
   either: the selection fixed at start (`reader::selection`), and when a log
   that has not ended was last written (`reader::last_written_while_running`),
   which only times the wait out of the settle window. Both replay the whole
   log, but a value that disagrees with what the lock holder later finds can
   cost no more than a wrong wait length. The wait must come before the lock,
   so a Stop during it leaves the run untouched, lock included. A run only an
   older build's journal records has no record times, so it gets no wait.
2. Take the lock; open the log, importing the old journal if the log holds
   nothing of the run yet.
3. Clear markers.
4. End command process groups of unsettled attempts whose recorded identity
   still matches. Never signal a pid without that match.
5. Abandon open questions, then commit `Resumed`.
6. Commit `OutputLost` for passed agent nodes whose file is gone.
7. Archive outputs no passed node claims.
8. Schedule.

## Attempt Budget

`max_attempts` counts attempts in the current generation. A node whose pass
is taken away (a repair's `Invalidated`, or `OutputLost` on resume) starts a
new generation at attempt 1, so a crash before `RerunScheduled` still reruns
the members with a fresh count. A verdict failure and a user or overage
stop spend an attempt, and the count survives resume. An attempt cut short
by a crash or a quit spends none (`NodeState::crashed`).

## Stopping

`CancelToken` carries the reason (`StopKind`):

- `Stop` (a person, or the overage switch) settles the attempt as canceled
  and ends the run `Canceled`.
- `ShutDown` (the app is quitting) ends processes the same way, but records
  no settle, no ending and no marker, and keeps the lock. The run then reads
  as crashed and resumes like one.

## Tests That Pin the Engine

`schedule/tests/{parallel,ordering,budgets,repair,cancel,resuming,records}.rs`
hold the scheduler's long-standing invariants. A change may switch how one
reads a run directory, but must not change an expected value. A behavior
change belongs in a new or clearly named test.
