# daruda_core

Shared logic below both the GPUI app and the GPUI-free packages. Read this
file when adding core utilities or using process, path, shell, host, or
environment boundaries from any package.

## Admission and Dependencies

- No `daruda_*` or GPUI dependencies. Weigh external dependencies against
  their cost to every consumer; prefer target-gated dependencies.
- Shared pure logic normally has two or more consumers. A registry is weighed
  as one shared contract; a capability gate is admitted because it contains
  the only permitted OS call. See `src/lib.rs`.
- Modules are pure by default: values in, values out; no filesystem/network
  I/O or hidden caches. The exceptions are registered environment reads
  and the named platform capabilities.
- Language identity belongs in `language`; highlighting availability is a
  separate registry-dependent question answered in the app's highlighter.
  Pure path policy takes `path_style::PathStyle`, not the build OS.

## Environment Variables

Daruda-owned names have one spelling in `process_env`. Its opaque `Key`
allows reads of registered names only, without writes or caching.
Bootstrap writes stay where the caller can prove single-threaded execution;
`set_var` is unsafe after other threads may access the environment.
Enforcement: [environment lint](../../scripts/lint-env-literals.sh), including
examples and tests.

## Platform Capability Boundary

An OS call that differs by platform has one named gate; domain code calls it.

| Instead of | Call |
|---|---|
| `Command::new` | `daruda_core::process::command` |
| `libc::kill*` / `process_group` | `daruda_core::process::{lead_own_group, Group}` |
| `fs::canonicalize` | `daruda_core::path::{canonicalize, canonicalize_or_self}` |
| `env::var("SHELL")` | `daruda_core::shell::interactive` |
| `std::os::unix::fs::symlink` | `daruda_core::path::symlink` |
| `Url::{to,from}_file_path` / a `"file://"` literal | `daruda_core::file_url::{is_file_url, to_local_path, from_local_path, FileUrl}` |
| `gethostname` / `GetComputerNameExW` | `daruda_core::host::name` |
| `env::var("HOME")` | `dirs::home_dir` (Windows has no `HOME`) |
| User-typed program names such as `npx`, `claude`, an editor CLI | `daruda_core::process::command_on_path` (Windows PATHEXT shims) |
| Raw equality/prefix comparisons of paths from different producers | `daruda_core::path::{same_path, is_within, strip_root}` |

`Url::to_file_path` decodes by the build OS. A remote machine's path must
instead go through `file_url` with an explicit `PathStyle`, keeping
"is this a file URL?" separate from its local interpretation.
Use `join_paths` for PATH construction, never a hardcoded separator.

### Allowed Regions and Exceptions

- Core gates: `host`, `process`, `path`, `shell`, and the pure `file_url`.
- Desktop gates: `packages/app/src/platform/`, including native window APIs
  and listening-port inspection. Workspace polling and Lane attribution
  stay in their domain modules.
- Separate ownership gates: `packages/app/src/remote_channel/keychain.rs`
  (daruda secrets), `packages/agent/src/accounts/credentials.rs`
  (another program's credentials), `packages/app/src/shell_env.rs`
  (macOS app-launch PATH hydration, not shell selection).
- `terminal::pty` owns PTY I/O over private `pty/backend/` implementations,
  including ConPTY drain ordering.

Public entry points select private OS backends once. `core::process` retains
its API over `process/{unix,windows}.rs`; credential gates retain their
distinct ownership and profile contracts. File-error classification belongs
in `core::path::io_error`; retry budgets and cancellation stay with callers.
Pure policies remain values, such as `acp::node::layout` distributions and
`core::font` factory font names without GPUI objects.

### Platform Policy and Compilation

- Prefer values over `#[cfg]` for pure policy so every host can test it.
  Reserve `#[cfg]` for the OS leaf. Adding a platform extends a gate rather
  than scattering changes through domain crates.
- Gate platform-only modules, imports, functions, constants, and generated
  string overrides with the same `#[cfg]` as their callers. `cfg!()` still
  compiles both branches.
- Keep common control flow independent of platform-only statements; do not
  compile away a loop's only `continue`. Isolate retry policy in a capability.
- OS-only locale keys stay in both languages with a matching gated function
  in `surface/strings/custom/`; never edit generated output or silence dead code.

Enforcement: [platform lint](../../scripts/lint-platform-boundary.sh).
Shim selection and comparisons of paths from different producers require
review; text scans cannot infer their provenance. The lint's explicit
exceptions allow test fixtures spawning `git init` outside the domain gate.
