# daruda

Run multiple AI agents in parallel — each in its own git worktree, branch, and build cache — from a single macOS terminal window. Macro buttons in the bottom dock let you send preset commands to any terminal with one click or a keyboard shortcut.

**Worktree isolation is the core UX**: 1 worktree = 1 directory = 1 HEAD = 1 tab group = 1 Claude session.

## Project layout

```
daruda/
├── crates/
│   ├── app/              # main app binary (workspace, agent, ui, worktree, surface)
│   ├── daruda_config/    # config system (live reload)
│   ├── daruda_store/     # persistence + observability (NDJSON log)
│   ├── daruda_claude/    # Claude integration
│   ├── daruda_terminal/  # terminal emulation + GPUI rendering
│   ├── ghostty_vt/       # safe Rust wrapper over libghostty-vt
│   └── ghostty_vt_sys/   # Zig C FFI bindings
├── tools/vt_dump/        # diagnostic CLI
├── vendor/ghostty/       # Ghostty v1.2.3 submodule
└── scripts/
```

## Requirements

- **Rust**: 2024 edition (1.95.0+)
- **Zig**: 0.14.1 (`./scripts/bootstrap-zig.sh`)
- **macOS**: Apple Silicon or Intel + Xcode Command Line Tools

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
  -p daruda_config -p daruda_store -p daruda_claude --all-targets -- -D warnings
scripts/lint-inline-literals.sh
scripts/lint-paint-scope.sh
scripts/lint-reentrant-reads.sh
scripts/lint-direct-gpui-component.sh
scripts/lint-no-eprintln.sh
```

## Coding Best Practices

**Priority order** when trade-offs arise: Correctness > Maintainability > Performance > Brevity

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
6. **Reference comparison**: before adding a feature or fixing a bug, check how Alacritty, iTerm2, and gpui-ghostty implement the same concept.
7. **Text pixel mapping**: never use `index = offset_px / glyph_advance`. Always use the shaper's reverse-mapping API.
8. **Paint-scope state**: `window.text_style()` / `window.rem_size()` are invalid outside the paint walk. Share metrics via `cell_dimensions()`.
9. **Color palette**: `daruda_terminal/src/ux/theme.rs` uses a local `hsla()` with hue in degrees (0–360). `app/src/ui/theme.rs` is the gpui_component bridge using fractions (0–1). Never call `gpui::hsla` from the terminal theme file.

## Error reporting

`eprintln!` is forbidden in new code. All failures must go through the 3-layer pipeline (toast → details modal → NDJSON log).

| Scope | API |
|-------|-----|
| Inside `Workspace` | `self.report_error(report, cx)` |
| GPUI-free / pre-Workspace | `LogWriter::log(report)` |

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
             →  daruda_config
             →  daruda_store
             →  daruda_claude
             →  gpui, portable-pty
```

### UI component hierarchy

```
Workspace
├── TitleBar
├── BodyLayout
│   ├── LeftDock
│   │   ├── ViewSwitcher             — Worktrees / Git / Files tab strip
│   │   ├── WorktreesView            — worktree list (create / delete / merge)
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

**Concept-to-code mapping** (updated after refactoring — identifiers now match hierarchy names):

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
