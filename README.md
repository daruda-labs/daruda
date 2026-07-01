# daruda

Run multiple AI coding agents in parallel — each in its own git worktree, branch, and build cache. Chat with an agent in an in-app pane over the Agent Client Protocol, or drive its CLI in a terminal — all in one macOS window.

Built on [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) and [ghostty_vt](https://ghostty.org) (Zig SIMD). Each agent lives in its own git worktree with a live status indicator; macro buttons in the bottom dock send preset commands to any terminal with one click or a keyboard shortcut.

> **Platform**: macOS 12+ (Apple Silicon and Intel). Windows support is planned — see [Roadmap](#roadmap).

---

## Why daruda

- **In-app agent chat** — talk to a coding agent right inside a pane over the Agent Client Protocol (ACP), no separate CLI window. Pick model, reasoning effort, and permission mode inline; watch tool calls, file diffs, and plans render in place; approve permission prompts without leaving the workspace.
- **Worktree isolation** — every agent runs in its own git worktree, tab group, and working directory. No branch-switching, no `target/` cache thrashing between concurrent sessions.
- **Live Claude Code status** — real-time Working / NeedsAttention / Idle indicators per worktree, driven by Claude Code hooks (push) with JSONL polling as fallback. Detects active sessions via PTY process tree.
- **Left-dock IDE** — Worktrees, Git Changes, and File Explorer in one dock. Syntax-highlighted file viewer with word-level diff. 88 Material Design file icons. Event-driven git status (no polling).
- **Right panel** — token usage + cost estimate, slash-command CRUD (Skills), MCP server manager (Tools), and agent task tracker (Tasks) — all without leaving the workspace.
- **Macro panel** — bottom dock with user-defined macro tabs. Click or press a shortcut to send any command to the focused pane. Full GUI editor, shortcut record mode.
- **Notification surface** — dock bounce, desktop notifications (via `osascript`; no Developer ID required), and long-running command alerts via OSC 1337 / OSC 9 / OSC 777.
- **GPU Metal rendering** — GPUI-powered, 16 ms batch output, damage tracking. IME and Korean/CJK input first-class.

---

## Core concept: lane isolation

Each AI agent runs in its own **lane** — a git worktree (own directory, own HEAD, own branch) or a plain non-git directory. daruda tracks which agent lives in which lane, shows its status in real time, and lets you jump between agents with `Cmd+Ctrl+1–9`. No branch switching, no shared `target/` thrashing. The left dock still labels these "Worktrees" since that's the familiar git term.

---

## Features

### Agent chat

Talk to a coding agent in a pane instead of — or alongside — its CLI. daruda speaks the **Agent Client Protocol (ACP)**, driving Claude Code through the `claude-agent-acp` adapter.

- Pick **model**, **reasoning effort**, and **permission mode** from the input dock
- Markdown replies with drag-select / copy, syntax-highlighted code blocks, and mermaid diagrams
- Tool calls render as cards; file edits show inline word-level diffs in a read-only editor
- Permission requests answered inline — approve / reject without a terminal prompt
- Turn and tool-group folding, plus a live working indicator while the agent runs
- One session per Lane — chat agents and terminal agents tracked side by side

### Terminal

- VT100/ANSI, 256-color, 24-bit True Color
- Unicode-aware rendering — wide CJK and fullwidth glyphs draw at the font's natural size instead of being squeezed into the monospace cell, so two-cell-wide text stays legible (`unicode_width`):
  - Hangul
  - Chinese Hanzi, Japanese Kanji
  - Japanese kana (hiragana / katakana)
  - Fullwidth symbols
  - Most emoji

  Trade-off: the conventional terminal fixes CJK to a two-cell width. Because daruda draws these glyphs at their natural advance — often narrower than two cells — a line that contains them is shorter overall, so its line-level background fill can be drawn narrower than under the two-cell convention. This is a deliberate choice:
  - **Legibility** — glyphs keep the font's true shape and size instead of being stretched or crushed to fit a fixed two-cell box.
  - **No shaper fork** — snapping CJK to exactly two cells would require patching GPUI's line layout, which otherwise collapses a wide glyph into a single cell and overlaps the next one; the natural-advance path works with upstream GPUI unchanged, with no patch to carry across version bumps.
  - **Contained cost** — the resulting column drift is corrected in daruda's own paint code (`shaped_pixel_range_for_cols`), so the cursor, selection, and search highlights still land on real glyph boundaries.
- Box drawing (procedural — no font dependency)
- Alternate screen buffer (SMCUP/RMCUP)
- IME and Korean/CJK input (`set_marked_text` / `commit_text`)
- SGR mouse reporting (1000/1002/1003/1006)
- Bracketed paste (2004)
- Cursor styles: Block / Beam / Underline (`DECSCUSR`), blinking
- Visual bell, bold/italic/underline/strikethrough
- OSC 7 (CWD), OSC 8 (hyperlinks), OSC 52 (clipboard), OSC 133 (prompt marks)
- `DSR` replies, focus event reporting (`DECSET 1004`)

### Tabs and splits

- `Cmd+T` new tab · `Cmd+W` close · `Cmd+1–9` switch
- `Cmd+D` split right · `Cmd+Shift+D` split down
- `Cmd+Alt+←/→/↑/↓` directional pane focus (iTerm2 model)
- Pane and tab drag-reorder
- Left / bottom / right dock toggle (`Cmd+B` / `Cmd+J` / `Cmd+Shift+B`)

### Search

- `Cmd+F` — scrollback search with highlight and prev/next navigation
- `Cmd+Shift+H` — command history picker (FTCS B/C/D, fuzzy match, exit-code badge, jump to row)

### Left dock

**Worktrees view**
- One row per git worktree; `Cmd+Ctrl+1–9` to jump
- Per-worktree Claude Code status indicator: Working (spinner) · NeedsAttention (pulse) · Idle · Connecting
- Multi-session badge strip when ≥2 agents share a worktree
- Create / remove worktree with branch co-deletion option
- Right-click: Reveal in Finder · Copy Path · Rename · Edit Description
- Drag to reorder

**Git Changes view**
- `git status --porcelain` staged / unstaged diff, inline accordion
- Commit, push, refresh — all from the dock
- Event-driven refresh (FS watcher + app-side git commands + tab switch); no polling

**Files view**
- Lazy-expand tree with `notify` FSEvents watcher (idle: 0 % CPU)
- 88 Material Design file icons, extension-mapped; Color or Monochrome mode
- Per-subdirectory `.gitignore` + `.git/info/exclude` with negation override
- Virtual scroll (`uniform_list`), hidden-file toggle, keyboard navigation
- `Right` / `Left` to expand / collapse; `Alt+click` to collapse a subtree

**File viewer** (right-click a file in the tree)
- Raw and Changes (diff) modes with syntax highlighting (`syntect`)
- Word-level diff (`similar`), line numbers, hunk headers
- `Cmd+F` inline search with IME / Korean support

### Bottom macro panel

Customizable macro tabs — click or press a shortcut to send text to the focused pane.

- Add / rename / delete tabs; drag to reorder
- Per-macro fields: label, send text, auto-enter, display as icon, custom shortcut
- `[● Record]` shortcut capture mode
- First-run seeds Claude Code, Codex, and Gemini launch macros

### Right panel

**Usage tab** — live token consumption and cost estimate for each active Claude session, parsed from `~/.claude/projects/…/*.jsonl`. Stacked bar (input / output / cache).

**Skills tab** — browse, create, edit, rename, and delete Claude Code slash commands (`.claude/commands/*.md`) for Project and Global scope — without leaving the workspace.

**Tools tab** — view and manage MCP servers from `~/.claude/settings.json`. Enable / disable without restarting Claude Code.

**Tasks tab** — create agent tasks, launch them into a dedicated worktree (`claude -p "<prompt>"`), and track their state (Backlog → Running → Done / Error) alongside the Claude Code status indicator.

### Notifications

- Dock bounce — `OSC 1337 ; RequestAttention=<yes|no|once>`
- Desktop notifications — `OSC 9` and `OSC 777` (via `osascript`; no Developer ID required, but notifications appear under "Script Editor" identity rather than "daruda" — native app-identity notifications are on the roadmap)
- Long-running command alert — configurable threshold (default 30 s, iTerm2 default), FTCS B/D elapsed
- `OSC 1337 ; ClearScrollback`
- `OSC 1337 ; Copy=<sel>:<base64>` one-shot clipboard write (up to 10 MiB)

### Session restore

All tabs, panes, docks, dock view state, and worktree assignments persist across restarts (cold restore).

### Theme

Built-in `daruda_dark` and `daruda_light` presets; live switching without restart. Terminal palette and UI chrome are independently configurable.

---

## Requirements

| Dependency | Version |
|---|---|
| macOS | 12.0 Monterey or later |
| Rust | 1.95+ (edition 2024) |
| Zig | 0.14.1 (installed by `bootstrap-zig.sh`) |
| Xcode Command Line Tools | any recent |

Apple Silicon and Intel are both supported. Windows support is on the roadmap.

---

## Build from source

```bash
# 1. Clone with submodules (Ghostty VT core is a submodule)
git clone --recurse-submodules https://github.com/daruda-labs/daruda
cd daruda

# 2. Install pinned Zig (needed to compile ghostty_vt)
./scripts/bootstrap-zig.sh

# 3. Fetch Rust dependencies and apply the GPUI IME patch
cargo fetch && ./scripts/apply-gpui-patch.sh

# 4. Run tests
cargo test

# 5. Run the app
cargo run -p daruda

# 6. Build a release .app bundle
./scripts/build-app.sh

# 7. Package as .dmg (requires: brew install create-dmg)
./scripts/build-dmg.sh
```

> **GPUI IME patch** — `apply-gpui-patch.sh` adds a one-line fix to the GPUI upstream that routes non-ASCII key events (Korean jamo, Japanese kana, etc.) through the macOS IME-first path. It is idempotent: re-running it on an already-patched checkout is a no-op.

---

## Configuration

daruda is configured with TOML at `~/.config/daruda/config.toml`. All changes are applied live — no restart required.

```toml
[font]
family = "Zed Mono"     # any monospace font installed on the system
size = 14.0
vertical_spacing = 1.0
horizontal_spacing = 1.0

[cursor]
style = "block"         # block | beam | underline
blink = true

[window]
opacity = 1.0           # 0.0–1.0
blur = false

[theme]
terminal_preset = "default"   # default | solarized-dark | solarized-light | custom
ui_preset = "dark"            # dark | light

# When terminal_preset = "custom", define colors here:
# [colors]
# foreground = "#cdd6f4"
# background = "#1e1e2e"
# ... (16 ANSI slots: black, red, green, yellow, blue, magenta, cyan, white + bright variants)

[shell]
# program = "/bin/zsh"     # defaults to $SHELL
close_pane_on_exit = true

[scrollback]
# lines = 10000

[left_dock]
files_show_hidden = false
files_use_gitignore = true
file_icon_color_mode = "color"   # color | monochrome

[claude_status]
enable = true

[notifications]
# dock_bounce = true
# desktop = true
# long_running_threshold_secs = 30

[keybindings]
# Override any action — see docs for the full action list.
# "cmd-shift-enter" = "new_tab"
```

A per-project layer can be placed at `~/.config/daruda/projects/<name>/config.toml` and is merged on top of the user config when that project is open.

---

## Crate layout

```
daruda/
├── crates/
│   ├── app/              # binary — GPUI entry point, workspace, docks, panels
│   ├── daruda_terminal/  # TerminalView + TerminalSession (GPUI rendering + VT parsing glue)
│   ├── daruda_acp/       # Agent Client Protocol client — agent sessions, models, config options (GPUI-free)
│   ├── daruda_claude/    # Claude Code hook FSM + JSONL fallback parser (GPUI-free)
│   ├── daruda_config/    # TOML config loader (GPUI-free)
│   ├── daruda_store/     # panels, project state, tasks persistence (GPUI-free)
│   ├── ghostty_vt/       # safe Rust wrapper over libghostty-vt
│   └── ghostty_vt_sys/   # Zig FFI bindings
└── tools/
    └── vt_dump/          # headless VT diagnostic CLI
```

### Architecture

```
GPUI event loop (Metal)
  └── Workspace            tabs[] + panes[] (PaneLayout tree)
        └── daruda_terminal    TerminalSession (OSC/CSI) + TerminalView (cell rendering)
              └── ghostty_vt        Zig FFI
                    └── PTY              stdin_tx ←→ shell ←→ stdout_rx
```

### Tech stack

| Layer | Technology |
|---|---|
| UI framework | GPUI (Zed, `cff3ac6`) |
| Rendering | Metal (via GPUI) |
| VT core | ghostty_vt (Ghostty v1.2.3, Zig 0.14.1) |
| Agent protocol | Agent Client Protocol (`agent-client-protocol` 1.0) |
| PTY | `portable-pty` |
| Config | `toml` + `toml_edit` (live reload via `notify`) |
| Syntax highlight | `syntect` |
| Diff | `similar` |

---

## Keyboard shortcuts

| Key | Action |
|---|---|
| `Cmd+T` | New tab |
| `Cmd+W` | Close pane |
| `Cmd+1–9` | Switch to tab N |
| `Ctrl+Tab` / `Ctrl+Shift+Tab` | Next / previous tab |
| `Cmd+D` | Split right |
| `Cmd+Shift+D` | Split down |
| `Cmd+Alt+←/→/↑/↓` | Focus pane by direction |
| `Cmd+B` | Toggle left dock |
| `Cmd+J` | Toggle bottom panel |
| `Cmd+Shift+B` | Toggle right panel |
| `Cmd+Ctrl+1–9` | Jump to worktree N |
| `Cmd+F` | Search scrollback |
| `Cmd+Shift+H` | Command history picker |
| `Cmd+Shift+P` | Command palette |
| `Cmd+=` / `Cmd+-` | Increase / decrease font size |
| `Cmd+0` | Reset font size |
| `Cmd+Ctrl+F` | Toggle fullscreen |

All shortcuts can be remapped in `[keybindings]`.

---

## Roadmap

### Multi-agent support

daruda is built around Claude Code today, but the agent layer is designed to be extensible — in-app chat already speaks the vendor-neutral Agent Client Protocol (ACP), so any ACP-compatible agent can plug in. The `AgentType` field in the task model is reserved, and the macro panel already seeds launch macros for multiple agents on first run.

| Agent | Status | Notes |
|---|---|---|
| Claude Code | **Shipped** | Hook channel + JSONL fallback + PTY tracker |
| Gemini CLI | Planned | Status detection via process + log watching |
| OpenCode | Planned | Hook-compatible event model |
| Codex CLI | Planned | Session log watching |

Each agent will get its own status detection channel, usage parsing, and Skills / Tools integration as the respective CLIs stabilize their extension APIs.

### Windows support

GPUI (daruda's UI framework) already renders on Windows via DX12. The core VT stack (`ghostty_vt`, `portable-pty`) is cross-platform. What remains is porting the macOS-specific platform layer:

| Component | macOS (current) | Windows (planned) |
|---|---|---|
| Rendering | Metal (GPUI) | DX12 (GPUI) |
| Notifications | `osascript` | WinRT toast API |
| Credential store | Keychain (`security` CLI) | Windows Credential Manager |
| Attention request | AppKit `NSApplication` | Taskbar flash (`FlashWindowEx`) |
| FS events | `macos_fsevent` | `ReadDirectoryChangesW` (via `notify`) |

Linux support will follow Windows, as GPUI supports Vulkan on Linux.

### Other planned features

- Vi mode (scrollback navigation)
- Kitty keyboard protocol
- Image protocol (Sixel / iTerm2 inline / Kitty)
- Developer ID code-signing and notarization
- Homebrew Cask distribution

---

## License

AGPL-3.0. See [LICENSE](LICENSE).

daruda vendors [Ghostty](https://ghostty.org) as a git submodule under `vendor/ghostty`; third-party code remains under its respective licenses.
