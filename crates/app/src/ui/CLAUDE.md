# ui/ — widget primitives (`gpui_component` wrappers + preserved daruda widgets)

Single import surface for app-side widgets. Two layers under one
directory:

1. **Wrappers** over the vendored `gpui_component::*` crate
   (factory / variant / re-export shapes).
2. **Preserved daruda widgets** — IME-critical inputs, daruda-tone
   primitives, and bespoke layouts that have no clean
   `gpui_component` swap. Listed in the parent
   `crates/app/src/CLAUDE.md` under the `ui/` table.

App code under `crates/app/src/` must always go through `crate::ui::*`;
direct `use gpui_component::*` imports are forbidden (enforced by
`scripts/lint-direct-gpui-component.sh`).

This file documents the **wrapper authoring rules** only. For
preserved-widget conventions (`Entity<T>` + `Render` for stateful, plus
the IME / focus pitfalls), see root `CLAUDE.md` §3 + §4.

---

## Why the wrapper layer

1. **`xsmall()` auto-application** — daruda is a compact terminal UI;
   gpui_component's default `Medium` size is too large. Every `Sizable`
   widget must be constructed at `xsmall`. Factory functions apply it
   so call sites can't forget (CLAUDE.md §10).
2. **One place to retheme** — when default colours / variants / paddings
   need to change project-wide, this is the single file to edit.
3. **Single import path** — `use crate::ui::*;` is short and discoverable;
   `use gpui_component::button::{Button, ButtonVariants as _};` is not.
4. **Future-proof** — when `gpui_component` is upgraded or replaced, the
   blast radius is `ui/`, not every call site.

---

## Wrapper module layout

Wrapper files only — preserved daruda widgets are listed in the parent
CLAUDE.md `ui/` table.

```
ui/
├── mod.rs          # pub mod + factory re-exports + trait re-exports
├── theme.rs        # apply_daruda_palette(cx) — palette mapping
├── alert.rs        # error/warning/info/success(id, msg) factories
├── badge.rs        # Badge::new(label).monospace()/.bg_color()/... over Tag::custom
├── button.rs       # button / button_primary / button_danger / button_bare
├── checkbox.rs     # checkbox(id, label)
├── dialog.rs       # Dialog / DialogButtonProps / ButtonVariant / WindowExt re-exports
├── divider.rs      # Divider re-export
├── list.rs         # FilteredItem + FilteredDelegate + searchable_list_state + list(&state)
├── select.rs       # SelectOption + state_with_options + select(&state)
├── tab_bar.rs      # tab_bar(id) + tab(label) factories over gpui_component (Small + underline; tab() bakes 10px x-padding)
└── tooltip.rs      # tooltip::text(content) closure helper
```

`mod.rs` re-exports both factory functions and the type aliases callers
need most often (alongside the preserved-widget exports):

```rust
// gpui_component wrapper exports
pub use button::{Button, button, button_bare, button_danger, button_primary};
pub use checkbox::{Checkbox, checkbox};
pub use divider::Divider;

pub use gpui_component::button::{ButtonVariant, ButtonVariants};
pub use gpui_component::{ActiveTheme, Disableable, WindowExt};
```

---

## Three wrapper shapes

Pick the smallest shape that fits the underlying widget.

### A. Factory function — `Sizable` widgets

For widgets that implement `gpui_component::Sizable` and would otherwise
require `.xsmall()` at every call site.

```rust
// ui/checkbox.rs
use gpui::ElementId;
use gpui_component::Sizable as _;
use gpui_component::text::Text;

pub use gpui_component::checkbox::Checkbox;

pub fn checkbox(id: impl Into<ElementId>, label: impl Into<Text>) -> Checkbox {
    Checkbox::new(id).xsmall().label(label)
}
```

Rules:
- Return the underlying gpui_component type (`Checkbox`, not a newtype).
  Caller chains `.checked(b)`, `.disabled(b)`, `.on_click(...)` directly.
- Apply `.xsmall()` immediately after `::new(...)`, before any other call.
- Take the most ergonomic input types — `impl Into<ElementId>`,
  `impl Into<SharedString>` or `impl Into<Text>` (check the upstream
  signature; not all `label(...)` slots take `SharedString`).
- Re-export the type via `pub use` so callers that need to store
  `Checkbox` in a struct field can do so without bypassing ui/.

### B. Variant factories — Button-like widgets

When the widget has a small fixed set of visual variants, expose a
factory per variant rather than forcing the caller to import
`ButtonVariants` and chain `.primary()` / `.danger()` themselves.

```rust
// ui/button.rs
use gpui::{ElementId, SharedString};
use gpui_component::Sizable as _;
use gpui_component::button::ButtonVariants as _;

pub use gpui_component::button::Button;

pub fn button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    Button::new(id).xsmall().label(label)
}
pub fn button_primary(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    button(id, label).primary()
}
pub fn button_danger(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    button(id, label).danger()
}
/// Bare button (no label) — for icon-only buttons; caller chains `.icon(...)`.
pub fn button_bare(id: impl Into<ElementId>) -> Button {
    Button::new(id).xsmall()
}
```

Naming convention:
- The default (most common) variant takes the bare name (`button` =
  secondary).
- Other variants suffix the variant name (`button_primary`,
  `button_danger`, `button_bare`).
- Don't pass variant as a parameter — separate factories read better at
  the call site than `ui::button("id", "Save", Variant::Primary)`.

### C. Plain re-export — non-`Sizable` types and constructors

For types that don't need `xsmall()` and have no other default to inject,
just `pub use` from `gpui_component`.

```rust
// ui/divider.rs
pub use gpui_component::divider::Divider;
```

```rust
// ui/dialog.rs — call sites construct Dialog inside window.open_dialog()
pub use gpui_component::WindowExt;
pub use gpui_component::button::ButtonVariant;
pub use gpui_component::dialog::{Dialog, DialogButtonProps};
```

A re-export module doubles as the single-import-site rule enforcer:
callers write `use crate::ui::dialog::{ButtonVariant, DialogButtonProps};`
instead of pulling them straight from `gpui_component`.

### D. Closure / helper fn — when GPUI's API takes a closure

Some GPUI APIs (`.tooltip(...)`) take a `Fn(&mut Window, &mut App) -> AnyView`.
Provide a helper that hides the closure ceremony.

```rust
// ui/tooltip.rs
use gpui::{AnyView, App, SharedString, Window};
use gpui_component::tooltip::Tooltip;

pub fn text(content: impl Into<SharedString>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static
{
    let content: SharedString = content.into();
    move |window, cx| Tooltip::new(content.clone()).build(window, cx)
}
```

Call site:
```rust
div().tooltip(crate::ui::tooltip::text("Open in Finder"))
```

Same shape applies whenever the underlying API expects a `Fn` rather
than a value — clone any captured input once into the closure body.

---

## Trait re-exports — what to put in `mod.rs`

Re-export trait imports the call sites need to chain modifiers, not the
ones absorbed by factories.

| Trait | Why it's re-exported |
|---|---|
| `ButtonVariants` | Hand-rolled variant chains (`.outline()`, `.ghost()`) — variant factories cover the common cases, but not all. |
| `Disableable` | `.disabled(b)` is common across many widgets. |
| `WindowExt` | `window.open_dialog(...)` — extension method on `Window`. |
| `ActiveTheme` | `cx.theme()` for direct theme access (rare; mostly used by `ui::theme` itself). |
| `Sizable` | **Not re-exported** — `xsmall()` is auto-applied by factories; callers should not call sizing methods. |

Add a trait re-export only when at least one call site already chains
that trait's methods. Speculative re-exports rot.

---

## Adding a new wrapper

1. **Pick the shape**: A (Sizable factory), B (variant factories), C
   (plain re-export), or D (closure helper). When in doubt, A.
2. **Create `ui/<name>.rs`** with the chosen shape. Mirror the style of
   the closest existing module — `button.rs` for variants, `checkbox.rs`
   for single factory, `divider.rs` for re-export, `tooltip.rs` for
   closure helper.
3. **Re-export from `ui/mod.rs`**:
   - Submodule: `pub mod <name>;`
   - Public surface: `pub use <name>::{...};`
   - Add the section in alphabetical order so `mod.rs` reads as a
     catalogue.
4. **Add a row to the Module layout table at the top of this file** —
   the catalogue is the project's contract; missing rows make wrappers
   undiscoverable.
5. **Swing one or two existing call sites** to the new wrapper as part of
   the same commit. A wrapper with zero callers is dead weight.
6. **Verify**:
   - `cargo check -p daruda`
   - `scripts/lint-direct-gpui-component.sh` (passes if no call site
     bypasses ui/)
   - `cargo fmt -p daruda -- --check`

### When NOT to add to ui/

- **Workspace-specific widgets** (a button that knows about
  `Workspace`, `Pane`, etc.) — those live alongside the feature, not in
  `ui/`. `ui/` only holds primitives that are independent of daruda
  domain types.
- **Modal shells / dialog chrome** — `gpui_component::Dialog` provides
  the panel chrome (bg / border / padding / title / backdrop). Open via
  `crate::workspace::dialog_helpers::{open_form_modal,
  open_single_field_dialog, open_confirm_dialog}`. Don't re-create
  `FormModalShell`-style wrappers in ui/.
- **Composite layouts that consume ui primitives** — small helpers
  like `form_helpers::field_row` are fine in ui/, but anything that
  pulls in workspace types belongs alongside the feature.
- **One-off helpers** used in a single file — keep them inline; ui/ is
  for primitives that get reused.

---

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
| `src/ui/` | The wrapper files themselves — they are gpui_component by definition |

Don't grow this list. If a new file needs direct `gpui_component`
access, ask first whether the access belongs in `ui/` instead.

---

## Vendor patches in `crates/gpui_component/`

Eight small patches over upstream `longbridge/gpui-component` v0.5.1
keep daruda's theme propagation + modal tab containment + tab font /
gap / height control working. Re-apply on rev bump:

| Patch | File | What |
|---|---|---|
| Root render simplification | `src/root.rs` | drop the upstream's font / bg / window-border overrides so daruda's chrome stays intact |
| Root modal tab containment | `src/root.rs` (`on_action_tab` / `on_action_tab_prev` + `focus_within` helper) | when `active_dialogs` is non-empty, wrap Tab/Shift+Tab back into the topmost dialog's focus subtree. Without this, `window.focus_next` is viewport-wide and Tab from the last input in a modal leaks to the workspace behind. The wrap-back call only re-runs `focus_next` if it didn't already land inside the dialog, so a dialog with zero inner tab stops doesn't oscillate. |
| Dialog text color | `src/dialog.rs` | `text_color(cx.theme().foreground)` on the Dialog body so dark-theme text isn't black |
| Select trigger + selected | `src/select.rs` | `text_color(cx.theme().foreground)` on the trigger appearance; selected list item uses `bg(primary) + text_color(primary_foreground)` for visible highlight |
| Tab font-size decoupling | `src/tab/tab.rs` (`Tab::render`) | drop the `match self.size { XSmall => text_xs, Large => text_base, _ => text_sm }` block on `self.base` so font size inherits from the parent `TabBar`. daruda's `tab_bar()` factory bakes `.text_xs()` to keep labels compact even at `Size::Small` strip metrics. |
| TabBar inner-gap override | `src/tab/tab_bar.rs` (`TabBar` struct + `RenderOnce::render`) | add `inner_gap: Option<Pixels>` field + inherent `gap(impl Into<Pixels>) -> Self` method that shadows `Styled::gap`. The render reads `self.inner_gap.unwrap_or(variant_gap)` so the call site (`tab_bar(id).gap(px(0.))`) actually controls the spacing between adjacent tab boxes inside the inner `h_flex` — without this, `Styled::gap` only affects the outer container that holds prefix/h_flex/suffix. |
| Small Underline tab height 30 → 28 | `src/tab/tab.rs` (`TabVariant::height(Size::Small)`) | drop the Small + Underline tab box height from 30px to 28px so the left/right dock `tab_bar()` strip matches daruda's terminal tab bar height (`palette::TAB_BAR_HEIGHT = 28`). Inner metrics (inner_height 22, inner_margins top 2 / bottom 3) unchanged — `items_center` re-centers the inner h_flex inside the 28px box. |
| Button hover text color | `src/button/button.rs` (`RenderOnce::render`) | replace `text_color(crate::red_400())` with `text_color(hover_style.fg)` in the `.hover(...)` closure — upstream accidentally left a debug red literal instead of the theme foreground color, making all button text turn red on hover. |

Plus `[lints]` in `crates/gpui_component/Cargo.toml` silencing all
upstream clippy warnings (vendored code is not in our lint scope).

The patches live directly in the vendored tree (not in
`patches/<name>.patch`) because `crates/gpui_component/` is an
in-repo copy — `scripts/apply-gpui-patch.sh` only handles the
`cargo` git cache patch for the GPUI IME path. On a `gpui_component`
rev bump, re-apply each patch by re-doing the diff against the
upstream source.
