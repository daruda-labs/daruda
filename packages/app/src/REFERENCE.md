# App Reference

Maintained layout, subsystem contracts, and extension recipes. Read the
[App Instructions](AGENTS.md) for workflow and safety rules; use the relevant
sections below when changing a subsystem.

## Directory layout

```
app/src/
├── (top-level)           # App entry, window/menu lifecycle, PTY, config watcher, slot actions
├── startup/              # Desktop startup and optional replay, capture, and smoke flows
├── agent/                # Agent-side data models — MCP, skills — plus GPUI-free account + ACP launch resolution
├── runs/                 # Process-wide task log owner, read-only projection, and host-installed execution effects
├── surface/              # App-shell constants — name, shortcuts, strings, keybinding action map
├── title_bar/            # Window chrome — who draws the frame (GPUI-free `policy.rs`), the drag strip, the app-drawn caption controls, the application-menu button
├── ui/                   # Facade over `daruda_ui` (packages/ui/) + localized and agent-domain widgets
└── workspace/            # Workspace entity — projects, tabs, panes, docks
    ├── command/          # Command palette + history picker
    ├── group_ops.rs      # Group CRUD (add/rename/recolor/collapse/delete + move_project_to_group)
    ├── project_ops/      # Project CRUD (add/close/delete-on-disk/rename + window_open_policy)
    ├── project_palette_ops.rs  # Palette handlers: New Group / Rename Project / Move Project to Group…
    ├── layout/           # Dock entities (left/right/bottom), drag/toggle ops, snapshots
    ├── left_dock/        # Left-dock views — lanes/"Worktrees" (2-level tree), git changes, files
    ├── main_area/        # TabBar + PaneTree runtime
    │   ├── bottom_dock/  # Macro grid, terminal input, tab strip
    │   └── file_view_pane/  # File-viewer data + renderers
    ├── pages/            # Workspace-level Tasks / Flows pages, beside the lanes
    │   ├── tasks/editor/ # The Task editor — the Tasks page's detail
    │   └── flows/        # The Flows list, and its detail — a flow's graph or a past run's report
    ├── render/           # GPUI render — `impl Render` for Workspace
    ├── right_dock/       # Right-dock views — usage, skills, tasks, tools
    ├── sync/             # Background pumps — PTY, JSONL, limits, MCP, skills watchers
    ├── toast_layer/      # Toast overlay entity rendered above workspace
    └── tests/            # Lifecycle + pure-op tests
```

`crate::project` and `crate::lane` are re-exports of `daruda_project`
(`packages/project/`), which owns the runtime Project/Lane model and the
GPUI-free git CLI wrappers.

## Top-level (`app/src/*.rs`)

`main()` delegates to `startup::run`: helper commands exit before environment
hydration or GUI initialization, desktop cleanup exits before instance ownership,
and the desktop flow initializes the first window before scheduling inspection.

App-shell glue — process entry, native menu bar + Open Recent, window lifecycle (every window is a Workspace, wrapped in `gpui_component::Root`, with double-open guards), live config-reload watcher, PTY spawn + I/O threads, and tab/lane slot-action macros (`tab_slot_table!` / `lane_slot_table!` — the latter generates `ActivateLane*` actions, displayed as "Activate Worktree N" in the menu).

## Layered config (user → project)

1. **Defaults** — `Config::default()`
2. **User** — `<config_dir>/daruda/config.toml`
3. **Project** — `<config_dir>/daruda/projects/<basename>-<fnv1a_hash>/config.toml`

`Config::resolve(&ProjectConfig)` merges (project section replaces user section wholesale). `Workspace::reload_config` applies at runtime; `new_with_project` applies at construction. Status bar shows a dot when the project layer exists.

## `agent/`

Agent-side data models for the right dock — MCP servers and skills. Each submodule splits responsibility into a GPUI-free state core (state struct, snapshot, disk scan, persistence writers, parsers) and a sibling GPUI Global wrapper. Personal/plugin layers are user-global; project layers are keyed per-lane.

Not every submodule is a right-dock model with a Global wrapper: `account.rs` (`PreparedAccount`) and `launch_resolve.rs` (`resolve_launch` and its pure helpers) are a GPUI-free pair resolving what an ACP launch needs to spawn — command line, working directory, env, and session-host write-back — shared by the agent-chat connect pump and the shell-pane spawn funnel. `adapter_cli.rs` prepares an agent CLI run (login, auth status) through the same rules and installer, so it runs the adapter a session would.

## `runs/`

`RunsHost` owns the process-wide `TaskStore`. User commands enter through
`TaskIntent`; observed execution facts are proposed as `TaskEvent`. Each reply
follows its durable commit and foreground projection update. `GlobalTasks`
exposes the resulting `TasksBook` through read-only accessors, and windows
observe it instead of reloading task files. The desktop installs `RunsEffects`
callbacks for pane operations; this module does not depend on Workspace.
Execution bindings are process-owned and identify both workspace and pane.
The process owns one CLI tracker and event pump; closing a window removes its
pane registrations but retains observed processes until their exit is confirmed.
`TaskStore` imports an existing `tasks.json` once and never rewrites it.
Commands and the app-quit flush target only the task log; older builds read
the preserved, potentially stale legacy file.

## `surface/`

App-shell constants and binding glue — app/process name, user-visible labels (menu / landing / dock), keyboard shortcut string constants, the binding→display formatter, and the config-override → GPUI `Action` resolver. Pure strings + no sibling imports (except the action map, which reaches into workspace action types).

## `ui/`

The facade over `daruda_ui` (`packages/ui/`), which holds the reusable widget primitives in two layers. The facade itself only adds localized wrappers and the agent-domain widgets:

- **Wrappers** — thin factories / re-exports over `gpui_component::*` with daruda chrome (palette, `xsmall` defaults, modal tab-spec) baked in. Canonical post-migration shape.
- **Preserved daruda widgets** — kept where `gpui_component` lacks a clean swap: bespoke hover chains, multi-purpose macro tile, absolute-positioned context menu, multi-line input panel composite, procedural status indicator, layout helpers, and `picker_row` / `picker_empty` (`packages/ui/src/picker_row.rs`) — the shared row chrome of every search-and-pick overlay, which owns the focus-accent reservation, the label truncation, the trailing column's `flex_none`, and the row height `theme::PALETTE_MAX_HEIGHT` is derived from.

Theme overlay (`apply_daruda_palette`) is the only place inline `hsla()` / `px(N)` literals are allowed (G4 exempt). See [UI rules](../../ui/AGENTS.md) and the [wrapper recipe](../../ui/REFERENCE.md#adding-a-new-wrapper).

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

## `project/` (`daruda_project::project`)

Runtime [`Project`] — the workspace-visible counterpart to
`daruda_store::project::SerializedProject`. Owns the project root, its
non-empty `Vec<Lane>`, the `last_active_lane_id` snap target,
and visual metadata (color, `tab_order`, `group_id`). GPUI-free. The
dependency order is `workspace/ → project/ → lane/`; the package
cannot import from the app at all.

## `workspace/`

The Workspace entity and its subsystems.

- **Entity & actions** — `Workspace` struct, construction (`new_with_project`), config apply, `on_*` action shims, command palette + history picker, modal openers (`dialog_helpers`), persistence (save/restore/rebuild + `LaneRuntime`), status-bar snapshot, lane create/remove/activate. Holds `projects: Vec<Project>` + `groups: Vec<SerializedGroup>` + `active: LaneRef`; `lane_scoped: HashMap<LaneRef, LaneScoped>` owns per-lane git state, file data/watchers, and input history.
- **`project_ops/` / `group_ops.rs` / `project_palette_ops.rs`** — Project CRUD (add / close / delete-on-disk / rename + `window_open_policy`), Group CRUD (add/rename/recolor/collapse/delete + `move_project_to_group`), and palette dialog plumbing (`New Group`, `Rename Project`, `Move Project to Group…`).
- **`layout/`** — `Dock` entity (left/bottom/right; named `left_dock`/`right_dock`/`bottom_dock` on `Workspace`), divider + dock drag ops, plain-data snapshots for re-entrancy-safe render.
- **`main_area/`** — TabBar + recursive PaneTree runtime. Houses `MainAreaContext`, the pure pane split-tree, pane structs + PTY spawn, directional navigation, tab lifecycle, viewport resize propagation, and the recursive `PaneLayout` renderer. `pane/mod.rs` owns pane models and view coordination; `pane/accounts/model.rs` resolves account selection without GPUI; `pane/accounts/mod.rs` owns workspace queries and preparation; `pane/terminal.rs` starts PTYs; `pane/output.rs` drains output and owns the adaptive poll policy. Sub-domains: `file_view_pane/` (file viewer), `bottom_dock/` (macro grid + terminal input + tab strip + macro data ops).
- **`left_dock/`** — Lanes view (displayed as "Worktrees" tab) rendered as a 2-level tree (`TopRow::Group(GroupId)` / `TopRow::UngroupedProject(ProjectId)` at the top rank, expanding into project headers and lane rows). Group/Project drag payloads share a single `Vec<TopRow>` ordering pool with 0..N renumbering on drop; intra-project lane DnD stays within its project. Also git-changes view, files view, lazy file-tree context + walker, git-status fetch.
- **`pages/`** — Tasks and Flows, shown in place of the lane's tabs and belonging to no lane. What a page shows on top of its list lives in `Workspace.pages` and survives the page being hidden; only `leave_page_detail_then` (`pages/detail.rs`) clears it, asking first about unsaved edits. `tasks/editor/` is the Task editor and its prompt-file watcher; `flows/graph/` is the flow graph, which runs in the worktree it was opened for, not the active one.
- **`right_dock/`** — Usage / skills / tasks / tools views.
- **`render/`** — `impl Render for Workspace`. Reads only plain-data snapshots — no re-entrant entity reads.
- **`sync/`** — Background event pumps: PTY drain (tick), JSONL NDJSON watcher, HTTP usage/limits poll, MCP filesystem watcher, skills filesystem watcher.
- **`tests/`** — Async `#[gpui::test]` lifecycle tests and sync `#[test]` pure-op tests (layout, branch sanitization, etc.).

## `lane/` (`daruda_project::lane`)

**Concept**: "one Claude Code session per `Lane`". A `Lane` is a workspace unit — typically a git worktree (its own directory + HEAD + branch) but a plain non-git directory for repos that aren't initialized. Multiple agents run concurrently in the same window without branch-switching or `target/` cache thrashing. Lanes are permanent until the user clicks ×. Path stays as visible sibling (`<repo>-<branch>`), not hidden. The user-facing label remains "Worktree".

Runtime `Lane` model (id / path / status / `base_ref` / description) plus a GPUI-free blocking git-CLI layer (probe, list/add/remove git worktrees, branch ops). All git calls run on `background_executor`.

## Conventions (enforced)

- **No magic numbers / ad-hoc strings** — pixel sizes in `daruda_terminal::ux::theme`; strings in `surface::strings` or `daruda_terminal::ux::strings`.
- **No GPUI re-entry in `render()`** — snapshot into a plain struct first (see `StatusBarData`).
- **`pub(in crate::workspace)` for workspace internals.**
- **Git CLI on background executor** — `lane::git` is blocking; wrap with `window.spawn` + `background_executor`.
- **Re-entry guards** — `OPEN_IN_PROGRESS` around folder picker; modal key handlers must `cx.stop_propagation()`.
- **Tests alongside code** — new modules add `#[cfg(test)] mod tests`.
- **Errors and GPUI update Results** follow [Error Reporting](AGENTS.md#error-reporting).

## Extension recipes (quick reference)

| Recipe | Key touch points |
|--------|-----------------|
| New dock view | `LeftDockView` variant (or `RightDockView`) → `view_tabs.rs` → `left_dock/<view>/mod.rs` (export `render`) → `mod.rs` action + handler → `render.rs` match arm → test |
| New global action + keybinding | `actions!()` → handler in ops file → `surface/keybindings.rs` const → `main.rs` bind_keys → `action_map.rs` arm → `command/palette.rs` entry |
| New dock panel | `PlaceholderKind` variant → `Workspace::new_with_project` push → renderer module → `render.rs` dock match arm → persistence if needed |
| New modal | See [G9](AGENTS.md#g9--modals-go-through-gpui_componentdialog). `impl ModalView` + `.tab_group()` on root; open via `dialog_helpers::*`. |
| New pane content kind (plain-struct) | `PaneContent` variant + struct in `main_area/pane/mod.rs` → match arms (title/cwd/focus_handle/resize) → `main_area/mod.rs` walker arm (free-fn `render(&content, cx)`) → `daruda_project` persistence mirror + `#[serde(default)]` → `create_*_pane` constructor → `workspace/tests` round-trip. Rendered inline under `Workspace::render`, so its `cx.notify()` dirties the whole window. Fine for small, rarely-updating panes (File). |
| New pane content kind (entity-backed / cached) | For a pane needing **scroll/perf isolation or its own internal state** (Terminal, AgentChat): hold a thin `XxxContent { view: Entity<XxxView>, cached_title, cwd }` wrapper; `XxxView: Render + Focusable`, its `render` calls `track_focus(&self.focus_handle)` and `cx.notify()`s itself; walker arm embeds `AnyView::from(view.clone()).cached(StyleRefinement::default().size_full().flex())`; `wrapper_focus_handle → None`; `Pane::{title,cwd}` read the wrapper (cx-free), `focus_handle(cx)` reads `view.read(cx)`. A `cx.notify()` on the view then dirties only its subtree — siblings keep their cached paint. |
| Virtualized list inside a pane (variable-height) | Use gpui core `list(state, cx.processor(\|this, ix, win, cx\| …))` + `ListState` (`Top` align + `FollowMode::Tail` for chat-like append) — **not** `crate::ui::list` (that's the gpui_component delegate/searchable list) and not `uniform_list` (fixed height). Keep `list_state` count in sync after each items mutation; `remeasure_items(tail)` on streaming grow, `remeasure()` after any visible item's height changes (fold, async image/diff landing). Thumb + at-bottom: `crate::ui::scrollbar::{vertical_thumb_for_list, list_at_bottom}` (display-only thumb). Synthetic fold headers (turn ⊃ response ⊃ tool-group ⊃ block) are not stored — `rows::project(items, &fold)` derives a stable `Vec<RenderRow>` (header rows + `hidden`/`indent` flags) each rebuild; the list virtualizes over rows and `rebuild_rows` diff-splices so a fold toggle remeasures in place (no scroll drift). Reference: `packages/chat/src/{view/,render/,rows/,fold.rs}`. |
| Skills / Tools tab feature | Mutate the relevant Global via `cx.update_global::<GlobalSkills\|McpState, _>(...)` → renderer reads through the snapshot in `RightDockSnapshot` → `cx.observe_global` rebroadcasts to every Workspace, which pushes into its open Settings |
| Tasks feature | Dispatch a `TaskIntent` through `RunsHost` → durable commit → read-only `GlobalTasks` projection → each Workspace's `cx.observe_global` notification |
| Worktree drag/context menu | Data ops in `lane/mod.rs` → `LaneDrag` in `layout/ops.rs` → actions in `lane_ops/` → UI in `left_dock/projects/list.rs` |
| New picker (search-and-pick overlay) | Plain shape, no "no list" state (copy lane switcher/palette): `is_open: bool` beside `pub picker: PickerState` on the view state, Workspace handler computes `visible_len` and calls `picker.on_key(...)` directly. Enum shape, has a "no list" state (copy flow picker): openness *is* the enum, `picker` module-private, view calls `FlowPicker::on_key` which answers Escape/Enter through the shared `picker::overlay_key` and delegates the rest only when there is a list. Either way: `PickerState` field → `ui::picker_row`/`picker_empty` → thin `on_*_key` handler in the view's `*_ops.rs` (next to its `execute_*`), opening with `let Some((key, ch)) = picker_keystroke(ev) else { return };` (`command/picker_key.rs` — returning early *without* `stop_propagation` is what keeps `Cmd+W` working) → `.when(is_open, capture_key_down(...))` stays in `render/workspace_frame.rs` |

## Where things go (decision matrix)

| New feature touches… | Lives in |
|---|---|
| Pure data / algorithm, no GPUI | `lane/`, `agent/mod.rs`, `workspace/main_area/pane_tree.rs`, `daruda_project`, `daruda_config` |
| GPUI render only | `agent/<view>.rs`, `workspace/left_dock/<view>/`, `workspace/render/` |
| Workspace action handler | `workspace/mod.rs` (tab/pane/focus) · `workspace/lane_ops/` · `workspace/layout/ops.rs` |
| Workspace data discarded on lane/project teardown | `workspace/lane_scoped.rs::LaneScoped`; runtimes and flow runs retain their own lifecycle containers |
| New pane content kind | `main_area/pane/mod.rs` + `main_area/mod.rs` walker arm + `daruda_project` + `workspace/construct.rs` constructor |
| New modal / text input in modal | See [G9](AGENTS.md#g9--modals-go-through-gpui_componentdialog). |
| Reusable widget | `crate::ui`. Never inline `div().flex().hover(...).on_mouse_down(...)` at call site. |
| Right-click menu on any element | `workspace/root_menu.rs::root_context_menu`. Never the vendored `.context_menu(...)` — it renders inside the caller's subtree, where an ancestor clip cuts the menu *and* its hit-testing. |
| Save/restore logic | `workspace/persistence/` |
| App-global action | `main.rs` (`on_action`) |
| Window lifecycle | `windows.rs` |
| Settings section / field | `packages/settings/src/sections/` + a `spec/` row; opened and closed in `workspace/settings_ops.rs` |
| Menu bar | `menus.rs` |
| Key-binding string | `surface/keybindings.rs` |
| Keybinding-override arm | `surface/action_map.rs` |
| User-visible label | a `locales/en.yml` key (→ `surface::strings::<section>`) or `daruda_terminal::ux::strings` |
| User-tunable value | `daruda_config` |
| Pixel/color constant | `daruda_terminal::ux::theme` |
| Blocking subprocess | `lane/git.rs` style — GPUI-free, callers wrap with `background_executor` |
| Cross-session persistence | `daruda_project` schema + `migrate_legacy` + `#[serde(default)]` |

## Crate Dependency Graph

```
daruda (app)  →  daruda_terminal  →  ghostty_vt  →  ghostty_vt_sys
             |                   →  daruda_core
             →  daruda_config     →  daruda_store  →  daruda_core
             →  daruda_store
             →  daruda_agent      →  daruda_store
             →  daruda_acp        →  daruda_core    # GPUI-free ACP client core
             →  daruda_flow       →  daruda_acp, daruda_runlog, daruda_core  # flow runs kept in a run log
             →  daruda_flow_edit  →  daruda_flow   # YAML edits that keep comments and order
             →  daruda_project    →  daruda_config, daruda_store, daruda_core  # GPUI-free Project/Lane
             →  daruda_content    →  merman, resvg, image   # GPUI-free image + mermaid pipeline
             →  daruda_control_types  →  daruda_store   # GPUI-free control contract, PaneRef
             →  daruda_ui         →  daruda_terminal, daruda_config, gpui_component, ferrum_flow  # widgets
             →  daruda_transcript →  daruda_acp, daruda_config, daruda_ui   # transcript presentation, shared by chat + Settings
             →  daruda_chat       →  daruda_transcript, daruda_acp, daruda_content, daruda_ui, daruda_config, daruda_store, daruda_core  # the chat pane view
             →  daruda_settings   →  daruda_transcript, daruda_agent, daruda_project, daruda_config, daruda_ui  # Settings + the config store
             →  daruda_core                         # shared, GPUI-free
             →  daruda_update
             →  gpui, gpui_component, merman, portable-pty

gpui_component  →  daruda_core                         # vendored; shares the text primitives
daruda_store    →  daruda_runlog → daruda_core          # GPUI-free task state machine and run log
```

The app can reach every package; GPUI-free packages cannot reach the app.
Shared logic below that boundary follows [Core admission](../../core/AGENTS.md).
Configuration and agent packages use `daruda_store`'s persistence APIs; see
[profile isolation](../../store/AGENTS.md). Shared image/Mermaid and the two
Markdown stacks are documented in [Content](../../content/AGENTS.md).
Vendored dependency rules are in [UI](../../ui/AGENTS.md#vendor-dependencies).

## UI Component Hierarchy

```
Workspace
├── TitleBar                         — shared by Workspace / Settings; off macOS also carries the app-menu button and the caption controls (`app/src/title_bar/`)
├── SettingsView                     — replaces BodyLayout while open, docks included (`packages/settings/`)
│   ├── SettingsSidebar              — back button + search + section nav
│   └── SettingsSection              — one BuiltinSection body at a time
├── BodyLayout                       — shown when SettingsView is not
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
│   │   │           └── AgentChatView — cached chat entity (packages/chat/)
│   │   ├── Page                     — Tasks / Flows, in place of TabBar + PaneTree; belongs to no lane
│   │   │   ├── PageList             — the page's list
│   │   │   └── PageDetail           — one at a time, over the list: TaskEditor · FlowGraph · RunDetail
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
├── Dialog / Sheet / Notification    — gpui_component::Root overlay layers
└── ToastLayer                       — app error notifications (workspace/toast_layer/)
```

**Concept-to-code mapping** — identifiers match the hierarchy names above:

| Hierarchy name | Code identifier | Location |
|---|---|---|
| `LeftDock` | `left_dock` field / `Dock` entity (position=Left) | `workspace/left_dock/`, `workspace/layout/mod.rs`, `render/workspace_frame.rs` |
| `RightDock` | `right_dock` field / `Dock` entity (position=Right) | `workspace/right_dock/`, `workspace/layout/mod.rs`, `render/workspace_frame.rs` |
| `ViewSwitcher` | `render()` in `view_tabs.rs` (both docks) | `left_dock/view_tabs.rs`, `right_dock/view_tabs.rs` |
| `DockSwitcher` | `PanelTabStrip` | `main_area/bottom_dock/tab_strip.rs` |
| `TerminalInputDock` | `TerminalInputPanel` | `main_area/bottom_dock/terminal_input.rs` |
| `MacroKey` | `MacroKey` | `ui/macro_key.rs` |
| `PaneTree` | `PaneLayout` enum | `workspace/main_area/pane_tree.rs` |
| `Pane` | `PaneLayout::Pane` | `workspace/main_area/pane_tree.rs` |
| `TerminalPane` | `PaneContent::Terminal` | `workspace/main_area/pane/mod.rs` |
| `FileViewPane` | `PaneContent::File` | `workspace/main_area/pane/mod.rs` |
| `Page` | `workspace_page: Option<PageState>` | `workspace/pages/mod.rs` |
| `PageDetail` | `pages: PageDetails`, left only through `leave_page_detail_then` | `workspace/pages/detail.rs` |
| `TaskEditor` | `TasksPage.detail` | `workspace/pages/tasks/editor/` |
| `FlowGraph` / `RunDetail` | `FlowsPage.detail` (`FlowDetailBody`) | `workspace/pages/flows/graph/`, `workspace/pages/flows/run_detail.rs` |
| `ToastLayer` | `toast_layer: Entity<ToastLayer>` | `workspace/toast_layer/mod.rs` |
| `SettingsView` | `settings: Option<SettingsHost>` — `Some` *is* settings mode | `packages/settings/`, opened/closed in `workspace/settings_ops.rs` |
| Project (runtime) | `crate::project::Project` (re-export of `daruda_project::project`) | `packages/project/src/project/mod.rs` |
| Group (runtime) | `daruda_store::project::SerializedGroup` (used directly — no separate runtime newtype) | `packages/store/src/project/` + `workspace/group_ops.rs` |
| Lane (runtime) | `crate::lane::Lane` (re-export of `daruda_project::lane`) — UI label remains "Worktree" | `packages/project/src/lane/mod.rs` |
| Lane (persisted) | `daruda_store::project::SerializedLane` + `LaneKind { Git { .. }, Default }` | `packages/store/src/project/lane.rs` |
| Active focus ref | `daruda_store::project::LaneRef { project, lane }` — JSON keys remain `worktree` via `#[serde(rename = "worktree", alias = "lane")]` | `packages/store/src/project/`; per-lane caches keyed by ref in `workspace/mod.rs` |
| `ProjectsView` 2-level tree | `TopRow` enum dispatch + `group_header_row` / `project_header_row` / `worktree_row` (function name retained — UI affordance) | `workspace/left_dock/projects/rows/{headers,worktree}.rs` |
| Multi-project DnD | `DragPayload { Worktree | Project | Group }` + `dnd_ops.rs` reorder pool | `workspace/left_dock/projects/drag.rs`, `workspace/dnd_ops.rs` |
