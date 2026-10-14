# Contributing to daruda

This document is for contributors building, testing, packaging, or changing
daruda locally. For a short product overview and first run instructions, see
[README.md](README.md).

---

## Table of Contents

- [Requirements](#requirements)
- [Setup](#setup)
- [Coding Agent Instructions](#coding-agent-instructions)
- [Storage locations](#storage-locations)
- [Local Checks](#local-checks)
- [Tests and CI](#tests-and-ci)
- [Visual Verification](#visual-verification)
- [Packaging](#packaging)
- [Project Structure](#project-structure)
- [Code Style](#code-style)
- [Commit Conventions](#commit-conventions)
- [Pull Requests](#pull-requests)
- [Contributor License Agreement](#contributor-license-agreement)

---

## Requirements

| Dependency | Version / status |
|---|---|
| macOS | 12.0 Monterey or later; Apple Silicon and Intel supported |
| Linux | x86_64 `.deb` and portable tar.gz; CI builds, tests, and smoke-tests the window under Xvfb and sway; a real desktop is still unverified; requires system `libfontconfig`, `libxcb`, `libxkbcommon`, `libwayland`, and a Vulkan driver |
| Windows | Experimental native MSVC build; GUI runtime still needs desktop verification |
| Rust | 1.95+; CI pins the floor to 1.95 |
| Zig | 0.14.1; `./scripts/bootstrap-zig.sh` installs the pinned version on macOS |
| Xcode Command Line Tools | Required on macOS |

Rust uses edition 2024. The minimum version is declared once in
`[workspace.package]`; first-party crates inherit `rust-version.workspace = true`.
CI pins the floor exactly, while local development can use newer toolchains.
The vendored `gpui_component*` manifests remain unchanged for re-vendoring;
`ferrum_flow` declares the floor to enable `clippy::incompatible_msrv`.

Zig can be bootstrapped with `scripts/bootstrap-zig.sh` on macOS/Linux or
`scripts/bootstrap-zig.ps1` on Windows x86_64; alternatively set `ZIG` or put
it on PATH. macOS is the primary desktop-verified target. Linux and Windows
have CI build/test/startup gates; real input, IME, menus, tray behavior, and
Windows transitions between monitors with different DPI need desktop checks.

---

## Setup

Clone with submodules because the Ghostty VT core is vendored under
`vendor/ghostty`.

```bash
git clone --recurse-submodules https://github.com/daruda-labs/daruda
cd daruda

git submodule update --init --recursive
```

On macOS and Linux, install the pinned Zig version:

```bash
./scripts/bootstrap-zig.sh
```

Alternatively, install Zig 0.14.1 manually and either put `zig` on `PATH` or
export `ZIG` with the absolute path to the executable.

For a Windows native build, install Git for Windows, Rust (MSVC toolchain),
Visual Studio 2022 Build Tools with the C++ workload and Windows SDK, and
Zig 0.14.1. These tools can be installed from PowerShell:

```powershell
winget install --id Microsoft.VisualStudio.2022.BuildTools --exact --source winget --override "--wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
winget install --id Rustlang.Rustup --exact --source winget
winget install --id zig.zig --version 0.14.1 --exact --source winget
```

Open a new PowerShell session so the updated PATH is available, then run:

```powershell
./scripts/build-windows.ps1
./target/debug/daruda.exe
```

The Windows script runs `cargo build --locked -p daruda`; ordinary Cargo
commands work on every supported host. Set `ZIG` to the executable path if
Zig is not on PATH. Pass `-Release` to build `target/release/daruda.exe`.
No symlink privileges or source-tree preparation are required.

The full test suite does create symlinks: enable Windows Developer Mode or
use a shell with the symlink privilege before running it. Use Git for Windows
2.28 or newer and include its `bin` and `usr/bin` directories on the test PATH.
Runtime shell flows and ACP environment filtering also require Git for Windows;
daruda resolves its POSIX tools relative to `git.exe` when needed.

Use `./scripts/test-windows.ps1` to select a supported Git from PATH, add its
shell tools, check symlink permission, and run the full package suite. Use
`-CheckOnly` to verify prerequisites without compiling. The script restores
PATH on exit and does not change Git configuration or Windows settings.
The test process and its child tools share a hidden console; output still
streams to the invoking terminal. For other validation commands, use:

```powershell
./scripts/run-hidden.ps1 -FilePath cargo.exe -ArgumentList @('clippy', '--locked', '-p', 'daruda', '--all-targets')
```

These runners accept non-interactive commands. Run
`./scripts/test-hidden-process.ps1` to verify console inheritance, argument
quoting, output streaming, and exit-code propagation on Windows.

On macOS and Linux:

```bash
cargo build --locked -p daruda
cargo run --locked -p daruda
```

Ghostty stages its inputs in Cargo's `OUT_DIR` and builds for Cargo's target,
while its Unicode generators run on the build host. Zig version, target ABI,
and submodule availability are checked before compilation. Changes to Zig or
Ghostty sources invalidate the native build, including custom target directories.
This target mapping does not install cross-compilation SDKs or linkers for the
rest of the application.

GPUI's patched crate is checked in under `vendor/zed`, selected for all consumers
by Cargo's `[patch]` table. Ordinary builds never modify the shared Cargo cache.
The remaining Zed crates stay pinned to the same revision in `Cargo.toml`.

To verify the vendored source after fetching dependencies:

```bash
cargo fetch --locked
cargo run --locked -p vendor_gpui -- --check
cargo test --locked -p vendor_gpui
```

To update GPUI, change the pinned Zed revision consistently, fetch dependencies,
and generate into a new directory:

```bash
cargo run -p vendor_gpui -- <zed-checkout> --output <new-directory>
```

Replace `vendor/zed` with that output and review source and lockfile changes
together. The tool reads pristine Git objects, applies the three patches in
`patches/`, retains upstream licenses and test fonts, and materializes the Apache
license symlink as a regular file on every OS. Git and tar are required for this
maintenance check; no patching step is required before building.
`scripts/apply-gpui-patch.sh` remains a read-only verification entry point.

Windows CI requires the native build, all app target compilation (including
`screenshot`), Clippy, and the same full test package list as macOS and Linux.
It also checks platform adapters, MCP transport, packaging, and `--smoke` startup.
UI behavior still needs native desktop verification beyond these automated checks.

Windows remote-channel secrets use Credential Manager. Listening-port discovery
uses native IPv4/IPv6 TCP tables and reports owning PIDs. Managed subprocesses
join checked Job Objects before resuming; cancellation verifies group termination.
Native IME composition and changes between monitors with different DPI still
require desktop verification.

Build with `--features screenshot` to capture a visible Windows window:

```powershell
cargo build --locked -p daruda --features screenshot
$env:DARUDA_DATA_DIR = "$PWD/target/visual-state"
./target/debug/daruda.exe --screenshot "$PWD/target/settings.png" --screenshot-scenario settings:font --screenshot-theme light,dark
```

Windows capture reads the composed native client surface and keeps its physical
pixel dimensions. Launch the GUI normally; a launcher that hides its window
cannot produce a capture. Blank captures fail instead of producing a success.
macOS continues to use offscreen Metal capture; Linux capture remains unavailable.

---

## Coding Agent Instructions

The repository uses `AGENTS.md` for shared and directory-specific rules.
Do not add duplicate `CLAUDE.md` files or import-only bridges. Detailed,
maintained subsystem material belongs in adjacent `REFERENCE.md` files;
the applicable instruction file links to the sections a task must read.

### Tool Support and Loading

Use a current Claude Code release with its built-in AGENTS support enabled
(native support was introduced in v2.1.277). Upgrade older clients.
By default, a project or ancestor `CLAUDE.md`, `.claude/CLAUDE.md`, or
`CLAUDE.local.md` disables AGENTS loading. A user-wide
`~/.claude/CLAUDE.md` does not. If you need an ancestor or local Claude file,
select `claude-md-and-agents-md` under `/config` -> Project instructions.
That choice must come from user/managed settings or a `--settings` file;
project/local settings cannot enforce it. See
[Claude's loading rules](https://code.claude.com/docs/en/memory#agentsmd).

Codex discovers project instructions from the repository root to its launch
directory, with a default combined budget of 32 KiB. It does not preload
every descendant file. Follow the root's Required Reading links when working
across packages. See
[Codex instruction discovery](https://learn.chatgpt.com/docs/agent-configuration/agents-md).

### Authoring and Maintenance

- Put shared constraints at the root and package-specific constraints next
  to their code. Use the exact filename `AGENTS.md`.
- State the trigger, required action, reason, and allowed exception. Keep
  non-obvious safety contracts; remove duplication, not safeguards.
- Move long examples, catalogues, diagrams, and vendor patch explanations
  into maintained reference documents. Link each from a task-specific
  Required Reading row; do not make essential contracts optional.
- Do not eagerly import every reference. Markdown links are navigation;
  Claude's `@path` imports load content eagerly and do not reduce context.
- Keep each root-to-directory instruction chain, including separators,
  below 32 KiB. Aim below 20 KiB at the root and 12 KiB per package, with
  extra headroom when adding another nested level. Splitting into more
  ancestor files does not bypass the combined budget.
- Around 200 lines is a useful editing target for a scoped instruction file,
  not a reason to delete required constraints. Check bytes as well as lines.
- Keep the shared pre-commit checklist in [Local Checks](#local-checks), and package
  coverage in `scripts/ci-cargo.sh`. Mechanical enforcement belongs in the
  existing tests/lints, not repeated prose.

These conventions follow
[Claude's authoring guidance](https://code.claude.com/docs/en/best-practices#write-an-effective-claudemd)
and its [size/import guidance](https://code.claude.com/docs/en/memory#write-effective-instructions).

### Verify Loading After Changes

Check local document links and heading anchors, and measure each ancestor
chain before changing an instruction file's scope or adding a nested file.
Do not change personal agent settings as part of a repository edit.

In a fresh Claude session, use `/context` to inspect Memory files, open a
package source file with Read, then check that its `AGENTS.md` is loaded too.
In a fresh Codex session started in a package directory, ask it to list its
instruction sources; expect the root and that directory's applicable file.
These are manual loading checks, not something a Cargo pass establishes.

---

## Storage locations

Configuration and account data retain their existing platform
config directory (`daruda` for release, `daruda-debug` for debug, or
`daruda-<profile>` for a named profile). Managed Node installations and resource
locks also retain their existing shared locations.

New logs use native local storage:

| Platform | Default log directory |
|---|---|
| Windows | `%LOCALAPPDATA%\daruda\logs\<profile>` |
| macOS | `~/Library/Logs/daruda/<profile>` (`release`, `debug`, or a named profile) |
| Linux | `$XDG_STATE_HOME/daruda/logs/<profile>` (default `~/.local/state`) |

`DARUDA_DATA_DIR` selects an exact isolated root, with logs in `<root>/logs`
and the existing configuration, Node and lock paths under that root. Relative
overrides are resolved against the working directory at the first storage
lookup. The override and profile are captured once per process.

Old `~/.daruda/logs/<profile>` files remain untouched. The Help menu's diagnostic
export reads bounded, sanitized events from both old and new default logs;
an isolated override reads only its own logs. Explicit ACP/Telegram trace
environment overrides still select their requested destinations.

Profiles (`release`, `debug`, or a named profile) share one application log
root on every OS. macOS follows the
[`~/Library/Logs/<AppName>` convention](https://www.electronjs.org/docs/latest/api/app#setapplogspathpath).
Windows uses [local app data](https://learn.microsoft.com/en-us/windows/apps/develop/windows-app-restore#machine-specific-app-data),
and Linux uses [`XDG_STATE_HOME` for log history](https://specifications.freedesktop.org/basedir/0.8/).
Earlier native logs under the `daruda[-profile]` application directories
remain read-only diagnostic sources; they are not moved or deleted.

Workspace UUID records and the recent list use a separate repository:

| Platform | Workspace repository |
|---|---|
| Windows | `%LOCALAPPDATA%\daruda[-profile]\state\workspace` |
| macOS | `~/Library/Application Support/daruda[-profile]/state/workspace` |
| Linux | `$XDG_STATE_HOME/daruda[-profile]/workspace` |
| Explicit override | `<DARUDA_DATA_DIR>/state/workspace` |

The primary desktop instance prepares this repository before opening windows.
On the first launch it validates and copies only UUID JSON records and
`recent-workspaces.json`, then publishes the verified directory atomically.
Original files, project config/Flow directories and unknown files stay untouched.
An interrupted copy is retried from the original snapshot; abandoned staging
directories are never read as state. An existing destination without a valid
layout marker, malformed source records or newer schemas stop startup rather
than overwrite data. Later launches use the published repository exclusively.
Running an older app after migration reads the preserved, potentially stale
original state; automatic downgrade synchronization is not supported.

## Local Checks

Unit tests must not launch desktop applications against temporary fixtures.
The workspace external-file boundary records requests per GPUI app in test
builds; assert the requested path and editor with `take_external_open_requests`
instead of invoking an installed editor. This applies on every OS, without
requiring a default file association or an editor on `PATH`. Keep native launch
verification in an explicit manual smoke check with a persistent file.

Before committing, run the complete checklist below and fix every failure.
During code review, the [review-mode rule](AGENTS.md#review-mode) takes precedence:
only execute build, test, lint, or compile commands when the user explicitly
requests them in that review request.

Cargo package coverage is maintained once in
[scripts/ci-cargo.sh](scripts/ci-cargo.sh). Use its named gates instead of
copying package lists into documentation. A successful check exits zero;
`fmt` reports no formatting drift, Clippy/doc report no warnings, tests pass,
and the preset generator reports no generated drift.

```bash
cargo fmt --all -- --check
bash scripts/ci-cargo.sh clippy
bash scripts/ci-cargo.sh app-features
bash scripts/ci-cargo.sh test
bash scripts/ci-cargo.sh doc
cargo test -p daruda --features screenshot
cargo run -p gen_acp_presets -- --check
```

The lint checklist includes CI checks and additional local checks.
Every script must exit zero:

```bash
./scripts/lint-inline-literals.sh
./scripts/lint-paint-scope.sh
./scripts/lint-reentrant-reads.sh
./scripts/lint-direct-gpui-component.sh
bash ./scripts/lint-input-factories.sh
bash ./scripts/lint-input-factories.sh --self-test
bash ./scripts/lint-modal-fields.sh
bash ./scripts/lint-modal-fields.sh --self-test
./scripts/lint-direct-ferrum-flow.sh
./scripts/lint-no-eprintln.sh
./scripts/lint-viewport-row-scroll.sh
./scripts/lint-render-purity.sh
./scripts/lint-landing-no-disk-read.sh
./scripts/lint-platform-boundary.sh
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
```

On Windows, run non-interactive checks through `scripts/run-hidden.ps1`
or the test suite through `scripts/test-windows.ps1`. Every newly launched
parent or nested shell must also be hidden; preserve output and exit codes.
Background shells use `Start-Process -WindowStyle Hidden` or equivalent.
This does not apply to GUI screenshot launches, which need a visible window.

During iteration, `cargo test -p daruda -- --skip workspace::tests` avoids
the expensive real-Workspace tests, which create tabs and spawn shells.
It is an iteration shortcut, not a replacement for the full pre-commit suite.

The ACP preset drift gate is offline. It regenerates the generated block in
`packages/config/src/agent/preset.rs` from
`tools/gen_acp_presets/registry-snapshot.json` and fails if committed output is
stale. Use `scripts/sync-acp-registry.sh` only when intentionally refreshing the
registry snapshot.

---

## Tests and CI

[ci.yml](.github/workflows/ci.yml) defines current job coverage. It runs
macOS, Linux, and Windows build/test jobs plus one source-only Lint job.
The Lint job runs format and selected lint scripts; the full local checklist
above remains required even for checks not yet wired into CI.

[scripts/ci-cargo.sh](scripts/ci-cargo.sh) owns the shared package coverage
for `clippy`, `app-features`, `test`, and `doc`; extra Cargo options follow
the gate name. Update coverage there. The `test` gate includes
`gpui_component`'s daruda-authored patch tests. The doc-link gate covers its
explicit crate subset, not every workspace crate; Clippy does not validate
intra-doc links. All app features are off by default, so the `app-features`
gate checks feature-only code on every OS.

[setup-build](.github/actions/setup-build/action.yml) owns the shared
Rust/Zig setup, locked fetch, and vendored-GPUI check for CI and release.
Linux dependencies live in `scripts/linux/{build,smoke}-deps.txt`;
`scripts/linux/smoke.sh {x11|wayland} <binary>` also runs locally.
Linux checks both X11 and Wayland startup/paint; Windows checks native
builds, startup, platform adapters, and packaging.

A local pass establishes only the host's compiled branches. Platform changes
require default and all-feature Clippy on all three OSes; report other OS
results as pending until CI passes. For rendering, IME, and window state,
tests alone are insufficient: use the capture workflow below and report
remaining desktop checks.

---

## Visual Verification

Render the UI offscreen to a PNG and read it back — text, layout, colors, images, and toasts all render, permission-free (no Screen Recording grant). Capture goes through gpui's `render_to_image`, gated upstream behind `test-support`; the `--screenshot` path below requires it, plus `gpui_macos/font-kit` (without that feature glyphs don't rasterize — shapes render but **text is invisible**).

**macOS:** offscreen capture uses `MetalHeadlessRenderer`. **Windows:**
`--screenshot` uses the visible composed client surface instead of
`render_to_image`, including native fonts and physical DPI-scaled pixels.
Launch the GUI normally; `run-hidden.ps1` can hide its first window and make
capture fail. Hidden and uniform blank captures return an error. **Linux:**
no capture implementation is available; use `daruda --smoke` for startup checks.

**Whole app** — the `--screenshot` flag captures the live workspace window:

```bash
cargo build -p daruda --features screenshot
target/debug/daruda --screenshot /tmp/shot.png   # opens, settles ~2s, captures, quits
```

The opt-in `screenshot` feature enables `gpui/test-support` + `gpui_macos/font-kit`; it is off by default to keep the shipping binary clean. Entry point: `packages/app/src/screenshot.rs`.

Verification loop: render → PNG → an agent reads the PNG and checks the result. This catches both rendering bugs and runtime state (e.g. error toasts), so it doubles as a smoke test of the real app's startup.

### Driving the Captured State

`--screenshot` takes no view argument — it captures the **restored** workspace (empty or not), so you steer it by steering what gets restored *before* launch. Two isolation levers + three reach tiers:

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

**Tier 2 — persisted state under `DARUDA_DATA_DIR` (layout & structure):** `state/workspace/workspaces/<uuid>.json` (`WorkspaceState`: dock open/size, window bounds, active project+lane, `active_dock_view`, `active_right_panel_view`, focused pane, groups), `state/workspace/projects/<uuid>.json` (`ProjectState`: lanes, tabs, pane split tree, file-pane path + view_mode), `panels.json` (macro grid), `tasks.json` (task list). (Schemas in `daruda_store::project::{WorkspaceState,ProjectState}`.) **Easiest seeding: set `DARUDA_DATA_DIR`, drive the app by hand to the scenario once, quit, re-run with `--screenshot` against the same dir** — no schema guessing.

**Tier 3 - transient/live state:** restore recomputes it, so config or persisted
state alone cannot reach it. `--screenshot-scenario <name>` applies a scenario
after settle and forces a repaint before capture.

| Area | Example scenarios and capture intent |
|---|---|
| Overlays | `command-palette`, `error-modal`, `toast`, `pane-context-menu`, `mermaid-lightbox` |
| Navigation | `lane-switcher` seeds an overflowing label; `landing` empties the restored workspace |
| Settings | `settings[:<section-slug>]`, `settings-error`, `settings-search` |
| Agent catalog | `agent-catalog-expanded`, `agent-catalog-editor[:<fold\|filter\|range>]` |
| Window chrome | `client-chrome` forces app-drawn controls; native macOS traffic lights still cover the left inset, so assess the right edge and spacing |
| Tab status | `tab-indicators` seeds each status dot; inspect its color |
| Status bar | `status-ports`, `status-flows`, `status-usage`, `status-account`, `keep-awake`, `keep-awake-failed` — each chip's popup open over seeded data |
| Agent chat | `agent-chat`, `agent-chat-working`, `agent-chat-empty`, `agent-chat-failure` |
| Chat interaction | `agent-chat-fold`, `agent-chat-interrupted`, `agent-chat-overage-stopped`, `agent-chat-options[:<tab>]`, `agent-chat-queue-parked`, `agent-chat-queue-armed` |
| Chat details | `agent-chat-running-tool`, `agent-chat-plan[-stopped]`, `agent-chat-thoughts[-working\|-only]`, `agent-chat-sole-reply` |
| Transcript boundaries | `agent-chat-tail[-open]`, `agent-chat-group-tail[-open]`, `agent-chat-subagent-tail[-open]` |
| CLI transcript | `agent-chat-cli-running`, `agent-chat-cli-ended` |
| Flows | `flow-picker`, `flow-profile-picker`, `flow-running`, `flow-asking`, `flow-resumable`, `flow-delete-confirm` |
| Flow graph | `flow-graph`, `flow-graph-running`, `flow-graph-form[-refused]`, `flow-graph-pinned`, `flow-graph-authoring`, `flow-run-detail` |

Bracketed forms summarize related names, not literal CLI arguments. The
[scenario name registry](packages/app/src/workspace/screenshot_scenario/names.rs)
is authoritative for supported tokens and parameter axes:
Settings uses `BuiltinSection::slug`; chat options use
`ActivityOptionsTab::token`. Add a scenario to `ScreenshotScenario` and
`screenshot_scenario::drive`, and register its CLI name in `names.rs`.

Agent-chat scenarios seed fixed transcripts through
`AgentChatView::seed_transcript_for_shot` without an ACP session.
The states below still need a real source or live interaction.

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

---

## Packaging

Build the macOS app bundle:

```bash
./scripts/build-app.sh
```

Package a DMG:

```bash
brew install create-dmg
./scripts/build-dmg.sh
```

Build the Linux x86_64 packages — a portable tar.gz and a `.deb` — on a
Debian or Ubuntu host:

```bash
./scripts/linux/apt-install.sh scripts/linux/build-deps.txt jq imagemagick dpkg-dev
./scripts/bootstrap-zig.sh
./scripts/package-linux.sh
```

To check that a build opens its window without a desktop, install
`scripts/linux/smoke-deps.txt` the same way and run
`./scripts/linux/smoke.sh x11 target/release/daruda` (or `wayland`).

Both land in `target/packages/`. The `.deb` installs `/usr/bin/daruda`, the
launcher entry (`scripts/linux/daruda.desktop`), and the hicolor icons;
`dpkg-shlibdeps` derives its `Depends` from the binary, plus what is loaded
at runtime and so absent from it: `libvulkan1` and the Wayland client
libraries. The tar.gz runs from wherever it is extracted.
Neither updates itself: Settings › About announces a newer release and the
package is reinstalled from it. `--profile debug --skip-build` packages an
existing debug build.

Windows builds embed `assets/icon.ico` into `daruda.exe`; the installer and
uninstaller use the same icon. After changing `assets/icon.png`, regenerate
the committed ICO with `./scripts/make-icon.ps1` on Windows before building.

Build a portable Windows x86_64 ZIP (including MSVC runtime DLLs):

```powershell
./scripts/bootstrap-zig.ps1
./scripts/package-windows.ps1
```

The package is written to `target/packages/daruda-<version>-windows-x86_64.zip`.
Extract the entire archive and run `daruda.exe`; keep its DLLs beside it.
This is an unsigned portable build, with experimental Windows GUI support.
To package an existing debug build for testing, use
`./scripts/package-windows.ps1 -Profile debug -SkipBuild`.

Build a per-user installer from that ZIP with NSIS 3.11 on PATH:

```powershell
./scripts/build-windows-installer.ps1 -Archive target/packages/daruda-0.2.16-windows-x86_64.zip
./scripts/test-windows-installer.ps1 -Installer target/packages/daruda-0.2.16-windows-x86_64-setup.exe
```

The installer defaults to `%LOCALAPPDATA%\Programs\daruda`, requires no
administrator permission, and registers shortcuts and an uninstall entry.
It checks every shipped file for locks before replacing or removing files.
Uninstall preserves application data and user-created files, keeps registration
on removal failure, and removes shared integration only when its registered
directory still belongs to that installation. The installation test refuses to
run if existing daruda desktop integration would be overwritten.

Installer deployments download the checksum-verified setup asset rather than
the portable ZIP. Restart opens that installer, which waits for the old process
to exit, updates its file manifest and Windows registration, and relaunches the
app after successful installation. The verified setup remains in the user's
temporary directory for retry or diagnosis. Portable ZIP deployments retain
their existing file-swap update path.
For signed builds, pass `-CertificateThumbprint <thumbprint>` to both the ZIP
packaging script and the installer
build script. The certificate must be in `Cert:\CurrentUser\My` with an accessible
private key. The app, uninstaller, and installer are signed and timestamped;
signature verification failure stops the build. CI builds remain unsigned
until a release signing identity is provisioned. The release workflow accepts
`WINDOWS_SIGNING_PFX_BASE64` and `WINDOWS_SIGNING_PFX_PASSWORD` repository secrets.
Set the `WINDOWS_SIGNING_REQUIRED` repository variable to `true` to refuse an
unsigned Windows release. Signing or signature-verification errors fail packaging.
Updates to an already signed installation must retain its verified publisher;
existing unsigned releases continue to use release checksums.

Workspace and project records retain their previous readable `.bak` generation.
Recovery preserves damaged original bytes before restoring a valid backup;
an unrecoverable project prevents partial workspace restoration from replacing
the saved workspace. Portable updates use a durable swap journal and recover
interrupted replacements before clearing rollback files.

To retain the terminal's last visible text across app restarts, opt in:

```toml
[shell]
restore_output = true
```

This setting also supports project overrides. Each pane retains at most 64 KiB,
within an 8 MiB workspace limit, in profile-scoped storage. Restored text is
marked as previous output and control sequences are filtered. A fresh shell
starts; the snapshot does not preserve a process, colours, or full scrollback.
Disable the setting to remove stored snapshots on the next workspace save.

Desktop preferences can be set in the profile's `config.toml`:

```toml
[desktop]
tray_enabled = true                   # Windows
close_to_tray = true                  # Windows
keep_awake = "agent"                  # "off" (default) | "agent" | "on"
keep_awake_approval_wait_minutes = 30
```

Tray creation takes effect on the next launch. Closing a window then hides it;
the tray or a second launch restores it. Explicit Quit retains the running-work
and unsaved-edits prompts. `keep_awake` holds off system sleep on every
platform: `agent` while an agent chat or Claude terminal works (at most 2 hours
per stretch) or waits for approval (at most the configured minutes), `on`
always. The display may still sleep, and closing the lid follows the device's
power settings.
Help > Export Diagnostics writes a ZIP with version/platform information and
bounded error metadata, excluding raw messages, personal paths, prompts, and transcripts.
Windows toast clicks reveal the originating pane while the app is running.

GitHub Actions produces Windows downloads in two ways:

- **CI** (push, pull request, or **Run workflow**): the Windows job uploads
  `daruda-windows-x86_64-debug` under the run's **Artifacts**, retained for 14 days.
- **Release**: pushing a `v<version>` tag matching `Cargo.toml` builds the macOS
  DMG, the Windows release ZIP and per-user installer, and the Linux `.deb` and
  tar.gz, then publishes them after every build succeeds. **Run workflow** builds downloadable release artifacts
  without publishing a GitHub Release.

Both Windows workflows use the same Zig installer and package script as local
builds. The installer verifies the pinned archive's SHA256. Missing package
files fail the workflow instead of producing an empty download.

---

## Project Structure

```text
daruda/
├── packages/                   # first-party packages (directory ≠ Cargo name)
│   ├── app/                    # daruda: workspace, agent UI, docks, surface
│   ├── acp/                    # daruda_acp: Agent Client Protocol client core
│   ├── agent/                  # daruda_agent: agent provider integrations
│   ├── config/                 # daruda_config: config system and agent presets
│   ├── content/                # daruda_content: image + mermaid processing
│   ├── control-types/          # daruda_control_types: control command contract
│   ├── core/                   # daruda_core: shared dependency-free utilities
│   ├── flow/                   # daruda_flow: declarative ACP flow engine
│   ├── flow-edit/              # daruda_flow_edit: source-preserving flow-file edits
│   ├── project/                # daruda_project: runtime Project/Lane model, git ops
│   ├── store/                  # daruda_store: persistence and observability
│   ├── terminal/               # daruda_terminal: terminal emulation, GPUI rendering
│   ├── transcript/             # daruda_transcript: transcript presentation model + editors
│   ├── chat/                   # daruda_chat: the agent chat pane view
│   ├── settings/               # daruda_settings: the Settings surface and the config store
│   ├── ui/                     # daruda_ui: domain-free widgets and theme bridge
│   ├── update/                 # daruda_update: app update checking
│   ├── ghostty-vt/             # ghostty_vt: safe Rust wrapper over libghostty-vt
│   ├── ghostty-vt-sys/         # ghostty_vt_sys: Zig C FFI bindings
│   └── visual-tests/           # visual_tests: offscreen render snapshot tests
├── tools/
│   ├── acp_replay/             # ACP wire-log replay helper
│   ├── gen_acp_presets/        # generated agent preset drift gate
│   ├── gen_licenses/           # third-party license manifest generator
│   └── vt_dump/                # headless VT diagnostic CLI
├── vendor/                     # third-party sources, one directory per upstream
│   ├── ghostty/                # Ghostty v1.2.3 submodule
│   ├── zed/                    # patched GPUI; regenerate with tools/vendor_gpui
│   ├── gpui-component/         # vendored gpui-component crates
│   └── ferrum-flow/            # vendored node-graph canvas
└── scripts/
```

GPUI imports belong in UI-facing crates only. GPUI-free crates should stay
usable from background logic, tests, and command-line tools.

---

## Code Style

All code, comments, identifiers, and Markdown documents must be written in
English.

Use `cargo fmt`; formatting drift should not be hand-waved. Add tests for
behavior changes, with inline unit tests for small modules and integration tests
under `crate_root/tests/` when cross-module behavior is involved.

User-visible failures should route through the app error-reporting path instead
of `eprintln!`, which is invisible to users running the `.app` bundle.

A few rules that often come up in review:

- No inline magic numbers or colors outside the theme/surface layers.
- No re-entrant entity reads inside GPUI render/update paths.
- App code routes through `crate::ui::*`; do not import `gpui_component` or
  `daruda_ui` directly outside the facade at `packages/app/src/ui/`.
- Blocking subprocess calls must run on the background executor.
- User-facing actions need the action type, registered handler, shortcut
  constant, and command palette entry.

For the full agent workflow and project guide, see [AGENTS.md](AGENTS.md).

---

## Commit Conventions

daruda follows Conventional Commits:

```text
<type>(<scope>): <short summary>
```

Common types:

| Type | When to use |
|---|---|
| `feat` | New user-visible feature |
| `fix` | Bug fix |
| `refactor` | Code restructure with no behavior change |
| `perf` | Performance improvement |
| `test` | Adding or fixing tests |
| `chore` | Build, CI, dependency updates |
| `docs` | Documentation only |

Keep the summary under 72 characters. Use the body for the reason behind the
change.

Coding agents follow the narrower [agent commit rules](AGENTS.md#branches-and-commits):
use an unscoped imperative subject, and stage, commit, or push only when requested.

---

## Pull Requests

1. Open an issue first for non-trivial changes so the approach can be aligned
   before implementation.
2. Keep PRs focused on one logical change.
3. Update tests for behavior changes.
4. Run the local checks that apply to the change.
5. Keep CI green before review.
6. Squash or rebase before merge; do not use merge commits on `main`.

PR titles should follow the same `type(scope): summary` format as commits.

---

## Contributor License Agreement

Contributions are accepted under the [Contributor License Agreement](CLA.md).
The CLA is not a copyright assignment: contributors keep ownership of their
contributions while granting daruda the rights needed to review, maintain, and
distribute accepted work.
