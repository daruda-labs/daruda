# daruda

Run multiple AI coding agents in parallel — each in its own `Lane` (a git worktree-backed workspace), branch, and build cache — in a single macOS window. Talk to an agent in an in-app **chat pane** over the Agent Client Protocol (ACP), or drive its CLI in a **terminal pane**; either way the session is pinned to a Lane. Macro buttons in the bottom dock send preset commands to any terminal with one click or a keyboard shortcut.

**Concept model**: `Workspace → N × Project (= git repo) → N × Lane`. A `Lane` is a worktree-like space — a checked-out branch (git worktree) or a plain directory — and is the unit a Claude session attaches to. Users see "Worktree" in the UI; "Lane" is the internal type.

**Multi-project workspace**: one window holds N `Project`s (each opened repo root). Projects can be bundled into user-defined `Group`s in the left dock; ungrouped projects render at the same rank as groups. The active focus is a single `LaneRef { project, lane }`, so cross-project state (`MainAreaContext` swap key, per-lane caches) is keyed by ref rather than lane id alone.

## Project layout

```
daruda/
├── crates/
│   ├── app/                  # main app binary (workspace, agent, ui, lane, surface)
│   ├── daruda_acp/           # Agent Client Protocol client core (GPUI-free)
│   ├── daruda_config/        # config system (live reload)
│   ├── daruda_store/         # persistence + observability (NDJSON log)
│   ├── daruda_claude/        # Claude integration
│   ├── daruda_terminal/      # terminal emulation + GPUI rendering
│   ├── daruda_update/        # app update checking
│   ├── ghostty_vt/           # safe Rust wrapper over libghostty-vt
│   ├── ghostty_vt_sys/       # Zig C FFI bindings
│   ├── gpui_component/       # vendored gpui-component fork (do not lint/edit — see below)
│   └── visual_tests/         # offscreen render snapshot tests
├── tools/
│   ├── vt_dump/               # diagnostic CLI
│   └── gen_licenses/          # generates third-party license manifest
├── vendor/ghostty/           # Ghostty v1.2.3 submodule
└── scripts/
```

## Requirements

- **Rust**: 2024 edition (1.95.0+)
- **Zig**: 0.14.1 (`./scripts/bootstrap-zig.sh` on macOS; on Linux install manually and set `ZIG=<path>` or put `zig` on `PATH`)
- **macOS**: Apple Silicon or Intel + Xcode Command Line Tools — the primary, fully-verified target.
- **Linux**: builds and tests pass; GUI runtime (window/menu/tray) not yet verified on a real desktop. Needs system `libfontconfig`/`libxcb`.
- **Windows**: not yet ported.

## Build

```bash
git submodule update --init --recursive
./scripts/bootstrap-zig.sh
cargo fetch && ./scripts/apply-gpui-patch.sh
cargo build -p daruda
./scripts/build-app.sh         # .app bundle
```

## Tests & CI

```bash
cargo test
cargo fmt --all -- --check
cargo clippy -p ghostty_vt -p ghostty_vt_sys -p daruda_terminal -p daruda \
  -p daruda_config -p daruda_store -p daruda_claude -p daruda_update --all-targets -- -D warnings
scripts/lint-inline-literals.sh
scripts/lint-paint-scope.sh
scripts/lint-reentrant-reads.sh
scripts/lint-direct-gpui-component.sh
scripts/lint-no-eprintln.sh
scripts/lint-no-silent-update.sh
scripts/lint-viewport-row-scroll.sh
scripts/lint-agent-activity.sh
scripts/lint-daruda-path-literals.sh
scripts/lint-file-size.sh
scripts/lint-mark-dirty-direct-call.sh
```

Note: `.github/workflows/ci.yml` gates only a subset of the above (fmt, the 7-crate clippy list, and the 6 lints through `lint-viewport-row-scroll.sh`); `lint-no-silent-update.sh`, `lint-agent-activity.sh`, `lint-daruda-path-literals.sh`, `lint-file-size.sh`, and `lint-mark-dirty-direct-call.sh` are local/reviewer checks not yet wired into CI, and neither the CI clippy/test `-p` list nor the commands above include `daruda_acp`.

## Visual verification

Render the UI offscreen to a PNG and read it back — text, layout, colors, images, and toasts all render, permission-free (no Screen Recording grant). Capture goes through gpui's `render_to_image`, gated upstream behind `test-support`; the `--screenshot` path below requires it, plus `gpui_macos/font-kit` (without that feature glyphs don't rasterize — shapes render but **text is invisible**).

**Whole app** — the `--screenshot` flag captures the live workspace window:

```bash
cargo build -p daruda --features screenshot
target/debug/daruda --screenshot /tmp/shot.png   # opens, settles ~2s, captures, quits
```

The opt-in `screenshot` feature enables `gpui/test-support` + `gpui_macos/font-kit`; it is off by default to keep the shipping binary clean. Entry point: `crates/app/src/screenshot.rs`.

Verification loop: render → PNG → an agent reads the PNG and checks the result. This catches both rendering bugs and runtime state (e.g. error toasts), so it doubles as a smoke test of the real app's startup.

### Driving the captured state

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

**Tier 3 — transient / live state: NOT reachable by config or state alone.** Restore recomputes these fresh. The `--screenshot-scenario <name>` flag drives one transient overlay into view after settle and before capture (it forces a repaint since `render_to_image` captures the last *painted* frame). Implemented scenarios: `command-palette`, `error-modal`, `settings` / `settings:<section-slug>` (Settings is a separate window — the scenario captures *that* window; slug = `BuiltinSection::slug`, e.g. `font`, `keymap`, `notifications`), `toast`. Add a variant to `ScreenshotScenario` + `screenshot_scenario::drive` (`crates/app/src/workspace/screenshot_scenario.rs`) to cover more. The rest below still need a real backing source, not just a scenario:

```bash
target/debug/daruda --screenshot /tmp/shot.png --screenshot-scenario command-palette
target/debug/daruda --screenshot /tmp/s.png   --screenshot-scenario settings:font
```

| State | On restore |
|---|---|
| Modals, command palette, Settings | closed — use `--screenshot-scenario` (`error-modal`, `command-palette`, `settings[:<slug>]`) |
| Toasts | empty queue — use `--screenshot-scenario toast` |
| Drag / hover / text selection | gone (real-time only) |
| Terminal output | fresh shell prompt — scrollback is **not** persisted |
| File-viewer content / git-changes | reloaded from disk — needs **real files / git repo** |
| Usage / Skills / Tools panels | fetched fresh (net / FS / MCP); cold → placeholder |
| Live Claude session badge | in-memory — needs a real attached session |

## Coding Best Practices

**Design reference**: before any UI/design work or refactoring/improvement pass, read [`DESIGN.md`](./DESIGN.md) — the design language (colors, surface ladder, typography, accent rules, component chrome). Align changes with it.

**Priority order** when trade-offs arise: Correctness > Maintainability > Performance > Brevity

**Root-cause priority**: fix the root cause, not the symptom — never ship a band-aid when the underlying defect is reachable.
- When a workaround is the only thing that comes to mind, that is the signal to stop and surface the root problem **first**: name the actual defect, where it lives, and why the symptom appears — then decide whether to fix it or, only with explicit reason, defer.
- A workaround is acceptable **only** when the root cause is out of scope (upstream dependency, separate subsystem, or deliberately deferred). In that case, mark it inline (`// WORKAROUND: <root cause> — <why deferred>`) and report it; never let it pass silently as if the problem were solved.

### Task Complexity Assessment
Before starting, classify the task:
- **Trivial** (single file, obvious change) → execute immediately
- **Moderate** (2–5 files, clear scope) → brief plan then execute
- **Complex** (architectural impact, ambiguous requirements) → full research first

### Architectural Constraints
- Before adding any feature, determine which layer owns the responsibility
- Before changing a business rule, grep all downstream consumers, verify the change is valid for each, and report before proceeding

### Anti-patterns
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

### MVU-flavored guiding rules

Daruda is not strict MVU, but the architecture leans on three rules. Treat them as the default; deviate only with a `// SAFETY:`-style comment that names the exception.

- **View purity** — `render()` and the event-handler closures it builds must not carry state-transition logic. Closure bodies are one-line dispatches: `weak_ws.update(cx, |ws, cx| ws.method(args, cx))`. Same Model → same screen. *Exception*: layout-geometry caching inside `canvas()` (bounds, hitbox, scroll offsets) is allowed — GPUI requires it. Don't smuggle Model changes through this exception.
- **One-way data flow** — Views dispatch; only `Workspace` (and `*_ops.rs`) modify Model. "Modify" covers every state-change verb — `add_*`, `remove_*`, `set_*`, `insert_*`, `delete_*`, `clear_*`, `toggle_*`, `update_*`, `open_*`, `close_*`. The View calls one of these by name; the body lives in `Workspace` / `*_ops.rs`, not in the closure. A View must not reach across entities to write child state directly.
- **Single source of truth** — When state is mirrored (e.g. config → cached field), there is exactly one update site. Adding a new mirror means extending that one entry point — not a parallel sync path.

## Development rules

- **License**: AGPL-3.0-only.
- **Language**: all code, comments, and identifiers in English.
- **Tests**: every new module needs `#[cfg(test)] mod tests`.
- **Error handling**: custom error types + `Display`. `unsafe` requires `// SAFETY:` comment.
- **GPUI dependency**: only view/UI code may import GPUI. PTY, config, git stay GPUI-free.
- **`gpui_component` access**: app code must go through `crate::ui::*`; direct imports forbidden. See `crates/app/src/ui/CLAUDE.md`.
- **Commit only when explicitly asked** — never `git add`/`git commit` without direct instruction.
- **Commit messages**: `<type>: <subject>` (imperative, ≤72 chars). Types: `feat` `fix` `refactor` `perf` `test` `chore` `ci` `docs`. Body only when WHY is non-obvious. Prohibitions: no Phase/Step/ticket numbers, no "what I did" lists (diff shows that), no future-work notes.
- **User-facing values go through config** (`daruda_config`). Pixel/color constants → `ux/theme.rs`.
- **User-facing strings go through i18n** — every string visible to the user must be a `pub fn` in `surface/strings.rs` backed by a key in `crates/app/locales/en.yml` (+ matching key in `ko.yml`). Never embed raw string literals at call sites. See `crates/app/locales/CLAUDE.md` for the full checklist.
- **Comments**: current logic only. No history, no "used to be X".
- **In-progress docs**: keep outside the repo in a personal document store.

## File-structure rules

- One `.rs` file = one responsibility. GPUI-free and GPUI-dependent code in separate files.
- Split order: multiple responsibilities → by domain; tests ≥ 40% → `tests.rs`; single domain > 300 lines → directory module.
- `impl Render` lives in its own file. `actions!()` macros stay at `mod.rs`.

## Pitfall-prevention rules

1. **Coordinates**: never mix byte offsets with grid coordinates. Always convert window coordinates via `mouse_position_to_local()`.
2. **Magic numbers**: escape bytes, codes, buffer capacities, colors, pixels, and strings belong only in their designated files (`ansi.rs`, `vt_codes.rs`, `vt_limits.rs`, `theme.rs`, `strings.rs`, `constants.rs`, `keybindings.rs`).
3. **Zig FFI**: Ghostty enums are `u16`. Always range-check before casting.
4. **IME**: printable characters must go through `replace_text_in_range` → `commit_text` → PTY. Never send directly from `on_key_down`.
5. **GPUI Entity reentrancy**: calling `.read(cx)` on the same entity during `render()` or `entity.update()` panics. `persist_state` must only be called via `mark_dirty_and_save` (`cx.defer`).
6. **Reference comparison**: before adding a feature or fixing a bug, check how Alacritty, iTerm2, **zed** (`~/.cargo/git/checkouts/zed-a70e2ad075855582/193b55a/crates/{workspace,gpui}/` — the exact rev daruda builds against, see `gpui` in `Cargo.toml`), and gpui-ghostty implement the same concept. For GPUI-specific patterns (entity lifecycles, window contexts, async re-entry) zed is the closest reference; always read the version-matched source above rather than a standalone clone of a different version.
7. **Text pixel mapping**: never use `index = offset_px / glyph_advance`. Always use the shaper's reverse-mapping API.
8. **Paint-scope state**: `window.text_style()` / `window.rem_size()` are invalid outside the paint walk. Share metrics via `cell_dimensions()`.
9. **Color palette**: `daruda_terminal/src/ux/theme.rs` uses a local `hsla()` with hue in degrees (0–360). `app/src/ui/theme.rs` is the gpui_component bridge using fractions (0–1). Never call `gpui::hsla` from the terminal theme file.
10. **Render-cost containment** (`window.refresh()` ban + cache rules): GPUI has **no partial redraw** — any dirty view repaints the whole window tree, and cost scales with node count. Two rules keep that cost contained:
    - **Never call `window.refresh()` / `cx.refresh_windows()` on a hot path.** Refresh sets `window.refreshing`, which **bypasses every `AnyView::cached`** for that frame (see gpui `view.rs` prepaint `!window.refreshing` guard). It is reserved for genuinely global invalidation (theme swap in `ui/theme.rs`). For everything else use **targeted `cx.notify(entity)`** so only that view subtree (and its ancestors) goes dirty and sibling `.cached()` views stay cached. Reference: zed PR #25009.
    - **Caching a child view requires notify-on-change.** A view that renders from a parent-staged snapshot (e.g. `Dock::snap`) must be marked dirty (`cx.notify(child)`) when that snapshot's content changes, or `.cached()` will show stale data. Self-notifying views (TerminalView, ToastLayer) are already safe. Bare `entity.update(cx, |e, _| e.field = …)` without notify is incompatible with caching that entity.
11. **Agent-chat single activity source**: every "is the pane working / did it just finish" decision reads `activity_state()` / `is_busy()` / `activity_elapsed()` on `AgentChatView` — never the raw prompt `Turn`, which settles busy→idle before trailing background subagents finish. Completion side effects (notification + backing-task done) fire only via `fire_activity_completion` at the busy→idle settle edge that `reconcile_activity` detects — never straight from an `AcpEvent::TurnEnded` / `AcpEvent::Error` arm (early + double-fire). `Turn` is module-private to `agent_chat_pane/view.rs` for prompt-queue sequencing only; tests reach it through the `#[cfg(test)]` hooks (`set_turn_in_flight` / `set_turn_idle` / `turn_is_idle`). Enforced by `scripts/lint-agent-activity.sh`. Stop (`cancel_turn`) settles the turn locally and immediately (responsive + hung-safe) and stashes `Stopped`; it records one expected `cancelled` `TurnEnded` in `pending_cancel_acks`, which `apply_event` **swallows** so a stale cancel-ack can't be misattributed to a turn the user re-prompted (the stop-then-reprompt race). This is sound because `daruda_acp::session.rs` is strictly FIFO — 1 prompt → 1 `TurnEnded`, in order, and a hung turn blocks all later ones.

## Error reporting

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

## Architecture & data flow

```
GPUI event loop (Metal)
  └── app/workspace/
        └── daruda_terminal  →  ghostty_vt (Zig FFI)  →  pty.rs (PTY)

Output: Shell → PTY → stdout_rx → 16ms batch → TerminalSession → ghostty_vt → GPUI paint
Input:  GPUI KeyDown → TerminalInput → stdin_tx → PTY → Shell
```

### Crate dependency graph

```
daruda (app)  →  daruda_terminal  →  ghostty_vt  →  ghostty_vt_sys
             →  daruda_config     →  daruda_store
             →  daruda_store
             →  daruda_claude     →  daruda_store
             →  daruda_acp                          # GPUI-free ACP client core
             →  daruda_update
             →  gpui, gpui_component, portable-pty
```

`gpui_component` is a vendored copy of `longbridge/gpui-component` (Apache-2.0), forwarded as-is so re-vendoring stays a pure file copy — it is excluded from clippy/lint/comment-cleanup passes; app code reaches it only through `crate::ui::*` (see "`gpui_component` access" above).

`daruda_config` and `daruda_claude` both depend on `daruda_store` for `persistence::default_data_dir()` (see Cross-profile data isolation below).

### Cross-profile data isolation

**Rule: any path or identifier for something daruda itself writes and reads back across restarts must go through `daruda_store::persistence::default_data_dir()`** (or `profile_suffix()` for a non-path identifier, e.g. a Keychain service name) — never a fresh `dirs::config_dir()` / hardcoded `daruda` directory literal.

**Why this is called out explicitly:** four separate places independently re-derived a `daruda`/`.daruda` path instead of calling the shared resolver — `daruda_config::config_path`, `daruda_config::project::project_config_dir`, `daruda_claude::hooks::status_file::default_dir`, and `workspace::sync::limits::activity_paths`'s cache path — so a debug build silently read and overwrote a real release install's `config.toml`, hook-status files, and activity cache. A fifth case (the Telegram bridge's Keychain-stored bot token sharing one service name across profiles) caused two profiles to 409-conflict polling Telegram with the same token, since Telegram's `getUpdates` rejects a second concurrent poller on one token. Each was fixed independently before the pattern was named — this section and the guardrails below exist so the next one is caught before it ships, not after a live incident.

**Enforcement:**
- `clippy.toml`'s `disallowed-methods` bans a bare `dirs::config_dir` call outside `daruda_store::persistence`'s own two call sites (each marked `#[allow(clippy::disallowed_methods)]` with a comment).
- `scripts/lint-daruda-path-literals.sh` greps for a hand-rolled `.join("daruda")` / `.join(".daruda")` outside the canonical files (`persistence.rs`, `profile.rs`, `observability/log_writer.rs`) and a short, explicit allow-list of genuinely non-profile-scoped exceptions (the per-repo `.daruda/task-*.md` files, the single global `~/.daruda/hooks/notify.sh`).
- Neither tool catches a hardcoded Keychain/OS-credential-store service name (not a directory path) — review any new one by hand against `crates/app/src/telegram/keychain.rs`'s `service_name()`.

### UI component hierarchy

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
