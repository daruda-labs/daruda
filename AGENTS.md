# Agent Instructions

Shared instructions for coding agents working in this repository. This file
is the single source of truth for workflow rules, project conventions,
architecture notes, verification expectations, and common pitfalls.

Claude Code reads `CLAUDE.md`, which imports this file with `@AGENTS.md`.

## Where to read first

This file is long and the rule that decides a change is rarely the one
nearest the top. Find the row for what you are about to touch and read
those first; the rest of this file is reference you come back to.

Paths point at files that live beside the code and go deeper than this
one. A `CLAUDE.md` under `crates/` is loaded automatically when you open
a file near it, but it is worth reading up front — it is where that
subsystem's real constraints are written down.

| Working on… | Read |
|---|---|
| **Anything at all, before committing** | [Pre-commit checks](#pre-commit-checks) · [Verification](#verification) |
| **Terminal, VT parsing, PTY, scrollback** | `crates/daruda_terminal/src/view/CLAUDE.md` · [Pitfalls](#pitfall-prevention-rules) 1 (coordinates), 3 (Zig FFI), 7 (text↔pixel), 8 (paint scope), 9 (palette) |
| **Agent chat, ACP, adapters, wire log** | `crates/daruda_acp/CLAUDE.md` · [Pitfall](#pitfall-prevention-rules) 11 (single activity source) |
| **A widget, a modal, anything visual** | `crates/app/src/ui/CLAUDE.md` · [`DESIGN.md`](./DESIGN.md) · [Pitfalls](#pitfall-prevention-rules) 10 (render cost), 12 (mouse buttons) |
| **Workspace layout — tabs, panes, docks** | `crates/app/src/CLAUDE.md` · [UI component hierarchy](#ui-component-hierarchy) · [MVU rules](#mvu-flavored-guiding-rules) |
| **Any string a user will see** | `crates/app/locales/CLAUDE.md` |
| **Where a new file or crate goes** | [Crate dependency graph](#crate-dependency-graph) · [File-structure rules](#file-structure-rules) · [Change-impact discipline](#change-impact-discipline) |
| **A daruda-owned environment variable** | `daruda_core::process_env` · [`lint-env-literals.sh`](./scripts/lint-env-literals.sh) |
| **Anything written to disk or keyed per profile** | [Cross-profile data isolation](#cross-profile-data-isolation) |
| **Spawning a process, resolving a path or shell** | [Platform capability boundary](#platform-capability-boundary) · [`lint-platform-boundary.sh`](./scripts/lint-platform-boundary.sh) |
| **A failure path — error, toast, log** | [Error reporting](#error-reporting) |
| **Checking a change actually renders** | [Visual verification](#visual-verification) · [Driving the captured state](#driving-the-captured-state) |
| **GPUI entity lifecycle, async re-entry** | [Pitfalls](#pitfall-prevention-rules) 5 (reentrancy), 10 (render cost) · zed at the pinned rev (Pitfall 6) |

## Workflow

### Branch & commit

- Work on the `main` branch unless the user asks for a feature branch.
- Keep commits small and reviewable. Prefer one logical change per
  commit.
- Only commit when the user asks. Never push to `origin` without an
  explicit request from the user.

### Pre-commit checks

Run these locally and make them pass before committing:

```sh
cargo fmt --all -- --check
cargo clippy -p ghostty_vt -p ghostty_vt_sys -p daruda_terminal -p daruda \
  -p daruda_config -p daruda_store -p daruda_agent -p daruda_update \
  -p daruda_acp -p daruda_core -p daruda_flow -p ferrum_flow \
  --all-targets -- -D warnings
./scripts/lint-inline-literals.sh
./scripts/lint-paint-scope.sh
./scripts/lint-reentrant-reads.sh
./scripts/lint-direct-gpui-component.sh
./scripts/lint-direct-ferrum-flow.sh
./scripts/lint-no-eprintln.sh
./scripts/lint-viewport-row-scroll.sh
./scripts/lint-platform-boundary.sh
cargo test -p ghostty_vt -p ghostty_vt_sys -p daruda_terminal -p daruda \
  -p daruda_config -p daruda_store -p daruda_agent -p daruda_update \
  -p daruda_acp -p daruda_core -p daruda_flow -p ferrum_flow -p gpui_component
./scripts/lint-no-silent-update.sh
./scripts/lint-agent-activity.sh
./scripts/lint-daruda-path-literals.sh
./scripts/lint-env-literals.sh
./scripts/lint-env-literals.sh --self-test
./scripts/lint-file-size.sh
./scripts/lint-mark-dirty-direct-call.sh
./scripts/lint-fold-header.sh
./scripts/lint-agent-list-sync.sh
./scripts/lint-declarative-context-menu.sh
./scripts/lint-acp-air-gate.sh
./scripts/lint-raw-mouse-button.sh
./scripts/lint-raw-mouse-button.sh --self-test
./scripts/lint-comment-length.sh
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps \
  -p daruda_flow -p daruda_core -p daruda_update -p ghostty_vt_sys \
  -p ghostty_vt -p daruda_agent
cargo run -p gen_acp_presets -- --check
cargo check -p daruda --features screenshot
```

If a hook fails, fix the underlying issue rather than bypassing it
(`--no-verify`, `-c commit.gpgsign=false`, etc. are off-limits unless
the user explicitly asks).

### Scope discipline

- Implement only what the current task explicitly requires. No
  speculative refactors or abstractions.
- When a refactor is needed to keep a change reviewable, do it in a
  separate commit with a clear reason.
- Don't leave half-finished code paths in the repo; if a change cannot
  land end-to-end, pause and surface the blocker instead.

### Change-impact discipline

Before editing a shared or public type, decide the *extension point* — not
just the change — and pick the one with the smallest blast radius.

- Before changing any shared/public type, function signature, or enum,
  grep all usages and call sites first. Don't assume isolated impact;
  state the blast radius (how many sites this forces you to touch).
- Prefer additive, backward-compatible changes (a new optional field /
  parameter, a new opt-in hook) over invasive ones (a new enum variant, a
  changed signature). A host-specific behavior belongs in an opt-in
  callback installed where it's needed — not in a shared enum that every
  exhaustive `match` must now grow an arm for.
- If a single logical change forces the *same mechanical edit* across many
  unrelated files (Shotgun Surgery — e.g. an identical no-op `match` arm
  added in N places), stop: the seam is wrong, redesign rather than push
  the churn through. Copy-pasting the same edit a second time is the tell.
- Look for an existing precedent in the same module before inventing a new
  mechanism, and mirror it only after checking *why* it's shaped that way
  (an existing enum variant is cheap to match; a *new* one is a breaking
  change to every consumer).
- When two designs are equally correct, the one that touches fewer sites on
  the next change wins (Correctness > Maintainability).

### Verification

- Provide explicit verification steps (commands + expected outcomes)
  for every non-trivial change.
- Do not claim to have executed a command unless the tool output for
  that execution is actually visible in the conversation.
- For UI-visible behavior (rendering, IME, window state), a test suite
  pass alone is not proof — call out what still needs manual
  verification.

### Documentation

- In-progress docs stay out of the repo: session handoffs, progress
  reports, and other short-lived notes do not belong in git.
- Design notes, plans, ADRs, and session handoffs go to a personal
  document store outside the repository, not `docs/` or anywhere in the
  repo.
- Repo-level `.md` is reserved for files that ship with the code
  (README, CHANGELOG, in-tree architecture docs the project explicitly
  maintains, license files). When in doubt, use the vault.

### Language

- Code, identifiers, comments, and Markdown documents: English only.
- Discussion with the user: whatever language the user uses.

## Project Guide

Run multiple AI coding agents in parallel in a single desktop window. Use separate git worktrees for independent tasks. Talk to an agent in an in-app **chat pane** over the Agent Client Protocol (ACP), or drive its CLI in a **terminal pane**. New chats open in the active `Lane`; chats in the same Lane share its working directory and branch. Macro buttons in the bottom dock send preset commands to any terminal with one click or a keyboard shortcut.

**Concept model**: `Workspace → N × Project (= git repo) → N × Lane`. A `Lane` is a worktree-like space — a checked-out branch (git worktree) or a plain directory — and is the unit a Claude session attaches to. Users see "Worktree" in the UI; "Lane" is the internal type.

**Multi-project workspace**: one window holds N `Project`s (each opened repo root). Projects can be bundled into user-defined `Group`s in the left dock; ungrouped projects render at the same rank as groups. The active focus is a single `LaneRef { project, lane }`, so cross-project state (`MainAreaContext` swap key, per-lane caches) is keyed by ref rather than lane id alone.

### Project layout

```
daruda/
├── crates/
│   ├── app/                  # main app binary (workspace, agent, ui, lane, surface)
│   ├── daruda_acp/           # Agent Client Protocol client core (GPUI-free)
│   ├── daruda_flow/          # declarative ACP flow engine (GPUI-free)
│   ├── daruda_core/          # shared dependency-free utilities + core logic
│   ├── daruda_config/        # config system (live reload)
│   ├── daruda_store/         # persistence + observability (NDJSON log)
│   ├── daruda_agent/         # agent provider integrations
│   ├── daruda_terminal/      # terminal emulation + GPUI rendering
│   ├── daruda_update/        # app update checking
│   ├── ghostty_vt/           # safe Rust wrapper over libghostty-vt
│   ├── ghostty_vt_sys/       # Zig C FFI bindings
│   ├── ferrum_flow/          # vendored node-graph canvas (do not lint/edit — see below)
│   ├── gpui_component/       # vendored gpui-component fork (do not lint/edit — see below)
│   └── visual_tests/         # offscreen render snapshot tests
├── tools/
│   ├── acp_replay/            # ACP wire-log replay helper
│   ├── gen_acp_presets/       # generated agent preset drift gate
│   ├── gen_licenses/          # generates third-party license manifest
│   └── vt_dump/               # diagnostic CLI
├── vendor/ghostty/           # Ghostty v1.2.3 submodule
├── vendor/zed/               # patched GPUI; regenerate with tools/vendor_gpui
└── scripts/
```

### Requirements

- **Rust**: 2024 edition (1.95.0+). The floor is declared once in `[workspace.package]` and every first-party crate inherits it with `rust-version.workspace = true`, so clippy's `incompatible_msrv` catches a newer std API at the call site. CI pins the toolchain to exactly 1.95, which is what actually enforces the floor — develop on a newer toolchain freely. The three vendored `gpui_component*` crates deliberately stay undeclared to keep the re-vendor diff a file copy; `ferrum_flow` does declare it, because its manifest is daruda-authored either way and the declaration is what arms `incompatible_msrv` there.
- **Zig**: 0.14.1 (`./scripts/bootstrap-zig.sh` on macOS/Linux, `./scripts/bootstrap-zig.ps1` on Windows x86_64; alternatively set `ZIG=<path>` or put `zig` on `PATH`)
- **macOS**: Apple Silicon or Intel + Xcode Command Line Tools — the primary, fully-verified target.
- **Linux**: built and tested by the `linux` CI job, which gates like the macOS one — the claim is green, not merely measured. GUI runtime (window/menu/tray) is still unverified on a real desktop, since CI has no one to look at the window. Needs system `libfontconfig`/`libxcb` and real fonts (the job installs them).
- **Windows**: native MSVC build via `cargo build --locked -p daruda` or `scripts/build-windows.ps1`. CI gates the build, app target compilation, Clippy, and native/platform tests; full runtime tests remain experimental. GUI runtime still needs desktop verification.

### Build

See [contributor setup](CONTRIBUTING.md#setup) for platform-specific Zig
installation and source builds, and [packaging](CONTRIBUTING.md#packaging)
for macOS app and DMG bundles.

### Tests & CI

```bash
cargo fmt --all -- --check
cargo clippy -p ghostty_vt -p ghostty_vt_sys -p daruda_terminal -p daruda \
  -p daruda_config -p daruda_store -p daruda_agent -p daruda_update \
  -p daruda_acp -p daruda_core -p daruda_flow -p ferrum_flow \
  --all-targets -- -D warnings
scripts/lint-inline-literals.sh
scripts/lint-paint-scope.sh
scripts/lint-reentrant-reads.sh
scripts/lint-direct-gpui-component.sh
scripts/lint-direct-ferrum-flow.sh
scripts/lint-no-eprintln.sh
scripts/lint-viewport-row-scroll.sh
scripts/lint-platform-boundary.sh
cargo test -p ghostty_vt -p ghostty_vt_sys -p daruda_terminal -p daruda \
  -p daruda_config -p daruda_store -p daruda_agent -p daruda_update \
  -p daruda_acp -p daruda_core -p daruda_flow -p ferrum_flow -p gpui_component
scripts/lint-no-silent-update.sh
scripts/lint-agent-activity.sh
scripts/lint-daruda-path-literals.sh
scripts/lint-env-literals.sh
scripts/lint-env-literals.sh --self-test
scripts/lint-file-size.sh
scripts/lint-mark-dirty-direct-call.sh
scripts/lint-fold-header.sh
scripts/lint-agent-list-sync.sh
scripts/lint-declarative-context-menu.sh
scripts/lint-acp-air-gate.sh
scripts/lint-raw-mouse-button.sh
scripts/lint-raw-mouse-button.sh --self-test
scripts/lint-comment-length.sh
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps \
  -p daruda_flow -p daruda_core -p daruda_update -p ghostty_vt_sys \
  -p ghostty_vt -p daruda_agent
cargo run -p gen_acp_presets -- --check
cargo check -p daruda --features screenshot
```

While iterating, `cargo test -p daruda -- --skip workspace::tests` is the
same suite minus the one module that dominates it: measured 2026-09-18,
`workspace::tests` is 388 of 2,863 app tests (13%) but ~74% of the wall time
(109s → 29s when skipped), because those tests build a real `Workspace` and
each tab they open spawns an actual shell. It is an iteration loop, not a
gate — the [pre-commit checks](#pre-commit-checks) still run everything.

`ci.yml` has a second job, `linux`, which builds and tests the same package list on `ubuntu-latest`. It runs no lint scripts — those read source rather than platform, and the macOS job already ran them. What it does carry is the `#[cfg]` arms the macOS job can never compile: its first three runs turned up two unused imports and a font-resolution defect that left every mermaid label blank on Linux, none of which macOS could have seen.

Note: `.github/workflows/ci.yml` gates fmt, the clippy list above, the 8 lint scripts through `lint-platform-boundary.sh`, `lint-env-literals.sh` with its self-test, `lint-no-silent-update.sh`, `lint-agent-activity.sh`, the `cargo doc` link check, and the package-scoped `cargo test` list above.

The doc-link gate covers six crates rather than all of them: clippy does not
read intra-doc links, so a deleted item leaves a dangling `[`Name`]` in the
prose that explains the module. These six are clean today; the rest carry a
backlog and join the list a crate at a time as that is worked off. Measured
2026-09-18: `daruda_config` 8, `daruda_store` 8, `daruda_terminal` 11,
`daruda_acp` 16, `daruda` 85 — the app crate is most of what is left. `lint-daruda-path-literals.sh`, `lint-file-size.sh`, `lint-mark-dirty-direct-call.sh`, `lint-fold-header.sh`, `lint-agent-list-sync.sh`, `lint-declarative-context-menu.sh`, `lint-acp-air-gate.sh`, `lint-raw-mouse-button.sh`, `lint-comment-length.sh`, `gen_acp_presets -- --check`, and `cargo check -p daruda --features screenshot` are local/reviewer checks not yet wired into CI.

That last one is why it is on the list at all. `screenshot` is off by default, so every item it reaches — the `*_for_shot` seams, `screenshot_scenario`'s two modules — looks unused to a build that does not enable it. A visibility-narrowing pass took that at face value and left the feature uncompilable for a while, with nothing to say so. `cargo check` with the feature on is the cheapest thing that notices.

`gen_acp_presets -- --check` is the ACP preset drift gate: it regenerates the `// BEGIN GENERATED` block of `crates/daruda_config/src/agent/preset.rs` from the committed `tools/gen_acp_presets/registry-snapshot.json` and fails on any difference. It is offline; `scripts/sync-acp-registry.sh` is the separate path that refreshes the snapshot from the live registry.

### Visual verification

Render the UI offscreen to a PNG and read it back — text, layout, colors, images, and toasts all render, permission-free (no Screen Recording grant). Capture goes through gpui's `render_to_image`, gated upstream behind `test-support`; the `--screenshot` path below requires it, plus `gpui_macos/font-kit` (without that feature glyphs don't rasterize — shapes render but **text is invisible**).

**Whole app** — the `--screenshot` flag captures the live workspace window:

```bash
cargo build -p daruda --features screenshot
target/debug/daruda --screenshot /tmp/shot.png   # opens, settles ~2s, captures, quits
```

The opt-in `screenshot` feature enables `gpui/test-support` + `gpui_macos/font-kit`; it is off by default to keep the shipping binary clean. Entry point: `crates/app/src/screenshot.rs`.

Verification loop: render → PNG → an agent reads the PNG and checks the result. This catches both rendering bugs and runtime state (e.g. error toasts), so it doubles as a smoke test of the real app's startup.

#### Driving the captured state

`--screenshot` takes no view argument — it captures the **restored** workspace (or welcome screen), so you steer it by steering what gets restored *before* launch. Two isolation levers + three reach tiers:

- **`DARUDA_DATA_DIR=<dir>`** — points the whole state dir at `<dir>` (verbatim). Pre-seed it; isolates from your real workspace.
- **`DARUDA_PROFILE=<name>`** — `release` → `daruda/`, else `daruda-<name>/`. Debug builds already use `daruda-debug/`, so runs are isolated by default.

```bash
export DARUDA_DATA_DIR=/tmp/daruda-shot-state   # throwaway, pre-seeded state
cargo build -p daruda --features screenshot && target/debug/daruda --screenshot /tmp/shot.png
```

**Tier 1 — `config.toml` (appearance, mostly live-reload):** `[theme]` (terminal_preset / ui_preset), `[colors]`, `[font]` (size/spacing/inset), `[cursor]`, `[window]`, `[file_viewer]`, `[left_dock]`, `[panels]`, `[render]`, etc. Does **not** control which tab/pane/view is active — that's Tier 2.

For the UI theme specifically, `--screenshot-theme <light|dark>` overrides `ui_preset` at capture time (via `apply_ui_theme`) without touching config — orthogonal to and composable with `--screenshot-scenario` (e.g. shoot the command palette in light mode). Pass a **comma list** (`light,dark`) for a batch: one PNG per theme in a single launch (output names get a `.<theme>` suffix — `shot.png` → `shot.light.png` / `shot.dark.png`), the scenario stays applied and is re-themed in place between captures.

```bash
target/debug/daruda --screenshot /tmp/s.png --screenshot-theme light --screenshot-scenario command-palette
target/debug/daruda --screenshot /tmp/s.png --screenshot-theme light,dark   # batch: s.light.png + s.dark.png
```

Two more capture knobs (compose with everything above):
- `--screenshot-size WxH` — fix the captured window size (e.g. `1280x800`) instead of the restored bounds; for stable doc / pixel-regression shots.
- `DARUDA_SCREENSHOT_SETTLE_MS=<ms>` — override the 2 s post-launch settle (raise on slow CI / big workspaces, lower for quick local shots).

**Tier 2 — persisted state under `DARUDA_DATA_DIR` (layout & structure):** `workspaces/<uuid>.json` (`WorkspaceState`: dock open/size, window bounds, active project+lane, `active_dock_view`, `active_right_panel_view`, focused pane, groups), `projects/<uuid>.json` (`ProjectState`: lanes, tabs, pane split tree, file-pane path + view_mode), `panels.json` (macro grid), `tasks.json` (task list). (Schemas in `daruda_store::project::{WorkspaceState,ProjectState}`.) **Easiest seeding: set `DARUDA_DATA_DIR`, drive the app by hand to the scenario once, quit, re-run with `--screenshot` against the same dir** — no schema guessing.

**Tier 3 — transient / live state: NOT reachable by config or state alone.** Restore recomputes these fresh. The `--screenshot-scenario <name>` flag drives one transient overlay into view after settle and before capture (it forces a repaint since `render_to_image` captures the last *painted* frame). Implemented scenarios: `command-palette`, `lane-switcher` (a real lane whose label is swapped for one wide enough to overflow the popup, so clipping is visible), `error-modal`, `settings` / `settings:<section-slug>` (slug = `BuiltinSection::slug`, e.g. `font`, `keymap`, `notifications`), `settings-error` (the one banner every failed Settings action reports through — a unit test can only see the field behind it), `toast`, `pane-context-menu`, `mermaid-lightbox`; agent chat: `agent-chat`, `agent-chat-working` (mid-turn — the run's last prose sits inside a step, the one shape the settled seed cannot reach), `agent-chat-narrowed`, `agent-chat-fold`, `agent-chat-interrupted` (a run a Stop cut, closed by the marker), `agent-chat-queue-parked` / `agent-chat-queue-armed` (the queued-prompt strip holding a queue a Stop parked, before and after an empty-composer Enter arms the resume gesture — a restored pane has no queue, so the strip is reachable no other way), `agent-chat-sole-reply` (prose-only replies — the `/usage` shape — with the first reply's own fold shut and the second open; such a run renders one block, which the response bar cannot fold), `agent-chat-running-tool` (a tool card mid-call, the one badge state a settled seed cannot reach and the only one carrying a number), `agent-chat-plan` / `agent-chat-plan-stopped` (the plan region mid-run and after a Stop — its four status icons carry meaning by shape, which only a capture can judge), `agent-chat-tail` / `agent-chat-tail-open` (the tail window's boundary row closed / open, nothing floating over it), `agent-chat-group-tail` / `agent-chat-group-tail-open` (the same boundary one level in — a tool group holding more calls than the window keeps, closed / open), `agent-chat-subagent-tail` / `agent-chat-subagent-tail-open` (the same boundary inside a subagent card, whose flattened children own no row, closed / open), `agent-chat-empty`, `agent-chat-failure`, `agent-chat-options[:<tab>]` (tab = `ActivityOptionsTab::token` — `fold`, `filter`, `recent-steps`; the compact Activity Bar with the combined popover on that tab and every axis adjusted); flows: `flow-picker`, `flow-profile-picker`, `flow-running`, `flow-asking`, `flow-resumable`, `flow-graph`, `flow-graph-running`, `flow-graph-form`, `flow-graph-form-refused`, `flow-graph-pinned`, `flow-graph-authoring`, `flow-delete-confirm`. Agent-chat scenarios are seeded through `AgentChatView::seed_transcript_for_shot`, so no ACP session is needed. Add a variant to `ScreenshotScenario` + `screenshot_scenario::drive` (`crates/app/src/workspace/screenshot_scenario.rs`) to cover more. The rest below still need a real backing source, not just a scenario:

```bash
target/debug/daruda --screenshot /tmp/shot.png --screenshot-scenario command-palette
target/debug/daruda --screenshot /tmp/s.png   --screenshot-scenario settings:font
```

| State | On restore |
|---|---|
| Modals, command palette, Settings | closed — use `--screenshot-scenario` (`error-modal`, `command-palette`, `settings[:<slug>]`, `settings-error`) |
| Toasts | empty queue — use `--screenshot-scenario toast` |
| Drag / hover / text selection | gone (real-time only) |
| Terminal output | fresh shell prompt — scrollback is **not** persisted |
| File-viewer content / git-changes | reloaded from disk — needs **real files / git repo** |
| Usage / Skills / Tools panels | fetched fresh (net / FS / MCP); cold → placeholder |
| Agent-chat transcript | not persisted — `SerializedAgentChatContent` stores only the pane's cwd / `session_id` / title / agent + account / view preferences, and a restored pane is refilled by the adapter replaying the conversation on `session/load`. For a capture, use the `agent-chat*` scenarios: they seed a fixed transcript and need no session |
| Live Claude session badge | in-memory — needs a real attached session |

### Coding Best Practices

**Design reference**: before any UI/design work or refactoring/improvement pass, read [`DESIGN.md`](./DESIGN.md) — the design language (colors, surface ladder, typography, accent rules, component chrome). Align changes with it.

**Priority order** when trade-offs arise: Correctness > Maintainability > Performance > Brevity

**Root-cause priority**: fix the root cause, not the symptom — never ship a band-aid when the underlying defect is reachable.
- When a workaround is the only thing that comes to mind, that is the signal to stop and surface the root problem **first**: name the actual defect, where it lives, and why the symptom appears — then decide whether to fix it or, only with explicit reason, defer.
- A workaround is acceptable **only** when the root cause is out of scope (upstream dependency, separate subsystem, or deliberately deferred). In that case, mark it inline (`// WORKAROUND: <root cause> — <why deferred>`) and report it; never let it pass silently as if the problem were solved.

#### Task Complexity Assessment
Before starting, classify the task:
- **Trivial** (single file, obvious change) → execute immediately
- **Moderate** (2–5 files, clear scope) → brief plan then execute
- **Complex** (architectural impact, ambiguous requirements) → full research first

#### Architectural Constraints
- Before adding any feature, determine which layer owns the responsibility
- Before changing a business rule, grep all downstream consumers, verify the change is valid for each, and report before proceeding

#### Anti-patterns
- ❌ Multiplying `if` branches for quick fixes — prefer polymorphism or the strategy pattern
- ❌ A type with more than one reason to change (SRP violation)
- ❌ Bypassing existing abstractions with direct calls (breaks encapsulation)
- ❌ `bool` flag + an `Option`/value field that is only meaningful when the flag is `true` — declare them as separate fields
  → `enum { Inactive, Active { data } }` to make the invalid state unrepresentable
- ❌ `match (a, b) { ... _ => unreachable!() }` — a hidden state machine encoded as bool combinations
  → replace with an `enum` whose variants cover only valid states
- ❌ Two `Option` fields that are always `Some`/`None` together
  → `Option<(A, B)>` or a dedicated struct
- ❌ The same group of fields set directly across multiple call paths
  → extract a single `fn` and seal the fields with `pub(super)` — two or more call paths copying the same N-step sequence is the extraction signal

#### MVU-flavored guiding rules

Daruda is not strict MVU, but the architecture leans on three rules. Treat them as the default; deviate only with a `// SAFETY:`-style comment that names the exception.

- **View purity** — `render()` and the event-handler closures it builds must not carry state-transition logic. Closure bodies are one-line dispatches: `weak_ws.update(cx, |ws, cx| ws.method(args, cx))`. Same Model → same screen. *Exception*: layout-geometry caching inside `canvas()` (bounds, hitbox, scroll offsets) is allowed — GPUI requires it. Don't smuggle Model changes through this exception.
- **One-way data flow** — Views dispatch; only `Workspace` (and `*_ops.rs`) modify Model. "Modify" covers every state-change verb — `add_*`, `remove_*`, `set_*`, `insert_*`, `delete_*`, `clear_*`, `toggle_*`, `update_*`, `open_*`, `close_*`. The View calls one of these by name; the body lives in `Workspace` / `*_ops.rs`, not in the closure. A View must not reach across entities to write child state directly.
- **Single source of truth** — When state is mirrored (e.g. config → cached field), there is exactly one update site. Adding a new mirror means extending that one entry point — not a parallel sync path.

### Development rules

- **License**: AGPL-3.0-only.
- **Language**: all code, comments, and identifiers in English.
- **Tests**: every new module needs `#[cfg(test)] mod tests`.
- **Error handling**: custom error types + `Display`. `unsafe` requires `// SAFETY:` comment.
- **GPUI dependency**: only view/UI code may import GPUI. PTY, config, git stay GPUI-free.
- **Workspace per-lane state**: data discarded on lane/project teardown belongs in `workspace/lane_scoped.rs::LaneScoped`; keep `LaneRuntime` and `FlowRuns` in their separate lifecycle containers.
- **`gpui_component` access**: app code must go through `crate::ui::*`; direct imports forbidden. See `crates/app/src/ui/CLAUDE.md`.
- **Commit only when explicitly asked** — never `git add`/`git commit` without direct instruction.
- **Commit messages**: `<type>: <subject>` (imperative, ≤72 chars). Types: `feat` `fix` `refactor` `perf` `test` `chore` `ci` `docs`. Body only when WHY is non-obvious. Prohibitions: no Phase/Step/ticket numbers, no "what I did" lists (diff shows that), no future-work notes.
- **User-facing values go through config** (`daruda_config`). Pixel/color constants → `ux/theme.rs`.
- **User-facing strings go through i18n** — every string visible to the user must be a `pub fn` in `surface/strings.rs` backed by a key in `crates/app/locales/en.yml` (+ matching key in `ko.yml`). Never embed raw string literals at call sites. See `crates/app/locales/CLAUDE.md` for the full checklist.
- **Comments**: current logic only. No history, no "used to be X". Keep each to 2-3 lines — summarize, don't explain at length. Don't restate what's already verifiable by reading the code (e.g. what a well-named function/variable does); only note the non-obvious WHY.
- **In-progress docs**: keep outside the repo in a personal document store.

### File-structure rules

- One `.rs` file = one responsibility. GPUI-free and GPUI-dependent code in separate files.
- Split order: multiple responsibilities → by domain; tests ≥ 40% → `tests.rs`; single domain > 300 lines → directory module.
- `impl Render` lives in its own file. `actions!()` macros stay at `mod.rs`.

### Pitfall-prevention rules

1. **Coordinates**: never mix byte offsets with grid coordinates. Always convert window coordinates via `mouse_position_to_local()`.
2. **Magic numbers**: escape bytes, codes, buffer capacities, colors, pixels, and strings belong only in their designated files (`ansi.rs`, `vt_codes.rs`, `vt_limits.rs`, `theme.rs`, `strings.rs`, `constants.rs`, `keybindings.rs`).
3. **Zig FFI**: Ghostty enums are `u16`. Always range-check before casting.
4. **IME**: printable characters must go through `replace_text_in_range` → `commit_text` → PTY. Never send directly from `on_key_down`.
5. **GPUI Entity reentrancy**: calling `.read(cx)` on the same entity during `render()` or `entity.update()` panics. `persist_state` must only be called via `mark_dirty_and_save` (`cx.defer`).
6. **Reference comparison**: before adding a feature or fixing a bug, check how Alacritty, iTerm2, **zed** (`gpui` itself is `vendor/zed/crates/gpui/` — the pinned rev with daruda's patches applied, which is what actually compiles; the rest of zed at that rev, `workspace`, `gpui_macos`, `gpui_platform`, is the cargo checkout `~/.cargo/git/checkouts/zed-a70e2ad075855582/193b55a/crates/`), and gpui-ghostty implement the same concept. For GPUI-specific patterns (entity lifecycles, window contexts, async re-entry) zed is the closest reference; always read the version-matched source above rather than a standalone clone of a different version.
7. **Text pixel mapping**: never use `index = offset_px / glyph_advance`. Always use the shaper's reverse-mapping API.
8. **Paint-scope state**: `window.text_style()` / `window.rem_size()` are invalid outside the paint walk. Share metrics via `cell_dimensions()`.
9. **Color palette**: `daruda_terminal/src/ux/theme.rs` uses a local `hsla()` with hue in degrees (0–360). `app/src/ui/theme.rs` is the gpui_component bridge using fractions (0–1). Never call `gpui::hsla` from the terminal theme file.
10. **Render-cost containment** (`window.refresh()` ban + cache rules): GPUI has **no partial redraw** — any dirty view repaints the whole window tree, and cost scales with node count. Two rules keep that cost contained:
    - **Never call `window.refresh()` / `cx.refresh_windows()` on a hot path.** Refresh sets `window.refreshing`, which **bypasses every `AnyView::cached`** for that frame (see gpui `view.rs` prepaint `!window.refreshing` guard). It is reserved for genuinely global invalidation (theme swap in `ui/theme.rs`). For everything else use **targeted `cx.notify(entity)`** so only that view subtree (and its ancestors) goes dirty and sibling `.cached()` views stay cached. Reference: zed PR #25009.
    - **Caching a child view requires notify-on-change.** A view that renders from a parent-staged snapshot (e.g. `Dock::snap`) must be marked dirty (`cx.notify(child)`) when that snapshot's content changes, or `.cached()` will show stale data. Self-notifying views (TerminalView, ToastLayer) are already safe. Bare `entity.update(cx, |e, _| e.field = …)` without notify is incompatible with caching that entity.
11. **Agent-chat single activity source**: every "is the pane working / did it just finish" decision reads `activity_state()` / `is_busy()` / `activity_elapsed()` on `AgentChatView` — never the raw prompt `Turn`, which settles busy→idle before trailing background subagents finish. Completion side effects (notification + backing-task done) fire only via `fire_activity_completion` at the busy→idle settle edge that `reconcile_activity` detects — never straight from an `AcpEvent::TurnEnded` / `AcpEvent::Error` arm (early + double-fire). `Turn` is module-private to the `agent_chat_pane/view/` module tree (`mod.rs` + its `apply_event.rs` / `queue_ops.rs` / `session_ops.rs` / `tests.rs` submodules) for prompt-queue sequencing only; tests reach it through the `#[cfg(test)]` hooks (`set_turn_in_flight` / `set_turn_idle` / `turn_is_idle`). Enforced by `scripts/lint-agent-activity.sh`. Stop (`cancel_turn`) settles the turn locally and immediately (responsive + hung-safe) and stashes `Stopped`; the turn moves to `Turn::AwaitingCancelAck`, and the one `cancelled` `TurnEnded` that state expects is **swallowed** by `apply_event` so a stale cancel-ack can't be misattributed to a turn the user re-prompted (the stop-then-reprompt race). A prompt typed inside that window buffers client-side rather than going on the wire, which is what `Turn::can_dispatch` answers — the cancel window and the in-flight turn are one value, not a bool beside an enum. This is sound because `daruda_acp::session.rs` is strictly FIFO — 1 prompt → 1 `TurnEnded`, in order, and a hung turn blocks all later ones.

12. **Raw mouse listeners name their button**: `window.on_mouse_event::<MouseDownEvent>()` hears *every* button, unlike `div().on_click()` (left-only by construction — gpui's `elements/div.rs` routes the rest to `on_aux_click`) and `.on_mouse_down(MouseButton::X, ..)`. A hand-rolled `Element` has no `div`, so it can only use the raw tier, and forgetting the filter is silent: a right click opened agent-chat links, jumped any scrollbar (swallowing the host context menu with it), and dropped a live flow-editor wire. Check `event.button`, or mark a genuinely button-agnostic listener `// ANY-BUTTON: <reason>` — the ones that qualify either *end* a drag or dismiss on an outside press. Enforced by `scripts/lint-raw-mouse-button.sh`, which scans the vendored crates too, since that is where all three bugs lived and a re-vendor drops the gates. Its `--self-test` drives the shapes a naive detector misses (turbofish, path-qualified type, no `move`, a `MouseButton` mentioned only in a comment, a brace inside a string, a braceless listener ahead of a real one) so the guard cannot go quietly green.

### Error reporting

`eprintln!` is forbidden in new code. All failures must go through the 3-layer pipeline (toast → details modal → NDJSON log).

| Scope | API |
|-------|-----|
| Inside `Workspace` | `self.report_error(report, cx)` |
| GPUI-free / pre-Workspace | `LogWriter::log(report)` |

**GPUI Result handling**: `cx.update_window` / `cx.update` / `entity.update` / `entity.update_in` return `Result<T>` (the target window/entity can be gone by the time async or modal-callback code re-enters). `let _ = cx.update_window(...)` is **forbidden** — it silently swallows "window not found" failures and leaves users with no signal. Required forms:

- `match cx.update_window(handle, ...) { Ok(_) => …, Err(e) => report_error / LogWriter::log }`
- `cx.update_window(handle, ...)?` (when the enclosing fn returns `Result`)
- `crate::windows::try_update_workspace_window(handle, cx, "site_label", |window, cx| …)` — auto-logs failures with the site label
- `// SILENT-OK: <concrete reason>` on the previous line — reserved for cases where the failure genuinely doesn't matter (focus restore on a possibly-closed window, test fixtures, etc.). Bare `// SILENT-OK:` without a reason is a review failure.

Enforced by `scripts/lint-no-silent-update.sh`.

Reference: `crates/app/src/workspace/error_ops.rs`, `crates/daruda_store/src/observability/`

### Architecture & data flow

```
GPUI event loop (Metal on macOS)
  └── app/workspace/
        └── daruda_terminal  →  ghostty_vt (Zig FFI)  →  pty.rs (PTY)

Output: Shell → PTY → stdout_rx → 16ms batch → TerminalSession → ghostty_vt → GPUI paint
Input:  GPUI KeyDown → TerminalInput → stdin_tx → PTY → Shell
```

#### Crate dependency graph

```
daruda (app)  →  daruda_terminal  →  ghostty_vt  →  ghostty_vt_sys
             |                   →  daruda_core
             →  daruda_config     →  daruda_store  →  daruda_core
             →  daruda_store
             →  daruda_agent      →  daruda_store
             →  daruda_acp        →  daruda_core    # GPUI-free ACP client core
             →  daruda_flow       →  daruda_acp, daruda_core
             →  daruda_core                         # shared, dependency-free
             →  daruda_update
             →  ferrum_flow                         # vendored flow graph canvas
             →  gpui, gpui_component, merman, portable-pty

gpui_component  →  daruda_core                         # vendored; shares the text primitives
```

`gpui_component` is a vendored copy of `longbridge/gpui-component` (Apache-2.0), forwarded as-is so re-vendoring stays a pure file copy — it is excluded from clippy/lint/comment-cleanup passes; app code reaches it only through `crate::ui::*` (see "`gpui_component` access" above). Its **tests** are a separate question and do run in CI: the patches in `patches/README.md` carry daruda-authored tests, and nothing else would ever execute them.

`ferrum_flow` is a vendored copy of `tu6ge/ferrum-flow` at `43b762ce` (Apache-2.0) — the node-graph canvas behind the flow editor. Vendored for the same reason and on the same terms: patched only where daruda hits a real defect, excluded from the same lint passes, reached only through `crate::ui::*`. It carries five source patches (`viewport.rs` — culling against a not-yet-measured drawable blanked the canvas on its only frame; `canvas.rs` — a read-only `viewport()` accessor so a test can assert where the canvas put the graph; and three in `plugins/port/interaction.rs` — a dragged wire coloured only its refusal so a port that would take the drop looked exactly like empty space, a release over empty space left a dangling line whose endpoint built a node the flow file has no place for, and a release re-ran the hit test against a smaller box than the one the wire's colour came from); like `gpui_component`'s, they live in the vendored tree and are listed in `patches/README.md`. Upstream takes `gpui` from crates.io, which resolves to a different crate instance than daruda's pinned zed git rev, so routing it through `gpui = { workspace = true }` is the whole point. It differs from `gpui_component` in one way: it declares `rust-version` and leaves `clippy::incompatible_msrv` armed, so a std API newer than CI's pinned toolchain is caught locally. Provenance, the re-vendor procedure, and why the unused plugin modules are kept rather than pruned (measured: 17% fewer lines buys 0.02s of compile time and costs three retained files on every re-vendor) are in `patches/README.md`.

`merman` (crates.io, MIT/Apache-2.0) is the mermaid → SVG renderer behind every ` ```mermaid ` fence in the file viewer and agent chat, pulled as a normal dependency (not vendored — no daruda-side patch needed) and validated against upstream Mermaid.js SVG baselines. Host theming goes through merman's renderer-level site config, not a document directive: `file_view_pane::mermaid_host_theme` builds the profile from a `MermaidPalette` and `visual::render_mermaid_svg` applies it with `with_host_theme`. A source that sets `theme` / `themeVariables` / `themeCSS` — in an `%%{init}%%` directive or under a leading `---` frontmatter block's `config:` mapping, the two places mermaid accepts a theme — opts out of daruda's **colours** (`source_declares_own_theme`), on the reasoning recorded there that merman's site config overrides the document's own config wholesale rather than merging per-field; a source that only tunes layout or behaviour keeps the host chrome. The opt-out stops at colour: `mermaid_render_profile` is applied unconditionally because the same profile also carries `htmlLabels: false` and the `ResvgSafe` pipeline, and resvg cannot paint the `<foreignObject>` labels merman emits without them — a diagram rendered profile-free comes back as boxes and arrows with every label blank. Both hosts (`file_view_pane::file_content` and `agent_chat_pane::reconcile`) reach it through `visual::render_mermaid_raster` → `merman::render::HeadlessRenderer::render_svg_sync`.

Interactive Markdown has two independent rendering stacks. The file viewer uses `pulldown-cmark` to build its workspace-owned `MdBlock` / `MdSpan` IR and renders it in `file_view_pane/render/markdown/` (`prose.rs` compiles a span run, `inline.rs` / `block.rs` / `image.rs` render it, `selection.rs` carries the block-selection plumbing); agent chat goes through `crate::ui::markdown`, a wrapper over the vendored `gpui_component::text::TextView` and its mdast parser. A behavior intended for both hosts must be implemented in both stacks and added to the shared rendered-conformance table in `file_view_pane/render/markdown/layout_tests.rs`. Parser coverage for the file-viewer IR lives beside `parse_markdown` and must include semantic field states (`checked`, `loose`, ordered start), not enum variants alone.

`daruda_config` and `daruda_agent` both depend on `daruda_store` for `persistence::default_data_dir()` (see Cross-profile data isolation below).

`daruda_core` sits below everything so knowledge needed on both sides of the GPUI boundary has one home — the app can reach every crate, but the GPUI-free crates cannot reach the app. Admission is deliberately narrow (a "core" name otherwise becomes a junk drawer). Because every consumer points here and this crate points at none of them, the dependency rule is **directional, not a count**: no `daruda_*` dependency (that inverts the layering), and never `gpui` (which would put the crate back out of reach of the GPUI-free crates it exists to serve). Weigh any other external dependency against the fact that every consumer inherits it and `daruda_acp` is deliberately light — prefer a target-gated one, which costs the platforms that do not need it nothing (`libc` is here on `cfg(unix)` for that reason); `serde` would qualify on weight alone and stays out until something here needs it. Modules are **pure by default**: values in, values out, no filesystem/network I/O or hidden caches — with two named exceptions. `process_env` is a read boundary: its opaque `Key` type permits reads of registered daruda-owned names only, and it never writes or caches, because `set_var` is unsafe once other threads may access the environment (bootstrap writes stay where the caller can prove the process is single-threaded). The **platform capability** modules call the OS, because containing those calls is what they exist for — see [Platform capability boundary](#platform-capability-boundary). `scripts/lint-env-literals.sh` enforces single spelling in first-party Rust, including examples and tests. Current contents: `process_env` — the environment-name registry and read boundary; `language` — file extension → source language *identity*, shared by the file viewer's highlighter and the ACP adapter's fenced-output rewriter; `text` — UTF-8 word / logical-line expansion and the selection cell hit-test, shared by the vendored editor widget and the app; `git` — ref-naming rules as pure predicates, shared by the task store's silent filter and the app form's inline diagnostic; `process`, `path`, `shell` — the platform capability gates. Whether a language can actually be highlighted is a separate, registry-dependent question the app answers in `crate::ui::highlighter`.

#### Cross-profile data isolation

**Rule: any path or identifier for something daruda itself writes and reads back across restarts must go through `daruda_store::persistence::default_data_dir()`** (or `profile_suffix()` for a non-path identifier, e.g. a Keychain service name) — never a fresh `dirs::config_dir()` / hardcoded `daruda` directory literal.

**Why this is called out explicitly:** four separate places independently re-derived a `daruda`/`.daruda` path instead of calling the shared resolver — `daruda_config::config_path`, `daruda_config::project::project_config_dir`, `daruda_agent::hooks::status_file::default_dir`, and `workspace::sync::limits::activity_paths`'s cache path — so a debug build silently read and overwrote a real release install's `config.toml`, hook-status files, and activity cache. A fifth case (the Telegram bridge's Keychain-stored bot token sharing one service name across profiles) caused two profiles to 409-conflict polling Telegram with the same token, since Telegram's `getUpdates` rejects a second concurrent poller on one token. Each was fixed independently before the pattern was named — this section and the guardrails below exist so the next one is caught before it ships, not after a live incident.

**The deliberate exceptions, and what makes one.** Three paths under
`daruda_store::persistence` are profile-**independent** on purpose, and each
is named there with its reasoning:

- `node_install_dir()` — a pinned Node.js runtime is tens of MB and the same
  bytes for every profile, so one install serves all of them.
- `flow_lock_root()` — a flow run's lock is not daruda's state at all; it is a
  mutex on something every profile shares, the user's working tree. A release
  build and a debug build running flows in one checkout have to exclude each
  other, so isolating the lock per profile would defeat it.
- `remote_lock_root()` — the same shape, one layer out: a claim on the bot
  account, which is not daruda's either. Telegram serves `getUpdates` to one
  poller per token and Slack hands each event to one of an app's open sockets,
  so a release install and a debug build pointed at one bot must exclude each
  other or the user's replies get split between two routing tables.

The test is what the path *is*, not where it lives: state daruda writes and
reads back is profile-scoped, and a claim on a resource outside daruda is not.
Adding a fourth means adding it to `clippy.toml`'s allow reasoning too.

**Enforcement:**
- `clippy.toml`'s `disallowed-methods` bans a bare `dirs::config_dir` call outside `daruda_store::persistence`'s own call sites (each marked `#[allow(clippy::disallowed_methods)]` with a comment).
- `scripts/lint-daruda-path-literals.sh` greps for a hand-rolled `.join("daruda")` / `.join(".daruda")` outside the canonical files (`persistence.rs`, `profile.rs`, `observability/log_writer.rs`) and a short, explicit allow-list of genuinely non-profile-scoped exceptions (the per-repo `.daruda/task-*.md` files, the single global `~/.daruda/hooks/notify.sh`).
- Neither tool catches a hardcoded Keychain/OS-credential-store service name (not a directory path) — review any new one by hand against `crates/app/src/telegram/keychain.rs`'s `service_name()`.

#### Platform capability boundary

**Rule: an OS call that differs by platform lives in exactly one place, and domain code calls it by name.**

| Instead of | Call |
|---|---|
| `Command::new` | `daruda_core::process::command` |
| `libc::kill*` / `process_group` | `daruda_core::process::{lead_own_group, kill_tree}` |
| `fs::canonicalize` | `daruda_core::path::{canonicalize, canonicalize_or_self}` |
| `env::var("SHELL")` | `daruda_core::shell::interactive` |
| `std::os::unix::fs::symlink` | `daruda_core::path::symlink` |

**Why this is called out explicitly:** the same call kept being written out per crate. Killing a child's process tree was spelled four times across `daruda_acp`, `daruda_agent` (twice) and `daruda_flow`, in two spellings of one POSIX call (`killpg(pid)` and `kill(-pid)`); `create_owner_only_dir` existed verbatim in two files; the skills, flow and MCP watchers had each defined their own `canonicalize_or_self`; and `account_login_ops.rs` rebuilt a `PATH` with a hardcoded `:` next to a correct `join_paths` helper ten lines away in the crate it was calling. Each was fine alone. Together they meant a second platform would be written four times, which is [Shotgun Surgery](#change-impact-discipline) — the seam being wrong rather than the work being doubled.

**The two allowed regions:**
- `daruda_core`'s capability modules (`process`, `path`, `shell`) — the gates. `daruda_core` is otherwise pure-by-default; these are the named exception, stated in its `lib.rs`.
- `crates/app/src/platform/` — capabilities needing a window handle, which a GPUI-free crate cannot hold.

Plus three files that are gates of their own, each the single door to its capability: `app/src/remote_channel/keychain.rs` (daruda's own secrets), `daruda_agent/src/accounts/credentials.rs` (an entry *another program* owns), `app/src/shell_env.rs` (a macOS-only `.app`-launch PATH problem, not the "which shell" question).

**Prefer a value over a `cfg`.** `#[cfg(windows)]` code never compiles on a macOS dev machine, so it is only ever checked by CI. Decide the platform once at the boundary with `cfg!()` and pass the answer down as a value — `daruda_core::shell::login_args_for(program)` answers "does this shell take `-l`?" from the program name, so a Windows shell's rules are asserted from macOS. Reserve `#[cfg]` attributes for the leaf that actually calls the OS.

**Adding a platform** = adding an arm inside the boundary. If it means touching domain crates, the capability is in the wrong place.

**Enforcement:** `scripts/lint-platform-boundary.sh`. Deliberately a grep rather than `clippy.toml` — `disallowed-methods` has no per-file exception, so under `--all-targets` it would also catch test fixtures spawning `git init`, which have no reason to go through the gate.

#### UI component hierarchy

```
Workspace
├── TitleBar
├── BodyLayout
│   ├── LeftDock
│   │   ├── ViewSwitcher             — Worktrees / Git / Files tab strip
│   │   ├── WorktreesView            — 2-level tree: Group ▸ Project ▸ Worktree
│   │   │   ├── GroupHeader          — collapsible accordion (color, name, ▼/▶, context menu)
│   │   │   ├── ProjectHeader        — project row (inside a group, or ungrouped at top rank)
│   │   │   └── WorktreeRow          — leaf (create / delete / merge actions, Claude badges)
│   │   ├── GitChangesView
│   │   └── FilesView
│   ├── MainArea
│   │   ├── TabBar
│   │   ├── PaneTree                 — one per tab; recursive split tree (single pane = 1-leaf root)
│   │   │   └── Pane                 — leaf node, single content slot
│   │   │       ├── PaneHeader       — per-pane title bar, visible in split mode only
│   │   │       └── PaneContent
│   │   │           ├── TerminalPane — PTY terminal (TerminalTextElement)
│   │   │           ├── FileViewPane — file viewer (toolbar + virtual list)
│   │   │           └── TaskEditPane — task edit inline form
│   │   └── BottomDock
│   │       ├── DockSwitcher         — BottomDock top tab row
│   │       ├── TerminalInputDock    — multiline input + submit / action buttons
│   │       └── MacroDock            — N-column macro key grid
│   │           └── MacroKey         — macro key (icon or text mode)
│   └── RightDock
│       ├── ViewSwitcher             — Usage / Skills / Tasks / Tools tab strip
│       ├── UsageView
│       ├── SkillsView
│       ├── TasksView
│       └── ToolsView
├── StatusBar
├── ModalLayout                      — modal overlay container (absolute-positioned)
│   └── ModalView
└── ToastLayout                      — toast notification container (absolute-positioned)
    └── ToastView
```

**Concept-to-code mapping** — identifiers match the hierarchy names above:

| Hierarchy name | Code identifier | Location |
|---|---|---|
| `LeftDock` | `left_dock` field / `Dock` entity (position=Left) | `workspace/left_dock/`, `workspace/layout/mod.rs`, `render/mod.rs` |
| `RightDock` | `right_dock` field / `Dock` entity (position=Right) | `workspace/right_dock/`, `workspace/layout/mod.rs`, `render/mod.rs` |
| `ViewSwitcher` | `render()` in `view_tabs.rs` (both docks) | `left_dock/view_tabs.rs`, `right_dock/view_tabs.rs` |
| `DockSwitcher` | `PanelTabStrip` | `main_area/bottom_dock/tab_strip.rs` |
| `TerminalInputDock` | `TerminalInputPanel` | `main_area/bottom_dock/terminal_input.rs` |
| `MacroKey` | `MacroKey` | `ui/macro_key.rs` |
| `PaneTree` | `PaneLayout` enum | `workspace/main_area/pane_tree.rs` |
| `Pane` | `PaneLayout::Pane` | `workspace/main_area/pane_tree.rs` |
| `TerminalPane` | `PaneContent::Terminal` | `workspace/main_area/pane.rs` |
| `FileViewPane` | `PaneContent::File` | `workspace/main_area/pane.rs` |
| `TaskEditPane` | `PaneContent::TaskEditPane` | `workspace/main_area/pane.rs` |
| `ToastLayout` | `toast_layer: Entity<ToastLayer>` | `workspace/toast_layer/mod.rs` |
| Project (runtime) | `crate::project::Project` | `crates/app/src/project/mod.rs` |
| Group (runtime) | `daruda_store::project::SerializedGroup` (used directly — no separate runtime newtype) | `crates/daruda_store/src/project/` + `workspace/group_ops.rs` |
| Lane (runtime) | `crate::lane::Lane` (was `Worktree`) — UI label remains "Worktree" | `crates/app/src/lane/mod.rs` |
| Lane (persisted) | `daruda_store::project::SerializedLane` + `LaneKind { Git { .. }, Default }` | `crates/daruda_store/src/project/lane.rs` |
| Active focus ref | `daruda_store::project::LaneRef { project, lane }` — JSON keys remain `worktree` via `#[serde(rename = "worktree", alias = "lane")]` | `daruda_store/src/project/`; per-lane caches keyed by ref in `workspace/mod.rs` |
| `ProjectsView` 2-level tree | `TopRow` enum dispatch + `group_header_row` / `project_header_row` / `worktree_row` (function name retained — UI affordance) | `workspace/left_dock/projects/rows.rs` |
| Multi-project DnD | `DragPayload { Worktree | Project | Group }` + `dnd_ops.rs` reorder pool | `workspace/left_dock/projects/drag.rs`, `workspace/dnd_ops.rs` |
