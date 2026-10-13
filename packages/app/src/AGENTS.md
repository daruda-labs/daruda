# App Instructions

GPUI-side of daruda: app entry, windows, Workspace, panes, and host integration.
These guardrails apply to app source. Detailed maps and recipes live in
[App Reference](REFERENCE.md); read the matching section before changing it.

## Required Reading

| Working on | Read |
|---|---|
| File placement, module ownership, or a new affordance | [Layout](REFERENCE.md#directory-layout), [recipes](REFERENCE.md#extension-recipes-quick-reference), [decision matrix](REFERENCE.md#where-things-go-decision-matrix) |
| Config precedence, agent data, or workspace lifecycle | [Subsystem contracts](REFERENCE.md#layered-config-user--project) through the workspace and lane sections |
| Modals, focus, or inputs | G9 below, [tab navigation](REFERENCE.md#tab-navigation-in-modals), [UI rules](../../ui/AGENTS.md) |
| Package dependencies or desktop composition | [Crate graph](REFERENCE.md#crate-dependency-graph), [UI hierarchy](REFERENCE.md#ui-component-hierarchy) |
| Host Settings or chat | [Settings](../../settings/AGENTS.md), [Chat](../../chat/AGENTS.md) |

## Maintenance guardrails

### G1 — Responsibility-driven splits

A file is split when its single responsibility breaks down, not when it crosses a line count. Line count is a *signal* to inspect, not a rule to enforce. A cohesive file with one clear responsibility stays as one file regardless of length.

**Real split triggers** (two or more apply → split):

- Two or more domains coexist in the file.
- Methods cluster into groups by naming prefix or shared private helpers.
- Tests require distinct fixtures for different parts of the file.
- Two engineers cannot work in parallel without merge conflicts.
- `tests.rs` exceeds 1.5× the parent `mod.rs` (test-data clustering signal).

**Line-count signals** — review trigger, not a hard cap:

- `mod.rs` ~800 lines / regular `.rs` ~600 lines → inspect whether responsibility is still single. Split only if a real trigger above is also true.

### G2 — Responsibility fences

- `render.rs` is `impl Render` + leaf UI builders only. No `std::process`, `std::fs`, or multi-field string formatting. Add display methods on data structs (e.g. `Pane::display_cwd()`).
- Data modules (`lane/`, `agent/mod.rs`, `main_area/pane_tree.rs`) must not import `gpui::Element`, `Context<Workspace>`, or `Window`.
- `lane/git.rs` stays GPUI-free. UI callers wrap with `background_executor`.
- Display-string helpers live next to the call site, not as data struct methods.

### G3 — 4-point chain rule

Every user-facing affordance must update all four:
1. Type — `actions!()`
2. Handler — `register_action` or `cx.bind_keys`
3. Constant — `surface/keybindings.rs::SHORTCUT_*` or a `locales/en.yml` key
4. Discoverability — `command::palette::PALETTE_ENTRIES` + `surface/action_map.rs` arm

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
main.rs → menus.rs, windows.rs → workspace/
  → title_bar/ → ui/, surface/
  → agent/, project/, lane/, surface/, pty.rs, config_watcher.rs

project/ → lane/
agent/ → lane/
```

- `surface/` imports nothing from siblings (except `action_map.rs` → workspace action types).
- `lane/` imports nothing from `workspace/`, `project/`, or `agent/`.
- `project/` imports `lane/` only; nothing from `workspace/` or `agent/`.
- `agent/` imports `lane/` only; nothing from `workspace/` or `project/`.
- The Settings view is `daruda_settings` (`packages/settings/`); its host is
  `workspace/settings_ops.rs`, and the boundary between them is in
  `packages/settings/AGENTS.md`.

When a function references a lane across module boundaries, pass
the full `daruda_store::project::LaneRef { project, lane }` —
plain `LaneId` is only valid inside `Project::*` methods where the
project context is implicit in `self`.

### G9 — Modals go through `gpui_component::Dialog`

- Every modal entity opens via `crate::workspace::dialog_helpers::*` — `open_form_modal` (entity owns full body), `open_single_field_dialog` (one input + OK/Cancel) — or, for a plain confirmation, `crate::ui::dialog::{open_confirm_dialog, confirm_destructive}` (title + body + OK/Cancel; domain-free, so it lives in `daruda_ui`). All wrap `gpui_component::WindowExt::open_dialog`; Dialog handles backdrop, Escape, and outer chrome.
- New modal entity: `impl ModalView` (supertrait of `Render + Focusable`) for entities passed to `open_form_modal`. No `Modal` trait, no `EventEmitter<DismissEvent>`, no `ModalLayer`.
- `ModalView` applies to `open_form_modal` only; `open_confirm_dialog` / `open_alert_dialog` / `open_single_field_dialog` do not use it.
- Focus is automatically restored to the previously focused element when the dialog closes (handled by `gpui_component::Root`).
- Entity owns only the body (fields, validation banner, footer buttons). Never re-render Dialog's chrome (`bg`/`border`/`p`/`rounded`/title) inside the entity.
- Dismiss via `window.close_dialog(cx)`. In `cx.defer` / async paths without a live `&mut Window`, capture `window.window_handle()` and re-enter via `cx.update_window(handle, ...)`.
- Async continuations: never nest two `entity.update` on the same `app_cx`. Run workspace finalize first, then update modal.
- Text input: `Entity<crate::ui::InputState>` rendered via `crate::ui::input(&state, cx, tab)`. IME is handled by `gpui_component`.
- **Modal callback `&mut Window` is live — use it, don't recapture handles**. When a modal's submit/dismiss callback fires with `(…, &mut Window, &mut App)`, that `Window` is the live workspace window inside its own update cycle. Re-entering `cx.update_window(captured_handle, …)` from the callback triggers a *nested* re-entry on the same window and GPUI returns `"window not found"` — the May-2026 add-project regression. Use the callback's `window` directly with `weak_workspace.update(cx, |ws, cx| ws.action(path, window, cx))`. Captured `AnyWindowHandle` is for `cx.defer` continuations where no `&mut Window` is in scope.

### G10 — Debugging policy

Reference: `superpowers:systematic-debugging` skill (Iron Law: no fixes before root cause).

- **Trace before fix** — `cx.update_window` failures, modal-callback hangs, dock not updating, etc. start with `LogWriter::log(Info)` traces at each component boundary (`prompt_and_open_folder_with_policy` → `handle_picked_folder` → `add_path_to_workspace` → `add_project`). Read `~/.daruda/logs/<profile>/daruda-YYYY-MM-DD.log`, grep the dedup tag prefix, identify the layer that breaks. Only then propose a fix.
- **Fix counter ≥ 2 = stop** — if two consecutive fixes have failed, the architecture is wrong, not the patch. Compare against reference (zed, version-matched to the built rev: `~/.cargo/git/checkouts/zed-a70e2ad075855582/193b55a/crates/{workspace,gpui}/`). Don't try a third patch in the same vein.
- **Reference comparison when unclear:** when GPUI lifecycle, window-context, or async re-entry behavior is unclear, consult the version-matched zed source before changing the implementation. Relevant examples include `workspace::update_in`, `cx.spawn_in(window, ...)`, and modal patterns.
- **Callback signature check** — when reading or writing a callback, account for **every** parameter. `_window` / `_cx` underscored prefixes are silent acceptances that the parameter is unused. If the parameter is `&mut Window`, suspect that you should be using it instead of capturing a handle.
- **Domain unfamiliar → admit + read references** — "I don't fully understand X" goes in the message, not pretending. Skip ahead only when you can name the reference you checked.

## MVU-Flavored Guiding Rules

Daruda is not strict MVU. These defaults apply to workspace views; an exception
needs a `// SAFETY:`-style comment naming the reason.

- **View purity:** `render()` and its handler closures carry no model-transition
  logic. Closures dispatch in one line, such as
  `weak_ws.update(cx, |ws, cx| ws.method(args, cx))`. Same model means the same
  screen. Layout-geometry caching inside `canvas()` (bounds, hitboxes, scroll
  offsets) is allowed because GPUI requires it; model changes are not.
- **One-way data flow:** views dispatch; Workspace and `*_ops.rs` own model
  changes (`add`, `remove`, `set`, `insert`, `delete`, `clear`, `toggle`,
  `update`, `open`, `close`). A view does not directly write another entity.
- **One source of truth:** mirrored state has exactly one update site.
  Extend that entry point rather than adding a parallel sync path.

## State Modeling

- Prefer an owning strategy over multiplying `if` branches for quick fixes.
  Keep each type responsible for one reason to change.
- Use existing abstractions rather than bypassing them with direct calls.
- A bool plus conditional data becomes `enum { Inactive, Active { data } }`.
- Replace flag combinations with valid enum states instead of
  `match (a, b) { ... _ => unreachable!() }`.
- Fields always `Some` or `None` together become `Option<(A, B)>` or one struct.
- Repeated multi-field transitions go through one function; seal the fields
  with `pub(super)` rather than repeating the same sequence in call paths.

## Error Reporting

New `eprintln!` is forbidden. Workspace failures go through
`self.report_error(report, cx)`: toast, details modal, and NDJSON log.
GPUI-free or pre-Workspace failures use `LogWriter::log(report)`.

GPUI re-entry (`cx.update_window`, `cx.update`, `entity.update`,
`entity.update_in`) returns a Result because its target can have closed.
Never swallow it with bare `let _ =`. Accepted forms:

- Match `Ok` / `Err` and report the error.
- Propagate with `?` when the enclosing function returns a Result.
- Use `crate::windows::try_update_workspace_window(handle, cx, "site_label", ...)`
  to log failures with a site label.
- An intentional ignore has `// SILENT-OK: <concrete reason>` on the preceding
  line. This is reserved for inconsequential failures such as restoring focus
  to a possibly closed window or a test fixture; an empty reason is invalid.

Enforcement: [update-result lint](../../../scripts/lint-no-silent-update.sh).
References: `workspace/error_ops.rs` and
`packages/store/src/observability/`. Persistence is deferred through
`mark_dirty_and_save`, never direct `persist_state` during an entity update.
See [GPUI render safety](../../ui/AGENTS.md#gpui-render-and-event-safety).
