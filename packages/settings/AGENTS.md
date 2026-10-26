# daruda_settings

The Settings surface (`SettingsView`: sidebar plus one `BuiltinSection` page)
and the user-config store it edits (`store::SettingsStore`). A `Workspace`
hosts the view in place of its body; the app keeps that host in
`packages/app/src/workspace/settings_ops.rs` and re-exports the store as
`crate::settings_store`.

## The host boundary

The view knows nothing of `Workspace`, windows, accounts' owners or the app's
runtimes. Everything crosses in one of four ways:

| Direction | Through |
|---|---|
| Host → view, at construction | `SettingsView::new_with_section`: a `SharedSnapshot` (accounts, login busy, sign-in readings, agent vocabulary, plugin skills, update state), an `Rc<dyn SettingsServices>`, and the remote channels' own settings as an `AnyView` |
| Host → view, later | pushes from the host's observers: `set_accounts` / `set_auth_statuses` / `set_agent_vocabulary` / `set_plugin_skills` / `set_update_state` / `set_project_open` |
| View asks, needs the answer now | `SettingsServices` — pane counts across windows, the Telegram credential and bridge |
| View → host, anything that changes windows | `SettingsEvent` — close, login, accounts written, plugins changed, an update step |

A new need picks the narrowest row: state that changes under Settings is a
push, a synchronous question is a `SettingsServices` method, an effect is an
event. Never a dependency on the app.

## Rules

- **Store isolation in tests.** `SettingsStore` starts at defaults and writes to
  a temp file under `cfg(any(test, feature = "test-support"))`. The app's
  dev-dependency turns `test-support` on; without it the app's tests would
  read and rewrite the developer's own `config.toml`.
- **Test seams return plain values** (`accounts()`, `auth_statuses()`, …),
  gated on the same `cfg`; capture seams (`seed_*_for_shot`, `EditorShot`) sit
  behind `screenshot`.
- **Strings** — this package's `locales/` holds every key the view shows,
  generated through `strings_gen`. The app keeps its own copy of a key only
  where its own surface shows it too. A value both name — an auth domain, a
  model choice, a refused session host, a section — has one label, in
  `labels` or `section_label`, which the app calls.
- **Widgets** come from `daruda_ui` directly, never `gpui_component`.

## Layout

- `initialize/` builds widgets from the host's snapshot. Its private appearance,
  workspace, behavior, and render select constructors keep allocation separate
  from the ordered subscription and focus wiring in `mod.rs`; `bindings.rs`
  owns ordinary input subscriptions and value synchronization.
- `catalog/` owns agent rows, subscriptions, collection and saving;
  `validation.rs` is pure, `path_warning.rs` checks executables on edit, never
  while rendering.
- `hosts/` owns session-host rows; `hosts/reconcile.rs` is GPUI-free.
- `persistence.rs` is the one save boundary: field patches, drafts, external
  changes, conflicts.
- `sections/` and `render/` build the surface; the crate root keeps view state,
  navigation, events and presentation helpers.
