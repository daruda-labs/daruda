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

- **Rust**: 2024 edition (1.86.0+)
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

## Development rules

- **License**: AGPL-3.0-only.
- **Language**: all code, comments, and identifiers in English.
- **Tests**: every new module needs `#[cfg(test)] mod tests`.
- **Error handling**: custom error types + `Display`. `unsafe` requires `// SAFETY:` comment.
- **GPUI dependency**: only view/UI code may import GPUI. PTY, config, git stay GPUI-free.
- **`gpui_component` access**: app code must go through `crate::ui::*`; direct imports forbidden. See `crates/app/src/ui/CLAUDE.md`.
- **Commit only when explicitly asked** — never `git add`/`git commit` without direct instruction.
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
