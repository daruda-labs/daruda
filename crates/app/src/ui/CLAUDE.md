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
├── button.rs       # button / button_primary / button_danger / button_bare
├── button_group.rs # button_group(id) — segmented single-select strip over gpui_component::ButtonGroup (theme-aware selected styling; `.children(buttons.selected(..))` + `.on_click(|indices|..)`)
├── chart.rs        # BarChart re-export over gpui_component::chart (Plot-backed; caller wraps in a fixed-height container)
├── checkbox.rs     # checkbox(id, label)
├── code_copy_button.rs # code_copy_button(id, code, window, cx) — hover-reveal copy button for rendered-markdown code blocks (✓ feedback + 2s targeted-notify revert); wired globally by markdown.rs, revealed by the node.rs `gpui-code-block` group patch
├── dialog.rs       # Dialog / DialogButtonProps / ButtonVariant / WindowExt re-exports
├── disclosure.rs   # disclosure(id, is_open) — stateless chevron toggle (ChevronDown open / ChevronRight closed; .color()/.size()/.on_toggle()); caller owns fold state
├── group_box.rs    # group_box() factory over gpui_component::GroupBox (.outline()/.fill()/title)
├── highlighter.rs  # LanguageRegistry / LanguageConfig re-export (tree-sitter language data; GPUI-free, used by the file-viewer highlighter)
├── divider.rs      # Divider re-export
├── list.rs         # FilteredItem + FilteredDelegate + searchable_list_state + list(&state)
├── markdown.rs     # markdown(id, text) — rendered, drag-selectable/copyable markdown over gpui_component::text::TextView (RenderOnce; .selectable()/.color()/.text_size()/.full_width())
├── menu.rs         # ContextMenuExt / DropdownMenu / PopupMenu / PopupMenuItem re-exports
├── progress.rs     # progress(value) factory over gpui_component::Progress (Styled fill bar)
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
| Button hover text color | `src/button/button.rs` (`RenderOnce::render`) | replace `text_color(crate::red_400())` with `text_color(hover_style.fg)` in the `.hover(...)` closure — upstream accidentally left a debug red literal instead of the theme foreground color, making all button text turn red on hover. |
| PopupMenu small size | `src/menu/popup_menu.rs` (`PopupMenu::small` + `render_menu_item`) | change `pub(crate)` to `pub`; also wire `Size::Small` into the font — `text_xs` for small, `text_sm` otherwise (upstream only shrank item height to 20 px but left font at `text_sm`). |
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
| `is_disabled` accessor | `src/input/state.rs` (`pub fn is_disabled(&self) -> bool`) | reads back the state's disabled flag. `Input::render` unconditionally writes `state.disabled = self.disabled` every frame, so an `Input` element built without `.disabled(true)` clobbers a `set_disabled(true)` the host applied to the state (read-only diff/file viewers) on the next paint. `crate::ui::file_viewer_editor` forwards this value into `.disabled(...)` so the reconciliation is a no-op and read-only mode survives. |
| `Input::scroll_wheel` policy | `src/input/input.rs` (`pub enum ScrollWheelBehavior { Both, Horizontal, Vertical }` + `Input::scroll_wheel` field/builder), `src/input/state.rs` (`scroll_wheel` field + `on_scroll_wheel` dispatch) | which axes the editor scrolls on the wheel; the unhandled axis bubbles to an outer scroller. Default `Both` (upstream: scroll both axes, consume on offset change). `Horizontal` scrolls/consumes only horizontal-dominant gestures (`|dx| > |dy|`) and bubbles vertical ones; `Vertical` is the mirror. The agent-chat diff embed (`render/diff.rs`, sized to `rows × row_h` inside the chat list) uses `Horizontal`: a vertical swipe scrolls the transcript, a horizontal swipe scrolls the long non-wrapped diff line. Modelled as an enum (not a bool pair) so the axis policies stay mutually exclusive. |
| Empty custom gutter reserves no width | `src/input/element.rs` (`layout_line_numbers` `has_gutter_content` gate) | when custom `line_decorations` are installed but every gutter string is empty (the diff viewer hiding snippet-relative line numbers), `line_number_width` is `0` instead of `content + 6px + LINE_NUMBER_RIGHT_MARGIN` — otherwise an all-blank gutter reads as an unexplained ~16px indent. Sequential numbering (no decorations) and any non-empty custom gutter keep their width. The row-background decorations still paint (they are gated on `line_number` mode being on, which the diff editor keeps), so hiding numbers does not drop the add/del tint. |
| TextView list-item wrap | `src/text/node.rs` (`Node::ListItem` render) | wrap the list-item content div in `flex_1().min_w_0()` (was a bare `div().overflow_hidden()`) so item text wraps to the available width instead of laying out at its intrinsic max-content width and clipping the wrapped continuation. Without it, agent-chat markdown list items (`crate::ui::markdown`) overflow the pane — no wrap, second line cut off — until the pane is narrowed. |
| TextView list-item block children | `src/text/node.rs` (`ListItemChildLayout` enum + `classify_list_item_children` + `render_list_item`) | `render_list_item`'s child loop only matched `Paragraph` / `List` and dropped every other block on a `_ => {}` arm, so a fenced **code block** (or blockquote / table / heading / rule) nested inside a list item silently vanished — and the paragraph after it wrongly merged into the previous line (the old `last_not_list` merge only excluded `List`, not code blocks). Now a pure `classify_list_item_children` maps each child to `{ LeadParagraph, MergedParagraph, ContinuationParagraph, NestedList, Block }`: consecutive paragraphs still merge, a paragraph after a nested list keeps its prefixed leading line, and any other block — plus a paragraph following one — renders as an indented `div().ml(rems(1.)).min_w_0()` continuation instead of being dropped (the `min_w_0` overrides the flex default `min-width: auto` so the continuation shrinks to the list width instead of laying out at intrinsic max-content width and clipping at wider panes — same width-dependent class as the lead-paragraph / table-cell patches). Repro: an agent-chat reply with a numbered/bulleted item whose body contains an indented code fence (e.g. a code-review "add a `catch`:" step). The classifier is unit-tested (`text::node::tests`), but the crate's lib-test target does not build in-repo (pre-existing: `tree.rs` `#[gpui::test]` lacks `gpui/test-support`; `dock/state.rs` `include_str!`s a trimmed fixture) — verify visually, as with the sibling render patches. |
| TextView table-cell wrap | `src/text/node.rs` (`render_table` cell div) | drop `.truncate()` (which sets `white-space: nowrap` + ellipsis + `overflow_hidden`, forcing every cell to a single clipped line) and instead add `.min_w_0()` on the cell + wrap the cell text in a `div().min_w_0().overflow_hidden()`. `min_w_0` on the cell lets it shrink to its proportional `relative(len)` width inside the `flex_row` (otherwise cells hold their min-content size, the row overflows, and no cell reaches a width the text can wrap into); the `min_w_0` inner div then shrinks below its content so the text wraps, while the cell's `justify_*` (column align) still positions text that's narrower than the cell. Same class as the list-item wrap fix — agent-chat markdown table cells (`crate::ui::markdown`) laid out at intrinsic max-content width and truncated instead of wrapping. |
| TextView code-block render override | `src/text/{text_view.rs,node.rs}` | `code_block_render` hook (mirrors `code_block_actions`) lets the host replace a code block's rendering with a custom element; used by the agent chat to render ` ```mermaid ` blocks as a diagram. The closure receives the `CodeBlock` (with `lang()` / `code()`) and returns `Some(element)` to override or `None` to keep default. `None`/absent = default rendering. |
| Inline-code + code-block background = background-derived tint | `src/text/node.rs` (`style.code` inline highlight; `codeblock` div fill + border) | inline code (`` `code` ``) and fenced code blocks fill with a translucent neutral tint picked by `cx.theme().background.l < 0.5` (white over dark, black over light) instead of the upstream `accent` (inline) / `muted` + `border` (block). Inline fill = `0.08`; block fill = `0.05`, block border = `0.28` (the shared structural-line tint, matching the table + `<hr>` rule). daruda's `accent` is a scarce chromatic signal (active lane / focus ring / primary CTA / Claude badge — DESIGN.md §Accent scarcity, "never a panel fill") and read as noise repeated across spans; a translucent tint sits one step off the background on any theme and lets the pane opacity show through, mirroring the host's `theme::agent_chat_tint` / `agent_chat_border_tint` used for tool cards. |
| Table / divider / blockquote lines = background-derived tint | `src/text/node.rs` (`render_table` `line_color`; `Node::Divider` `rule_color`; `Node::Blockquote` border) | structural markdown lines used fixed theme colors near-invisible against the agent-chat pane's mirrored terminal background: **table** frame/row/cell separators + the **horizontal rule** (`---`) used `cx.theme().border` (daruda's `HAIRLINE`, L≈0.15); the **blockquote** left bar used `cx.theme().secondary_active` (daruda's canvas, L≈0.03, effectively black). Table and divider now pick a translucent neutral (`hsla(0,0,1,0.28)` over dark, `hsla(0,0,0,0.28)` over light) via `cx.theme().background.l < 0.5` — the shared structural-line tint, matching the code-block border (`0.28` alpha); brighter than the `0.12` originally used, since the fainter hairline read as invisible next to near-white chat text. The blockquote bar now uses `cx.theme().muted_foreground` — the same muted gray as the quote's own text, so it reads clearly and coherently. |
| Shared word-boundary + line-range module | `src/text_selection.rs` (new module, `pub mod text_selection` in `src/lib.rs`) | `pub enum CharType { Word, Whitespace, Newline, Other }` + `pub fn word_range(len, char_at, offset) -> Option<Range<usize>>` + `pub fn logical_line_range(len, char_at, offset) -> Range<usize>` — generic byte-offset word-boundary and logical-line-range logic shared by the input widget. Both take a `Fn(usize) -> Option<char>` closure, so `RopeExt::char_at` and `&str` both satisfy it without any ropey import in the shared module. |
| `input/selection.rs` delegation | `src/input/selection.rs` | removed local `CharType` + old walk loop; `TextSelector::word_range` now delegates to `crate::text_selection::word_range(text.len(), \|i\| text.char_at(i), offset)`. Tests updated to `use crate::text_selection::CharType`. |
| Triple/quad-click + mode-aware drag | `src/input/state.rs` (`SelectMode` enum + `select_mode`/`select_anchor` fields + `on_mouse_down` + `on_drag_move` + helpers `word_range_at`/`line_range_at`) | `SelectMode { Character, Word(Range), Line(Range), All }` drives drag extension granularity. Triple-click selects the logical (newline-delimited) line and sets `SelectMode::Line`; quad-click selects all and sets `SelectMode::All`. Double-click (existing) now also sets `SelectMode::Word(anchor)`. Drag-move dispatches on `select_mode`: `Character` → `select_to`, `All` → no-op (keeps full selection, never shrinks on drag), Word → union of anchor and word-at-cursor, Line → union of anchor and line-at-cursor. Single-click resets to `Character`. `line_range_at` delegates to `crate::text_selection::logical_line_range`. |
| `indoc` dev-dependency | `Cargo.toml` (`[dev-dependencies] indoc = "2"`) | existing `selection.rs` tests referenced `indoc::indoc!` without declaring the dep — a pre-existing test compile failure. Adding it as a dev-dependency fixes the test suite for `gpui_component` (the remaining test failures in `tree.rs` are a separate pre-existing upstream issue). |
| TextView word/line/all click selection | `src/text/text_view.rs` (`pub enum SelectMode { Character, Word, Line, All }` — unit variants; `pub fn select_mode_for_click_count(n) -> SelectMode` pure fn; `start_selection(pos, click_count)` calls it; `select_mode` field + `has_selection` guard + `select_mode()` getter), `src/text/inline.rs` (`layout_selections` mode-aware expansion) | Double-click (count=2) → `SelectMode::Word`: `layout_selections` expands the raw pixel-hit byte range to full word boundaries via `text_selection::word_range`. Triple-click (count=3) → `SelectMode::Line`: expands to the full visual (wrapped) line by scanning all chars whose Y position overlaps the raw selection's Y band. Quad-click (count≥4) → `SelectMode::All`: selects the entire Inline text. Character (count=1) → unchanged drag behaviour. `has_selection` returns `true` for Word/Line/All even when start==end (no drag) so the expansion fires on a bare click. `SelectMode` and `select_mode_for_click_count` are `pub` and re-exported from `crate::ui`. |
| Click-point char hit (degenerate selection box) | `src/text_selection.rs` (`pub fn char_cell_hit_x`), `src/text/inline.rs` (`char_in_text_selection` delegates to it) | A word/line click produces a zero-width selection box (start==end); the old center-threshold test matched no char, so nothing selected (only quad-click, which bypasses the scan, worked). `char_cell_hit_x` treats a degenerate span (`sel_left == sel_right`) as "cell contains the click point" and keeps the center threshold for real drags. Pure fn, re-exported from `crate::ui`, tested in the app crate. |
| Drag selection is a document range, not a rectangle | `src/text/text_view.rs` (`TextViewState::selection_span`), `src/text/inline.rs` (`endpoint_cut_x` + `char_in_text_selection` replace `point_in_text_selection`; `layout_selections` pass 1 consumes the span) | Hit-testing read the drag as a normalized `Bounds`, which loses which x belongs to the upper endpoint and which to the lower one — so it only behaved for a drag whose lower end was also further right. Drag from line 1 down to line 2 and then left past the anchor's x and the rect inverted: line 1 grew leftwards from the anchor and line 2 grew rightwards instead of shrinking. Its `single_line` test (box height `<= line_height`) compounded this: a drag crossing a band boundary by a few px bounded *both* lines by the normalized x span. (Pass 1 keeps only the first and last hit, so a purely rightward drag ends up with the same contiguous envelope either way — the visible break is the leftward case above.) Now each endpoint is reduced to a cut on the character's own line band (`-inf` above / `+inf` below / its x when on it) and the two cuts bound that line, so document order — not the drag's shape — decides each line's extent. Lines outside the drag collapse to two equal infinities, which `char_cell_hit_x`'s degenerate branch rejects, so no separate vertical test is needed. `selection_bounds` stays for the (commented-out) debug painter. Band width is `text_layout.line_height()` — the row pitch `position_for_index` itself uses, and the one `paint_selection` paints with; `window.line_height()` is a separate rounding of the same value and can tile the bands with overlap. Unit-tested in `inline.rs`; the bug cases fail against the old model. On a rev bump the upstream `point_in_text_selection` + its `test_point_in_text_selection` must be removed again. |
| Plain-text TextView (`TextViewType::Text`) | `src/text/text_view.rs` (`TextViewType::Text` variant, `TextView::plain(id, text, window, cx)` constructor, `parse_content` arm) | Renders the raw string verbatim as a single paragraph (no Markdown/HTML interpretation) through the same selectable Inline path — a selectable primitive for non-markdown text (command output, logs, titles). Mirrors zed's `Markdown::new_text` (`parse_links_only`) role. Wrapped as `crate::ui::selectable_text`. |
| Multi-byte word/line selection | `src/text/inline.rs` (`layout_selections` char-width via `offset + c.len_utf8()`, Word/Line expansion via `text_selection::floor_char_boundary`), `src/text_selection.rs` (`floor_char_boundary`/`ceil_char_boundary` helpers; `CharType::from_char` uses `is_alphanumeric`) | Word/line selection was ASCII-only: `offset + 1` / `raw_end - 1` split 3-byte Hangul/CJK glyphs (cell width collapsed, `word_range` bailed at a non-boundary), and `is_ascii_alphanumeric` classified Hangul/CJK as `Other` (non-connectable). Now the byte steps round to char boundaries and any-script letters are `Word`, so double-click selects Hangul/CJK words. `floor_char_boundary`/`ceil_char_boundary` are the canonical helpers to reach for instead of `offset ± 1`; re-exported from `crate::ui`. |
| Selection survives streaming | `src/text/text_view.rs` (`update_bounds` clears only on **width** change; reparse no longer calls `clear_selection`) | A streaming markdown block reparses and grows in height on every chunk; both the unconditional reparse-clear and the any-size-change clear dropped the user's selection mid-stream. Since a block's text only appends (stable layout), keep the pixel selection across reparse and height growth; only a width change (real reflow) clears. |
| Code-block copy-button hover reveal | `src/text/node.rs` (`CodeBlock::render`) | add `.group("gpui-code-block")` to the code-block container div and gate the actions overlay with `.invisible().group_hover("gpui-code-block", \|s\| s.visible())`, so the host's `crate::ui::code_copy_button` (wired for all markdown via `crate::ui::markdown`) is revealed only on hover and an idle block shows no persistent action chip. |
| Streaming re-parse debounce 200ms → 33ms | `src/text/text_view.rs` (`UpdateFuture` delay arg in the `request_layout` init path) | Upstream's 200ms trailing debounce makes streamed agent-chat markdown land in ~200ms steps ("chunky"); 33ms (~30Hz) lets it flow near-continuously like zed's eager reparse (`markdown.rs` `parse` has no fixed delay, only `pending_parse`/`should_reparse` coalescing). Parsing is off the main thread, so a lower value only raises background-parse frequency; static content parses once and is unaffected. Sole `Duration::from_millis` in the file — tune there. |
| Code-block font size inherits ambient | `src/text/node.rs` (`CodeBlock::render`) | drop `.text_size(cx.theme().mono_font_size)` (a fixed 13px theme field the host never wires up) so the code-block div inherits the ambient text size from `TextView`'s outer `.refine_style(&self.style)` (`text_view.rs:800`) instead — the same class of fix as the "Tab font-size decoupling" patch above. Every daruda call site (`crate::ui::markdown` in `blocks.rs`/`tool.rs`) already sets `.text_size(px(theme::agent_chat_font_size(cx)))`, so a fenced code block (assistant-authored snippet, Read's syntax-highlighted output, …) now scales with the user's agent-chat font size instead of staying pinned at a different, fixed size. `.font_family(cx.theme().mono_font_family.clone())` is kept — the monospace face is still correct, only the size was the bug. |
| Expose active text selection | `src/global_state.rs` (`GlobalState::selecting_state: Option<Entity<TextViewState>>`), `src/text/text_view.rs` (paint mouse handlers set/unset the field; `pub struct TextSelectionHandle` + `pub fn active_text_selection(cx) -> Option<TextSelectionHandle>`) | Per-block selection lives in the private `TextViewState`; the host had no way to read/extend a live drag-selection. The selectable block's paint mouse-down handler registers its `Entity<TextViewState>` into `GlobalState.selecting_state` right after `start_selection` (the handle only exists in the handler — the state's own methods take `&mut self`), and mouse-up (`end_selection`) / outside-clear (`clear_selection`) null it. `active_text_selection(cx)` returns an opaque `TextSelectionHandle` wrapping that entity with `block_bounds` / `is_selecting` / `extend_to(pos)` (reuses `update_selection` — no duplicated `pos - bounds.origin` math) / `clear` (clears selection **and** deregisters). Drives agent-chat auto-scroll: a poll driver reads the live selection and extends it into newly-revealed text while dragging past the viewport edge. |
| `gpui` test-support dev-dep | `Cargo.toml` (`[dev-dependencies] gpui = { workspace = true, features = ["test-support"] }`) | the lib-test target needs `#[gpui::test]` / `TestAppContext`, which require gpui's `test-support` feature; without it `tree.rs` (and any App-context test) fails to compile and takes the whole `cargo test -p gpui_component` suite down. Dev-only — never in the shipping binary. Fixes the pre-existing `tree.rs` compile failure noted in the sibling render-patch rows and unblocks App-context unit tests (e.g. the active-text-selection test). |

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
| `text_selection` | `src/lib.rs` (`pub mod text_selection`) | shared word-boundary primitives (`CharType` + `word_range`) — no GPUI dependency, usable from any `gpui_component` consumer. |

The patches live directly in the vendored tree (not in
`patches/<name>.patch`) because `crates/gpui_component/` is an
in-repo copy — `scripts/apply-gpui-patch.sh` only handles the
`cargo` git cache patch for the GPUI IME path. On a `gpui_component`
rev bump, re-apply each patch by re-doing the diff against the
upstream source.
