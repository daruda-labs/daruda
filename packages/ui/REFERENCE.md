# UI Reference

Maintained wrapper recipes, module catalogue, and preserved vendor patches.
The governing rules are in [UI Instructions](AGENTS.md); read the relevant
sections here before authoring wrappers or changing a vendored widget.

## Why the wrapper layer

1. **`small()` auto-application** — daruda is a compact terminal UI;
   gpui_component's default `Medium` size is too large. Every `Sizable`
   widget must be constructed at `small`. Factory functions apply it
   so call sites can't forget. `xsmall()` is reserved for explicit
   overrides where a tighter fit is intentional (icon-only chrome
   buttons, badge glyphs with pixel-level sizing). For `PopupMenu`
   builders (`.context_menu` / `.dropdown_menu`), always go through
   `crate::ui::menu_builder(...)` — it injects `small()` automatically.
2. **One place to retheme** — when default colours / variants / paddings
   need to change project-wide, this is the single file to edit.
3. **Single import path** — `use crate::ui::*;` is short and discoverable;
   `use gpui_component::button::{Button, ButtonVariants as _};` is not.
4. **Future-proof** — when `gpui_component` is upgraded or replaced, the
   blast radius is `ui/`, not every call site.

---

## Wrapper module layout

Wrapper files only — preserved daruda widgets are listed in the
[app UI reference](../app/src/REFERENCE.md#ui).

```
ui/
├── mod.rs          # pub mod + factory re-exports + trait re-exports
├── theme/          # apply_daruda_palette(cx), semantic colors, shared metrics, list content budgets
├── alert.rs        # error/warning/info/success(id, msg) factories
├── badge.rs        # Badge::new(label).monospace()/.bg_color()/... over Tag::custom
├── button.rs       # button / button_primary / button_danger / button_bare; the `*_on_surface` family for terminal-mirrored panes — button_on_surface (bare label), button_bare_on_surface (glyph), button_chip_on_surface (label + always-on hairline, so a word-bearing control is not mistaken for the static readouts beside it)
├── button_group.rs # button_group(id, cx) — segmented single-select strip over gpui_component::ButtonGroup (`.children(buttons.selected(..))` + `.on_click(|indices|..)`). Accent appears on the *selected* fill only: resting segments take `text_muted` on a hairline, since upstream's Primary-outline path paints every unselected label and border in accent (3.45:1 as text on the float rung, and one accent element per segment against DESIGN.md's 3–4 budget). Needs the `selected_foreground` vendor patch — see the table below. choice_variant(cx) is the same segment chrome for single-select choices laid out apart (the activity-range editor's spaced choices). button_group_on_surface(id, &PaneSurfaceTokens, cx) is the pane-local variant: the pane mirrors the *terminal* palette, so the UI accent has no verified contrast there and fill/border/hover come from the surface instead
├── chart.rs        # BarChart re-export over gpui_component::chart (Plot-backed; caller wraps in a fixed-height container)
├── checkbox.rs     # checkbox(id, label)
├── code_copy_button.rs # copy_button(id, text, icon, tooltip, copied_icon, copied_tooltip, window, cx) -> Button — icon/tooltip-agnostic copy-to-clipboard button (✓ feedback + 2s targeted-notify revert); returns the `Button` so the caller picks its chrome (overlay chip keeps `button_bare`'s fill, an inline chrome-row icon chains `.ghost()`). code_copy_button(id, code, window, cx) is the markdown-code-block wrapper over it, wired globally by markdown.rs into the vendored `TextView` code-block actions overlay (top-right corner of the block, revealed on hover by the node.rs `gpui-code-block` group patch)
├── code_editor.rs  # `Input`-based editor factories over gpui_component::input — `markdown_editor(&impl TextState)` (bordered prose/code surface) + `make_markdown_state` (`EditorState`) / `make_markdown_prose_state` (`TextareaState`) state builders, and the two read-only chrome variants over an `EditorState` sharing one private `code_editor_chrome`: `file_viewer_editor(&state)` (full-size, file-viewer pane) and `embedded_code_viewer(&state)` (height-capped agent-chat tool-output / diff embed; fallback text colour tracks the pane's terminal-preset background). Re-exports `LineDecoration`
├── dialog.rs       # Dialog / DialogButtonProps / ButtonVariant / WindowExt re-exports
├── disclosure.rs   # disclosure(id, is_open) — stateless chevron toggle; caller owns fold state. `.axis(DisclosureAxis)` picks the glyph pair: `Horizontal` (default, ChevronRight closed / ChevronDown open) for a tree node, `Vertical` (ChevronDown closed / ChevronUp open) for a control that is a *boundary* rather than a node, so it cannot be mistaken for the rows it sits among (the agent chat's tail-window boundary). Plus .color()/.size()/.on_toggle()
├── group_box.rs    # group_box() factory over gpui_component::GroupBox (.outline()/.fill()/title)
├── highlighter.rs  # LanguageRegistry / LanguageConfig re-export + `language_for_extension` — the one extension→language resolver for every highlighting surface. Layers registry *capability* (non-empty highlight query) over the extension→language *identity* in `daruda_core::language`; unresolvable → `PLAIN_LANGUAGE`. `language_for_name` is the same capability check for a caller holding a canonical language name (a fenced block's token) instead of an extension
├── icons.rs        # Glyph paths + icon(path): explicit 16px SVG artwork. Controls use Material Symbols (`icons/ui/`); dock tree/list item glyphs use Lucide (`icons/lucide/`) at stroke 1.65, pinned by a test — list new Lucide files in `LUCIDE` and credit them in `LICENSES/lucide-LICENSE.txt`. button_icon / button_icon_danger / button_icon_on_surface add a 24px target; button_icon_glyph(id, path, size, cx) makes the artwork its own target, for a tree row's leading glyph that must not push the label; button_with_icon keeps 16px artwork before a text label, independent of the vendor button tier.
├── divider.rs      # Divider re-export
├── list.rs         # FilteredItem + FilteredDelegate + searchable_list_state + list(&state)
├── list_page.rs    # Shared page chrome for domain-owned list screens: compact header row, toolbar, search field with clear affordance, result summary row, group heading, and empty-state body. Callers own state transitions, counts, localization, scopes, and filters; this module owns repeated spacing, typography, and disclosure/search chrome.
├── list_table.rs   # Domain-free column schema for list headers/body rows. The same `ListTable<K>` drives header and row cells, derives horizontal-scroll minimum width from visible columns + gaps + insets, and distinguishes truncating fixed cells from unclipped control/action cells.
├── markdown/       # markdown(id, text) — rendered, drag-selectable/copyable markdown over gpui_base::TextView (RenderOnce; .selectable()/.color()/.text_size()/.heading_size()/.full_width()). A caller-set `.text_size()` also rebases the view's vertical metrics on that size (`TextViewStyle` line height / paragraph gap / heading base), since the vendored defaults are rem-anchored and stop agreeing with the body at any other size. `tests.rs` holds the click/selection gates plus the rendered-metric regressions (probe opens a real window and reads painted heights back)
├── menu.rs         # DropdownMenu / PopupMenu / PopupMenuItem re-exports + menu_builder + stacked_menu_builder (a dropdown opened inside another float: lifted fill + `text_subtle` edge) + popup_menu_deferred. `ContextMenuExt` is deliberately NOT re-exported: its `.context_menu(..)` renders inside the caller's subtree, where an ancestor clip cuts the menu and its hit-testing — right-click menus go through `crate::workspace::root_menu::RootContextMenuExt` (enforced by `scripts/lint-declarative-context-menu.sh`)
├── popover.rs      # Popover/PopoverState re-export — trigger-anchored panel for browsing surfaces (clicks inside keep it open; outside/Escape dismiss); menus stay on .dropdown_menu + menu_builder
├── progress.rs     # progress(id, value) factory over gpui_component::Progress (`.color()` fill, id keys its value transition)
├── scrollbar/      # vertical_thumb/horizontal_thumb(id, target: impl ThumbTarget, …) → Thumb overlays (preserved daruda widget): an 8px press strip (`SCROLLBAR_HIT_W`) where a left press on the track jumps and one on the thumb band (full strip width) drags, while right presses and hover pass through to the content; a host that activates on any press reads the strip's press with `forget_press`/`take_press` (`element.rs`; `target.rs` implements ThumbTarget for ScrollHandle / UniformListScrollHandle / ListState / Entity<InputState>) + scroll_area(id, max_h, content) — capped overflow body + gutter + pinned built-in draggable Scrollbar (owns the inset-0 pinning invariant); `.fill()` swaps the `max_h` cap for the height a flex column leaves (a popover body between pinned bands), no gutter + Scrollbar/ScrollbarMode re-exports
├── select.rs       # SelectOption + state_with_options + select(&state)
├── dependent.rs    # dependent(parent_on, cx) — indented container for rows that apply only while a parent switch is on; dims (not hides) them while it is off
├── switch.rs       # switch(id, checked, cx) — shadow-free controlled toggle over Button; preserves Tab, Space/Enter, focus and disabled behavior
├── selectable_text.rs # selectable_text(id, text) — verbatim drag-selectable/copyable plain text over gpui_base::SelectableText (RenderOnce; .selectable()/.color()/.text_size()/.heading_size()/.full_width()); no markdown interpretation (zed `new_text` analog)
├── tab_bar.rs      # tab_bar(id) + tab(label) factories over gpui_component (Small + underline; tab() bakes 10px x-padding)
└── tooltip.rs      # tooltip::text(content) and tooltip::with_description(title, description) closure helpers; the latter puts the primary name above muted context
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
require `.small()` at every call site.

```rust
// ui/checkbox.rs
use gpui::ElementId;
use gpui_component::Sizable as _;
use gpui_component::text::Text;

pub use gpui_component::checkbox::Checkbox;

pub fn checkbox(id: impl Into<ElementId>, label: impl Into<Text>) -> Checkbox {
    Checkbox::new(id).small().label(label)
}
```

Rules:
- Return the underlying gpui_component type (`Checkbox`, not a newtype).
  Caller chains `.checked(b)`, `.disabled(b)`, `.on_click(...)` directly.
- Apply `.small()` immediately after `::new(...)`, before any other call.
- Take the most ergonomic input types — `impl Into<ElementId>`,
  `impl Into<SharedString>` or `impl Into<Text>` (check the upstream
  signature; not all `label(...)` slots take `SharedString`).
- Re-export the type via `pub use` so callers that need to store
  `Checkbox` in a struct field can do so without bypassing ui/.

Text fields have a stricter contract: `Input` is not re-exported. Use
`input` / `input_on` for standard 28px fields, `form_input` for 36px authoring
fields, and `search_input` / `search_input_on` for search. They return opaque
elements so callers cannot override height or padding. `readonly_input` and
`readonly_form_input` use the same profiles; `select::select` and
`select::form_select` share their metrics. Width belongs to the parent.
The private `field_geometry` module owns the recipe and `theme::metrics`
owns its values. Do not copy the recipe into a screen-specific wrapper.

Authoring modals use `form_input` and `select::form_select`, including
single-field dialogs. Search queries keep the standard search profile in any
host. `list_page::search` delegates suffix placement and focus restoration to
`input::search_with_clear`; do not overlay clear buttons over full-width text.

Standard action buttons use pixel metrics. Compact actions use named compact
factories or `compact_button` for composed text buttons. Icon-only actions use
`button_bare_compact` / `compact_icon_button`, which own square targets and
zero padding; chaining `xsmall` alone
does not change explicit pixel geometry. Popover field triggers use
`field_trigger`, which shares standard Input/Select geometry. `form_helpers`
owns authoring labels, stacked fields, validation and footer bands.
`empty_message` / `message_line` supply themed quiet text. `popover_panel`
owns viewport sizing; set `PanelLimits.chrome` to the actual outer insets.

Composers go through `input_with_action_grow` or `InputPanel`, whose text
metrics share one internal factory. Prose and code editors use the editor
factories and keep their configured font, line height and row/scroll behavior.
`scripts/lint-input-factories.sh` prevents raw construction in other packages;
it also covers raw Button/Select construction, including type aliases.
`input::geometry_tests` checks rendered dimensions and text insets in both
themes, including disabled, masked, capped multi-line and focused fields.

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

## Shared Constraints

Token ownership, placement restrictions, destructive-action confirmation,
import exceptions, and GPUI safety are maintained in [UI Instructions](AGENTS.md).

## Vendor patches in `vendor/gpui-kit/`

The widgets come from upstream gpui-component 0.8.1, vendored at
`vendor/gpui-kit/crates/` as `gpui-base` (`gpui_base`: headless text/input
engine, `TextView`, `SelectableText`, `Root`, window-wide text selection,
dialog and popover primitives) and `gpui-component` (`gpui_component`: styled
widgets), plus `gpui-kit-assets` and `gpui-component-macros`. Outside this package,
only the app's UI facade and infrastructure sites reach them directly. Each daruda change
in vendored source is marked `// daruda patch:`; the full record, with the
reason for each patch and the re-vendor procedure, is in
[patches/README.md](../../patches/README.md#vendorgpui-kit--vendored-gpui-component-081-patched-in-place).

The patches this package's wrappers depend on:

| Patch | Used by |
|---|---|
| `ButtonCustomVariant::{selected_foreground, border}` | `button_group` (`choice_variant`), `button_group_on_surface` |
| `Checkbox::indeterminate` | `checkbox` callers with a partial selection (the transcript's display filter) |
| `PopupMenu::{small, chrome, max_h_in_window, pin_head, pin_foot}`, `PopupMenuItem::tooltip` | `menu` (`menu_builder`, `stacked_menu_builder`) and the app's status-bar menus |
| `PopoverState::trigger_bounds` | popover panels sized against the room below their trigger (transcript editors) |
| `InputBaseState::{on_empty_text_input, has_marked_text}` and composition-finalizing `Change` | composer question shortcuts and IME-safe arrow navigation |
| `InputState::set_value` emits `InputEvent::Change`; `set_value_silently` keeps the silent write | every field whose derived state follows `Change` |
| `Input::from_state` + public `TextInputState`; `Input::{input_padding, editor_background, show_scrollbar}` | `input`, `code_editor` (`TextState`, `FieldState`, `file_viewer_editor`, `embedded_code_viewer`) |
| `LineDecoration`, `set_line_decorations`, `set_highlight_override`, `scroll_size`, `display_rows`, `move_cursor_to_end` | editor callers through `code_editor`'s re-exports: the diff and output embeds, the file viewer and its scrollbar, the composer |
| `ThemeStyle::new` + `bold` / `italic`, `query_compilations` | `theme::syntax`, `highlighter` |
| `TextView::{code_block_render, debug_inline_bounds}`, `markdown-task-checkbox` selector, loose/tight list spacing | `markdown` and the app's Markdown conformance probe |
| `take_right_clicked_link` | re-exported from `lib.rs` for the chat's link context menu |
| Left-button gates on scrollbar and resize-handle presses | `scrollbar` (`a_right_press_on_the_track_does_not_jump_the_view`) |

The manifests allow every upstream lint (vendored code is outside daruda's
lint scope). Upstream modules are kept whole for reproducible re-vendoring.

## Surface presentation sizes

`theme::UiMetrics` resolves a `daruda_config::UiSize` into text, icon, spacing,
row, and minimum target dimensions. Hosts pass it as plain snapshot data. The
`button_icon_sized`, `button_icon_danger_sized`, `button_with_icon_sized`,
`dock_tab_sized`, and `dock_tab_bar_sized` factories and
`SectionHeader::with_metrics` opt into it; existing factories retain Normal.
Text never falls below the shared readable minimum, and targets stay at least
24px. This is independent of terminal/chat font preferences and window zoom.

`button_sized` and `button_compact_sized` own text-action geometry. An
`InputPanel` opts in with `with_metrics` and adopts live changes with
`set_metrics`; its input buffer and focus identity survive a size change.

`button_split_caret_sized` uses the same height as a sized text action and
keeps icon artwork independent of the vendor size tier. `InputPanel` composes
the two halves through the shared dropdown-menu path.
