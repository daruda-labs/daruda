# UI Instructions

Domain-free widgets, shared theme metrics, and the app's UI facade.
App callers use `crate::ui::*`; sibling widget packages use `daruda_ui`.
Direct `gpui_component` and `ferrum_flow` paths outside their allowed
infrastructure sites are forbidden, including fully qualified paths.
The flow canvas is exposed through `crate::ui::flow_canvas`. The boundary
is enforced by `lint-direct-gpui-component.sh` and `lint-direct-ferrum-flow.sh`
under `scripts/`.

## Required Reading

Read the matching section of [UI Reference](REFERENCE.md) before changing
wrappers or vendored behavior. These detailed contracts are not optional.

| Working on | Read |
|---|---|
| New or changed wrapper | [Shapes and examples](REFERENCE.md#three-wrapper-shapes), [trait exports](REFERENCE.md#trait-re-exports--what-to-put-in-modrs), [checklist](REFERENCE.md#adding-a-new-wrapper) |
| Widget placement or the current wrapper catalogue | [Module layout](REFERENCE.md#wrapper-module-layout), [app UI](../app/src/REFERENCE.md#ui) |
| Vendored widgets or an upstream update | [Preserved patches](REFERENCE.md#vendor-patches-in-vendorgpui-componentgpui_component), [patch workflow](../../patches/README.md) |
| Modals and input focus | [App modal rules](../app/src/AGENTS.md#g9--modals-go-through-gpui_componentdialog), [tab navigation](../app/src/REFERENCE.md#tab-navigation-in-modals) |
| Theme presets | [Theme instructions](../../assets/themes/AGENTS.md), [Design](../../DESIGN.md) |

## Wrapper Contracts

- Shared widgets take user-visible strings as arguments. The app facade
  supplies them through `surface::strings`; localized wrappers and
  agent-domain widgets stay in that facade, not this domain-free package.
- Use the existing factory, variant factory, plain re-export, or closure-helper
  shape. Factories apply shared chrome and `xsmall()`; callers do not repeat
  sizing or per-button geometry.
- Prefer named variant factories to boolean mode arguments. Re-export only
  traits callers actually need; do not re-export `Sizable`.
- Stateful preserved widgets use `Entity<T>` + `Render`. IME-critical inputs
  keep their composition and focus contracts.
- New wrappers update the reference catalogue and migrate real callers;
  do not add unused primitives or speculative trait exports.

## Token ownership

- Shared numeric scales live in `theme/metrics.rs`: padding, gaps, fonts,
  radii, and standard icon/target sizes. Normal callers use `theme::*`;
  `palette::*` re-exports the same definitions for compatibility.
- Colors stay in the palette and live theme. Chrome reads `theme::current`;
  terminal-mirrored panes use `PaneSurfaceTokens` and surface-aware factories.
- A component owns its recipe. Use primitive tokens directly for ordinary
  spacing, type, and corners, not a new screen-prefixed alias. Independent
  component constraints live beside their owner: `switch.rs` owns its two
  `Metrics` recipes, `form_helpers.rs` owns its label gutter, and
  `section_header.rs` owns the existing 10.5px quiet label size. These are
  definition sites for named metrics, not exemptions for inline `px` literals.
- Icon buttons and dock toggles share a 24px target and 4px corners. Standard
  artwork is 16px; dock toggles keep 14px artwork and tree glyph buttons keep
  their explicit targets. Do not repeat the factory's geometry at call sites.
  Passive `Badge` labels, actionable chips, and custom status pills remain
  distinct contracts. Buttons stay outside Tab unless the caller opts in;
  switches retain their keyboard participation and disabled behavior.
- `theme/list_metrics.rs` holds actual shared column-content budgets.
  `ListTable` derives geometry from the visible schema; `list_page` owns page
  chrome. Neither acquires domain filtering, selection, or state transitions.
- Keep independent content budgets and semantic color roles even if their
  values happen to match. Keep runtime limits and timers with feature ops.
- Before adding a token, check the existing scale, then the existing
  component, then the independent constraint and its narrowest visibility.
  `bash scripts/report-ui-token-aliases.sh` reports new direct numeric aliases
  against HEAD, including untracked UI source. It is advisory; a meaningful
  role can carry an inline `// token-role: <reason>` exception. Legacy aliases
  and textual unused candidates are review inputs, not automatic deletions.

## When NOT to add to ui/

- **Workspace-specific widgets** (a button that knows about
  `Workspace`, `Pane`, etc.) — those live alongside the feature, not in
  `ui/`. `ui/` only holds primitives that are independent of daruda
  domain types.
- **Modal shells / dialog chrome** — `gpui_component::Dialog` provides
  the panel chrome (bg / border / padding / title / backdrop). Open via
  `crate::workspace::dialog_helpers::{open_form_modal,
  open_single_field_dialog}`, or `dialog::{open_confirm_dialog,
  confirm_destructive}` here for a plain confirmation. Don't re-create
  `FormModalShell`-style wrappers in ui/.
- **Composite layouts that consume ui primitives** — small helpers
  like `form_helpers::field_row` are fine in ui/, but anything that
  pulls in workspace types belongs alongside the feature.
- **One-off helpers** used in a single file — keep them inline; ui/ is
  for primitives that get reused.

---

## Destructive actions confirm first

An action that cannot be undone, or that reaches a system outside daruda
(the credential store, a paired chat, a removed config entry), opens
`dialog::confirm_destructive` and runs only from its OK.
Its button label ends in `…`. A reversible value change, including Reset,
never confirms.

## Allowed exceptions to the import rule

A small set of infrastructure files legitimately calls `gpui_component::*`
directly. The lint script (`scripts/lint-direct-gpui-component.sh`)
allow-lists them:

| File | Why |
|---|---|
| `src/main.rs` | `gpui_component::init(cx)` + `ui::theme::apply_daruda_palette(cx)` at app startup |
| `src/test_support.rs` | `init(cx)` + palette overlay for unit-test fixtures |
| `src/windows.rs` | `gpui_component::Root::new(...)` Workspace window wrapping |
| `src/workspace/render/mod.rs` | `Root::render_*_layer(...)` in `Workspace::render` |
| `src/ui/` | The facade over this package |

Don't grow this list. If a new file needs direct `gpui_component`
access, ask first whether the access belongs in `ui/` instead.

---

## GPUI Render and Event Safety

- Reading the same entity with `.read(cx)` during its `render()` or
  `entity.update()` can panic. Stage plain snapshots and avoid re-entry.
  App persistence must go through deferred `mark_dirty_and_save`.
- GPUI has no partial window redraw; every dirty view repaints the window
  tree. Keep sibling subtrees cached and notify only the changed entity.
- Never call `window.refresh()` or `cx.refresh_windows()` on a hot path.
  Refresh sets `window.refreshing` and bypasses every `AnyView::cached`
  (GPUI `view.rs` prepaint checks `!window.refreshing`).
  Reserve it for global invalidation, such as a theme swap in `ui/theme.rs`.
  Use targeted `cx.notify(entity)` for other changes; see zed PR #25009.
- Cached children require notify-on-change. Parent-staged snapshots
  (such as `Dock::snap`) must notify the child when their content changes.
  Self-notifying TerminalView and ToastLayer are safe; bare field assignment
  through `entity.update` without notify is incompatible with caching.
- Accessing an off-screen entity during render re-arms its invalidation:
  GPUI tracks accessed entities as displayed by that window. Do not stage
  snapshots or read geometry for hidden docks. Apply the visibility condition
  before the entity access, not just before constructing its element.
- `window.text_style()` and `window.rem_size()` are paint-scope state.
  Events must obtain cached/shared metrics rather than use those calls
  outside the paint walk. Terminal details and measurement exceptions are in
  [paint metrics](../terminal/src/view/AGENTS.md#paint-scope-metrics).
- Text/pixel hit testing uses the shaper's reverse mapping, never
  `index = offset_px / glyph_advance`.
- Raw `window.on_mouse_event::<MouseDownEvent>()` hears every button.
  Filter `event.button` explicitly. Unlike that API, `div().on_click()`
  is left-only, and `.on_mouse_down(MouseButton::X, ...)` names its button.
  A button-agnostic listener needs `// ANY-BUTTON: <reason>`; accepted
  cases end a drag or dismiss on an outside press.
- [Raw-mouse lint](../../scripts/lint-raw-mouse-button.sh) scans vendored
  crates too, because re-vendoring can remove a button gate. Keep its
  `--self-test` coverage for generic/path-qualified/braceless listeners,
  comments, and braces inside strings.

## Vendor Dependencies

`gpui_component` is a vendored `longbridge/gpui-component` copy (Apache-2.0).
`ferrum_flow` is a vendored `tu6ge/ferrum-flow` copy at `43b762ce`
(Apache-2.0), providing the flow-editor canvas. Patch real defects only;
exclude upstream code from unrelated Clippy/lint/comment-cleanup passes.

Existing explicit gates still apply: the raw-mouse lint scans both trees,
daruda-authored `gpui_component` patch tests run in CI, and `ferrum_flow`
declares `rust-version` with `clippy::incompatible_msrv` armed.
The app accesses both through its UI facade; sibling widget packages use
`daruda_ui`, never `gpui_component`.

`ferrum_flow` uses `gpui = { workspace = true }` so its types share daruda's
pinned GPUI instance. Preserve upstream modules for reproducible re-vendoring,
rather than pruning unused ones. Provenance, retained patches, and re-vendor
procedures live in [patches/README.md](../../patches/README.md).

GPUI itself is generated in `vendor/zed` from the pinned source and
`patches/gpui-*.patch` by `tools/vendor_gpui`; never edit that tree by hand.
