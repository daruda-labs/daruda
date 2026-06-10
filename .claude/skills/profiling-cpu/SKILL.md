---
name: profiling-cpu
description: >-
  Use when the daruda app (or any GPUI/Rust macOS app) uses unexpected CPU while
  idle, shows CPU spikes/bursts, jank, or fan spin-up, and you need the root
  cause and the exact trigger. Covers no-rebuild triage (sample/top/ps) and
  deeper profiling (samply/Instruments). Keywords: idle CPU, high CPU, busy loop,
  redraw, repaint, sysinfo, polling, display link, flamegraph.
---

# Profiling CPU (GPUI / Rust on macOS)

## Core principle

**Separate two questions — they have different answers and different tools:**

1. **WHERE** is CPU spent? (`sample` call graph — the hot stacks)
2. **WHAT** triggers it? (the event/timer that *causes* the hot stacks)

The #1 mistake is answering only WHERE. In a GPUI app every repaint has the
**same** draw stack (`step → Window::draw → request_layout`) regardless of what
dirtied the view, so the draw stack alone never tells you the trigger. Find the
periodic `notify` / timer / fsevents / PTY-output source separately.

Also separate **main-thread** cost (draw) from **background-thread** cost
(pollers, `rayon`/`sysinfo` scans). `ps %CPU` is the *sum* across threads — a
near-idle main thread can sit next to a busy background scan.

## When to use

- The app burns CPU while you're not interacting.
- CPU is bursty/spiky (fan spins up periodically) or there's UI jank.
- You suspect a busy poll loop, a runaway repaint, or a background scan.

## Step 1 — No-rebuild triage (always start here)

A symbolicated `.app` lets `sample` give demangled Rust stacks with zero build
changes.

```bash
PID=$(pgrep -x <app>)
ps -p "$PID" -o pid,%cpu,%mem,etime,command   # instantaneous %CPU
ps -M "$PID"                                   # per-thread STIME/UTIME
top -l 2 -pid "$PID" -stats pid,cpu,th,mem | tail -3   # NB: first -l sample is a 0.0 artifact
sample "$PID" 5 -file /tmp/d.txt               # 5s stack-sample, ~1ms interval
```

Catch **bursts** (a periodic scan shows up as a spike between low baselines):

```bash
for i in $(seq 1 12); do printf "%s " "$(ps -p $PID -o %cpu=)"; sleep 0.5; done; echo
```

## Step 2 — Read the sample

Isolate the call-graph section (the trailing "Binary Images" block pollutes greps):

```bash
awk '/^Call graph:/{g=1} /^Total number|^Binary Images:/{g=0} g' /tmp/d.txt > /tmp/cg.txt
```

**Main thread — idle vs work split** (GPUI main queue):

```bash
grep -oE '[0-9]+ ReceiveNextEventCommon' /tmp/cg.txt | head -1            # idle wait
grep -oE '[0-9]+ __CFRUNLOOP_IS_SERVICING_THE_MAIN_DISPATCH_QUEUE__' /tmp/cg.txt | head -1  # = draw work
grep -oE '[0-9]+ gpui::window::Window::draw[^ ]*' /tmp/cg.txt | head -1
```

**Which thread is actually busy** — every thread prints the *total* sample count
at its root, so that number is meaningless. Find real work by filtering out
blocking frames:

```bash
grep -E '[0-9]+ ' /tmp/cg.txt \
  | grep -vE 'mach_msg|kevent|__workq_kernreturn|__psynch_cvwait|semaphore_wait|swtch|poll|read_|sigwait|Thread_|_pthread|start_thread' \
  | sed -E 's/^[ !:+|]+//' \
  | awk '{c=$1;$1="";s[$0]+=c} END{for(k in s) print s[k],k}' | sort -rn | head -25
```

Inspect a named thread's stack:

```bash
awk '/Thread_.*CVDisplayLink/{f=1} f&&/^\s*[0-9]+ Thread_/&&!/CVDisplayLink/{exit} f' /tmp/cg.txt | head -30
```

## GPUI-specific reads

| Observation in the sample | Means |
|---|---|
| `CVDisplayLink` thread mostly in `waitUntil → __psynch_cvwait` | display link **throttled** (good) — not a stuck 60/120fps animation |
| `CVDisplayLink` busy in `performIO` / `display_link_callback` every frame | **free-running** — a `with_animation` / `.repeat()` or `on_next_frame` never clears |
| draw recurses into multiple sibling view subtrees | whole tree repainted → **root `cx.notify()`** OR no view caching |
| `rayon::…bridge_producer_consumer` + `sysinfo::…update_process` / `get_process_infos` | a periodic **full-system process scan** on a background thread |
| expensive clone/alloc (e.g. snapshot building) inside the draw/render path | per-frame work that should be a change-driven cached snapshot |

**Caching matters:** if stable subtrees aren't wrapped in `.cached()` (+ paired
with targeted `cx.notify(child)`), *any* notify repaints the whole window tree —
the draw stack then looks identical whether the trigger is a leaf (caret) or the
root. So don't infer the trigger from "which views got repainted."

## Finding the trigger

The draw stack won't name the trigger. Correlate the repaint cadence with a
candidate source:

- **Fixed-rate, steady** ⇒ a timer (animation, blink, poll loop). Match the
  observed fps to a known interval.
- **Bursty/periodic spikes** ⇒ an event or a periodic scan, not a steady timer.
- **Background `rayon`/`sysinfo`** ⇒ a process-enumeration poll; its cost scales
  with `ps -ax | wc -l`.
- **Filesystem-driven** ⇒ watch the watched dir's mtimes over a few seconds; a
  write storm fans out to repaints.

A profiler with a timeline (Step 3) shows *which* background timer fires right
before each repaint — `sample` aggregates and loses that ordering.

## Step 3 — Deeper profiling (only if triage is inconclusive; needs a build)

Prefer `samply` (no Xcode, gives a flamegraph + timeline); reach for Instruments
only when you suspect a GPU/Metal bottleneck (`Metal System Trace`). Both need a
one-time install:

```bash
cargo install samply cargo-instruments   # not built-in; cargo-instruments also needs Xcode
```

Add to root `Cargo.toml` so stacks aren't bare addresses:

```toml
[profile.profiling]
inherits = "release"
debug = true
```

```bash
cargo build --profile profiling -p <crate>
samply record ./target/profiling/<bin>          # default: flamegraph + timeline
samply record -p "$(pgrep -x <app>)"            # or attach to the running app (no relaunch)
cargo instruments -p <crate> --profile profiling -t "Time Profiler"   # native Instruments (escape hatch)
```

## Common mistakes

- **Tunnel vision on the draw stack.** It's identical for every trigger. Find the `notify` source separately.
- **Trusting `ps -M` root counts / per-thread totals from `sample`.** They're the wall-clock sample count, not per-thread work. Filter blocking frames instead.
- **Reading `top -l 2`'s first sample.** It's a `0.0` artifact; use the second.
- **Calling it "60fps" without checking the display link.** If `CVDisplayLink` is blocked in `waitUntil`, it's throttled — the cost is low-rate repaints, not free-run.
- **Assuming a fixed-rate animation when CPU is bursty.** Bursty ⇒ event/periodic-scan driven, not a steady timer.
