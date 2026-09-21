# Contributing to daruda

This document is for contributors building, testing, packaging, or changing
daruda locally. For a short product overview and first run instructions, see
[README.md](README.md).

---

## Table of Contents

- [Requirements](#requirements)
- [Setup](#setup)
- [Local Checks](#local-checks)
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
| Linux | Builds and tests pass; GUI runtime still needs desktop verification; requires system `libfontconfig` and `libxcb` |
| Windows | Experimental native MSVC build; GUI runtime still needs desktop verification |
| Rust | 1.95+; CI pins the floor to 1.95 |
| Zig | 0.14.1; `./scripts/bootstrap-zig.sh` installs the pinned version on macOS |
| Xcode Command Line Tools | Required on macOS |

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

Windows CI gates the native app build, all app target compilation (including
`screenshot`), Clippy, Ghostty tests, vendor tool tests, platform adapters, and MCP tests.
The full Windows runtime suite still reports experimental diagnostics;
macOS and Linux retain their full required checks. GUI runtime requires a real
desktop check on each platform.

Windows runtime gaps remain: OS credential storage and listening-port discovery
are unavailable, and subprocess cancellation does not yet terminate descendant
processes as Unix process groups do.

---

## Local Checks

Run these before committing:

```bash
cargo fmt --all -- --check
cargo clippy -p ghostty_vt -p ghostty_vt_sys -p daruda_terminal -p daruda \
  -p daruda_config -p daruda_store -p daruda_agent -p daruda_update \
  -p daruda_acp -p daruda_core -p daruda_flow -p ferrum_flow \
  --all-targets -- -D warnings
./scripts/lint-inline-literals.sh
./scripts/lint-paint-scope.sh
./scripts/lint-reentrant-reads.sh
./scripts/lint-direct-gpui-component.sh
./scripts/lint-direct-ferrum-flow.sh
./scripts/lint-no-eprintln.sh
./scripts/lint-viewport-row-scroll.sh
cargo test -p ghostty_vt -p ghostty_vt_sys -p daruda_terminal -p daruda \
  -p daruda_config -p daruda_store -p daruda_agent -p daruda_update \
  -p daruda_acp -p daruda_core -p daruda_flow -p ferrum_flow
./scripts/lint-no-silent-update.sh
./scripts/lint-agent-activity.sh
./scripts/lint-daruda-path-literals.sh
./scripts/lint-file-size.sh
./scripts/lint-mark-dirty-direct-call.sh
./scripts/lint-fold-header.sh
./scripts/lint-declarative-context-menu.sh
./scripts/lint-acp-air-gate.sh
./scripts/lint-comment-length.sh
cargo run -p gen_acp_presets -- --check
```

The ACP preset drift gate is offline. It regenerates the generated block in
`crates/daruda_config/src/agent/preset.rs` from
`tools/gen_acp_presets/registry-snapshot.json` and fails if committed output is
stale. Use `scripts/sync-acp-registry.sh` only when intentionally refreshing the
registry snapshot.

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

GitHub Actions produces Windows downloads in two ways:

- **CI** (push, pull request, or **Run workflow**): the Windows job uploads
  `daruda-windows-x86_64-debug` under the run's **Artifacts**, retained for 14 days.
- **Release**: pushing a `v<version>` tag matching `Cargo.toml` builds the macOS
  DMG and Windows release ZIP, then publishes both in one GitHub Release after
  both builds succeed. **Run workflow** builds downloadable release artifacts
  without publishing a GitHub Release.

Both Windows workflows use the same Zig installer and package script as local
builds. The installer verifies the pinned archive's SHA256. Missing package
files fail the workflow instead of producing an empty download.

---

## Project Structure

```text
daruda/
├── crates/
│   ├── app/                    # main app binary: workspace, agent UI, docks, surface
│   ├── daruda_acp/             # Agent Client Protocol client core
│   ├── daruda_agent/           # agent provider integrations
│   ├── daruda_config/          # config system and agent presets
│   ├── daruda_core/            # shared dependency-free utilities
│   ├── daruda_flow/            # declarative ACP flow engine
│   ├── daruda_store/           # persistence and observability
│   ├── daruda_terminal/        # terminal emulation and GPUI rendering
│   ├── daruda_update/          # app update checking
│   ├── ferrum_flow/            # vendored node-graph canvas
│   ├── ghostty_vt/             # safe Rust wrapper over libghostty-vt
│   ├── ghostty_vt_sys/         # Zig C FFI bindings
│   ├── gpui_component*/        # vendored gpui-component crates
│   └── visual_tests/           # offscreen render snapshot tests
├── tools/
│   ├── acp_replay/             # ACP wire-log replay helper
│   ├── gen_acp_presets/        # generated agent preset drift gate
│   ├── gen_licenses/           # third-party license manifest generator
│   └── vt_dump/                # headless VT diagnostic CLI
├── vendor/ghostty/             # Ghostty v1.2.3 submodule
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
- App code routes through `crate::ui::*`; do not import `gpui_component::*`
  directly outside `crates/app/src/ui/`.
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
