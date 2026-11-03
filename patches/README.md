# patches/

Tracked diffs against external sources. Two distinct flavours live
here — they serve different goals and have different application
flows.

## Shared Widget Dependencies

The vendored gpui-kit manifests (`vendor/gpui-kit/crates/*/Cargo.toml`)
inherit shared registry requirements from the root `[workspace.dependencies]`.
Keep these `workspace = true` declarations when re-vendoring, preserving
per-package features, optional flags, and target conditions. `chrono` and
`notify` explicitly opt into defaults where upstream used them; the shared
entries disable defaults for narrower first-party users. The dependency policy
checks these declarations in addition to retained patches.

## Compiler Compatibility Dependencies

The pinned GPUI uses upstream `stacksafe` 1.x; no compatibility patch is
needed for the retired 0.1 macro dependencies.

The current dependency graph no longer contains `block` 0.1.6; its old
uninhabited-static compatibility override is unnecessary.

## GPUI patches — `gpui-*.patch`

GPUI compiles from `vendor/zed/crates/gpui`, a pristine copy of the
pinned zed revision (`vendor/zed/UPSTREAM`) with the patches below
applied. `tools/vendor_gpui` is the only thing that writes that tree:
it fetches the pinned rev, applies each `patches/gpui-*.patch`, and
`--check` — run by every CI job and by
`scripts/apply-gpui-patch.sh`, kept as a compatibility entry point —
fails on any drift between the tree and what fetch + patches produce.
So a patch is edited here and materialised there; never edit
`vendor/zed` by hand.

There is no macOS IME patch. The one this directory used to carry
targeted `crates/gpui/src/platform/mac/window.rs`, a path that does not
exist at the pinned rev (the platform code moved to `gpui_macos`), so
it had never applied; upstream's `is_ime_printable_key` in
`gpui_macos/src/window.rs` now routes printable keys IME-first whenever
an IME input source is active, which is the behaviour it wanted.

## `gpui-held-key-keeps-modality.patch`

Applied into `vendor/zed` by `tools/vendor_gpui` as above; the marker
`PlatformInput::KeyDown(ev) if !ev.is_held` is what to look for there.

One line in `Window::dispatch_event`'s input-modality decision: an
**auto-repeat no longer counts as new keyboard input**.

`last_input_modality` exists for focus-visible styling and hover
suppression, and every change to it calls `Window::refresh` — which
sets `refreshing` and so **bypasses every `AnyView::cached` for that
frame** (the one global invalidation daruda otherwise bans on a hot
path; see the root `AGENTS.md`). Only `KeyDown` and
`MouseMove`/`MouseDown` set it; `MouseUp` leaves it alone.

So a key *held* through a mouse drag — daruda's space-to-pan on the
flow canvas — put the window in a loop: each OS auto-repeat took the
modality back to Keyboard, each of the drag's own moves handed it to
Mouse, and each flip refreshed the whole tree. At macOS's repeat rate
that is tens of full, cache-defeating repaints a second for as long as
the drag lasts. Measured on a minimal workspace, a refreshed frame
costs **2.0×** a plain one (1.11 ms → 2.26 ms, release); the real
figure is higher, since what `refresh` throws away is exactly the
cached subtrees a minimal workspace does not have.

The same flip-flop is why the pointer kept vanishing: the hide fires
inside a `key_char` KeyDown, so every repeat re-armed it.

Semantics: a repeat says nothing a first press has not already said.
The first press still claims the modality. What changes is that a key
held while the mouse is used no longer takes it back — and having moved
the mouse, Mouse is the truthful answer. Upstream has not addressed
this, and zed never reaches it: its app code uses `is_held` nowhere,
its drag-modifying gestures are all real modifiers (which arrive as
`ModifiersChanged` and do not touch the modality), and its one
hand-cursor pan — the image viewer — needs no key at all. daruda is on
this path because it wanted the canvas convention (space to pan), so
the risk here is daruda's own.

Guarded by `an_auto_repeat_does_not_flip_the_input_modality`
(`workspace/tests/flow/graph.rs`), which fails with the patch reverted.

## `gpui-text-wrap-cache.patch`

Applied into `vendor/zed` by `tools/vendor_gpui` as above; the marker
`Condition 2 is an exact match on purpose` is what to look for there.

One condition in `TextLayout::layout`'s measure closure
(`crates/gpui/src/elements/text.rs`): **a max-content measure no longer
answers from a size that was measured at a definite wrap width.**

```rust
- && (wrap_width.is_none() || wrap_width == text_layout.wrap_width)
+ && wrap_width == text_layout.wrap_width
```

The comment three lines above the condition already states the intended
rule — *"wrap_width matches (or both are None)"* — which the code did not
implement: the leading `wrap_width.is_none() ||` returned the cached size
for **any** unconstrained measure, whatever width that size came from.

Taffy legitimately probes a shrink-to-fit box at zero width while
computing its content contribution. Under the old condition that probe's
wrapped result became the answer to every later max-content query for
the element, so the box settled at its minimum and the text laid out one
character per line. Measured in the file viewer's markdown preview, a
list item or blockquote holding an inline image collapsed to
`0px × 136.5px` where the same content in a paragraph measured
`51px × 19.5px`.

The cost is bounded: `layout_wrapped_line` keys its own cache on
`wrap_width` and checks the current *and* previous frame
(`text_system/line_layout.rs`), so the extra work is one re-shape per
genuinely new `(text, wrap_width)` pair rather than per query.

`render_md_prose` (`file_view_pane/render/markdown/inline.rs`) stacks its
runs with block layout rather than a flex column, which keeps that probe
from being decisive even on an unpatched checkout. Both are wanted: the
patch fixes the cause, the block column means a build against an
unpatched gpui does not show a collapsed list item.

## Upstream GPUI fixes retained by the pin

The pinned revision includes the nested deferred-draw reuse fix and its
`test_nested_deferred_draws_with_reused_views` regression test. Deferred
prepaint ranges refer to the stable draw vector across nested rounds.

`TestWindow` returns `HandleError::NotSupported` for native window/display
handles rather than panicking. Both fixes are upstream; no local patches
are applied for these paths.

## Taffy patch release

`gpui-taffy-patch-release.patch` advances GPUI's generated workspace from
Taffy 0.15.0 to 0.15.1 while retaining the pinned Zed revision. Regenerate
through `tools/vendor_gpui`; do not edit the generated manifest directly.

## `vendor/gpui-kit/` — vendored gpui-component 0.9.0, **patched in place**

daruda's widget library is upstream gpui-component 0.9.0 (`longbridge/gpui-kit`,
Apache-2.0), vendored as four crates:

| Directory | Cargo package | Library | Role |
|---|---|---|---|
| `crates/base` | `gpui-base` | `gpui_base` | headless text/input engine, `TextView`, `Root`, window-wide text selection, dialog/popover primitives |
| `crates/component` | `gpui-component` | `gpui_component` | styled widgets over `gpui-base` |
| `crates/assets` | `gpui-kit-assets` | `gpui_kit_assets` | bundled icons |
| `crates/component-macros` | `gpui-component-macros` | `gpui_component_macros` | proc macros |

The crates compile against daruda's pinned GPUI (`vendor/zed`) rather than
upstream's `gpui-pre`, so their types unify with every first-party crate.
Unlike `vendor/zed`, this tree is not generated: daruda is the source of truth
for the vendored copy, and the patches are applied in place rather than
recorded as `.patch` files.

Every daruda change in vendored source carries a `// daruda patch:` comment
(a few older ones read `daruda vendor patch`), and every button-agnostic raw
mouse listener carries `// ANY-BUTTON:`. The markers are the checklist:

```sh
grep -rn "daruda patch\|daruda vendor patch\|ANY-BUTTON" vendor/gpui-kit/crates
```

The sections below explain each group. The manifests are daruda's own files:
regenerated so shared dependencies inherit the root workspace, every
upstream lint is allowed (vendored code is outside daruda's lint scope),
`autoexamples` / `autobenches` / `autotests` are off, and `gpui-base`,
`gpui-component`, and `gpui-kit-assets` declare an empty `gpui-fast` feature
so upstream's `gpui-fast` cfg gates compile out (the Fast backend is not
vendored). `rust-version.sh`
skips packages named `gpui-*`, so the crates declare no Rust floor.
`LICENSE-APACHE`, `README.md`, `examples/fixtures/test.md`, and
`themes/aurora.json` stay at the tree root because vendored tests read them.

`scripts/ci-cargo.sh test` runs `-p gpui-base -p gpui-component`, so the
daruda-authored patch tests and the upstream suites run in CI.
Keep `gpui-base`'s Linux-only `gpui_platform` dev-dependency: its native
shaping tests call `current_platform(true)` for the headless text system.
`scripts/lint-direct-gpui-component.sh` forbids `gpui_component` and
`gpui_base` outside the app facade and `daruda_ui`, and
`scripts/lint-raw-mouse-button.sh` scans `vendor/gpui-kit`.

### Re-vendor procedure

1. Copy the upstream 0.9.x `crates/{base,component,assets,component-macros}`
   into `vendor/gpui-kit/crates/`, keeping the root files listed above and
   `crates/component/locales/ko.yml`.
2. Regenerate each manifest: inherit root `[workspace.dependencies]` where an
   entry exists (keeping per-package features, optional flags, and target
   tables), allow all lints, disable the auto targets, keep the `gpui-fast`
   feature stub, and keep the dev-dependencies (`gpui` with `test-support`,
   `indoc`, and `gpui-base` with `test-support` for `gpui-component`).
3. Re-apply every `daruda patch` site and `ANY-BUTTON` marker described below.
   A fresh copy drops them all; `scripts/lint-raw-mouse-button.sh` re-fires on
   each ungated listener.
4. Check the Korean locale's key set (see
   [the locale section](#vendorgpui-kitcratescomponentlocaleskoyml--vendored-an-added-file-not-a-patch)).
5. Run `cargo test -p gpui-base -p gpui-component`, then the paired daruda
   tests each section names.

---

## `vendor/gpui-kit/crates/component/Cargo.toml` - shared command grammars

The Bash and Python grammars inherit the root workspace entries
(`tree-sitter-bash = "0.25.1"`, `tree-sitter-python = "0.25.0"`) that the
transcript's command analysis and `daruda_ui` also use. Each grammar exports
unversioned C symbols, so linking different versions for highlighting and
analysis can resolve both consumers to the same native parser. Keep these two
dependencies on `workspace = true` when re-vendoring. The transcript's
`links_the_current_grammar_abi` test guards against linking an older parser.

---

## `vendor/gpui-kit/` — vendored, **the pinned GPUI is not `gpui-pre`**

### Why

Upstream 0.9.0 builds against `gpui-pre`. A few sites behave differently on
daruda's pinned GPUI; each is a marked adaptation, not a behavior change.

### What diverges

| Site | Adaptation |
|---|---|
| `component/src/input/input.rs` (accessibility probe test) | The expected accessibility value is always `Some("initial")`. The flag that decides it belongs to `gpui-base`, whose `test-support` feature this crate's dev-dependency always enables. |

The macro crate's package lookup is the remaining adaptation; see the next
section.

### Re-vendor procedure

Re-apply the accessibility probe expectation. Upstream 0.9 now uses the
window text system's `Rc` identity for its test registry and table cache, and
advances the executor clock in the popover test, so those adaptations are
covered upstream and must not be reapplied.

---

## `vendor/gpui-kit/crates/component-macros/src/crate_path.rs` — vendored, **the macros find daruda's `gpui`**

`gpui()` resolves the consumer's GPUI crate with `proc_macro_crate::crate_name`
over a package list. Upstream lists `gpui-kit`, `gpui-pre`, and `gpui-fast`;
daruda builds GPUI from zed under its own `gpui` package name, so `gpui` is
appended. Without it, expanding `IntoPlot` (the chart plot derive) fails
with "requires a direct dependency on gpui-kit, gpui-pre, or gpui-fast".
Re-apply by appending `"gpui"` to the list.

---

## `vendor/gpui-kit/crates/component/src/highlighter/` — vendored, **compiled-query cache**

### Why

`SyntaxHighlighter` compiled the language's tree-sitter queries from scratch
on every construction, and a markdown body builds one highlighter **per fenced
code block**. Compiling is the expensive part —
`ts_query__perform_analysis` over the whole grammar — and the result is
immutable and depends only on the language, so every construction after the
first was waste.

It surfaced as a stall when switching to an agent-chat pane. A pane in an
inactive tab is not rendered; GPUI drops every element state a frame did not
touch (`Frame::finish`), including the view state holding a markdown body's
parse, and the view re-parses when it is laid out again. So one tab switch
recompiled the queries for every code block on screen. Measured in release on
the 0.5.1 tree, 1920×1080: an ordinary repaint was ~1 ms, a switch back ~23 ms
with one visible fence and ~66 ms with three — linear in visible fences,
independent of conversation length. After the cache, ~1.3 ms in all cases.

zed has the same work but never pays it twice: compiled queries live in
`language_core::Grammar`, built once per language and shared through
`Arc<Language>` in the `LanguageRegistry`.

### What diverges

1. **`highlighter/highlighter.rs`** — `LanguageQueries` holds the per-language
   half of a highlighter (the compiled `Query`, the injection query, the
   pattern indices, and the capture indices). `language_queries` hands it out
   as an `Arc` from a process-wide `QUERY_CACHE` keyed by
   `LanguageConfig::name`. Keying by the registry's canonical name rather than
   the caller's spelling is load-bearing: callers include markdown fence
   tokens, which are arbitrary user text, so keying by them would let a
   conversation mint unbounded entries, each paying its own compile.
   `query_compilations(language)` reports how many times that language has
   actually been compiled, so the regression test asserts the defect (a
   recompile happened) rather than a wall-clock budget.
2. **`highlighter/registry.rs`** — `LanguageRegistry::register` calls
   `evict_language_queries`, so a language re-registered with different query
   sources is recompiled instead of serving a stale compilation.
3. **`highlighter/registry.rs`** — `ThemeStyle::new(color)` plus `bold()` /
   `italic()`. Upstream builds `ThemeStyle` only through deserialization (its
   fields are private), so the host could not seed `SyntaxColors` from its own
   syntax palette. `daruda_ui::theme` maps the selected editor syntax palette
   through it, so the raw editor and the diff view share one colour source.

Thread safety holds: `tree_sitter::Query` is `Send + Sync`, and nothing mutates
a query after construction. A concurrent cache miss may build twice —
identical result, last write wins — which is deliberate: holding the lock
across a multi-millisecond compile would serialise every caller behind it.

### Re-vendor procedure

Copy the fresh upstream `highlighter/` in, then re-apply the split, the
eviction, and the constructor. `query_compilations` is re-exported through
`crate::ui::highlighter`; the guard is
`packages/app/src/workspace/tests/agent_switch_cost.rs`, which registers its
own private language so the process-global counter cannot be moved by another
test running in parallel. It fails to compile, then fails its assertion, if
the cache is dropped.

---

## `vendor/gpui-kit/crates/component/src/checkbox.rs` - vendored, **mixed-state checkbox**

Adds `Checkbox::indeterminate(bool)` for partial selections (the agent-chat
display filter's `Tool calls` parent over five categories). The checkbox draws
`IconName::Minus`, treats checked and indeterminate as visible marks for
border, fill, and animation, reports the mixed accessibility state, and
resolves a mixed-state click to checked. `radio.rs` passes `false` to the
shared `checkbox_check_icon` helper's added argument.

Re-vendor by re-adding the field, the builder, the `marked` / `next_checked`
derivations in `render`, and the helper argument.

---

## `vendor/gpui-kit/crates/component/src/button/button.rs` — vendored, **a selected segment can pick its own label colour**

### Why

`ButtonCustomVariant` carried a single `foreground` used in every state. A
segmented strip therefore had to paint its selected segment's label in the same
colour as the resting ones — and on daruda's palette every single choice breaks
a stated DESIGN.md rule. Measured against the accent fill (`#5e6ad2`):
`text_muted` 1.45:1, `text_body` 3.26:1, `text_primary` 4.40:1 — all under the
4.5:1 body-text floor — while `accent_fg` (`#ffffff`) clears it at 4.70:1 but is
banned as text on the near-black resting surface (§Readability, "never
pure-white text"). Upstream's Primary-outline chrome sidesteps the question by
painting every *unselected* segment's label and border in accent, which trades
one violation for two: accent as small text and one accent element per segment,
against DESIGN.md's 3–4 budget.

### What diverges

| Delta | Detail |
|---|---|
| `selected_foreground: Option<Hsla>` + builder | `ButtonVariant::selected` reads `colors.selected_foreground.unwrap_or(colors.foreground)` for `Custom`. `None` is upstream behaviour. |
| `border: Option<Hsla>` + builder | `ButtonVariant::border_color` returns it for `Custom` in every state; `None` keeps upstream's edge derived from `color`. The segment strips draw their hairline through it. |
| Caller text size wins | `RenderOnce::render` applies the size tier's `button_text_size` only when the instance style sets no font size, so a label can follow a size the caller set on the button. |
| Caller justification reaches the label row | The label row is a `size_full` child that centers its content, so a `justify_*` the caller set on the button (the dock's left-aligned page links) is copied onto that row. Unset keeps upstream's centering. |
| A named custom border is drawn | 0.8 draws the 1px edge only for `Default` and outline buttons. `ButtonVariant::has_custom_border` adds a `Custom` variant whose `border` is set, so the status-bar pills keep the hairline every 0.5 button had. |

Consumers: `crate::ui::button_group` (`text_muted` resting at 5.00:1,
`accent_fg` selected at 4.70:1) and `crate::ui::button_group_on_surface`, which
brightens to the pane surface's own full foreground because a terminal-mirrored
pane has no verified contrast against the UI accent.

### Re-vendor procedure

Re-add both fields, their `new()` initialisers and builders, the two `Custom`
arms, the font-size gate, the justification copy, and the `has_custom_border`
edge condition. Re-check the contrast pairs above if the palette
moved in the meantime — DESIGN.md §Iteration Guide requires computing the ratio
for the actual fg/bg pair, not eyeballing it.

---

## `vendor/gpui-kit/crates/component/src/menu/` — vendored, **a capped menu scrolls only its body**

### Why

daruda caps every status-bar chip menu at the room above the bar
(`workspace/status_bar/menu.rs`, `dropdown`), which makes them scrollable; a
long account list in a short window otherwise snaps down over the bar.
Upstream's scrollable menu takes only a fixed `max_h`, computed once when the
menu's builder runs, so the cap outlived a window resize; and the whole item
column scrolled, so an overflowing menu lost its header on the first wheel and
started with its footer action out of view.

### What diverges

| Delta | Detail |
|---|---|
| Window-resolved cap | `max_height` is a private `MaxHeight` enum: `Fixed` (what `max_h` sets, upstream behaviour) or `Window`, set by `max_h_in_window(Fn(&Window) -> Pixels)` and resolved in every render. Unset keeps upstream's half-window default. |
| Pinned head and foot | `pin_head()` / `pin_foot()` mark the items added before / after the call. A `scrollable` menu lays out pinned head, body, pinned foot under the cap; only the body (`items-body`, tracked by `scroll_handle`) scrolls and carries the scrollbar. `set_selected_index` scrolls only to body items, offset by the head. Without pins the body is every item. |
| `PopupMenu::small()` | Public setter for the small item size. Upstream keeps the `size` the items render with but no longer exposes it. |
| `PopupMenu::chrome(Fn(&App) -> (bg, border))` | Replaces the float chrome's fill and edge after `popover_style`, for a menu opened over another float (`crate::ui::stacked_menu_builder`). A closure because the menu entity outlives a theme change while open. `None` keeps `popover_style`. |
| Body rows clear the scrollbar | `RenderOptions::trailing_pad`: rows inside a scrolling body take `gpui_base::SCROLLBAR_WIDTH` (a new public alias of the scrollbar's private hitbox width in `base/src/scrollbar.rs`) as trailing padding, so the bar never covers a row's end and a press there picks the row. |
| `PopupMenuItem::tooltip` | Hover text on `PopupMenuItem::Item`, forwarded to `MenuItemElement::tooltip` (`menu_item.rs`) and rendered independently of the disabled-gated click handler, so it can explain why an action is unavailable. |

### Re-vendor procedure

Copy upstream `menu/` in, then re-apply the rows above. The paired tests in
`packages/app/src/workspace/status_bar/menu.rs` fail if you forget:
`a_menu_that_fits_does_not_scroll`,
`a_long_menu_stays_above_its_chip_in_a_short_window`,
`an_open_menu_stays_above_its_chip_when_the_window_shrinks`,
`a_long_menu_keeps_its_header_and_footer_in_view`, and
`a_long_menus_scrollbar_leaves_its_rows_pressable`. Dropping
`max_h_in_window`, `pin_head`, or `pin_foot` fails to compile in that module;
the stacked-menu contrast is pinned by
`crate::ui::menu::tests::a_stacked_menu_stands_off_the_float_it_opens_over`.

---

## `vendor/gpui-kit/crates/component/src/root.rs` — vendored, **the root paints no fill**

0.8's `WindowState::style` fills the whole root surface with
`tokens.background`. daruda paints its own surfaces at the configured window
opacity, and 0.5's `Root` painted no fill, so the opaque fill made a
translucent window opaque. The patch drops the `.bg(..)` and keeps the font
family and text colour. `root::tests::the_root_paints_no_window_fill` fails
without it.

---

## `vendor/gpui-kit/crates/component/src/tab/tab_bar.rs` — vendored, **a caller's gap spaces the tabs**

### Why

0.8 moved the variant's tab spacing from the bar onto an inner `tabs-inner`
row, so a `gap` the caller sets on the `TabBar` (which refines the outer
container) no longer reaches the tabs. The dock strips set `gap(px(0.))`
because each dock tab already has a fixed `DOCK_TAB_WIDTH` target; without the
patch they spread 12px apart.

### What diverges

`RenderOnce::render` reads the instance style's horizontal gap and, when set,
uses it for the inner row instead of the variant's gap. Unset keeps upstream.

### Re-vendor procedure

Re-apply the `caller_gap` read and the `.gap(caller_gap.unwrap_or(...))` on the
inner row, then compare a landing capture's dock tab strip with the previous
build.

---

## `vendor/gpui-kit/crates/component/src/dialog/dialog.rs` — vendored, **a custom body owns its Enter; the surface is a float**

A dialog with no `on_ok` and no footer is a custom body that supplies its own
buttons. GPUI activates a focused button on key release, and only after the
button also received the key down. Upstream's dialog handled the `Confirm`
key down itself and ran the default OK handler, which closes the dialog, so
the focused button never activated. With
`body_owns_enter`, the dialog's `on_ok` calls `cx.propagate()` and returns
`false`, so Enter reaches the focused control. Dialogs with an `on_ok` or a
footer are unchanged. Re-apply the `body_owns_enter` derivation and the early
return in the `on_ok` wrapper.

The dialog surface also fills from `tokens.popover` instead of
`tokens.background`. `Theme::popover` is the one float slot every popover,
menu, and tooltip reads; the canvas slot is daruda's `surface-1`, the same rung
as the docks a dialog covers. Verify with
`--screenshot-scenario error-modal --screenshot-theme dark,light`: the dialog
must read lighter than the docks behind it.

---

## `vendor/gpui-kit/crates/base/src/input/base/state.rs` — vendored, **`set_value` reports the change**

### Why

gpui-component 0.5 emitted `InputEvent::Change` from `set_value`; 0.8.1 does
not. daruda keeps derived state in step through `Change` whether an edit was
typed or programmatic (row-count editors, the bottom dock's height,
warnings), and history recall writes through `set_value`.

### What diverges

`set_value` calls `set_value_silently` (upstream's body, renamed) and then
emits the full replacement as `InputEditsEvent` followed by one
`InputEvent::Change`. Vendored widgets that write their own field
programmatically keep upstream's silent write through `set_value_silently`, so
a write made while handling a change does not feed back: the list query (`component/src/list/list.rs`, which starts its search
itself), the color picker's hex field (`base/src/color_picker.rs`), the
questionnaire, the inspector, the search input, the command palette state, and
the settings number and string fields.

### Re-vendor procedure

Rename upstream's `set_value` to `set_value_silently`, add the emitting
wrapper with an `InputEditsEvent` replacement snapshot, then grep for
`set_value(` inside `vendor/gpui-kit/crates` and switch
every vendored widget that writes its own field to `set_value_silently`
(`grep -rn set_value_silently vendor/gpui-kit/crates` lists the current
sites). A missed site re-enters its own `Change` handling.

---

## `vendor/gpui-kit/crates/{base,component}/src/input/` — vendored, **host hooks on the text engine**

Additive APIs a host needs to render diffs and file views through the editor
and to size chrome around it. All default to upstream behaviour.

| Delta | Site | Detail |
|---|---|---|
| `LineDecoration` | `base/src/input/base/state.rs` (`LineDecoration`, `line_decorations`, `set_line_decorations`), `base/src/input/base/element.rs` (`gutter_label_len`, gutter width, row-background paint), re-exported from `component/src/input/mod.rs` | Per-buffer-row `background` fill and `gutter` label. Row backgrounds paint under everything else; host labels size the gutter to the widest one instead of the line count. The diff viewer packs its old/new line numbers into one gutter column. |
| Highlight override | `base/src/input/editor/decorations.rs` (`set_highlight_override`), `base/src/input/base/kind.rs` (`EditorExtras::highlight_override`) | Paints host spans through a decoration collection in place of syntax highlighting, switching the highlighter to plain `text` while installed: a synthetic +/- diff buffer is not valid source for any grammar. `None` clears the spans and restores the previous language. |
| Layout readbacks | `base/src/input/base/state.rs` (`scroll_size`, `display_rows`, `move_cursor_to_end`) | `scroll_size` is the scrollable extent from the last layout, for a host overlaying its own thumb. `display_rows` is the soft-wrapped row count clamped by an auto-grow range, which the composer and the agent-chat embeds size from. `move_cursor_to_end` places the cursor after the last character without moving focus (history recall). |
| One renderer for every state | `component/src/input/state.rs` (`TextInputState` public), `component/src/input/input.rs` (`Input::from_state`) | A host builds one `Input` for an `InputState`, `TextareaState`, or `EditorState` (`daruda_ui::input`'s `TextState`). |
| Host-owned chrome | `component/src/input/input.rs` (`input_padding`, `editor_background`, `show_scrollbar`), `component/src/input/state.rs` (`set_scrollbar_renderer` forwarding) | `input_padding(false)` drops the size-tier inner padding so the host container sets the inset; `editor_background(color)` paints a code editor on a host surface instead of the theme's `editor_background`; `show_scrollbar(false)` hides the built-in bar for a host that overlays its own thumb. |

Re-apply each row. The file viewer, the diff embed tests in
`packages/chat/src/view/tests/`, and `daruda_ui::code_editor` fail to compile
without them.

---

## `vendor/gpui-kit/crates/base/src/input/base/state.rs` — vendored, **a re-wrap reaches the host**

### Why

The layout pass that re-wraps the text runs from the element's paint. When the
painted width changes it updates the row count `display_rows()` reports and
calls `cx.notify()` — from inside the draw. GPUI drops that notify:
`WindowInvalidator::invalidate_view` records the entity but skips
`dirty = true` while a draw phase is active, and `Window::draw` clears
`dirty_views` once the frame is painted (the same behaviour the ferrum_flow
viewport patch below works around). So a host that sizes itself from
`display_rows()` — the agent-chat embed, which caps its box at the editor's
row count — painted the editor at its *logical* line count and stayed there
until an unrelated event happened to dirty the window.

### What diverges

The notify after `display_map.on_layout_changed` becomes
`cx.defer(move |cx| cx.notify(entity_id))`. Deferred, it lands after the frame,
where GPUI honours it. It fires only on a layout change, so nothing is added to
the steady-state frame.

### Re-vendor procedure

Restore the deferred notify. Paired tests fail if you forget —
`packages/chat/src/view/tests/output_layout.rs`'s
`long_output_lines_wrap_to_the_embed_width` and
`packages/chat/src/view/tests/diff_layout.rs`'s
`long_diff_lines_wrap_to_the_embed_width`.

---

## `vendor/gpui-kit/crates/base/src/input/base/state.rs` — vendored, **the wheel chains at the text's end**

A height-capped editor embedded in the agent-chat transcript must hand the
wheel back to the transcript once it cannot scroll further. Upstream claimed a
scroll whenever the text overflowed along the gesture's axis and stopped
propagation unconditionally.

- `takes_scroll` claims a wheel event only when the text can still move in
  that direction (offset short of the extent for a scroll toward the end,
  offset below zero for a scroll toward the start).
- `on_scroll_wheel` stops propagation only when the offset actually changed or
  the scroll bounced.

Re-apply both. `packages/chat/src/view/tests/output_layout.rs`'s
`capped_embed_wheel_chains_only_after_the_embed_is_exhausted` covers holding
the wheel while the embed can scroll and passing it on at the end.

---

## `vendor/gpui-kit/crates/base/src/` — vendored, **word and click selection**

| Delta | Site | Detail |
|---|---|---|
| CJK and Hangul words | `base/src/text_boundary.rs` (`CharacterKind::from`), `base/src/text/selection.rs` (`is_word_char`) | Any-script alphanumerics (`char::is_alphanumeric`, plus `_`) are word characters, so a double click selects a Hangul or CJK word rather than one syllable. The first covers the input engine, the second `TextView`. Upstream test expectations in `base/src/input/base/selection.rs` and `base/src/text/selection.rs` change to match (a CJK run is one word). |
| Fourth click selects all | `base/src/input/base/state.rs` (mouse-down ladder, `test_fourth_click_selects_all`) | A left `click_count >= 4` selects the whole text, as in macOS text views; three clicks still select the line. |

Re-apply both and their test expectations; the vendored suites fail on the
upstream expectations otherwise.

---

## `vendor/gpui-kit/crates/base/src/popover.rs` — vendored, **a panel can read its trigger**

`PopoverState::trigger_bounds()` returns the trigger's window bounds as of the
popup's last positioning (`None` before the first). The popover's
`on_position` callback records them into a shared cell. A panel's content
builder receives the `PopoverState`, but upstream keeps the bounds private, so
a panel could cap its height only against the whole window — and a tall panel
under a low trigger was then shifted up over the control that opened it. The
transcript editors (`packages/transcript/src/editor/mod.rs`) cap at the space
below the trigger instead. Re-add the field, its initialiser, the accessor, and
the `set` in `on_position`.

---

## `vendor/gpui-kit/crates/` — vendored, **a raw mouse listener names its button**

### Why

GPUI has two tiers of mouse input with opposite defaults. `div().on_click()` is
left-only by construction — `elements/div.rs` routes every other button to
`on_aux_click` — and `.on_mouse_down(MouseButton::X, ..)` takes the button as an
argument. But a hand-rolled `Element` reaches for
`window.on_mouse_event::<MouseDownEvent>()`, which hears every button and
leaves the filter as a field you have to remember to read.

### What diverges

| Delta | Detail |
|---|---|
| Track and thumb press gate on `Left` | `base/src/scrollbar.rs`. A right press on a scrollbar no longer jumps or drags the view and `stop_propagation`s the context menu with it. zed's own scrollbar gates this the same way. |
| Resize-handle press gates on `Left` | `base/src/resizable/resize_handle.rs`. Only the primary button lights the pressed state and starts a resize. |
| Link press/release | `base/src/text/inline.rs`; see [the link section](#vendorgpui-kitcratesbasesrctextinliners--vendored-a-link-opens-from-the-press-that-chose-it). |
| `// ANY-BUTTON: <reason>` markers | `component/src/input/popovers/hover_popover.rs`, `component/src/diff/selection.rs`, `component/src/dock/dock.rs`, `base/src/text_selection.rs`, `base/src/scrollbar.rs` (release), `base/src/input/base/element.rs`, `base/src/resizable/resize_handle.rs` (release), `base/src/resizable/panel.rs`. Each listener answers every button on purpose: it ends a drag that only a left press can have started, resets multi-click tracking, or dismisses on an outside press. The marker separates an intent from an omission. |

### Re-vendor procedure

`scripts/lint-raw-mouse-button.sh` is the checklist: a fresh upstream copy
drops the gates and markers, and the lint re-fires on each site. The paired
`packages/ui/src/scrollbar/tests.rs` tests
(`a_left_press_on_the_track_jumps_the_view` /
`a_right_press_on_the_track_does_not_jump_the_view`) cover the scrollbar gate
from both directions.

---

## `vendor/gpui-kit/crates/base/src/text/inline.rs` — vendored, **a link opens from the press that chose it**

### Why

Upstream opens a link on a hovered left or middle *release* alone. A drag
begun elsewhere — on the pane behind, or inside a popover's padding — and
ended over a link therefore navigated, though the user never pressed it. And a
link in agent chat is rendered by a hand-rolled element, so the host's
right-press handler, which opens the pane context menu, had no way to ask what
link sat under the press. zed's `crates/markdown/src/markdown.rs` records the
pressed link on a hitbox-gated mouse-down and exposes the context-menu link the
same way.

### What diverges

| Delta | Detail |
|---|---|
| Press recorder | A **capture**-phase `MouseDownEvent` listener, gated on the inline hitbox, records the link under the press in the `LinkPressRecord` global: a left press stores it as `pressed`; a right press clears `pressed` and stores `(position, url)` as `right`; any other button clears `pressed`. Capture phase runs before every bubble listener, so the record lands before the host's bubble-phase menu opener whichever order they registered in. |
| Release match | The release handler opens a link on a left release only if `pressed` (taken) is that same URL. Middle releases keep upstream's behaviour. |
| `take_right_clicked_link(cx, at)` | Re-exported from `gpui_base` (`base/src/text/mod.rs`). Takes the right-press record and returns its URL only when `at` equals the press position, so a record no menu consumed cannot be adopted by a later, unrelated menu. A global rather than view state because an `Inline` is an element built fresh each frame with no handle the host can reach. Drives the chat's Open in File View / Open Externally / Copy Link Address entries. |

### Re-vendor procedure

Re-apply the three rows. Paired tests in `packages/ui/src/markdown/tests.rs`
fail loudly if you forget: `a_click_on_a_link_opens_it`,
`a_click_inside_an_occluding_panel_does_not_open_the_link_under_it`,
`a_drag_that_merely_ends_on_a_link_does_not_open_it`, and
`a_non_primary_click_on_a_link_does_not_open_it`.

---

## `vendor/gpui-kit/crates/base/src/text/` — vendored, **`TextView` host hooks**

| Delta | Site | Detail |
|---|---|---|
| `TextView::code_block_render` | `text_view.rs` (`CodeBlockRenderFn`, builder), `node.rs` (`CodeBlock` render consults it first) | Replaces a code block's rendering when the closure returns `Some(element)`; `None` falls through to the default block. The agent chat renders ` ```mermaid ` fences as diagrams through it, falling through while the image is not ready. |
| `TextView::debug_inline_bounds` | `text_view.rs` (`DebugInlineBoundsFn`, builder), `inline.rs` (`Inline::debug_bounds`, reported from paint) | Unset by default. The app's two-renderer Markdown conformance probe observes the exact bounds each prose `Inline` paints; measuring the outer paragraph div would stay positive even if its text child collapsed. |
| Task checkbox selector | `node.rs` (`debug_selector(|| "markdown-task-checkbox")`) | Lets the same probe find a task-list checkbox. `debug_selector` is a no-op without `test-support`, and neither probe adds a layout wrapper. |
| Code-block actions on hover | `node.rs` (`CODE_BLOCK_GROUP`; the block container takes `.group`, the actions overlay `.opacity(0.).group_hover(..)`, which keeps its layout, plus its own `.hover(..)` because the button takes the pointer from the block; a scrolling block moves the overlay left by `SCROLLBAR_WIDTH`) | The host's copy button appears only while the pointer is over the block, so an idle block shows no persistent action chip, and it never sits under the block's vertical scrollbar. |
| Loose and tight lists | `format/markdown.rs` (`Node::List` carries the list's `spread` onto its items), `document.rs` (`NodeRenderOptions::loose_list`), `node.rs` (list arm derives it; list items take `pt(paragraph_gap)` after the first only when loose) | CommonMark calls a list loose when any item is spread, and mdast reports blank lines *between* items on the list and blank lines *inside* one on the item. A tight list keeps one line pitch, so a wrapped item is exactly two rows tall; a loose list spaces its items like paragraphs. |

Re-apply each row. `packages/ui/src/markdown/tests.rs` and the conformance
table in
`packages/app/src/workspace/main_area/file_view_pane/render/markdown/layout_tests.rs`
fail without the hooks and the list spacing.

---

## `vendor/gpui-kit/crates/component/locales/ko.yml` — vendored, **an added file, not a patch**

The only Korean the vendored widgets have. Upstream's `locales/ui.yml` carries
`en` / `pt-BR` / `ru` / `zh-CN` / `zh-HK` / `zh-TW` / `it`, so
`gpui_component::set_locale("ko")` would fall back to English, and a Korean
window would show "Cut / Copy / Paste / Select All" in every text input's
right-click menu and "Cancel" on every confirm dialog.

### Why it is not a source patch

`rust_i18n::i18n!("locales", …)` in `src/lib.rs` does not name its files. The
macro globs `CARGO_MANIFEST_DIR/locales/**/*.{yml,yaml,json,toml}` at expansion
time and merges every match by locale (rust-i18n-support `load_locales`). A new
file is therefore picked up with **no vendored `.rs` change**, and re-vendoring
`ui.yml` cannot collide with it: upstream owns that file and its locales,
daruda owns this one and `ko`.

Runtime injection is not available: the store is a private
`static _RUST_I18N_BACKEND` inside whichever crate called `i18n!`, with no
`set_backend`, and `t!` expands to `crate::_rust_i18n_t!` — so the app's `t!`
can only ever see the app's own backend.

### Re-vendor procedure

Copy `ui.yml` from upstream as usual and leave this file alone. Then check the
key sets still line up — an upstream-added key silently renders English:

```sh
diff <(grep -oE '^  [^ ].*:$' vendor/gpui-kit/crates/component/locales/ui.yml | sort) \
     <(grep -oE '^  [^ ].*:$' vendor/gpui-kit/crates/component/locales/ko.yml | sort)
```

### Editing it leaves a stale binary

cargo does not track a proc-macro's file reads, so changing only a locale file
rebuilds nothing and the old strings stay compiled in.
`touch vendor/gpui-kit/crates/component/src/lib.rs` (or
`cargo clean -p gpui-component`) forces the re-read. The same holds for
`packages/app/locales/`.

---

## Covered upstream in 0.8.1

These daruda patches against the 0.5.1 tree were retired because 0.8.1
implements the behavior itself:

- **IME marked-text selection** (`gpui-component-input-state-ime-selection.patch`):
  `replace_and_mark_text_in_range` converts AppKit's marked-string-relative
  UTF-16 selection inside the new marked text and offsets it by the marked
  range's start.
- **Modal Tab containment** (`gpui-component-root.patch`): `Root`'s Tab and
  Shift+Tab actions (`advance_focus` in `base/src/root.rs`) keep focus inside
  the active focus trap a dialog installs (`base/src/dialog.rs`), and park it
  on the trap's own handle when no stop inside is reachable. The patch's other
  hunks targeted the 0.5.1 `Root` render and focus signatures, which 0.8.1 no
  longer has.
- **Single-line newline guard**: `normalize_input` strips `\n` and `\r` from
  every edit to a single-line input.
- **The popover trigger consumes its press**: the trigger's `on_mouse_down`
  calls `cx.stop_propagation()`.
- **Link release asks its hitbox; a right release does not open a link**: the
  release handler is gated on `hitbox.is_hovered`, and the default link
  handler opens only on a left or middle release.
- **Intrinsic table columns**: a row wider than the delimiter row is truncated
  to its width, and column widths come from shaped text measurement
  (`TableColumnWidths`).
- **Tooltip radius**: the tooltip reads `cx.theme().radius`.

## GPUI revision compatibility in vendored consumers

The widget and flow versions remain unchanged when updating GPUI. Preserve
these API adaptations when re-vendoring:

- gpui-kit targets upstream's `gpui-pre`; every adaptation to daruda's pinned
  GPUI is a marked site, listed under
  [the pinned GPUI](#vendorgpui-kit--vendored-the-pinned-gpui-is-not-gpui-pre)
  and [the macros](#vendorgpui-kitcratescomponent-macrossrccrate_pathrs--vendored-the-macros-find-darudas-gpui).
- `ferrum_flow` uses `ScrollEvent` / `on_scroll` in place of the deprecated
  wheel-only names.
- `ferrum_flow/src/plugins/background.rs`: pass the tile bounds as both
  clipping and image bounds to `Window::paint_image`.
- The workspace enables `rust-embed/include-exclude` explicitly for assets;
  GPUI no longer supplies this feature transitively.

## `vendor/ferrum-flow/ferrum_flow/` — vendored, **seven source patches**

Provenance for a vendored crate, plus the source deltas it now carries.
Re-vendoring stays a file copy followed by re-applying those.

| | |
|---|---|
| Upstream | [tu6ge/ferrum-flow](https://github.com/tu6ge/ferrum-flow) — Apache-2.0 |
| Pinned at | `43b762ced6f61313bbc2b388871bdc93f64893d0` on `master` (2026-06-05) |
| Version string | `0.3.1` — the last tag, 127 commits behind this pin. Upstream has been silent since 2026-06-07, so `master` is effectively its final state; the tag would have forgone level-of-detail node rendering and lazy paint-order traversal, both of which matter as a flow grows. |
| Copied | `crates/core/src/` → `vendor/ferrum-flow/ferrum_flow/src/`, plus `crates/core/README.md` and the repository-root `LICENSE` |
| Not copied | `crates/core/examples/` — they call `Application::new()`, which no longer exists at daruda's pinned GPUI rev. Also the sibling `crates/sync_plugin` (Yrs CRDT collaboration), which daruda does not use. |

### Source patches

| Patch | File | What |
|-------|------|------|
| `FlowCanvas::viewport` accessor | `src/canvas.rs` | read-only `pub fn viewport(&self) -> &Viewport`, mirroring the existing `graph()`. `Viewport` is already exported; only the getter was missing, so a host could see *what* the graph is but not *where the canvas put it* — which is decided by the host's own plugin choice plus a drawable size only layout knows. Same role as the vendored input state's `display_rows` / `scroll_size` readbacks: it is what lets a test assert the result instead of the intent. Backs `opening_a_graph_brings_every_node_into_view` (`workspace/tests/flow.rs`), which fails if the pane stops framing the graph into the drawable. |
| Unmeasured drawable fails open | `src/viewport.rs` (`is_world_bounds_visible` early return, plus a daruda-authored `mod tests` guarding it) | `Viewport::window_bounds` is `None` until GPUI layout measures the canvas in `on_children_prepainted`, and every node's and edge's visibility is decided against it. Returning `false` there culled the entire graph on the first frame — and nothing recovered it: gpui's `WindowInvalidator::invalidate_view` (`window.rs`) records the entity but **skips `dirty = true`** when `draw_phase != DrawPhase::None`, so the `cx.notify()` the canvas raises from that same prepaint cannot schedule a second frame. The canvas stayed blank until an unrelated event dirtied the window (a mouse move over it — which is why upstream's own examples look fine, and why `default_plugins()` gets away with omitting `FitAllGraphPlugin`, the only handler of `FlowEvent::DrawableBoundsReady`). Culling is an optimization, so an unknown drawable now fails open: one frame of overdraw instead of an empty canvas. Repro: `--screenshot-scenario flow-graph`; with the early return back at `false` the pane's card region collapses to the single app-canvas colour. |
| Dragged wire says yes as well as no | `src/plugins/port/interaction.rs` (`preview_tint` + its use in `PortConnecting::render`, plus a daruda-authored `mod tests` guarding it) | Upstream coloured only the **refusal**: `target_highlight` was computed solely when `validation_error` was set, and the line and dot only branched on the same flag. So a port that *would* take the drop rendered identically to empty space — the one thing a person needs to see while dragging was the one thing nothing said. The state was already there (`hovered_port` is set for every hovered candidate, valid or not), so the patch is the branch that reads it: over a valid port the wire takes `theme.success` and the port is ringed, over a refusal `theme.error` as before, over nothing upstream's two-tone neutral. Extracted into a pure `preview_tint` so the decision is testable without a window, which is also what keeps the edit inside `render` to one call. No new theme field — `FlowTheme::success` already existed and daruda maps it to the same green a passed card uses (`flow_theme`). Not reachable from outside: the active `InteractionState` is `pub(crate)` and `RenderContext` does not expose it, and a host plugin cannot even see the drag begin, since this plugin's priority (125) claims the port's `MouseDown` with `EventResult::Stop`. |
| Dangling links are opt-out | `src/plugins/port/interaction.rs` (`PortInteractionPlugin::dangling_links` + a `dangling` field threaded into `PortConnecting`, gating the `PendingLinkCommitted` emit in `on_mouse_up`) | Releasing a wire over empty space left a line to nowhere, drawn until the *next* click — and clicking its endpoint built a blank node plus an edge to it. Both are upstream features for a canvas whose nodes are created on it; daruda's come from a file, so the node has nowhere to be recorded and the line says something untrue until it is dismissed. Off, a release that landed on no port simply ends the drag. Shaped as a builder flag defaulting to `true` — the same extension point as the neighbouring `validator`, so upstream behaviour is what a caller gets without asking, and the re-vendor diff stays one field and one `if`. A drop on a port that is *refused* already ended immediately (it emits `FlowEvent::error` and returns), so only the empty-space case needed this. The consequence is separately guarded on daruda's side — `reconcile_edges` removes any node the flow file cannot name (`a_blank_node_the_file_never_named_is_taken_off_the_canvas`) — but that fires after the fact; this removes the cause. |
| A release lands where the wire said it would | `src/plugins/port/interaction.rs` (`PortConnecting::on_mouse_up` reads `hovered_port` / `validation_error` instead of hit-testing again) | The move pass decides which port a drag is over by `port_screen_big_bounds` — a 30×30 box — and the wire's colour reports that decision. The release then decided *again*, against `port_screen_bounds`, which is the port's own 12×12. So a ring roughly nine pixels wide around every port showed green and refused the drop. Now the release uses what the move pass already concluded, which is React Flow's structure rather than merely its numbers: `XYHandle` computes `closestHandle` and `isValid` on pointer-move and its `onPointerUp` reuses both, so the preview and the drop cannot disagree. Starting a drag still needs the port itself (`bounds`, on mouse-down) — strict to start and forgiving to land is the same asymmetry React Flow has, where a drag begins on the handle element but ends anywhere inside `connectionRadius`. One behaviour is dropped with the second hit-test: a refused release no longer emits `FlowEvent::error`. Nothing consumed it — the message has no reader in the vendor or in daruda — and the wire had already turned red under the cursor. |
| A drag hears its own release | `src/canvas.rs` (`on_mouse_up` steps aside for a live interaction; a `canvas()` child registers an ungated window listener that serves one) | The canvas starts an [`Interaction`] on mouse-down and ends it on mouse-up, but `div`'s mouse-up listeners are gated on `Hitbox::is_hovered` — which is false once the pointer has left (dragging a view is dragging it *away*), and, less obviously, false whenever `Window::last_input_was_keyboard` (gpui `window.rs`, in `is_hovered`). Only `KeyDown` and `MouseMove`/`MouseDown` set that modality; `MouseUp` leaves it alone. So a key held through a drag — daruda's space-to-pan, and any modifier-style hold — auto-repeats, and a release landing between two repeats was dropped: `on_mouse_up` never ran, the interaction stayed installed with nothing to end it, and the next mouse move carried on dragging with no button down. Intermittent by construction, since it turns on whether the last repeat beat the last move. The two listeners are mutually exclusive on `interaction.handler.is_some()`, so exactly one forwards any release and no plugin sees it twice. Upstream's own node and port drags have the same hole; this fixes it for all of them rather than for the one daruda noticed. The forwarder is gated on `MouseButton::Left`, matching the `on_mouse_up(MouseButton::Left, ..)` registration it stands in for — ungated, a right click during a live wire drag read as a release and dropped the wire (the raw-listener class written up under `vendor/gpui-kit`). That gate is sound only because every interaction the canvas can start is a Left one: the canvas div registers `on_mouse_down` for Left and Right alone, so `ViewportPlugin`'s Middle-button `Panning` arm is unreachable and cannot be stranded without a release. Re-check that if a Middle registration is ever added. |

**The other daruda-authored file is `Cargo.toml`.** It differs from
upstream's in four ways, none of which touch source:

1. `gpui = { workspace = true }` — the reason this crate is vendored at
   all. Upstream takes `gpui` from crates.io, which resolves to a
   *different* crate instance than daruda's pinned zed git rev, so the
   types would not unify.
2. `image` / `futures` / `uuid` declared locally at the versions already
   in `Cargo.lock`, since they are not in `workspace.dependencies`.
3. `[lints]` — every clippy and rustc group set to `allow` with
   `priority = -1`, matching the "vendored upstream, do not lint" stance
   used for gpui-kit. The `priority` is load-bearing: without it
   the one lint deliberately left armed (below) is buried by the groups.
4. `rust-version = "1.98.1"` plus `incompatible_msrv = "deny"` — unlike
   gpui-kit, this crate declares the floor. The manifest is
   daruda's own file, so declaring it costs nothing when re-vendoring,
   and it lets `cargo clippy -p ferrum_flow` catch a std API newer than
   CI's pinned toolchain without installing that toolchain locally.
   Note the check only sees std API stabilization, not language
   features; the pinned toolchain remains the final gate.

### Re-vendor procedure

```bash
git clone https://github.com/tu6ge/ferrum-flow /tmp/ff && cd /tmp/ff
git checkout <new-commit>
rm -rf <daruda>/vendor/ferrum-flow/ferrum_flow/src
cp -R crates/core/src <daruda>/vendor/ferrum-flow/ferrum_flow/src
cp crates/core/README.md LICENSE <daruda>/vendor/ferrum-flow/ferrum_flow/
# Cargo.toml is daruda's — reconcile it against upstream's dependency
# list by hand, re-apply the source patches above (the viewport one's
# `mod tests` fails loudly if you forget), then:
cargo build -p ferrum_flow && cargo clippy -p ferrum_flow && cargo test -p ferrum_flow
```

### Framing is daruda's, not the vendored fit's

`FitAllGraphPlugin` is not registered. It was, briefly, and it needed a patch
to stop it magnifying a small graph to 3× — which was the signal that the
policy belonged on this side: how far out it is worth zooming is a question
about what a card still says at that size, and daruda's cards drop rows as
they shrink. `flow_graph_pane/frame.rs` holds the whole rule (fit to shrink,
never magnify, floor at 0.2) for both entry points, opening and ⌘0, and
reaches the vendor only through public API — a `Plugin` for the two events and
a `Command` for the write, since `PluginContext` exposes no viewport setter.
That patch is therefore gone rather than kept.

### Unused modules are kept, on purpose

The crate ships plugins daruda never registers — clipboard, context menu,
minimap, zoom controls, snap guides, align, select-all, focus-selection,
node interaction, and the ⌘Z history plugin. Deleting them was tried and
reverted; the measurement is here so it does not get re-tried on the
assumption that it helps.

| | |
|---|---|
| Removed | 2,576 of 15,460 lines (17%) |
| Crate rebuild | 0.60s → 0.58s. The whole crate compiles in under a second either way, so the "less to compile on a gpui bump" argument has nothing to buy |
| Inline tests lost | 4 of 66 — and those tests are the cheapest signal that a gpui bump broke vendored code |
| Re-vendor cost | copy, then re-delete 10 paths, then re-edit three *retained* files: `plugins/mod.rs`, `plugins/node/mod.rs`, and `canvas.rs` — whose `default_plugins()` would silently stop matching upstream's, which is a trap for whoever reads it next |

The risk pruning was meant to buy down — a bump breaking code we do not
use — is also weaker than it looks: the unused plugins reach for the same
gpui surface (`div`, `Element`, mouse events) as the ones daruda does
register, so a bump that breaks them very likely breaks the rest too. If a
bump ever does break only an unused module, deleting it *then* is the same
work, paid only if it is ever needed.

## `gpui-component-input-empty-text-shortcuts.patch`

Applied in place to `vendor/gpui-kit/crates/base/src/input/base/state.rs`
and the host composer operations. Reapply when re-vendoring.

`InputBaseState::on_empty_text_input` optionally handles committed text before
insertion when the editor is empty and has no marked text. Explicit-range
and silent programmatic replacements bypass the hook. A true result consumes
the commit; false preserves normal editing. The callback must not read or
update the same InputState. The bottom-dock host uses it for digit and Space
question shortcuts because macOS IME input sources may commit these keys
before GPUI keybindings run. Other editors retain their previous behavior.

The host composer skips marked text through `has_marked_text()` so stale host snapshots
cannot consume composition arrow keys. Finalizing composition with `unmarkText`
emits the edit snapshot and one `Change` event when a marked range existed, keeping host text snapshots
current even when the IME does not send a replacement callback. Regression coverage lives in
`workspace/tests/agent_chat/questions/keys.rs`, including native-style text
commits, marked-text preservation, Space toggling and fallback typing.

### Composer Escape precedence

The composer handles Escape in the bubble phase so input-owned cancellation
(multiple cursors, inline completion, touch selection) runs first. In
`base/src/input/base/state.rs`, Escape returns after unmarking composition,
preventing that same gesture from reaching host stop actions. Preserve this
`daruda patch:` branch when re-vendoring. Composer Enter skips marked text.
