# app crate — source layout

GPUI-side of daruda. Owns the app entry point, windowing, workspace state
(tabs + panes + docks), sidebar, agent panel data model, PTY plumbing, and the
config/keybinding wiring.

## Directory layout

```
app/src/
├── main.rs               # GPUI entry point — bind_keys, global actions, config watcher
├── menus.rs              # Native menu bar + Open Recent submenu
├── windows.rs            # Window lifecycle — open / replace / close, folder picker, Open guard
├── config_watcher.rs     # Filesystem watch + debounce for live config reload
├── pty.rs                # PTY spawn + I/O threads + resize
├── slot_actions.rs       # `tab_slot_table!` / `worktree_slot_table!` — Cmd+1..9 / Cmd+Ctrl+1..9
├── welcome.rs            # Welcome window (no saved session / explicit New)
├── agent/                # Agent panels — data model, pure renderers, formatters
├── surface/              # App-shell constants (name / shortcuts / strings / action map)
├── ui/                   # Reusable widget primitives
├── workspace/            # Tabs + panes + docks Workspace entity
│   ├── actions.rs        # Trivial `on_*` action-handler shims
│   ├── dock_ops.rs       # Dock toggle + divider / dock drag state
│   ├── file_tree_ops/    # Sidebar Files view ops (mod.rs + walker.rs)
│   ├── file_viewer/      # File-viewer data model + diff parser (mod.rs) + renderers (render/)
│   ├── persistence.rs    # save_state / restore_state / rebuild_layout / WorktreeRuntime
│   ├── render/           # GPUI render — impl Render in mod.rs, layout walker in layout.rs
│   ├── resize.rs         # `resize_all_tabs` — propagate viewport size to PTYs + views
│   ├── worktree_ops.rs   # Worktree create/remove/activate + branch sanitization
│   ├── sidebar/          # Left-dock sidebar per-view subdirectories
│   └── tests/            # mod.rs = lifecycle tests, pure_ops.rs = sync helpers
└── worktree/             # Runtime Worktree + git CLI wrappers
```

## Top-level files

**`main.rs`** — GPUI app entry. Registers menu, spawns config watcher, restores last project or shows Welcome. `recent_slot_table!` macro is the single source of truth for Open Recent 0..9 slots. Dock-reopen hook re-opens Welcome when no windows remain.

**`menus.rs`** — `build_menu_bar(&[RecentEntry]) -> Vec<Menu>`. All labels from `surface::strings`. Rebuilt each launch so Open Recent is live.

**`windows.rs`** — Window lifecycle: `open_workspace_window`, `open_welcome_window`, `open_project_with_mode` (replace vs. new-window), `open_recent_idx`, `prompt_and_open_folder`. `OPEN_IN_PROGRESS: AtomicBool` guards double-fires. Workspace and Settings windows are wrapped in `gpui_component::Root::new(view, window, cx)` so `gpui_component` widgets (Dialog / Sheet / Notification overlays, `Input::TextElement`) can resolve `Root::read` during paint. The Settings inner entity is tracked in `WindowRegistry` via a typed `SettingsHandle` (window handle + `WeakEntity<SettingsWindow>`) — the singleton-focus path goes through `WindowRegistry::settings`, not `downcast`. Welcome is the lone unwrapped window because it renders only daruda-bespoke widgets (no `gpui_component::Input`); the Welcome→Workspace handoff still uses `downcast::<WelcomeScreen>` because Welcome stays its own window root.

**`config_watcher.rs`** — `spawn_config_watcher()`: notify crate watches recursively (catches atomic-rename saves + project-layer files); second thread debounces bursts. Filters to `config.toml` only.

**`pty.rs`** — `spawn_pty(PtyConfig) -> PtyHandle`. Three background threads: reader (PTY→stdout_rx), writer (stdin_tx→PTY), waiter (exit_rx).

**`slot_actions.rs`** — Single source of truth for `ActivateTab1..9` / `ActivateWorktree1..9`. Each macro exposes `@bindings`, `@try_bind_override`, `@register_listeners`, `@names` sections. Adding a tenth slot = one line per section + one entry in `actions!()`.

**`welcome.rs`** — `WelcomeScreen` entity + `WelcomeEvent { OpenFolder, OpenProject(path), NewEmpty }`.

## Layered config (user → project)

1. **Defaults** — `Config::default()`
2. **User** — `<config_dir>/daruda/config.toml`
3. **Project** — `<config_dir>/daruda/projects/<basename>-<fnv1a_hash>/config.toml`

`Config::resolve(&ProjectConfig)` merges (project section replaces user section wholesale). Phase 1 only honours `[shell]`. `Workspace::reload_config` applies at runtime; `new_with_project` applies at construction. Status bar shows dot when project layer exists.

## `agent/`

Agent-side data models. Each submodule keeps `mod.rs` GPUI-free (G2);
the GPUI Global wrapper lives in a sibling file so the data module
stays pure.

| File | Purpose |
|------|---------|
| `mod.rs` | Module index. Declares `mcp`, `skills`, `tasks_global`. |
| `mcp/mod.rs` | `McpState`, `McpSnapshot`, `ProjectMcp`. `personal` (user-global) + `project: BTreeMap<PathBuf, ProjectMcp>` (per-worktree). Snapshot exposed to renderers / modals. |
| `mcp/global.rs` | `impl Global for McpState` + idempotent `init(cx)`. |
| `mcp/parse.rs` | `.mcp.json` / `~/.claude/settings.json` parsers + path helpers. |
| `mcp/persist.rs` | Atomic `set_disabled` / `write_server` / `update_server` / `delete_server` (staged-copy + tempfile rename). |
| `skills/mod.rs` | `SkillsState`, `SkillsSnapshot`, `Skill`. Same shape as MCP — `personal` + `plugin` user-global, `project: BTreeMap<PathBuf, Vec<Skill>>` per-worktree. |
| `skills/global.rs` | `impl Global for SkillsState` + idempotent `init(cx)`. |
| `skills/{scan,persist,plugin_ops,plugins,frontmatter}.rs` | Disk scan, CRUD writers, plugin install/uninstall CLI runner, marketplace metadata, YAML frontmatter parser. |
| `tasks_global.rs` | Newtype `GlobalTasks(daruda_store::tasks::TasksState)` + `Deref`/`DerefMut` + `impl Global` + `init` + `load_from_dir`. Wraps the foreign type since orphan rule blocks `impl Global` directly on `TasksState`. |

## `surface/`

| File | Purpose |
|------|---------|
| `constants.rs` | `APP_NAME`, `TERM_PROGRAM_VALUE`. |
| `keybindings.rs` | `SHORTCUT_*` strings. Pure strings, no GPUI imports. |
| `strings.rs` | Menu labels, welcome copy, dock panel names. |
| `action_map.rs` | `apply_keybinding_overrides()` — config key-binding names → GPUI `Action` via `bind!` macro. |

## `ui/`

Two layers under one directory: **wrappers** (thin re-exports / factories
over `gpui_component::*`, the canonical post-migration shape) and
**preserved daruda widgets** (kept because the gpui_component equivalent
has missing or incompatible behaviour — IME, custom layout, daruda tone).
See `crates/app/src/ui/CLAUDE.md` for wrapper-authoring rules.

### Wrappers (factory / re-export over `gpui_component::*`)

| File | Purpose |
|------|---------|
| `mod.rs` | Module index + factory re-exports + trait re-exports (`Disableable`, `ButtonVariants`, `WindowExt`, `ActiveTheme`). |
| `theme.rs` | `apply_daruda_palette(cx)` — overlays daruda colors / radii on `gpui_component::Theme` after `gpui_component::init`. Only `ui/theme.rs` may hold inline `hsla(...)` / `px(N)` literals (G4 exempt). |
| `alert.rs` | `error/warning/info/success(id, msg)` — `gpui_component::Alert` factories, `xsmall` + `banner` style. |
| `badge.rs` | `Badge::new(label).monospace()...` builder over `gpui_component::Tag::custom`. Daruda palette injected via `TagVariant::Custom`; per-call padding/font/radius override Tag's size-driven defaults. No external `.hover()` (Tag's internal hover is the only one). |
| `button.rs` | `button` / `button_primary` / `button_danger` / `button_bare` factories over `gpui_component::Button`. `xsmall` + `.tab_stop(false)` baked in (footer Cancel / Save sit outside the Tab cycle by default — see "Tab navigation in modals" below). |
| `checkbox.rs` | `checkbox(id, label, tab)` factory over `gpui_component::Checkbox`. Third argument selects Tab participation via the `CheckboxTabSpec` trait: pass `isize` to cycle at that index, `()` to skip the cycle. |
| `input.rs` | `input(state, tab_index)` factory over `gpui_component::Input`. Outer `div` paints daruda chrome (`MODAL_INPUT_BG` / border / radius) while the inner `Input` runs `.appearance(false)`. `tab_index` is required so the inner `Input::tab_index(...)` lands in the modal's cycle order. |
| `dialog.rs` | Re-exports `Dialog`, `DialogButtonProps`, `ButtonVariant`, `WindowExt`. Construction lives at `dialog_helpers::*` call sites. |
| `divider.rs` | Re-export of `gpui_component::Divider`. |
| `radio.rs` | `radio(id, label, tab)` factory over `gpui_component::Radio`. Same `isize` / `()` tab spec as `checkbox`. Daruda has no caller yet — wrapper is shipped so future radio groups inherit the policy. |
| `select.rs` | `SelectOption { value, label }` + `state_with_options(opts, initial, ...)` + `select(&state, cx, tab)` over `gpui_component::Select`. Third argument is the `SelectTabSpec` trait (`isize` or `()`); since `Select` has no `tab_index` builder, the spec mutates the underlying `FocusHandle`'s tab fields. Emits `SelectEvent::Confirm(Option<value>)`. |
| `tab_bar.rs` | `tab_bar(id)` factory over `gpui_component::TabBar` (`Small + underline` baked) + `tab(label)` factory over `gpui_component::Tab` (10px horizontal padding baked to compensate for `TabVariant::Underline`'s zeroed `inner_paddings`). Call surface: `selected_index / children / on_click` (+ `w_full` for dock width), plus `suffix(...)` reserved for the bottom dock's `+` panel-tab create chip. `prefix` / `menu` intentionally unused. Sidebar / right-panel tabs keep action chips in the panel body. |
| `list.rs` | `FilteredItem` trait + `FilteredDelegate<I>` + `searchable_list_state(items, ...)` + `list(&state)` over `gpui_component::List`. Substring-search behaviour out of the box. Emits `ListEvent::{Confirm(IndexPath), Cancel, Select(IndexPath)}` — resolve `IndexPath::row` back to the source item via `delegate().item_at(ix)`. Replaces the legacy `Picker` widget. |
| `tooltip.rs` | `tooltip::text(s)` → closure for `.tooltip(...)`. |

### Preserved daruda widgets (no clean gpui_component swap)

| File | Purpose |
|------|---------|
| `context_menu.rs` | `ContextMenu::new(id, pos).item(...)`. Absolute-positioned flat list. |
| `input_panel.rs` | Composite of multi-line `gpui_component::Input` + action buttons. Three layouts: `ActionsRight`, `ActionsBelow`, `ActionsFloating`. Forwards `InputEvent::PressEnter { secondary: true }` (Cmd+Enter) as `InputPanelEvent::Submit`. `set_text` / `clear` / `insert_at_cursor` all take `&mut Window` because they delegate to `InputState::set_value` / `insert`. |
| `macro_tile.rs` | Bottom-dock macro grid tile (`MacroTile`). Single-purpose: `Text` / `Icon` display modes, `.icon_mode()`, `.fixed_width(px)` (Text-mode uniform grid cell + label truncate), `.on_right_click()`, `.tooltip(closure)`, custom hover. Wrap-incompatible with `gpui_component::Button` (single hover slot consumed internally; no right-click / closure-tooltip API). Modal-footer Primary/Secondary/Danger buttons go through `crate::ui::button*` factories instead. |
| `icon_button.rs` | `IconButton`: `close(id, group)`, `add(id)`, `toggle(id, icon, active)`. Three variants ride on bespoke `div()` because each needs an external `.hover(...)` chain (red destructive bg for Close, brightening for Add, hover-bg = active-bg preview for Toggle). gpui_component `Button` consumes its single `hover_style` slot internally and panics on a second `.hover()`, so wrapping is incompatible — kept as daruda-native. |
| `status_indicator.rs` | Session-status dot-grid indicator. Four states: `Idle`, `Connecting`, `NeedsAttention`, `Working`. Procedural GPUI drawing — no SVG assets. |
| `section_header.rs` | Reusable section header row — `SectionHeader::new(title).padding(...).actions(button).truncate_label(true)`. |
| `form_helpers.rs` | `field_row(label, input)` (fixed-width label + flex input) and `checkbox_row(widget)` (matching gutter). |

**Adding a new primitive:** if it wraps `gpui_component::*`, follow
`crates/app/src/ui/CLAUDE.md` (factory shape + `xsmall()` baked in).
Bespoke widget → `RenderOnce + #[derive(IntoElement)]` for stateless,
`Entity<T> + Render` for stateful. Builder-style API. Theme constants
only — G4 lint catches inline `px()` / colors. Re-export from `ui/mod.rs`.

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

## `workspace/`

| File/dir | Purpose |
|----------|---------|
| `mod.rs` | `Workspace` entity. Tabs/panes/active_tab_index, docks, worktrees. `apply_config`, `new_with_project`, tab/pane/split lifecycle, `focus_pane_in_direction`. |
| `dialog_helpers.rs` | `open_form_modal(...)` (entity-owned body), `open_single_field_dialog(...)` (one `gpui_component::Input` via `crate::ui::input` + OK/Cancel, Enter-to-submit via `Dialog::Confirm` action), `open_confirm_dialog(...)` (title + body + OK/Cancel for destructive flows). All wrap `gpui_component::WindowExt::open_dialog` and schedule initial focus via `cx.defer` *after* the dialog mounts. |
| `dock_ops.rs` | Dock toggle + divider/dock drag. `DividerDrag`, `DockDrag` structs. |
| `persistence.rs` | `save_state`/`restore_state`/`rebuild_layout` + `WorktreeRuntime`. Helpers: `effective_cwd` (anchors cwd to worktree_root), `anchor_worktree_paths_to_project_root`. |
| `worktree_ops.rs` | Worktree create/remove/activate, modal openers, slot-id allocator, `sanitize_branch_name`. |
| `pane.rs` | `Pane` = PTY + `TerminalView`. `create_pane()` spawns PTY. `Pane::display_cwd()` for display. |
| `layout.rs` | Pure pane tree. `PaneLayout::{Leaf, Split}`, insert/remove, `collect_pane_rects`, `find_divider`, `adjust_divider`. |
| `nav.rs` | iTerm2-model direction navigation. `pane_in_direction()`. |
| `render/mod.rs` | `impl Render for Workspace`. Snapshots into plain structs (no re-entrant reads). |
| `render/layout.rs` | `render_layout` recursive `PaneLayout` walker. `pane_header` lives here. |
| `actions.rs` | Trivial `on_*` one-liner shims forwarding to business logic. |
| `resize.rs` | `resize_all_tabs` — propagates viewport changes to PTY grids and `TerminalView`. |
| `file_tree_ops/` | Files view ops. `mod.rs`: `VisibleEntry` + workspace ops. `walker.rs`: pure `walk_into`/`visible_from`. |
| `file_viewer/mod.rs` | `PaneFileView`, `PaneFileContent`, `FileViewerSearch`, `VisualRow`, `CharSelection`. |
| `file_viewer/diff_parser.rs` | `DiffHunk`/`DiffLine`, `parse_diff_hunks`. |
| `file_viewer/render/` | Pane-area file viewer renderers (`render_pane_file_viewer` entry point). |
| `dock.rs` | `Dock { position, is_open, size, panels, active_panel }`. |
| `command_palette.rs` | `Cmd+Shift+P`. `PALETTE_ENTRIES` const array. Substring fuzzy filter. |
| `status_bar.rs` | `StatusBarData` snapshot. Surfaces cwd, branch, agent status, transient error. |
| `tests/mod.rs` | `#[gpui::test]` async lifecycle tests. `build_workspace` (default config, no project) + `build_workspace_with(cx, &config, project)` helpers — both wrap Workspace inside `gpui_component::Root::new` to mirror prod (windows.rs). |
| `tests/pure_ops.rs` | `#[test]` sync tests for layout ops, `adjust_divider`, `sanitize_branch_name`. |

## `workspace/sidebar/`

| Path | Purpose |
|------|---------|
| `view_tabs.rs` | Header strip mapping `SidebarView` variants to tabs. |
| `worktrees/list.rs` | Worktrees view body — list items, `[+]` button. |
| `worktrees/create_modal.rs` | `CreateWorktreeModal` — Dialog-rendered, focus + validation + background `git worktree add`. |
| `worktrees/remove_modal.rs` | `RemoveWorktreeModal` — Dialog-rendered, `allow_force` for dirty worktree retry. |
| `git_changes/mod.rs` | Placeholder (W-6). |
| `files/mod.rs` | Placeholder (W-7). |

Modal key handlers must call `cx.stop_propagation()` to swallow global action dispatch.

## `worktree/`

**Concept**: "one Claude Code session per worktree". `1 worktree = 1 directory = 1 HEAD = 1 tab group = 1 Claude session`. Multiple agents run concurrently in the same window without branch-switching or `target/` cache thrashing. Worktrees are permanent until user clicks ×. Path stays as visible sibling (`<repo>-<branch>`), not hidden.

| File | Purpose |
|------|---------|
| `mod.rs` | `Worktree { id, kind, path, name, tab_order, status, base_ref, description }`. `bootstrap_from_project` probes git and falls back to Default for non-git. `base_ref` (W-11), `description` (W-12). |
| `git.rs` | GPUI-free blocking git CLI wrappers: `has_git`, `repo_root`, `current_branch`, `list_worktrees`, `add_worktree`, `remove_worktree`, `delete_branch`, `probe_repo`. Must be called on `background_executor`. |

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
| New sidebar view | `SidebarView` variant → `view_tabs.rs` → `sidebar/<view>/mod.rs` (export `render`) → `mod.rs` action + handler → `render.rs` match arm → test |
| New global action + keybinding | `actions!()` → handler in ops file → `surface/keybindings.rs` const → `main.rs` bind_keys → `action_map.rs` arm → `command_palette.rs` entry |
| New dock panel | `PlaceholderKind` variant → `Workspace::new_with_project` push → renderer module → `render.rs` dock match arm → persistence if needed |
| New modal | `impl Render + Focusable` (no `Modal` trait, no `EventEmitter`). Open via `crate::workspace::dialog_helpers::open_form_modal(title, width, build, window, cx)`. Dismiss via `window.close_dialog(cx)`. Embed `Entity<crate::ui::InputState>` and render through `crate::ui::input(&state, cx, tab)` for text fields. Apply `.tab_group()` to the modal entity's root `div`. Never own backdrop/stop_propagation — Dialog covers it. |
| New pane content kind | `PaneContent` variant + struct in `pane.rs` → match arms (title/cwd/focus_handle/resize) → `render/layout.rs` walker arm → `daruda_project` persistence mirror + `#[serde(default)]` → `create_*_pane` constructor → `workspace/tests` round-trip |
| Skills / Tools / Tasks tab feature | Mutate the relevant Global via `cx.update_global::<SkillsState\|McpState\|GlobalTasks, _>(...)` → renderer reads through the snapshot in `RightDockSnap` → `cx.observe_global` rebroadcasts to every Workspace + the Settings window |
| Worktree drag/context menu | Data ops in `worktree/mod.rs` → `WorktreeDrag` in `dock_ops.rs` → actions in `worktree_ops.rs` → UI in `sidebar/worktrees/list.rs` |

## Where things go (decision matrix)

| New feature touches… | Lives in |
|---|---|
| Pure data / algorithm, no GPUI | `worktree/`, `agent/mod.rs`, `workspace/layout.rs`, `daruda_project`, `daruda_config` |
| GPUI render only | `agent/<view>.rs`, `workspace/sidebar/<view>/`, `workspace/render/` |
| Workspace action handler | `workspace/mod.rs` (tab/pane/focus) · `workspace/worktree_ops.rs` · `workspace/dock_ops.rs` |
| New pane content kind | `pane.rs` + `render/layout.rs` + `daruda_project` + `workspace/mod.rs` constructor |
| New modal | `impl Render + Focusable` beside feature; open via `dialog_helpers::open_form_modal` |
| Text input inside modal | Embed `Entity<crate::ui::InputState>`, render through `crate::ui::input(&state, cx, tab)`; subscribe `InputEvent`. Never re-implement caret/blink. |
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

### G1 — File size budgets

| Trigger | Action |
|---|---|
| `mod.rs` reaches 800 lines | Extract the largest domain before next feature commit. |
| Any `.rs` reaches 600 lines | Extract or split by responsibility. |
| `tests.rs` > 1.5× parent `mod.rs` | Convert to `tests/` directory by fixture flavor. |

### G2 — Responsibility fences

- `render.rs` is `impl Render` + leaf UI builders only. No `std::process`, `std::fs`, or multi-field string formatting. Add display methods on data structs (e.g. `Pane::display_cwd()`).
- Data modules (`worktree/`, `agent/mod.rs`, `layout.rs`) must not import `gpui::Element`, `Context<Workspace>`, or `Window`.
- `worktree/git.rs` stays GPUI-free. UI callers wrap with `background_executor`.
- Display-string helpers live next to the call site, not as data struct methods.

### G3 — 4-point chain rule

Every user-facing affordance must update all four:
1. Type — `actions!()`
2. Handler — `register_action` or `cx.bind_keys`
3. Constant — `surface/keybindings.rs::SHORTCUT_*` or `surface/strings.rs`
4. Discoverability — `command_palette::PALETTE_ENTRIES` + `surface/action_map.rs` arm

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
  → agent/, worktree/, surface/, pty.rs, config_watcher.rs
```

- `surface/` imports nothing from siblings (except `action_map.rs` → workspace action types).
- `worktree/` imports nothing from `workspace/` or `agent/`.
- `agent/` imports nothing from `workspace/`.

### G9 — Modals go through `gpui_component::Dialog`

- Every modal entity opens via `crate::workspace::dialog_helpers::*`:
  - `open_form_modal(title, width, build, window, cx)` — entity owns the full body (fields + banner + footer buttons). Use this for any modal with custom layout.
  - `open_single_field_dialog(workspace, title, placeholder, initial, on_submit, window, cx)` — one `gpui_component::Input` (via `crate::ui::input`), OK/Cancel provided by Dialog, Enter-to-submit wired through `InputState::enter -> cx.propagate() -> Dialog::Confirm`. Use for inline rename / "type one thing" flows.
  - `open_confirm_dialog(title, body, ok_label, ok_variant, on_ok, window, cx)` — title + body text + OK/Cancel for short destructive flows (delete macro / skill / tool / worktree). Dialog owns the buttons.
- All three wrap `gpui_component::WindowExt::open_dialog` so the overlay paints above the workspace and Escape/backdrop dismissal is handled by `gpui_component`.
- The modal entity (form-modal case) owns the body — fields, validation banner, footer buttons. Dialog provides only outer chrome (panel bg, border, padding, backdrop, title). **Do not** re-render panel chrome inside the entity (`bg`/`border`/`p`/`rounded`/title) — that produces a 2-layer modal.
- Dismiss via `window.close_dialog(cx)` (`crate::ui::WindowExt`). Inside `cx.defer` or async paths that lose the live `&mut Window`, capture `window.window_handle()` and re-enter via `cx.update_window(handle, |_, window, cx| ...)`.
- No `Modal` trait, no `EventEmitter<DismissEvent>`, no `ModalLayer`. New modal entities `impl Render + Focusable` only — emitting `DismissEvent` is a no-op since nothing subscribes.
- Async continuations: never nest two `entity.update` on the same `app_cx`. Run workspace finalize first, then update modal.
- Text input uses `Entity<crate::ui::InputState>` rendered via `crate::ui::input(&state, cx, tab)` — `gpui_component::Input` with daruda chrome wrapped around an `appearance(false)` inner element. IME is handled by `gpui_component` (verified equivalent to the retired daruda TextInput by the Path A ADR).
