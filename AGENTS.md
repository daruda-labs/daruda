# Agent Instructions

Shared coding-agent rules for daruda. Read the applicable documents in
[Required Reading](#required-reading) before changing their subsystem.
Instructions use `AGENTS.md` without parallel `CLAUDE.md` copies. Detailed
contracts live beside the code in linked reference documents.
See [agent setup and authoring](CONTRIBUTING.md#coding-agent-instructions)
for tool support, loading behavior, and maintenance rules.

## Project Summary

Daruda runs multiple AI agents in one desktop window, through ACP chat panes
or CLI terminal panes. The model is `Workspace -> Project -> Lane`; a Lane is
a git worktree or plain directory, displayed as "Worktree". Chats in a Lane
share its working directory and branch. Cross-project state uses
`LaneRef { project, lane }`, not a lane id alone.

A window can hold several Projects, optionally organized into Groups. The
active focus is one LaneRef, and per-lane caches and MainAreaContext swaps
must include both project and lane identity. Workspace owns the desktop
composition; package views receive host data and callbacks rather than
importing Workspace. Tasks and Flows pages belong to the workspace, not a lane.

First-party crates live in `packages/`; directory names differ from Cargo
package names. See [project structure](CONTRIBUTING.md#project-structure),
[crate dependencies](packages/app/src/REFERENCE.md#crate-dependency-graph), and
[setup requirements](CONTRIBUTING.md#requirements). License: AGPL-3.0-only.

## Workflow

### Review Mode

- Base review findings on the diff and source files only.
- Do not run build, test, lint, or compile commands unless the user explicitly
  requests them in the current review request. This takes precedence over
  pre-commit instructions. Read-only inspection commands are allowed.
- If a concern requires execution to confirm, report it as unverified.

### Scope and Change Impact

- Implement only the requested task; avoid speculative refactors and abstractions.
  Do not leave half-finished paths. Surface a blocker if an end-to-end change
  cannot be completed. Keep necessary refactors separate with a clear reason.
- Choose the owning layer before adding a feature. Before changing a public
  type, signature, enum, or business rule, search all consumers and call sites,
  verify the change for each, and report the affected sites before editing.
- Prefer additive, compatible extensions and host-installed callbacks.
  Repeated mechanical edits across unrelated consumers signal a design to
  reconsider. Check existing precedents and why they are shaped that way.
  A new enum variant changes every exhaustive match; an optional hook installed
  by the host can keep a host-specific behavior local. When two designs are
  equally correct, prefer the one that touches fewer consumers on the next change.
  Repeating an identical no-op match arm across unrelated modules is a reason
  to revisit the extension point, not continue copying it.
- Priority: Correctness > Maintainability > Performance > Brevity.
- Before UI/design work or a refactoring/improvement pass, read [DESIGN.md](DESIGN.md)
  and align the change with its design language.
- Fix the root cause. If it is upstream, outside scope, or deliberately deferred,
  name the defect and reason, mark `// WORKAROUND: <cause> - <why deferred>`,
  and report it. Do not present a workaround as a resolved root cause.
- Trivial tasks: execute. Moderate tasks (2-5 files, clear scope): give a brief
  plan. Complex tasks (architecture or ambiguous requirements): research first.

### Branches and Commits

- Work on `main` unless the user requests a feature branch.
- Only stage or commit when explicitly asked; push to `origin` only on an
  explicit request. Keep each commit small and focused on one logical change.
- Commit format: `<type>: <subject>`, imperative, at most 72 characters.
  Types: `feat`, `fix`, `refactor`, `perf`, `test`, `chore`, `ci`, `docs`.
  Body only for a non-obvious reason; no Phase/Step/ticket numbers,
  diff inventories, or future-work notes.
- Never bypass hooks with `--no-verify`,
  `-c commit.gpgsign=false`, or similar unless explicitly requested.

### Language and Documentation

- Code, identifiers, comments, and Markdown documents are English only.
  Discuss work in the user's language.
- Comments explain non-obvious reasons in 2-3 lines, describe current logic,
  and omit history or restatements of the code.
- In-progress notes, plans, ADRs, and handoffs belong in a personal document
  store outside the repository. In-repo Markdown is for maintained documents
  that ship with the code, including these subsystem instructions.

## Required Reading

Read each matching row; a cross-package change can require several documents.
Follow these links explicitly rather than relying on tool-specific loading.

| Working on | Read |
|---|---|
| Committing or changing verification coverage | [Local checks](CONTRIBUTING.md#local-checks), [CI coverage](CONTRIBUTING.md#tests-and-ci) |
| Terminal, VT, PTY, scrollback, IME, Ghostty FFI | [Terminal instructions](packages/terminal/src/view/AGENTS.md) |
| Agent chat, ACP, adapters, wire log | [Chat](packages/chat/AGENTS.md), [Transcript](packages/transcript/AGENTS.md), [ACP](packages/acp/AGENTS.md) |
| Widgets, modals, visual behavior, GPUI lifecycle | [UI instructions](packages/ui/AGENTS.md), [Design](DESIGN.md), [Visual verification](CONTRIBUTING.md#visual-verification) |
| Theme presets or theme field mappings | [Theme instructions](assets/themes/AGENTS.md) |
| Settings or the config store | [Settings](packages/settings/AGENTS.md), [Storage isolation](packages/store/AGENTS.md) |
| Workspace, tabs, panes, docks, actions, files or crates | [App instructions](packages/app/src/AGENTS.md) |
| User-visible strings | [Localization instructions](packages/app/locales/AGENTS.md) |
| Persistence, profile keys, credentials, diagnostics | [Store instructions](packages/store/AGENTS.md) |
| Failure paths, errors, toasts, logs | [App error reporting](packages/app/src/AGENTS.md#error-reporting), [Store](packages/store/AGENTS.md#error-reporting) |
| Run logs, event-sourced state machines, crash recovery | [Run log instructions](packages/runlog/AGENTS.md) |
| Flow engine, flow run directories, resume, stopping a run | [Flow instructions](packages/flow/AGENTS.md), [Run log instructions](packages/runlog/AGENTS.md) |
| Process launch, paths, shells, host identity, environment variables | [Core instructions](packages/core/AGENTS.md) |
| Images, Mermaid, Markdown shared across chat and file viewer | [Content instructions](packages/content/AGENTS.md) |
| Vendored dependencies or GPUI reference patterns | [Vendor rules](packages/ui/AGENTS.md#vendor-dependencies), [Patches](patches/README.md), [References](#reference-implementations) |

## Shared Constraints

### Ownership and State

- GPUI imports belong in view/UI code. PTY, config, git, and domain logic stay
  GPUI-free, in separate files from GPUI-dependent code.
- `daruda_core` cannot depend on `daruda_*` or GPUI. It is pure by default;
  registered environment reads and named platform capabilities are the
  exceptions. See [Core](packages/core/AGENTS.md).
- App widget access goes through `crate::ui::*`; direct `gpui_component`,
  `daruda_ui`, or `ferrum_flow` access outside the facade's allowed
  infrastructure sites is forbidden. Sibling widget packages use
  `daruda_ui`, never `gpui_component`.
- Workspace state discarded on lane/project teardown belongs in
  `workspace/lane_scoped.rs::LaneScoped`; `LaneRuntime` keeps its own
  lifecycle container, and flow runs belong to the process (`RunsHost`),
  outliving the lane and window that started them.
- Views dispatch to named operations; Workspace and its `*_ops.rs` own model
  transitions. Mirrored state has one update site. See
  [view rules](packages/app/src/AGENTS.md#mvu-flavored-guiding-rules).
  Handler closures are one-line dispatches to those operations. The exception
  is layout-geometry caching inside `canvas()` (bounds, hitboxes, scroll offsets),
  which GPUI requires; it cannot carry model transitions. Other exceptions need
  a `// SAFETY:`-style comment explaining why.
  When adding a mirrored field, extend its existing synchronization entry point
  so initial load, live reload, and later edits cannot drift apart.
- Encode valid states with enums or grouped values; avoid correlated bools,
  `Option` fields, and `unreachable!()` over flag combinations. Encapsulate
  multi-field transitions repeated across call paths behind one function and
  restricted visibility. See [state modeling](packages/app/src/AGENTS.md#state-modeling).

### Storage and Profile Isolation

A debug or alternate-profile process must not overwrite a release install's
configuration, status files, activity caches, or credential identifiers.
The storage resolver snapshots the profile and override once for the process;
a second directory lookup in domain code can silently bypass that isolation.

- Use purpose-specific `daruda_store::persistence` APIs. `default_data_dir()`
  remains the compatibility data root; `log_dir()` resolves native profile logs
  or `<DARUDA_DATA_DIR>/logs`. Non-path identifiers use `profile_suffix()`.
- Directory discovery belongs to `daruda_core::path::directories`; naming and
  storage policy belong to private `daruda_store::storage::StorageLayout`.
  Resolving a path must not move existing data.
- Workspace repository preparation validates a copy and publishes atomically,
  preserving old files. Domain code receives the repository, not its root path.
  Do not relocate config, accounts, managed tools, or locks by changing a resolver.
- Diagnostic export may read old default logs without copying/deleting them;
  an explicit data override excludes those compatibility sources.

The deliberate shared-resource exceptions are:

| API | Why it crosses profiles |
|---|---|
| `node_install_dir()` | One pinned runtime has identical bytes across profiles. |
| `flow_lock_root()` | All profiles must share the mutex on the user's working tree. |
| `remote_lock_root()` | All profiles must share the claim on the same bot account. |
| `inbox_dir()` | Durable command claims follow that shared bot across profile handoffs. |

An application-state record is profile-scoped; a claim on an external resource
must exclude competing profiles. Inbox history is part of the bot's claim,
not a profile's channel configuration: removing local config must not erase it.
New exceptions require documented reasoning
and the corresponding `clippy.toml` allowance. Review credential service names
manually: path-literal checks do not detect a hardcoded Keychain identifier.
Detailed contracts and enforcement: [Store](packages/store/AGENTS.md).

### Platform Boundaries

Domain code calls named capabilities; platform-dependent OS calls live at the
boundary. This prevents each consumer from growing its own Windows/Unix
implementation. Common replacements are:

| Operation | Boundary |
|---|---|
| Spawn a concrete executable | `daruda_core::process::command` |
| Launch a user-typed CLI name or Windows shim | `daruda_core::process::command_on_path` |
| Own/cancel a child process group | `daruda_core::process::{lead_own_group, Group}` |
| Normalize or compare paths from different producers | `daruda_core::path::{canonicalize, same_path, is_within, strip_root}` |
| Choose the interactive shell | `daruda_core::shell::interactive` |
| Decode a local/remote file URL | `daruda_core::file_url` with explicit `PathStyle` |

Pure platform policy takes values so any host can test it. Reserve `#[cfg]`
for OS leaves; `cfg!()` still compiles both branches. Gate platform-only imports,
functions, constants, and generated-string overrides with their callers.
App-owned desktop capabilities live in `packages/app/src/platform/`;
credentials, app-launch PATH hydration, and PTY I/O have named separate gates.
System-sleep inhibition is `app/src/platform/power`'s `SleepLease`, per OS.
The complete API table, exceptions, and enforcement are in
[Core](packages/core/AGENTS.md#platform-capability-boundary).

**Platform-gated tests:** whatever only a `#[cfg(unix)]`/`#[cfg(windows)]`
test uses (imports, constants, helpers) needs the same gate, or other OSes
fail on unused items. Recheck this whenever tests move between modules.

### Errors and Environment

- Daruda-owned environment names are registered in
  `daruda_core::process_env`; use its read API. Bootstrap writes must prove
  the process is still single-threaded.
- Use custom errors with `Display`. New `eprintln!` is forbidden.
  Workspace failures use `report_error` (toast, details, NDJSON);
  GPUI-free/pre-Workspace failures use `LogWriter::log`.
- Handle GPUI update Results with `match`, `?`, or the logging helper.
  Intentional ignores require `// SILENT-OK: <concrete reason>` on the
  preceding line. This exception is for inconsequential failures, such as focus
  restoration after a window closes; an empty reason is invalid. The logging
  helper is `crate::windows::try_update_workspace_window`, which records a site
  label. See [error reporting](packages/app/src/AGENTS.md#error-reporting).
- Every `unsafe` block needs a `// SAFETY:` comment.

### Code and UI

- User-tunable values go through `daruda_config`; pixels and colors use
  designated theme/component metrics, not literals at ordinary call sites.
  Escape bytes, VT codes, capacities, strings, and shortcuts also have
  designated definition modules. See the relevant subsystem instructions.
- User-visible strings use i18n keys with matching English and Korean entries,
  generated through `strings_gen`; never embed labels at call sites.
- One Rust file has one responsibility; `impl Render` has its own file and
  `actions!()` stays at `mod.rs`. Split mixed responsibilities by domain,
  move tests to `tests.rs` when they reach 40%, and use directory modules for
  a single domain over 300 lines. App-specific split guidance is in
  [G1](packages/app/src/AGENTS.md#g1--responsibility-driven-splits).
- New modules include `#[cfg(test)] mod tests`.
- Public-API-only tests of a library crate go in `tests/it/` (one binary,
  one module per topic); tests that reach internals stay in `src`. Never widen
  visibility to move a test. The app keeps all tests in `src`.
- The file viewer and chat have independent Markdown parsers/renderers. Shared
  behavior must be implemented in both and covered by rendered conformance.
  Mermaid sources can opt out of host colors, but never the resvg-safe label
  profile. See [Content](packages/content/AGENTS.md).

### Pitfall-Prevention Rules

These rules apply at callers as well as inside the package that owns the API.

1. **Coordinates:** never mix byte offsets with grid coordinates. Convert window
   positions through `mouse_position_to_local()`; identify the coordinate space
   before doing selection, viewport, or scrollback arithmetic.
2. **Definition sites:** escape bytes, VT codes, capacities, pixels, colors,
   strings, and shortcuts belong in their designated modules. Ordinary UI
   callers reuse component metrics and shared scales; read
   [UI metric ownership](packages/ui/AGENTS.md#token-ownership).
3. **Zig FFI:** Ghostty enums are `u16`. Range-check before narrowing casts.
4. **IME:** printable characters follow
   `replace_text_in_range -> commit_text -> PTY`.
   Sending them directly from `on_key_down` bypasses composition.
5. **GPUI re-entry:** reading the same entity inside its render/update can panic.
   Stage plain snapshots; persistence goes through `mark_dirty_and_save`
   (`cx.defer`), never direct `persist_state` inside that update.
   A callback's live `&mut Window` should be used directly; recapturing and
   re-entering the same window can report "window not found".
6. **Reference versions:** when consulting GPUI reference implementations,
   use zed at the pinned revision. See the paths below.
7. **Text/pixel mapping:** use the shaper's reverse mapping, never
   `index = offset_px / glyph_advance`; variable shaping and wide characters
   make fixed-width arithmetic incorrect.
8. **Paint-scope metrics:** `window.text_style()` and `window.rem_size()` do not
   provide valid per-pane state outside the paint walk. Events use shared metrics;
   terminal callers use `cell_layout()`. Its measurement and prepaint exceptions
   are documented in [Terminal](packages/terminal/src/view/AGENTS.md#paint-scope-metrics).
9. **Palette units:** the terminal theme's local `hsla()` takes degrees (0-360);
   GPUI and the UI theme bridge take fractions (0-1). Never use `gpui::hsla`
   inside the terminal theme file.
10. **Render cost and caching:** GPUI has no partial window redraw. A dirty view
    repaints the window tree, while cached sibling views can retain their paint.
    Never use `window.refresh()` / `cx.refresh_windows()` on hot paths: they
    bypass every `AnyView::cached`. Reserve them for global invalidation such as
    theme swaps; use targeted `cx.notify(entity)` otherwise.
    Cached children must notify when parent-staged snapshots change.
    Reading or staging an off-screen entity during render re-arms its
    invalidation, even if its element is never built. Apply the visibility gate
    before the entity access. See [UI safety](packages/ui/AGENTS.md#gpui-render-and-event-safety).
11. **Agent activity:** work/completion decisions use `activity_state()`,
    `is_busy()`, or `activity_elapsed()`, never private prompt `Turn`.
    A prompt can end while background subagents still run. Completion effects
    fire through `fire_activity_completion` at the reconciled busy-to-idle edge,
    not directly from `TurnEnded` / `Error`. Stop settles locally, then consumes
    its expected cancel acknowledgment; prompts in that window buffer until
    dispatch is allowed. Preserve the FIFO contract in
    [Chat](packages/chat/AGENTS.md#activity-and-cancellation).
12. **Mouse buttons:** raw `window.on_mouse_event::<MouseDownEvent>()` receives
    every button; explicitly filter `event.button`. `div().on_click()` is
    left-only, and `.on_mouse_down(MouseButton::X, ...)` names its button.
    A button-agnostic listener needs `// ANY-BUTTON: <reason>`, such as ending
    a drag or dismissing on an outside press. The raw-mouse lint also scans
    vendored code so re-vendoring cannot silently remove the checks.

Terminal details live in [Terminal](packages/terminal/src/view/AGENTS.md);
rendering and event exceptions live in [UI](packages/ui/AGENTS.md).

### Reference Implementations

When GPUI lifecycle, window-context, or async re-entry behavior is unclear,
consult the version-matched zed source before changing the implementation.
GPUI is
`vendor/zed/crates/gpui/` (pinned source plus daruda patches); other zed
crates at that revision are in
`~/.cargo/git/checkouts/zed-a70e2ad075855582/193b55a/crates/`.
Do not use a standalone clone of a different revision.

## Verification

- Provide commands and expected outcomes for non-trivial changes.
  Claim execution only when its tool output is present in the conversation.
- Platform changes require default and all-feature Clippy on macOS, Linux,
  and Windows. A host pass proves only its branches; report other OS results
  as pending until their CI jobs pass.
- UI behavior requires [visual verification](CONTRIBUTING.md#visual-verification):
  render a PNG and inspect it. Tests alone do not establish rendering, IME,
  or window-state correctness. State any remaining manual checks.
  Use a throwaway `DARUDA_DATA_DIR` to isolate capture state and
  `--screenshot-scenario` for transient UI. macOS captures offscreen; Windows
  captures a visible composed window; Linux uses startup smoke checks.
- On Windows, run non-interactive checks through `scripts/run-hidden.ps1`
  or `scripts/test-windows.ps1`. Every newly launched parent/nested shell must
  also be hidden; preserve its output and exit status. Background shells use
  `Start-Process -WindowStyle Hidden` or equivalent. GUI screenshot launches
  need a visible window; see the capture instructions.

### Choosing Verification

Verify intended behavior and guard against regressions. Select checks by impact
and risk, including affected consumers; broaden coverage when uncertain and
follow subsystem requirements. Reuse passing results only while relevant code,
dependencies, configuration, and environment remain unchanged. Report unverified
behavior. See [Local Checks](CONTRIBUTING.md#local-checks) for check selection;
review mode retains its execution restriction. Cargo gates delegate package
coverage to `scripts/ci-cargo.sh`; do not copy package lists.

The full checklist includes source lints and their listed self-tests, default
Clippy/tests, all-feature app Clippy, doc links, the offline ACP preset drift
gate, and screenshot-feature tests. CI runs a subset of local source checks;
a green CI run does not replace checks that are still local-only.
When workspace behavior is affected or the full suite is required, skipping
`workspace::tests` is an iteration shortcut, not completed verification.

Do not clean up vendored upstream code as part of a general lint/refactor pass.
Explicit vendor checks still apply, including mouse-button guards,
`gpui_component` patch tests, and `ferrum_flow`'s MSRV check. GPUI's vendored
tree is regenerated through `tools/vendor_gpui`; its patch workflow is in
[patches/README.md](patches/README.md).
