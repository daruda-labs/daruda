# Contributing to daruda

Thank you for your interest in contributing. This document covers everything you need to get started.

---

## Table of contents

- [Setup](#setup)
- [Contributor License Agreement](#contributor-license-agreement)
- [Code style](#code-style)
- [Commit conventions](#commit-conventions)
- [Pull requests](#pull-requests)
- [Project structure](#project-structure)
- [Design rules](#design-rules)

---

## Setup

```bash
git clone --recurse-submodules https://github.com/daruda-labs/daruda
cd daruda

# Install pinned Zig (required to compile ghostty_vt)
./scripts/bootstrap-zig.sh

# Fetch Rust deps and apply the GPUI IME patch (idempotent — safe to re-run)
cargo fetch && ./scripts/apply-gpui-patch.sh

cargo test
cargo run -p daruda
```

**Requirements**: macOS 12+, Rust 1.95+, Zig 0.14.1 (installed above), Xcode Command Line Tools.

---

## Contributor License Agreement

Contributions are accepted under the [Contributor License Agreement](CLA.md).
The CLA is not a copyright assignment: contributors keep ownership of their
contributions while granting daruda the rights needed to review, maintain, and
distribute accepted work.

---

## Code style

### Language

All code, comments, identifiers, and documentation must be written in **English**.

### Formatting

`cargo fmt` is non-negotiable. The CI check will fail on any drift.

### Lints

The project ships five custom lint scripts that enforce architectural rules. All five must pass:

| Script | Rule |
|--------|------|
| `lint-inline-literals.sh` | No `gpui::white()`, `px(N)`, or `hsla(…)` outside theme/surface files |
| `lint-paint-scope.sh` | No `window.text_style()` / `rem_size()` outside prepaint |
| `lint-reentrant-reads.sh` | No `persist_state` called outside `cx.defer` |
| `lint-direct-gpui-component.sh` | No `use gpui_component::*` outside `crate::ui::*` |
| `lint-no-eprintln.sh` | No `eprintln!` in new code — route through `report_error` / `LogWriter::log` |

### Comments

Comments describe **current logic only**. Do not record pre-change behavior, removed code, or revision history in comments — that belongs in commit messages.

### Error handling

User-visible failures must go through the three-layer pipeline:

- Inside `Workspace`: `self.report_error(report, cx)` → toast + log
- GPUI-free code: `LogWriter::log(report)` → log only
- Never `eprintln!` in new code (invisible to users running the `.app` bundle)

### Tests

Every new module needs `#[cfg(test)] mod tests`. Unit tests inline; extract to `tests.rs` when the file exceeds 800 lines. Integration tests under `crate_root/tests/`.

---

## Commit conventions

daruda follows **Conventional Commits**:

```
<type>(<scope>): <short summary>
```

| Type | When to use |
|------|-------------|
| `feat` | New user-visible feature |
| `fix` | Bug fix |
| `refactor` | Code restructure, no behavior change |
| `perf` | Performance improvement |
| `test` | Adding or fixing tests |
| `chore` | Build, CI, dependency updates |
| `docs` | Documentation only |

**Scope** is the crate or subsystem: `theme`, `workspace`, `dock`, `tasks`, `git`, `config`, `terminal`, etc.

Examples from the project history:
```
feat(tasks): add prompt file open button to task edit pane
fix(theme): re-bridge gpui_component palette on light-mode switch
refactor(workspace/bottom): unbundle terminal_input + fix reorder off-by-one
chore: bump version to 0.2.0
```

Keep the summary line under 72 characters. Use the body for the *why*, not the *what*.

---

## Pull requests

1. **Open an issue first** for non-trivial changes to align on approach before writing code.
2. Keep PRs focused — one logical change per PR.
3. Update tests for any behavior change.
4. Confirm that the CLA applies to the contribution before review.
5. All CI checks must be green before review.
6. Squash or rebase before the PR is merged — no merge commits on `main`.

PR title follows the same `type(scope): summary` format as commit messages.

---

## Project structure

```
daruda/
├── crates/
│   ├── app/              # binary — GPUI entry, workspace, docks, panels
│   ├── daruda_terminal/  # TerminalView + TerminalSession
│   ├── daruda_claude/    # Claude Code hook FSM + JSONL parser (GPUI-free)
│   ├── daruda_config/    # TOML config (GPUI-free)
│   ├── daruda_store/     # panels, tasks, project state persistence (GPUI-free)
│   ├── ghostty_vt/       # safe Rust wrapper over libghostty-vt
│   └── ghostty_vt_sys/   # Zig C FFI bindings
└── tools/
    └── vt_dump/          # headless VT diagnostic CLI
```

**Dependency direction** (one-way, enforced):

```
app  →  daruda_terminal  →  ghostty_vt  →  ghostty_vt_sys
app  →  daruda_claude
app  →  daruda_config
app  →  daruda_store
```

GPUI imports are allowed only in `app/` and `daruda_terminal/`. All other crates must remain GPUI-free.

---

## Design rules

A few rules that will come up in review:

**No inline magic numbers or colors** — pixel sizes go in `daruda_terminal::ux::theme`, user-visible strings in `surface::strings`, keyboard shortcuts in `surface::keybindings`.

**No re-entrant entity reads** — never call `.read(cx)` on an entity you are currently inside `render()` or `update()` for. Snapshot into a plain struct first.

**App code routes through `crate::ui::*`** — never import `gpui_component::*` directly from app code outside `crates/app/src/ui/`.

**Blocking subprocess calls on background executor** — git CLI wrappers and anything that calls `std::process::Command` must be wrapped with `cx.background_executor().spawn(...)`.

**G3 four-point chain** — every user-facing action must have all four: `actions!()` type, registered handler, `SHORTCUT_*` constant, and a `command_palette::PALETTE_ENTRIES` entry.

For deeper context on any of these, see `CLAUDE.md` in the repo root and in `crates/app/src/`.
