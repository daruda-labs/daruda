# app crate — source layout

GPUI-side of daruda. Owns the app entry point, windowing, workspace state
(tabs + panes + docks), agent panel, agent panel data model, PTY plumbing, and the
config/keybinding wiring.

## Directory layout

```
app/src/
├── (top-level)           # App entry, window/menu lifecycle, PTY, config watcher, slot actions, welcome
├── agent/                # Agent-side data models — MCP, skills, tasks (GPUI-free cores + Global wrappers)
├── project/              # Runtime Project model — `Vec<Worktree>` + group/color/tab_order (GPUI-free)
├── surface/              # App-shell constants — name, shortcuts, strings, keybinding action map
├── ui/                   # Reusable widget primitives — gpui_component wrappers + preserved daruda widgets
├── workspace/            # Workspace entity — projects, tabs, panes, docks
│   ├── command/          # Command palette + history picker
│   ├── group_ops.rs      # Group CRUD (add/rename/recolor/collapse/delete + move_project_to_group)
│   ├── project_ops.rs    # Project CRUD (add/close/delete-on-disk/rename + window_open_policy)
│   ├── project_palette_ops.rs  # Palette handlers: New Group / Rename Project / Move Project to Group…
│   ├── layout/           # Dock entities (left/right/bottom), drag/toggle ops, snapshots
│   ├── left_dock/        # Left-dock views — worktrees (2-level tree), git changes, files
│   ├── main_area/        # TabBar + PaneTree runtime
│   │   ├── bottom_dock/  # Macro grid, terminal input, tab strip
│   │   ├── file_view_pane/  # File-viewer data + renderers
│   │   └── task_edit_pane/  # Task-edit inline form + ops
│   ├── render/           # GPUI render — `impl Render` for Workspace
│   ├── right_dock/       # Right-dock views — usage, skills, tasks, tools
│   ├── sync/             # Background pumps — PTY, JSONL, limits, MCP, skills watchers
│   ├── toast_layer/      # Toast overlay entity rendered above workspace
│   └── tests/            # Lifecycle + pure-op tests
└── worktree/             # Runtime Worktree model + GPUI-free git CLI wrappers
```

## Top-level (`app/src/*.rs`)

App-shell glue — process entry, native menu bar + Open Recent, window lifecycle (workspace / welcome / settings, `gpui_component::Root` wrapping, double-open guards), live config-reload watcher, PTY spawn + I/O threads, tab/worktree slot-action macros, and the Welcome screen entity.

## Layered config (user → project)

1. **Defaults** — `Config::default()`
2. **User** — `<config_dir>/daruda/config.toml`
3. **Project** — `<config_dir>/daruda/projects/<basename>-<fnv1a_hash>/config.toml`

`Config::resolve(&ProjectConfig)` merges (project section replaces user section wholesale). `Workspace::reload_config` applies at runtime; `new_with_project` applies at construction. Status bar shows a dot when the project layer exists.

## `agent/`

Agent-side data models for the right dock — MCP servers, skills, tasks. Each submodule splits responsibility into a GPUI-free state core (state struct, snapshot, disk scan, persistence writers, parsers) and a sibling GPUI Global wrapper. Personal/plugin layers are user-global; project layers are keyed per-worktree. Tasks wraps `daruda_store::TasksState` as a newtype to satisfy the orphan rule.

## `surface/`

App-shell constants and binding glue — app/process name, user-visible labels (menu / welcome / dock), keyboard shortcut string constants, and the config-override → GPUI `Action` resolver. Pure strings + no sibling imports (except the action map, which reaches into workspace action types).

## `ui/`

Reusable widget primitives in two layers:

- **Wrappers** — thin factories / re-exports over `gpui_component::*` with daruda chrome (palette, `xsmall` defaults, modal tab-spec) baked in. Canonical post-migration shape.
- **Preserved daruda widgets** — kept where `gpui_component` lacks a clean swap: bespoke hover chains, multi-purpose macro tile, absolute-positioned context menu, multi-line input panel composite, procedural status indicator, layout helpers.

Theme overlay (`apply_daruda_palette`) is the only place inline `hsla()` / `px(N)` literals are allowed (G4 exempt). See `crates/app/src/ui/CLAUDE.md` for wrapper-authoring rules and the "Adding a new primitive" recipe.

### Tab navigation in modals

GPUI's tab cycle is viewport-wide and ordered by `(tab_group path,
tab_index, insertion order)`. The wrapper layer enforces a single
rule so each modal only has to pick the right number:

**Every input-style wrapper takes a `tab` argument.** Two intents,
shared `XxxTabSpec` trait across wrappers:

- **`isize`** — join the cycle at this index.
- **`()`** — skip the cycle (mouse-only).

| Wrapper | Signature |
|---------|-----------|
| `input` / `checkbox` / `radio` / `select` | `(…, tab)` — `tab: isize \| ()` |
| `button*` | `.tab_stop(false)` baked — opt-in to cycle via `.tab_stop(true).tab_index(n)` |

**Modal-level rules**

- Modal entity's root `div` must call `.tab_group()`. Cycle
  containment inside an active dialog is enforced by a
  `gpui_component::Root` patch — escapes wrap back to the topmost
  dialog's focus handle.
- Initial focus is scheduled via `cx.defer` **after** `open_dialog`
  (Root focuses its own handle synchronously; the defer lets the
  primary input win on the next update cycle). Handled by
  `dialog_helpers::*`.

**Numbering convention** — `tab_index` runs in visual order across
mixed widget types without resetting; reuse the same index pool
across chip-driven branches so the user lands on the same logical
slot regardless of which fields are visible.

## `project/`

Runtime [`Project`] — the workspace-visible counterpart to
`daruda_store::project::SerializedProject`. Owns the project root, its
non-empty `Vec<Worktree>`, the `last_active_worktree_id` snap target,
and visual metadata (color, `tab_order`, `group_id`). GPUI-free. The
dependency order is `workspace/ → project/ → worktree/`; `project/`
never imports from `workspace/`.

## `workspace/`

The Workspace entity and its subsystems.

- **Entity & actions** — `Workspace` struct, construction (`new_with_project`), config apply, `on_*` action shims, command palette + history picker, modal openers (`dialog_helpers`), persistence (save/restore/rebuild + `WorktreeRuntime`), status-bar snapshot, worktree create/remove/activate. Holds `projects: Vec<Project>` + `groups: Vec<SerializedGroup>` + `active: WorktreeRef`; per-worktree caches (`git_status_cache`, `file_tree.*`, etc.) key by `WorktreeRef` rather than `WorktreeId`.
- **`project_ops.rs` / `group_ops.rs` / `project_palette_ops.rs`** — Project CRUD (add / close / delete-on-disk / rename + `window_open_policy`), Group CRUD (add/rename/recolor/collapse/delete + `move_project_to_group`), and palette dialog plumbing (`New Group`, `Rename Project`, `Move Project to Group…`).
- **`layout/`** — `Dock` entity (left/bottom/right; named `left_dock`/`right_dock`/`bottom_dock` on `Workspace`), divider + dock drag ops, plain-data snapshots for re-entrancy-safe render.
- **`main_area/`** — TabBar + recursive PaneTree runtime. Houses `MainAreaContext`, the pure pane split-tree, pane structs + PTY spawn, directional navigation, tab lifecycle, viewport resize propagation, TaskEdit prompt-file watcher, and the recursive `PaneLayout` renderer. Sub-domains: `file_view_pane/` (file viewer), `task_edit_pane/` (task form), `bottom_dock/` (macro grid + terminal input + tab strip + macro data ops).
- **`left_dock/`** — Worktrees view rendered as a 2-level tree (`TopRow::Group(GroupId)` / `TopRow::UngroupedProject(ProjectId)` at the top rank, expanding into project headers and worktree rows). Group/Project drag payloads share a single `Vec<TopRow>` ordering pool with 0..N renumbering on drop; intra-project worktree DnD stays within its project. Also git-changes view, files view, lazy file-tree context + walker, git-status fetch.
- **`right_dock/`** — Usage / skills / tasks / tools views.
- **`render/`** — `impl Render for Workspace`. Reads only plain-data snapshots — no re-entrant entity reads.
- **`sync/`** — Background event pumps: PTY drain (tick), JSONL NDJSON watcher, HTTP usage/limits poll, MCP filesystem watcher, skills filesystem watcher.
- **`tests/`** — Async `#[gpui::test]` lifecycle tests and sync `#[test]` pure-op tests (layout, branch sanitization, etc.).

## `worktree/`

**Concept**: "one Claude Code session per worktree". `1 worktree = 1 directory = 1 HEAD = 1 tab group = 1 Claude session`. Multiple agents run concurrently in the same window without branch-switching or `target/` cache thrashing. Worktrees are permanent until user clicks ×. Path stays as visible sibling (`<repo>-<branch>`), not hidden.

Runtime `Worktree` model (id / path / status / `base_ref` / description) plus a GPUI-free blocking git-CLI layer (probe, list/add/remove worktrees, branch ops). All git calls run on `background_executor`.

## Conventions (enforced)

- **No magic numbers / ad-hoc strings** — pixel sizes in `daruda_terminal::ux::theme`; strings in `surface::strings` or `daruda_terminal::ux::strings`.
- **No GPUI re-entry in `render()`** — snapshot into a plain struct first (see `StatusBarData`).
- **`pub(in crate::workspace)` for workspace internals.**
- **Git CLI on background executor** — `worktree::git` is blocking; wrap with `window.spawn` + `background_executor`.
- **Re-entry guards** — `OPEN_IN_PROGRESS` around folder picker; modal key handlers must `cx.stop_propagation()`.
- **Tests alongside code** — new modules add `#[cfg(test)] mod tests`.
- **Errors flow through `report_error` / `LogWriter::log`** — `eprintln!` and `let _ = …` are forbidden in new code. Inside `Workspace` use `self.report_error(report, cx)` (toast + history + log); from GPUI-free / pre-Workspace sites use `LogWriter::log(report)` (log only). See the project root `CLAUDE.md` §Error reporting for the builder shape and `dedup` key conventions.

## Extension recipes (quick reference)

| Recipe | Key touch points |
|--------|-----------------|
| New dock view | `LeftDockView` variant (or `RightDockView`) → `view_tabs.rs` → `left_dock/<view>/mod.rs` (export `render`) → `mod.rs` action + handler → `render.rs` match arm → test |
| New global action + keybinding | `actions!()` → handler in ops file → `surface/keybindings.rs` const → `main.rs` bind_keys → `action_map.rs` arm → `command/palette.rs` entry |
| New dock panel | `PlaceholderKind` variant → `Workspace::new_with_project` push → renderer module → `render.rs` dock match arm → persistence if needed |
| New modal | See G9. `impl ModalView` + `.tab_group()` on root; open via `dialog_helpers::*`. |
| New pane content kind | `PaneContent` variant + struct in `main_area/pane.rs` → match arms (title/cwd/focus_handle/resize) → `main_area/mod.rs` walker arm → `daruda_project` persistence mirror + `#[serde(default)]` → `create_*_pane` constructor → `workspace/tests` round-trip |
| Skills / Tools / Tasks tab feature | Mutate the relevant Global via `cx.update_global::<SkillsState\|McpState\|GlobalTasks, _>(...)` → renderer reads through the snapshot in `RightDockSnapshot` → `cx.observe_global` rebroadcasts to every Workspace + the Settings window |
| Worktree drag/context menu | Data ops in `worktree/mod.rs` → `WorktreeDrag` in `layout/ops.rs` → actions in `worktree_ops.rs` → UI in `left_dock/worktrees/list.rs` |

## Where things go (decision matrix)

| New feature touches… | Lives in |
|---|---|
| Pure data / algorithm, no GPUI | `worktree/`, `agent/mod.rs`, `workspace/main_area/pane_tree.rs`, `daruda_project`, `daruda_config` |
| GPUI render only | `agent/<view>.rs`, `workspace/left_dock/<view>/`, `workspace/render/` |
| Workspace action handler | `workspace/mod.rs` (tab/pane/focus) · `workspace/worktree_ops.rs` · `workspace/layout/ops.rs` |
| New pane content kind | `main_area/pane.rs` + `main_area/mod.rs` walker arm + `daruda_project` + `workspace/mod.rs` constructor |
| New modal / text input in modal | See G9. |
| Reusable widget | `crate::ui`. Never inline `div().flex().hover(...).on_mouse_down(...)` at call site. |
| Save/restore logic | `workspace/persistence.rs` |
| App-global action | `main.rs` (`on_action`) |
| Window lifecycle | `windows.rs` |
| Menu bar | `menus.rs` |
| Key-binding string | `surface/keybindings.rs` |
| Keybinding-override arm | `surface/action_map.rs` |
| User-visible label | `surface/strings.rs` or `daruda_terminal::ux::strings` |
| User-tunable value | `daruda_config` |
| Pixel/color constant | `daruda_terminal::ux::theme` |
| Blocking subprocess | `worktree/git.rs` style — GPUI-free, callers wrap with `background_executor` |
| Cross-session persistence | `daruda_project` schema + `migrate_legacy` + `#[serde(default)]` |

## Maintenance guardrails

### G1 — Responsibility-driven splits

A file is split when its single responsibility breaks down, not when it crosses a line count. Line count is a *signal* to inspect, not a rule to enforce. A cohesive file with one clear responsibility stays as one file regardless of length.

**Real split triggers** (two or more apply → split):

- Two or more domains coexist in the file.
- Methods cluster into groups by naming prefix or shared private helpers.
- Tests require distinct fixtures for different parts of the file.
- Two engineers cannot work in parallel without merge conflicts.
- `tests.rs` exceeds 1.5× the parent `mod.rs` (test-data clustering signal).

**Line-count signals** — review trigger, not a hard cap:

- `mod.rs` ~800 lines / regular `.rs` ~600 lines → inspect whether responsibility is still single. Split only if a real trigger above is also true.

### G2 — Responsibility fences

- `render.rs` is `impl Render` + leaf UI builders only. No `std::process`, `std::fs`, or multi-field string formatting. Add display methods on data structs (e.g. `Pane::display_cwd()`).
- Data modules (`worktree/`, `agent/mod.rs`, `main_area/pane_tree.rs`) must not import `gpui::Element`, `Context<Workspace>`, or `Window`.
- `worktree/git.rs` stays GPUI-free. UI callers wrap with `background_executor`.
- Display-string helpers live next to the call site, not as data struct methods.

### G3 — 4-point chain rule

Every user-facing affordance must update all four:
1. Type — `actions!()`
2. Handler — `register_action` or `cx.bind_keys`
3. Constant — `surface/keybindings.rs::SHORTCUT_*` or `surface/strings.rs`
4. Discoverability — `command::palette::PALETTE_ENTRIES` + `surface/action_map.rs` arm

### G4 — Inline-literal ban

| Anti-pattern | Replace with |
|---|---|
| `gpui::white()` / `gpui::black()` etc. | Named const in `daruda_terminal::ux::theme` |
| `hsla(...)` inline | Named const in `theme.rs` |
| `px(N)` inline | Named metric const in `theme.rs` |
| `"daruda"` literal outside `surface/constants.rs` | `surface::constants::*` |

Enforced by `scripts/lint-inline-literals.sh`. When porting from reference implementations, hoist inline literals to theme constants in the same commit — never defer.

### G5 — Persistence compatibility (forward-only)

- New field → `#[serde(default)]` always.
- Renaming → keep both with `#[serde(alias = "old_name")]` for one release.
- Restructuring → add `migrate_legacy()` branch + regression test.

### G6 — Macro discipline

- `actions!()` calls at module entry point (`mod.rs`) only.
- Code-generating macros live in one file; extend the macro instead of adding parallel `match`/`const` arrays.

### G7 — Dependency direction (one-way)

```
main.rs → menus.rs, windows.rs → workspace/, welcome.rs
  → agent/, project/, worktree/, surface/, pty.rs, config_watcher.rs

project/ → worktree/
```

- `surface/` imports nothing from siblings (except `action_map.rs` → workspace action types).
- `worktree/` imports nothing from `workspace/`, `project/`, or `agent/`.
- `project/` imports `worktree/` only; nothing from `workspace/` or `agent/`.
- `agent/` imports nothing from `workspace/`.

When a function references a worktree across module boundaries, pass
the full `daruda_store::project::WorktreeRef { project, worktree }` —
plain `WorktreeId` is only valid inside `Project::*` methods where the
project context is implicit in `self`.

### G9 — Modals go through `gpui_component::Dialog`

- Every modal entity opens via `crate::workspace::dialog_helpers::*` — `open_form_modal` (entity owns full body), `open_single_field_dialog` (one input + OK/Cancel), `open_confirm_dialog` (title + body + OK/Cancel). All wrap `gpui_component::WindowExt::open_dialog`; Dialog handles backdrop, Escape, and outer chrome.
- New modal entity: `impl ModalView` (supertrait of `Render + Focusable`) for entities passed to `open_form_modal`. No `Modal` trait, no `EventEmitter<DismissEvent>`, no `ModalLayer`.
- `ModalView` applies to `open_form_modal` only; `open_confirm_dialog` / `open_alert_dialog` / `open_single_field_dialog` do not use it.
- Focus is automatically restored to the previously focused element when the dialog closes (handled by `gpui_component::Root`).
- Entity owns only the body (fields, validation banner, footer buttons). Never re-render Dialog's chrome (`bg`/`border`/`p`/`rounded`/title) inside the entity.
- Dismiss via `window.close_dialog(cx)`. In `cx.defer` / async paths without a live `&mut Window`, capture `window.window_handle()` and re-enter via `cx.update_window(handle, ...)`.
- Async continuations: never nest two `entity.update` on the same `app_cx`. Run workspace finalize first, then update modal.
- Text input: `Entity<crate::ui::InputState>` rendered via `crate::ui::input(&state, cx, tab)`. IME is handled by `gpui_component`.
