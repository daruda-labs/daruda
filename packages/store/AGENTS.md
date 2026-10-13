# daruda_store

Persistence, profile storage policy, workspace repositories, and observability.
Read these rules from any package that writes data or creates a profile key.

## Cross-Profile Data Isolation

Storage policy belongs to private `storage::StorageLayout`. Domain code
uses purpose-specific persistence APIs, not OS directory discovery.

- `persistence::default_data_dir()` is the compatibility root for persisted data.
- `persistence::log_dir()` resolves native profile logs or
  `<DARUDA_DATA_DIR>/logs`.
- Use `profile_suffix()` for non-path identifiers, including Keychain services.
- All paths share one process snapshot of the override and profile.
  Never rederive them through fresh `dirs::*` calls or directory literals.
- `daruda_core::path::directories` discovers OS roots without app naming or
  filesystem writes. Storage layout, diagnostic compatibility reads, and
  log retention are separate responsibilities.

Isolation includes configuration, hook-status files, activity caches, and
credential service names. Hardcoded credential identifiers can make two
profiles use one bot token; a path lint cannot detect that collision.

## Layout and Migration Contracts

- Resolving layout must not move data. `project::WorkspaceStore` owns UUID
  workspace/project records and the recent list. The primary desktop instance
  prepares its native state repository before creating windows.
- Preparation validates a copy and publishes it atomically, preserving old
  files. Domain code receives the repository, not its root path.
- Explicitly injected repositories keep the existing flat layout for tests.
- Configuration, account data, managed tools, and resource-lock locations keep
  their contracts. Do not relocate them by changing a resolver alone.
- Native logs begin at the current location. Diagnostic export can read old
  default logs without copying or deleting them; an explicit data override
  excludes those old logs.

See [storage locations](../../CONTRIBUTING.md#storage-locations) for the
contributor-facing directory map.

## Deliberately Shared Resources

Application state is profile-scoped; claims on resources outside daruda are
shared. Three persistence APIs are deliberately profile-independent:

| API | Reason |
|---|---|
| `node_install_dir()` | One pinned Node runtime has the same bytes for all profiles. |
| `flow_lock_root()` | A working-tree mutex must exclude flows from every profile. |
| `remote_lock_root()` | A bot-account claim must exclude competing Telegram pollers or Slack socket routing tables across profiles. |

Each exception is named with its reason in persistence. Adding another
requires updating `clippy.toml`'s allow reasoning too.

## Enforcement

- [Clippy rules](../../clippy.toml) forbid bare `dirs::config_dir` outside
  `daruda_core::path::directories`.
- [Path-literal lint](../../scripts/lint-daruda-path-literals.sh) forbids
  hand-rolled `.join("daruda")` / `.join(".daruda")` outside `storage.rs`
  and its explicit exceptions: per-repo `.daruda/task-*.md` files and the
  global `~/.daruda/hooks/notify.sh`.
- Review credential service names manually against
  `packages/app/src/telegram/keychain.rs::service_name()`; neither guard
  catches a hardcoded OS-credential identifier.

## Error Reporting

GPUI-free and pre-Workspace failures use `LogWriter::log(report)`.
Workspace failures use the host's reporting pipeline; see
[app error reporting](../app/src/AGENTS.md#error-reporting).
New `eprintln!` and silently swallowed GPUI update Results are forbidden.
