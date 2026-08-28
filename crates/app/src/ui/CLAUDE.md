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

The same rule covers the other vendored crate, `ferrum_flow` (the
node-graph canvas), through `crate::ui::flow_canvas` — enforced by
`scripts/lint-direct-ferrum-flow.sh`. That one also rejects a
fully-qualified `ferrum_flow::…` path, not just a `use` line, since such
a path needs no import and would otherwise walk past the boundary.

This file documents the **wrapper authoring rules** only. For
preserved-widget conventions (`Entity<T>` + `Render` for stateful, plus
the IME / focus pitfalls), see root `CLAUDE.md` §3 + §4.

---

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

Wrapper files only — preserved daruda widgets are listed in the parent
CLAUDE.md `ui/` table.

```
ui/
├── mod.rs          # pub mod + factory re-exports + trait re-exports
├── theme.rs        # apply_daruda_palette(cx) — palette mapping
├── alert.rs        # error/warning/info/success(id, msg) factories
├── badge.rs        # Badge::new(label).monospace()/.bg_color()/... over Tag::custom
├── button.rs       # button / button_primary / button_danger / button_bare; the `*_on_surface` family for terminal-mirrored panes — button_on_surface (bare label), button_bare_on_surface (glyph), button_chip_on_surface (label + always-on hairline, so a word-bearing control is not mistaken for the static readouts beside it)
├── button_group.rs # button_group(id, cx) — segmented single-select strip over gpui_component::ButtonGroup (`.children(buttons.selected(..))` + `.on_click(|indices|..)`). Accent appears on the *selected* fill only: resting segments take `text_muted` on a hairline, since upstream's Primary-outline path paints every unselected label and border in accent (3.45:1 as text on the float rung, and one accent element per segment against DESIGN.md's 3–4 budget). Needs the `selected_foreground` vendor patch — see the table below. button_group_on_surface(id, &PaneSurfaceTokens, cx) is the pane-local variant: the pane mirrors the *terminal* palette, so the UI accent has no verified contrast there and fill/border/hover come from the surface instead
├── chart.rs        # BarChart re-export over gpui_component::chart (Plot-backed; caller wraps in a fixed-height container)
├── checkbox.rs     # checkbox(id, label)
├── code_copy_button.rs # copy_button(id, text, icon, tooltip, copied_icon, copied_tooltip, window, cx) -> Button — icon/tooltip-agnostic copy-to-clipboard button (✓ feedback + 2s targeted-notify revert); returns the `Button` so the caller picks its chrome (overlay chip keeps `button_bare`'s fill, an inline chrome-row icon chains `.ghost()`). code_copy_button(id, code, window, cx) is the hover-reveal markdown-code-block wrapper over it, wired globally by markdown.rs and revealed by the node.rs `gpui-code-block` group patch
├── code_editor.rs  # `Input`-based editor factories over gpui_component::input — `markdown_editor(&state)` (bordered prose/code surface) + `make_markdown_state` / `make_markdown_prose_state` state builders, and the two read-only chrome variants sharing one private `code_editor_chrome`: `file_viewer_editor(&state)` (full-size, file-viewer pane) and `embedded_code_viewer(&state)` (height-capped agent-chat tool-output / diff embed; fallback text colour tracks the pane's terminal-preset background). Re-exports `LineDecoration`
├── dialog.rs       # Dialog / DialogButtonProps / ButtonVariant / WindowExt re-exports
├── disclosure.rs   # disclosure(id, is_open) — stateless chevron toggle (ChevronDown open / ChevronRight closed; .color()/.size()/.on_toggle()); caller owns fold state
├── group_box.rs    # group_box() factory over gpui_component::GroupBox (.outline()/.fill()/title)
├── highlighter.rs  # LanguageRegistry / LanguageConfig re-export + `language_for_extension` — the one extension→language resolver for every highlighting surface. Layers registry *capability* (non-empty highlight query) over the extension→language *identity* in `daruda_core::language`; unresolvable → `PLAIN_LANGUAGE`. `language_for_name` is the same capability check for a caller holding a canonical language name (a fenced block's token) instead of an extension
├── divider.rs      # Divider re-export
├── list.rs         # FilteredItem + FilteredDelegate + searchable_list_state + list(&state)
├── markdown.rs     # markdown(id, text) — rendered, drag-selectable/copyable markdown over gpui_component::text::TextView (RenderOnce; .selectable()/.color()/.text_size()/.full_width())
├── menu.rs         # ContextMenuExt / DropdownMenu / PopupMenu / PopupMenuItem re-exports
├── popover.rs      # Popover/PopoverState re-export — trigger-anchored panel for browsing surfaces (clicks inside keep it open; outside/Escape dismiss); menus stay on .dropdown_menu + menu_builder
├── progress.rs     # progress(value) factory over gpui_component::Progress (Styled fill bar)
├── scrollbar.rs    # vertical_thumb/horizontal_thumb → Thumb overlays (preserved daruda widget); display-only until `.on_drag(handler)` opts in (the agent-chat embed does) + scroll_area(id, max_h, content) — capped overflow body + gutter + pinned built-in draggable Scrollbar (owns the inset-0 pinning invariant) + Scrollbar/ScrollbarShow re-exports
├── select.rs       # SelectOption + state_with_options + select(&state)
├── selectable_text.rs # selectable_text(id, text) — verbatim drag-selectable/copyable plain text over gpui_component::text::TextView::plain (RenderOnce; .selectable()/.color()/.text_size()/.full_width()); no markdown interpretation (zed `new_text` analog)
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

Small patches over upstream `longbridge/gpui-component` v0.5.1
keep daruda's theme propagation + modal tab containment + tab font /
gap / height control + agent-chat markdown chrome working. Re-apply on
rev bump (the table below is the full list):

| Patch | File | What |
|---|---|---|
| Root render simplification | `src/root.rs` | drop the upstream's font / bg / window-border overrides so daruda's chrome stays intact |
| Root modal tab containment | `src/root.rs` (`on_action_tab` / `on_action_tab_prev` + `focus_within` helper) | when `active_dialogs` is non-empty, wrap Tab/Shift+Tab back into the topmost dialog's focus subtree. Without this, `window.focus_next` is viewport-wide and Tab from the last input in a modal leaks to the workspace behind. The wrap-back call only re-runs `focus_next` if it didn't already land inside the dialog, so a dialog with zero inner tab stops doesn't oscillate. |
| Dialog text color | `src/dialog.rs` | `text_color(cx.theme().foreground)` on the Dialog body so dark-theme text isn't black |
| Select trigger + selected | `src/select.rs` | `text_color(cx.theme().foreground)` on the trigger appearance; selected list item uses `bg(primary) + text_color(primary_foreground)` for visible highlight |
| Tab font-size decoupling | `src/tab/tab.rs` (`Tab::render`) | drop the `match self.size { XSmall => text_xs, Large => text_base, _ => text_sm }` block on `self.base` so font size inherits from the parent `TabBar`. daruda's `tab_bar()` factory bakes `.text_xs()` to keep labels compact even at `Size::Small` strip metrics. |
| TabBar inner-gap override | `src/tab/tab_bar.rs` (`TabBar` struct + `RenderOnce::render`) | add `inner_gap: Option<Pixels>` field + inherent `gap(impl Into<Pixels>) -> Self` method that shadows `Styled::gap`. The render reads `self.inner_gap.unwrap_or(variant_gap)` so the call site (`tab_bar(id).gap(px(0.))`) actually controls the spacing between adjacent tab boxes inside the inner `h_flex` — without this, `Styled::gap` only affects the outer container that holds prefix/h_flex/suffix. |
| Small Underline tab height 30 → 28 | `src/tab/tab.rs` (`TabVariant::height(Size::Small)`) | drop the Small + Underline tab box height from 30px to 28px so the left/right dock `tab_bar()` strip matches daruda's terminal tab bar height (`palette::TAB_BAR_HEIGHT = 28`). Inner metrics (inner_height 22, inner_margins top 2 / bottom 3) unchanged — `items_center` re-centers the inner h_flex inside the 28px box. |
| `ButtonCustomVariant::selected_foreground` | `src/button/button.rs` (`ButtonCustomVariant` field + builder + the `Custom` arm in `ButtonVariant::selected`) | `Custom` carried one `foreground` for every state, so a segmented strip could not give its selected segment a different label colour from the resting ones — and on daruda's palette every single-colour choice breaks a DESIGN.md rule: `text_muted` on the accent fill is 1.5:1, `text_body` 3.3:1, `text_primary` 4.4:1 (all under the 4.5:1 floor), and `accent_fg` (`#fff`) is banned as text on the near-black resting surface. Optional (`None` → `foreground`, upstream behaviour), so no existing call site changes; `crate::ui::button_group` sets `text_muted` resting / `accent_fg` selected (5.00:1 and 4.70:1 — measured on `t.popover`, i.e. the `surface-4` float rung), and `button_group_on_surface` brightens to the pane's full foreground. Without it the shipped strip painted every *unselected* segment's label **and** border in accent — 3.45:1 as text, and up to ~36 accent elements at once in the agent-chat fold editor against DESIGN.md's 3–4 budget. |
| Dialog reads the float slot | `src/dialog.rs` (`RenderOnce::render`) | `bg(theme().background)` → `bg(theme().popover)`. `background` is the app canvas (`surface-1` on daruda), which put a dialog on the same rung as the docks it covers and one step *below* the popovers floating above it. `Theme::popover` is upstream's only float-surface slot and `popover_style` already routes every menu, tooltip and popover through it; daruda maps it to `float_panel_bg` (`surface-4`) so all eight floating surfaces agree. |
| Tooltip radius reads the theme | `src/tooltip.rs` | `rounded(px(6.))` → `rounded(cx.theme().radius)`, so a tooltip and an adjacent popover stop disagreeing on their corners (6px vs 4px). Upstream's only hard-coded radius among the float surfaces. |
| Button hover text color | `src/button/button.rs` (`RenderOnce::render`) | replace `text_color(crate::red_400())` with `text_color(hover_style.fg)` in the `.hover(...)` closure — upstream accidentally left a debug red literal instead of the theme foreground color, making all button text turn red on hover. |
| PopupMenu small size | `src/menu/popup_menu.rs` (`PopupMenu::small` + `render_menu_item`) | change `pub(crate)` to `pub`; also wire `Size::Small` into the font — `text_xs` for small, `text_sm` otherwise (upstream only shrank item height to 20 px but left font at `text_sm`). |
| PopupMenuItem hover tooltip | `src/menu/popup_menu.rs` (`PopupMenuItem::Item.tooltip` field + `.tooltip()`), `src/menu/menu_item.rs` (`MenuItemElement.tooltip` field + render wiring) | System B had no hover-tooltip affordance — daruda uses it to explain *why* a disabled menu item is disabled (e.g. "Commit first" on a disabled merge action); daruda's now-removed System A `ContextMenuItem` already had it. Same `.tooltip(text)` builder shape as `Switch`/`Button`; renders independent of the disabled-gated click handler, so it survives disabled state. |
| GroupBox compact padding + gap | `src/group_box.rs` (`RenderOnce::render`) | content padding `p_4`→`p_2` (16→8px) and child spacing `gap_4`→`gap_1` (16→4px) so `.fill()`/`.outline()`/`.normal()` cards stay compact in the narrow right dock (Usage tab gauge bars + 3-up stat grid). Upstream's 16px overflows the 3-column stat row and over-spaces the gauge bar. |
| `ThemeStyle::new` + `bold()`/`italic()` | `src/highlighter/registry.rs` (`impl ThemeStyle`) | add `pub fn new(color: Hsla) -> Self` (a foreground-only style) plus `bold()` / `italic()` builders that set `font_weight` / `font_style`. Upstream only builds `ThemeStyle` via JSON deserialization (fields private), so the host can't seed `SyntaxColors` programmatically. daruda's `apply_daruda_palette` maps the selected `palette::editor_syntax_colors_of(..)` through this into `highlight_theme.style.syntax`, so the raw editor and the diff view share one syntax-colour source — and a palette's non-color channel (keyword bold / comment italic) reaches the editor. |
| Per-line editor decorations | `src/input/decoration.rs` (new), `src/input/mod.rs` (`mod`/`pub use`), `src/input/state.rs` (`line_decorations` field + `set_line_decorations`), `src/input/element.rs` (`layout_line_numbers` width, gutter build loop, active-line paint loop) | add `LineDecoration { background, gutter }` + `InputState::set_line_decorations(Vec<_>)`. Lets the host paint a per-row background fill and override the sequential `ix + 1` gutter with a custom string (the diff viewer packs dual old/new line numbers into one column). Empty list = upstream behaviour. Foundation for rendering the diff *through* the editor (renderer unification, 3b). |
| Editor diff/read-only support | `src/input/state.rs` (`highlight_override` field + `set_highlight_override` + `set_disabled` + `scroll_handle` getter), `src/input/element.rs` (`highlight_lines` override short-circuit), `src/input/input.rs` (`Input::show_scrollbar` field + builder + gate in `render_editor`) | render the diff *through* the editor (3b). `set_highlight_override(Vec<(Range, HighlightStyle)>)` replaces the tree-sitter highlighter (a synthetic +/- diff buffer isn't valid source — fg = syntax, bg = word-diff). `set_disabled` toggles read-only (blocks edit/IME, keeps selection/copy). `show_scrollbar(false)` + the public `scroll_handle()` let the file viewer suppress the built-in 16px bar and overlay its thin 4px daruda thumb so raw and diff match the other viewer modes. |
| Optional inner padding | `src/input/input.rs` (`Input::input_padding` field + builder + `.when(self.input_padding, …)` gate around `input_px`/`input_py` in `render`) | `appearance(false)` strips chrome but **not** the size-derived `input_px`/`input_py`, so a borderless editor still inherits `Size::Medium`'s 12px/8px gap before the line-number gutter. `input_padding(false)` zeroes it so the file-viewer raw/diff editor sits flush left like the markdown-raw / preview renderers; the host (body frame) owns any surrounding spacing. Default `true` — other `appearance(false)` call sites (`ui/input.rs` chrome cells) keep their `small` padding. |
| Single-line newline guard | `src/input/state.rs` (`replace_text_in_range` single-line branch) | strip `\n` from `new_text` for non-multi-line inputs at the one mutation entry point, so every path (`set_value`, programmatic `insert`, the search panel's selection pre-fill) honours the invariant the paste path already enforced. A single-line value reaches `TextElement::layout_lines`' single-line branch, which feeds the whole string to gpui's `shape_line` — and `shape_line` panics on an embedded newline. Repro: open the file-viewer editor search (Cmd+F) with a multi-line selection; `on_action_search` pre-fills the single-line search input with the selection → panic on next prepaint. |
| `on_secondary_tab` hook | `src/input/state.rs` (`on_secondary_tab` field + builder), `src/input/indent.rs` (`outdent_inline`) | host handler invoked on Shift+Tab in multi-line mode: it does the work and returns `true` to consume the key (input then skips its default outdent), `false` to fall through to outdent. A do-the-work callback (`Fn(&mut Window, &mut App) -> bool`) shaped like `on_completion_accept` — deliberately NOT a new `InputEvent` variant, which would force every exhaustive `match InputEvent` (the modal subscriptions) to grow a no-op arm. daruda's bottom input installs it to cycle the focused agent pane's session mode (`Workspace::cycle_agent_mode`), matching Claude Code's permission-mode cycle. `None` = upstream behaviour (Shift+Tab outdents). |
| `on_history_navigate` hook | `src/input/state.rs` (`HistoryDir` enum + `on_history_navigate` field + builder), `src/input/movement.rs` (`try_history_navigate`, `up`, `down`) | host handler invoked when ↑ is pressed and the cursor is on the first display line, or ↓ when on the last display line. It does the work (e.g. recall a history entry via `set_value`) and returns `true` to consume the key (input then skips cursor movement), `false` to fall through to normal up/down movement. Boundary detection uses `text_wrapper.offset_to_display_point(cursor).row` vs `text_wrapper.len()-1`; bails without calling the hook when the wrapper hasn't been prepared yet (`len()==0`). Same `Fn(HistoryDir, &mut Window, &mut App) -> bool` shape as `on_secondary_tab`. daruda's bottom input installs it to provide per-lane prompt/command history navigation (`Workspace::do_history_navigate`). `None` = upstream behaviour (↑/↓ always moves cursor). |
| `set_auto_grow` setter | `src/input/state.rs` (`set_auto_grow(min_rows, max_rows)`) | `&mut self` counterpart to the `auto_grow` builder, for updating the auto-grow bounds after construction — e.g. on a live config reload. No-op when mode is not `AutoGrow`. Lets `Workspace::apply_config` keep the `InputState`'s internal editor cap in sync with the `[agent] input_max_rows` config value (single source of truth: both the dock-height cap in `adapt_dock_to_input_lines` and the editor cap now read from the same `agent.input_max_rows` field). |
| `display_rows` accessor | `src/input/state.rs` (`pub fn display_rows(&self) -> usize`) | returns the soft-wrapped display-row count from `self.mode.rows()` — the value `update_auto_grow` computes from the text wrapper. Lets `adapt_dock_to_input_lines` track display rows (soft-wrapped) instead of hard newlines so a long line that wraps to N rows grows the dock by N×20px rather than 1×20px. |
| `move_cursor_to_end` | `src/input/state.rs` (`pub fn move_cursor_to_end(&mut self, cx)`) | moves the cursor to `text.len()` without changing content. Used after `set_value` in history navigation so the recalled entry has the cursor at the end (shell-history convention) — multi-line `set_value` default is start. |
| `last_bounds` accessor | `src/input/state.rs` (`pub fn last_bounds(&self) -> Option<Bounds<Pixels>>`) | read-only view of the text element's last painted bounds, mirroring the `scroll_handle()` accessor. Layout probe for host tests measuring the editor's painted viewport (the agent-chat diff embed regression tests in `workspace/tests/agent_diff_layout.rs`). |
| `scroll_size` accessor | `src/input/state.rs` (`pub fn scroll_size(&self) -> gpui::Size<Pixels>`) | read-only view of the text element's last painted total content size (the scrollable extent), pairing with `last_bounds()` (viewport) for a host's own scrollbar geometry. This crate's text element never calls `div().track_scroll(&scroll_handle)` — it manages scrolling manually via `set_offset` + this field — so `scroll_handle().bounds()`/`.max_offset()` stay permanently zero and are **not** a substitute. Fixes a File-viewer regression where the Raw/Diff editor's custom scrollbar thumb never drew (it read the always-zero handle getters) since the editor mode's overlay was introduced (`e6c3bd4`); also backs the agent-chat diff embed's horizontal thumb (`crate::ui::scrollbar::horizontal_thumb`). |
| `set_scroll_offset` setter | `src/input/state.rs` (`pub fn set_scroll_offset(&mut self, offset, cx)`) | the write counterpart of `scroll_handle()`, delegating to the private `update_scroll_offset` so the clamp and the `cx.notify()` stay in one place. A host that suppresses the built-in bar (`Input::show_scrollbar(false)`) and draws its own thumb needs to drive the editor when that thumb is **dragged**; writing `scroll_handle().set_offset` directly skips both, leaving the editor scrolled past its content or not repainting. Used by `agent_chat_pane/render/embed.rs`'s `drag_axis`. |
| `visible_rows` accessor | `src/input/state.rs` (`pub fn visible_rows(&self) -> Option<Range<usize>>`) | read-only view of `last_layout.visible_range` — the row range the last paint actually shaped and painted (`LastLayout` itself stays `pub(super)`). The only observation point proving this crate's visible-row virtualization is live, which is what makes the agent-chat bounded output embed a fix rather than a hope: `element.rs`'s `calculate_visible_range` derives that range from the painted height, so an embed that loses its height bound silently reports every row as visible and restores the linear per-line paint cost (the 99%-CPU repro: a tool card holding `seq 1 200000`'s ~11k lines). Asserted from both sides in `workspace/tests/agent_output_layout.rs` — too many rows means the bound is gone, too few means the editor collapsed inside its reserved box. |
| `code_editor_language` accessor | `src/input/state.rs` (`pub fn code_editor_language(&self) -> Option<&SharedString>`) | reads back the language a code-editor state highlights with (`None` outside code-editor mode). `InputMode` is `pub(crate)`, so a host that *derives* the language from its own data has no way to confirm which one reached the editor — the agent chat resolves a read tool's file extension to a language in `daruda_acp` and hands it to `create_output_editor`, and without this accessor a test can only prove the host computed it, not that the editor highlights with it. Asserted by `a_read_tool_s_unfenced_output_renders_through_the_capped_embed` in `workspace/tests/agent_output_layout.rs`. |
| Read-only code editors get no bottom scroll pad | `src/input/element.rs` (`empty_bottom_height` gate) | pad `scroll_size().height` by half a viewport only when the code editor is **not** `disabled`. The pad exists so an editor can lift its last line to mid-screen; a read-only viewer has no cursor to centre, and the pad made a fully-visible buffer scrollable into blank space. Inside an outer scroller that was visible as a bug: a short agent-chat output embed scrolled into empty space *and* dragged the transcript with it, because an uncapped embed deliberately does not occlude the list. Guarded by `a_short_output_block_neither_scrolls_nor_shows_a_thumb` in `workspace/tests/agent_output_layout.rs`. The file viewer's raw mode stays editable, so it keeps the pad; its diff mode and both agent-chat embeds are `set_disabled(true)` and lose it. |
| `is_disabled` accessor | `src/input/state.rs` (`pub fn is_disabled(&self) -> bool`) | reads back the state's disabled flag. `Input::render` unconditionally writes `state.disabled = self.disabled` every frame, so an `Input` element built without `.disabled(true)` clobbers a `set_disabled(true)` the host applied to the state (read-only diff/file viewers) on the next paint. `crate::ui::file_viewer_editor` forwards this value into `.disabled(...)` so the reconciliation is a no-op and read-only mode survives. |
| `Input::scroll_wheel` policy | `src/input/input.rs` (`pub enum ScrollWheelBehavior { Both, Horizontal, Vertical }` + `Input::scroll_wheel` field/builder), `src/input/state.rs` (`scroll_wheel` field + `on_scroll_wheel` dispatch) | which axes the editor scrolls on the wheel; the unhandled axis bubbles to an outer scroller. Default `Both` (upstream: scroll both axes, consume on offset change). `Horizontal` scrolls/consumes only horizontal-dominant gestures (`|dx| > |dy|`) and bubbles vertical ones; `Vertical` is the mirror. `Horizontal` fits an embed that shows every row, so only its long non-wrapped lines scroll; the agent-chat embeds (`render/embed.rs`, height-capped) instead keep the default `Both`, since rows below the cap are reachable only by scrolling inside the embed and `on_scroll_wheel` consumes an event only when the offset actually changed. Modelled as an enum (not a bool pair) so the axis policies stay mutually exclusive. |
| Editor handles the wheel in the capture phase | `src/input/element.rs` (`PrepaintState.scroll_hitbox` + `insert_hitbox` in `prepaint` + a capture-phase `ScrollWheelEvent` listener in `paint_mouse_listeners`), `src/input/input.rs` (the div's `on_scroll_wheel` removed) | `div::on_scroll_wheel` is bubble-only, and gpui runs bubble listeners in reverse registration order. A virtualized outer list registers its wheel handler *after* painting its items, so it always fired first and never stopped propagation — one gesture scrolled both it and an embedded editor. Capture runs before every bubble listener, and `InputState::on_scroll_wheel` already stops propagation only when the offset actually changed, so moving the registration is the whole fix: the editor takes the gesture while it has somewhere to go and hands it to the outer scroller at the extremes. That is scroll chaining — the host no longer needs `occlude()` on the embed (which bought no-double-scroll at the cost of never chaining). Guarded by `a_capped_embed_holds_the_wheel_while_it_can_still_scroll` and `a_capped_embed_at_its_end_passes_the_wheel_to_the_transcript` in `workspace/tests/agent_output_layout.rs`; disabling the listener fails the first, keeping `occlude()` fails the second. The hitbox must be the element's own rect, not the `bounds` `prepaint` shadows and shifts by the scroll offset — hit-testing against that walks it off screen and the wheel stops reaching the editor after the first notch (`a_capped_embed_keeps_scrolling_across_repeated_notches`). |
| Empty custom gutter reserves no width | `src/input/element.rs` (`layout_line_numbers` `has_gutter_content` gate) | when custom `line_decorations` are installed but every gutter string is empty (the diff viewer hiding snippet-relative line numbers), `line_number_width` is `0` instead of `content + 6px + LINE_NUMBER_RIGHT_MARGIN` — otherwise an all-blank gutter reads as an unexplained ~16px indent. Sequential numbering (no decorations) and any non-empty custom gutter keep their width. The row-background decorations still paint (they are gated on `line_number` mode being on, which the diff editor keeps), so hiding numbers does not drop the add/del tint. |
| TextView list-item wrap | `src/text/node.rs` (`Node::ListItem` render) | wrap the list-item content div in `flex_1().min_w_0()` (was a bare `div().overflow_hidden()`) so item text wraps to the available width instead of laying out at its intrinsic max-content width and clipping the wrapped continuation. Without it, agent-chat markdown list items (`crate::ui::markdown`) overflow the pane — no wrap, second line cut off — until the pane is narrowed. |
| TextView list-item block children | `src/text/node.rs` (`ListItemChildLayout` enum + `classify_list_item_children` + `render_list_item`) | `render_list_item`'s child loop only matched `Paragraph` / `List` and dropped every other block on a `_ => {}` arm, so a fenced **code block** (or blockquote / table / heading / rule) nested inside a list item silently vanished — and the paragraph after it wrongly merged into the previous line (the old `last_not_list` merge only excluded `List`, not code blocks). Now a pure `classify_list_item_children` maps each child to `{ LeadParagraph, MergedParagraph, ContinuationParagraph, NestedList, Block }`: consecutive paragraphs still merge, a paragraph after a nested list keeps its prefixed leading line, and any other block — plus a paragraph following one — renders as an indented `div().ml(rems(1.)).min_w_0()` continuation instead of being dropped (the `min_w_0` overrides the flex default `min-width: auto` so the continuation shrinks to the list width instead of laying out at intrinsic max-content width and clipping at wider panes — same width-dependent class as the lead-paragraph / table-cell patches). Repro: an agent-chat reply with a numbered/bulleted item whose body contains an indented code fence (e.g. a code-review "add a `catch`:" step). The classifier is unit-tested (`text::node::tests`). |
| TextView list-item continuation prose stacks | `src/text/node.rs` (`render_list_item` groups a paragraph with the `MergedParagraph` run after it and builds them into one `v_flex` content column; the `_` arm's container is a `v_flex` too) | `MergedParagraph` appended its block to `items.last_mut()` — the lead paragraph's container, which is an `h_flex` (a flex **row**). So a list item's second and later paragraphs rendered *beside* the first instead of under it: they added zero height and split the item's width with it, so the longer the lead line the narrower and taller the continuation column (measured 84px → 1050px for the same prose under a longer lead). Repro: any agent reply whose numbered findings each carry a claim plus explanatory paragraphs — every finding collapsed into side-by-side columns. Blocks after the first in a run get `pt(paragraph_gap)` because `render_block` zeroes a paragraph's bottom margin inside a list (`in_list \|\| is_last`), which would otherwise leave the stacked prose flush and reading as one block. Regression-tested at `crate::ui::markdown::tests::{a_list_item_stacks_its_continuation_paragraphs, a_list_item_continuation_keeps_the_full_width}`. |
| Markdown hard line break | `src/text/format/markdown.rs` (`parse_paragraph` `Node::Break` arm) | `mdast` reports a hard break (two trailing spaces, or a trailing backslash) as an **inline** child of the paragraph, but `parse_paragraph` had no arm for it, so it fell to the `_ => warn` catch-all and was dropped — with no separator pushed in its place, the runs on either side rendered glued into one word (`…제거합니다.lib.rs`). Now pushes a `"\n"` run, the same shape the sibling inline-`<br>` branch already used. Repro: an agent reply that puts a citation link on the line under its claim. Regression-tested at `text::format::markdown::tests::{a_hard_line_break_becomes_a_newline_run, a_backslash_line_break_becomes_a_newline_run}` + `crate::ui::markdown::tests::a_hard_line_break_starts_a_new_line`. |
| Emphasis keeps its children's marks | `src/text/format/markdown.rs` (`mark_children` + the `Emphasis` / `Strong` / `Delete` arms) | the three emphasis arms parsed their children into a throwaway `child_paragraph`, kept only the concatenated **text**, and pushed it as one run carrying just their own mark — so inline code, links, and nested emphasis inside `**…**` lost their formatting entirely (a link inside bold stopped being clickable). Now they push their mark over every child run and `merge` the child paragraph, the same shape the neighbouring `Link` arm already used; marks compose because `Node::Paragraph`'s render folds each mark into a separate `HighlightStyle`. Regression-tested at `text::format::markdown::tests::{bold_keeps_the_marks_of_its_children, bold_keeps_a_nested_link}`. |
| TextView table-cell wrap | `src/text/node.rs` (`render_table` cell div) | drop `.truncate()` (which sets `white-space: nowrap` + ellipsis + `overflow_hidden`, forcing every cell to a single clipped line) and instead add `.min_w_0()` on the cell + wrap the cell text in a `div().min_w_0().overflow_hidden()`. `min_w_0` on the cell lets it shrink to its proportional `relative(len)` width inside the `flex_row` (otherwise cells hold their min-content size, the row overflows, and no cell reaches a width the text can wrap into); the `min_w_0` inner div then shrinks below its content so the text wraps, while the cell's `justify_*` (column align) still positions text that's narrower than the cell. Same class as the list-item wrap fix — agent-chat markdown table cells (`crate::ui::markdown`) laid out at intrinsic max-content width and truncated instead of wrapping. |
| TextView heading wrap | `src/text/node.rs` (`Node::Heading` render) | wrap the heading's text child in `div().flex_1().min_w_0()` (was a bare `.child(children.render(...))` directly inside the `h_flex()` heading container). Same class as the list-item and table-cell wrap fixes: a flex-row item's default `min-width: auto` stops it shrinking below its unwrapped content width, so a `#`/`##`/`###` heading held its intrinsic max-content width and never re-wrapped as the pane narrowed — it just overflowed unbroken instead of breaking to a second line. Regression-tested at `crate::ui::markdown::tests::heading_reflows_narrower_than_wide` (app crate — opens two real windows at a narrow vs. wide width and asserts the narrow one paints taller, i.e. wraps to more lines); confirmed to fail without this patch (`narrow == wide`, no reflow). |
| TextView code-block render override | `src/text/{text_view.rs,node.rs}` | `code_block_render` hook (mirrors `code_block_actions`) lets the host replace a code block's rendering with a custom element; used by the agent chat to render ` ```mermaid ` blocks as a diagram. The closure receives the `CodeBlock` (with `lang()` / `code()`) and returns `Some(element)` to override or `None` to keep default. `None`/absent = default rendering. |
| Inline-code + code-block background = background-derived tint | `src/text/node.rs` (`style.code` inline highlight; `codeblock` div fill + border) | inline code (`` `code` ``) and fenced code blocks fill with a translucent neutral tint picked by `cx.theme().background.l < 0.5` (white over dark, black over light) instead of the upstream `accent` (inline) / `muted` + `border` (block). Inline fill = `0.08`; block fill = `0.05`, block border = `0.28` (the shared structural-line tint, matching the table + `<hr>` rule). daruda's `accent` is a scarce chromatic signal (active lane / focus ring / primary CTA / Claude badge — DESIGN.md §Accent scarcity, "never a panel fill") and read as noise repeated across spans; a translucent tint sits one step off the background on any theme and lets the pane opacity show through, mirroring the host's `theme::agent_chat_tint` / `agent_chat_border_tint` used for tool cards. |
| Table / divider / blockquote lines = background-derived tint | `src/text/node.rs` (`render_table` `line_color`; `Node::Divider` `rule_color`; `Node::Blockquote` border) | structural markdown lines used fixed theme colors near-invisible against the agent-chat pane's mirrored terminal background: **table** frame/row/cell separators + the **horizontal rule** (`---`) used `cx.theme().border` (daruda's `HAIRLINE`, L≈0.15); the **blockquote** left bar used `cx.theme().secondary_active` (daruda's canvas, L≈0.03, effectively black). Table and divider now pick a translucent neutral (`hsla(0,0,1,0.28)` over dark, `hsla(0,0,0,0.28)` over light) via `cx.theme().background.l < 0.5` — the shared structural-line tint, matching the code-block border (`0.28` alpha); brighter than the `0.12` originally used, since the fainter hairline read as invisible next to near-white chat text. The blockquote bar now uses `cx.theme().muted_foreground` — the same muted gray as the quote's own text, so it reads clearly and coherently. |
| Shared text primitives hoisted out of the vendored tree | `src/text_selection.rs` **deleted**; `src/input/{selection.rs,state.rs}` + `src/text/inline.rs` import `daruda_core::text`; `daruda_core` added to `Cargo.toml` | `CharType` / `word_range` / `logical_line_range` / `char_cell_hit_x` now live in `daruda_core::text`, so `daruda_terminal` and the app reach them too and this patch shrinks to a dependency line plus imports. The old `floor_char_boundary` / `ceil_char_boundary` stand-ins are gone — std stabilised both in 1.91, so callers use `str::{floor,ceil}_char_boundary` directly. |
| `input/selection.rs` delegation | `src/input/selection.rs` | removed local `CharType` + old walk loop; `TextSelector::word_range` now delegates to `daruda_core::text::word_range(text.len(), \|i\| text.char_at(i), offset)`. Tests updated to `use daruda_core::text::CharType`. |
| Triple/quad-click + mode-aware drag | `src/input/state.rs` (`SelectMode` enum + `select_mode`/`select_anchor` fields + `on_mouse_down` + `on_drag_move` + helpers `word_range_at`/`line_range_at`) | `SelectMode { Character, Word(Range), Line(Range), All }` drives drag extension granularity. Triple-click selects the logical (newline-delimited) line and sets `SelectMode::Line`; quad-click selects all and sets `SelectMode::All`. Double-click (existing) now also sets `SelectMode::Word(anchor)`. Drag-move dispatches on `select_mode`: `Character` → `select_to`, `All` → no-op (keeps full selection, never shrinks on drag), Word → union of anchor and word-at-cursor, Line → union of anchor and line-at-cursor. Single-click resets to `Character`. `line_range_at` delegates to `daruda_core::text::logical_line_range`. |
| `indoc` dev-dependency | `Cargo.toml` (`[dev-dependencies] indoc = "2"`) | existing `selection.rs` tests referenced `indoc::indoc!` without declaring the dep — a pre-existing test compile failure. Adding it as a dev-dependency fixes the test suite for `gpui_component` (the remaining test failures in `tree.rs` are a separate pre-existing upstream issue). |
| TextView word/line/all click selection | `src/text/text_view.rs` (`pub enum SelectMode { Character, Word, Line, All }` — unit variants; `pub fn select_mode_for_click_count(n) -> SelectMode` pure fn; `start_selection(pos, click_count)` calls it; `select_mode` field + `has_selection` guard + `select_mode()` getter), `src/text/inline.rs` (`layout_selections` mode-aware expansion) | Double-click (count=2) → `SelectMode::Word`: `layout_selections` expands the raw pixel-hit byte range to full word boundaries via `daruda_core::text::word_range`. Triple-click (count=3) → `SelectMode::Line`: expands to the full visual (wrapped) line by scanning all chars whose Y position overlaps the raw selection's Y band. Quad-click (count≥4) → `SelectMode::All`: selects the entire Inline text. Character (count=1) → unchanged drag behaviour. `has_selection` returns `true` for Word/Line/All even when start==end (no drag) so the expansion fires on a bare click. `SelectMode` and `select_mode_for_click_count` are `pub` and re-exported from `crate::ui`. |
| Click-point char hit (degenerate selection box) | `daruda_core::text` (`pub fn char_cell_hit_x`), `src/text/inline.rs` (`char_in_text_selection` delegates to it) | A word/line click produces a zero-width selection box (start==end); the old center-threshold test matched no char, so nothing selected (only quad-click, which bypasses the scan, worked). `char_cell_hit_x` treats a degenerate span (`sel_left == sel_right`) as "cell contains the click point" and keeps the center threshold for real drags. Pure fn, re-exported from `crate::ui`, tested in the app crate. |
| Drag selection is a document range, not a rectangle | `src/text/text_view.rs` (`TextViewState::selection_span`), `src/text/inline.rs` (`endpoint_cut_x` + `char_in_text_selection` replace `point_in_text_selection`; `layout_selections` pass 1 consumes the span) | Hit-testing read the drag as a normalized `Bounds`, which loses which x belongs to the upper endpoint and which to the lower one — so it only behaved for a drag whose lower end was also further right. Drag from line 1 down to line 2 and then left past the anchor's x and the rect inverted: line 1 grew leftwards from the anchor and line 2 grew rightwards instead of shrinking. Its `single_line` test (box height `<= line_height`) compounded this: a drag crossing a band boundary by a few px bounded *both* lines by the normalized x span. (Pass 1 keeps only the first and last hit, so a purely rightward drag ends up with the same contiguous envelope either way — the visible break is the leftward case above.) Now each endpoint is reduced to a cut on the character's own line band (`-inf` above / `+inf` below / its x when on it) and the two cuts bound that line, so document order — not the drag's shape — decides each line's extent. Lines outside the drag collapse to two equal infinities, which `char_cell_hit_x`'s degenerate branch rejects, so no separate vertical test is needed. `selection_bounds` stays for the (commented-out) debug painter. Band width is `text_layout.line_height()` — the row pitch `position_for_index` itself uses, and the one `paint_selection` paints with; `window.line_height()` is a separate rounding of the same value and can tile the bands with overlap. Unit-tested in `inline.rs`; the bug cases fail against the old model. On a rev bump the upstream `point_in_text_selection` + its `test_point_in_text_selection` must be removed again. |
| Plain-text TextView (`TextViewType::Text`) | `src/text/text_view.rs` (`TextViewType::Text` variant, `TextView::plain(id, text, window, cx)` constructor, `parse_content` arm) | Renders the raw string verbatim as a single paragraph (no Markdown/HTML interpretation) through the same selectable Inline path — a selectable primitive for non-markdown text (command output, logs, titles). Mirrors zed's `Markdown::new_text` (`parse_links_only`) role. Wrapped as `crate::ui::selectable_text`. |
| Multi-byte word/line selection | `src/text/inline.rs` (`layout_selections` char-width via `offset + c.len_utf8()`, Word/Line expansion via std's `str::floor_char_boundary`), `daruda_core::text` (`CharType::from_char` uses `is_alphanumeric`) | Word/line selection was ASCII-only: `offset + 1` / `raw_end - 1` split 3-byte Hangul/CJK glyphs (cell width collapsed, `word_range` bailed at a non-boundary), and `is_ascii_alphanumeric` classified Hangul/CJK as `Other` (non-connectable). Now the byte steps round to char boundaries and any-script letters are `Word`, so double-click selects Hangul/CJK words. std's `str::{floor,ceil}_char_boundary` are the canonical helpers to reach for instead of `offset ± 1`. |
| Selection survives streaming | `src/text/text_view.rs` (`update_bounds` clears only on **width** change; reparse no longer calls `clear_selection`) | A streaming markdown block reparses and grows in height on every chunk; both the unconditional reparse-clear and the any-size-change clear dropped the user's selection mid-stream. Since a block's text only appends (stable layout), keep the pixel selection across reparse and height growth; only a width change (real reflow) clears. |
| Code-block copy-button hover reveal | `src/text/node.rs` (`CodeBlock::render`) | add `.group("gpui-code-block")` to the code-block container div and gate the actions overlay with `.invisible().group_hover("gpui-code-block", \|s\| s.visible())`, so the host's `crate::ui::code_copy_button` (wired for all markdown via `crate::ui::markdown`) is revealed only on hover and an idle block shows no persistent action chip. |
| Streaming re-parse debounce 200ms → 33ms | `src/text/text_view.rs` (`UpdateFuture` delay arg in the `request_layout` init path) | Upstream's 200ms trailing debounce makes streamed agent-chat markdown land in ~200ms steps ("chunky"); 33ms (~30Hz) lets it flow near-continuously like zed's eager reparse (`markdown.rs` `parse` has no fixed delay, only `pending_parse`/`should_reparse` coalescing). Parsing is off the main thread, so a lower value only raises background-parse frequency; static content parses once and is unaffected. Sole `Duration::from_millis` in the file — tune there. |
| Code-block font size inherits ambient | `src/text/node.rs` (`CodeBlock::render`) | drop `.text_size(cx.theme().mono_font_size)` (a fixed 13px theme field the host never wires up) so the code-block div inherits the ambient text size from `TextView`'s outer `.refine_style(&self.style)` (`text_view.rs:800`) instead — the same class of fix as the "Tab font-size decoupling" patch above. Every daruda call site (`crate::ui::markdown` in `blocks.rs`/`tool.rs`) already sets `.text_size(px(theme::agent_chat_font_size(cx)))`, so a fenced code block (assistant-authored snippet, Read's syntax-highlighted output, …) now scales with the user's agent-chat font size instead of staying pinned at a different, fixed size. `.font_family(cx.theme().mono_font_family.clone())` is kept — the monospace face is still correct, only the size was the bug. |
| Popover trigger consumes its press | `src/popover.rs` (`cx.stop_propagation()` after `toggle_open` in the trigger's `on_mouse_down`) | opening a popover moves keyboard focus to it; the unconsumed press then bubbled to daruda's pane wrapper, which reads any left press in a pane as "activate this pane" (`focus_pane_on_click` → `focus_pane`: surfaces the bottom dock, moves focus there, and lazily connects an idle agent-chat session). Bubble is innermost-first, so the panel took focus and the pane took it straight back — panel open but deaf to `Escape`, tab group unreachable, and a view-settings click on a dormant pane spun up an agent process. Reaches every `.dropdown_menu()` too, since `DropdownMenuPopover` wraps `Popover`. Same distinction `set_menu_target_pane` already documents for right-click. Paired tests in `crate::ui::popover::tests`. |
| Selection start asks a hitbox | `src/text/text_view.rs` (`PrepaintState` `()` → `Hitbox`; `prepaint` inserts one; the `MouseDownEvent` guard adds `hitbox.is_hovered(window)`) | `occlude()` — how `Popover` makes its panel modal to the mouse — works *only* by suppressing `Hitbox::is_hovered` for hitboxes behind it (gpui `HitboxBehavior::BlockMouse`: "for mouse handlers **that check those hitboxes**"). It does not stop propagation. This handler asked no hitbox, so every left press inside an agent-chat options popover *also* started a drag-selection in the transcript block the panel covered, registered it as the global active selection, and extended it as the pointer crossed the panel's own controls. Only the press that **starts** a drag is gated: the move/up handlers are reachable only through a press this gate approved, and gating them would break dragging a selection outside the block (the autoscroll extends it past the viewport) and could strand `is_selecting` if the modality flipped to keyboard mid-drag. Safe against `is_hovered`'s own keyboard-modality suppression because `Window::dispatch_event` sets the modality to Mouse *from a `MouseDown`, before dispatching it*. Paired tests in `crate::ui::markdown::tests` (`a_press_on_selectable_prose_grabs_it` / `a_press_inside_an_occluding_panel_does_not_grab_the_prose_under_it`) fail from either side — remove the gate and the second fails, over-apply it and the first does. |
| Link navigation needs a press that agreed with it | `src/text/inline.rs` (`InlineState::pressed_link` + a hitbox-gated `MouseDownEvent` recorder; the `MouseUpEvent` handler requires the release to resolve to the same `LinkMark`) | the handler fired on mouse-*up* alone, gated only on `bounds.contains`, which gave it two holes: a release inside an `occlude()`d popover over a link underneath reached `cx.open_url`, and — with no overlay involved — a drag begun off the block (the pane behind, or a popover's own padding) and ended over a link navigated, though the user never pressed it. zed's `crates/markdown/src/markdown.rs` is the reference: it records `pressed_link` on a hitbox-gated mouse-down and its mouse-up is not hitbox-gated at all, since the press already established intent. Tests: `a_click_on_a_link_opens_it` / `a_click_inside_an_occluding_panel_does_not_open_the_link_under_it` / `a_drag_that_merely_ends_on_a_link_does_not_open_it`; the two gates fail independently under mutation. |
| Mixed-state checkbox | `src/checkbox.rs` (`Checkbox::indeterminate` builder, `mark_shown` / `next_checked` free fns, `checkbox_check_icon` gains an `indeterminate` param and renders `IconName::Minus`), `src/radio.rs` (its one call passes `false`) | the agent-chat display filter lists five tool categories under one `Tool calls` parent; a partial selection is neither checked nor unchecked, and upstream had no third state, so the parent had to lie in one direction. Not reachable from app code — `checkbox_check_icon` is `pub(crate)` and the border/fill decision is inside `Checkbox::render`. The fade gate keys on `mark_shown`, not `checked`: with a third state, all-checked → mixed reads as mark → no-mark, so the freshly shown minus animated to `opacity(0)` and snapped back. Inline tests cover both free fns. |
| Expose active text selection | `src/global_state.rs` (`GlobalState::selecting_state: Option<Entity<TextViewState>>`), `src/text/text_view.rs` (paint mouse handlers set/unset the field; `pub struct TextSelectionHandle` + `pub fn active_text_selection(cx) -> Option<TextSelectionHandle>`) | Per-block selection lives in the private `TextViewState`; the host had no way to read/extend a live drag-selection. The selectable block's paint mouse-down handler registers its `Entity<TextViewState>` into `GlobalState.selecting_state` right after `start_selection` (the handle only exists in the handler — the state's own methods take `&mut self`), and mouse-up (`end_selection`) / outside-clear (`clear_selection`) null it. `active_text_selection(cx)` returns an opaque `TextSelectionHandle` wrapping that entity with `block_bounds` / `is_selecting` / `extend_to(pos)` (reuses `update_selection` — no duplicated `pos - bounds.origin` math) / `clear` (clears selection **and** deregisters). Drives agent-chat auto-scroll: a poll driver reads the live selection and extends it into newly-revealed text while dragging past the viewport edge. |
| `gpui` test-support dev-dep | `Cargo.toml` (`[dev-dependencies] gpui = { workspace = true, features = ["test-support"] }`) | the lib-test target needs `#[gpui::test]` / `TestAppContext`, which require gpui's `test-support` feature; without it `tree.rs` (and any App-context test) fails to compile and takes the whole `cargo test -p gpui_component` suite down. Dev-only — never in the shipping binary. Fixes the pre-existing `tree.rs` compile failure noted in the sibling render-patch rows and unblocks App-context unit tests (e.g. the active-text-selection test). |
| Per-instance code-editor surface override | `src/input/input.rs` (`CodeEditorSurface`, `Input::code_editor_surface` field + `.code_editor_surface(surface)` builder, with `.editor_background(color)` kept as a compatibility shortcut), `src/input/state.rs` (`InputState::code_editor_surface` field + `pub(super) fn editor_background(&self, cx) -> Hsla` accessor falling back to `cx.theme().editor_background()`), `src/input/element.rs` (`TextElement::paint` reads `state.editor_background(cx)` once and reuses it for the ghost-line fill, the ghost-lines-height background quad, and the inline-completion background quad, instead of three separate `cx.theme().editor_background()` reads) | code-editor surfaces that intentionally paint on a host-picked background (the file viewer's terminal-mirrored pane surface, an agent-chat embed's dimmed chat surface) previously always fell back to the single App-wide `editor_background` theme slot, so their editor-owned paints (line-number gutter fill, ghost rows, inline-completion backdrop) mismatched the surrounding pane once that pane stopped using the fixed UI background. `CodeEditorSurface::default()` keeps upstream behaviour. |

Plus `[lints]` in `crates/gpui_component/Cargo.toml` silencing all
upstream clippy warnings (vendored code is not in our lint scope).

### Enabled upstream modules

daruda's vendored copy trims `src/lib.rs` to the modules in use; some
upstream widget modules ship as files but aren't `pub mod`-declared.
These are enabled on demand by adding the `pub mod` line (re-add on a
rev bump if the re-vendor trims them again):

| Module | `pub mod` added | Consumed by |
|---|---|---|
| `progress` | `src/lib.rs` | `crate::ui::progress` → Usage tab gauge bars |
| `group_box` | `src/lib.rs` | `crate::ui::group_box` → Usage tab gauge cards (`.outline()`) + totals (`.normal()`) |
| `chart` + `plot` | `src/lib.rs` | `crate::ui::chart::BarChart` — chart widgets. `chart` pulls in `plot` (axis/grid/scale/shape) as its rendering backend. |
| `text` | `src/lib.rs` (already declared) | `crate::ui::markdown` → agent chat selectable/copyable markdown (`TextView`: markdown render + char drag-select + copy + code highlight). |

The patches live directly in the vendored tree (not in
`patches/<name>.patch`) because `crates/gpui_component/` is an
in-repo copy — `scripts/apply-gpui-patch.sh` only handles the
`cargo` git cache patch for the GPUI IME path. On a `gpui_component`
rev bump, re-apply each patch by re-doing the diff against the
upstream source.
