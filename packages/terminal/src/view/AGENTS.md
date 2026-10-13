# Terminal View Instructions

Terminal rendering, input, IME, viewport, and scrollback contracts.
These rules also apply to callers outside `view/`. Read the matching section
of [Terminal Reference](REFERENCE.md) before changing that behavior.

## Required Reading

| Working on | Read |
|---|---|
| Module placement or helper APIs | [File responsibilities](REFERENCE.md#file-responsibilities), [helper modules](REFERENCE.md#shared-helper-modules--split-by-role) |
| Text shaping, highlights, cursor, or IME placement | [Pixel mapping](REFERENCE.md#render-side-pixel-calculation-rules), including the past-text cursor exception |
| Scrolling, selection, or input dispatch | [Coordinate spaces](REFERENCE.md#coordinate-spaces--grid-row-vs-viewport-row-vs-absolute-screen-row), [dispatch helpers](REFERENCE.md#event-handler-dispatch--route-through-the-single-source-helper-never-re-inline) |
| Shell integration, jump navigation, or search | [OSC 133](REFERENCE.md#osc-133-shell-integration-handling), [jump contracts](REFERENCE.md#jump-navigation-prompt--command), [search](REFERENCE.md#search) |
| Paint order, keybindings, or regression coverage | [Paint order](REFERENCE.md#paint-order-elementmodrspaint), [keybindings](REFERENCE.md#keybindings-modrsensure_key_bindings), [tests](REFERENCE.md#testing-strategy) |

## Terminal Safety Rules

- Never mix byte offsets and grid coordinates. Convert window positions
  through `mouse_position_to_local()`.
- Printable input follows `replace_text_in_range -> commit_text -> PTY`;
  never send it directly from `on_key_down`, which bypasses IME.
- Text/pixel hit testing uses the shaper's reverse mapping, not
  `offset_px / glyph_advance`.
- Ghostty FFI enums are `u16`; range-check before casting.
- Definition sites own literals: escape bytes in `ansi.rs`, codes in
  `vt_codes.rs`, capacities in `vt_limits.rs`, pixels/colors in `theme.rs`,
  labels in `strings/`, and other constants/shortcuts in their named modules.
- `src/ux/theme.rs`'s local `hsla()` accepts degrees (0-360).
  `daruda_ui::theme` and GPUI use fractions (0-1).
  Never call `gpui::hsla` from the terminal theme file.
- See [GPUI safety](../../../ui/AGENTS.md#gpui-render-and-event-safety)
  for re-entry, notify, caching, and raw mouse-button handling.

```text
Output: Shell -> PTY -> stdout_rx -> 16ms batch -> TerminalSession -> ghostty_vt -> GPUI paint
Input:  GPUI -> TerminalInput / IME commit -> stdin_tx -> PTY -> Shell
```

## Shaping and Coordinate Rules

- Column-based overlays use `text_metrics::shaped_pixel_range_for_cols`
  through `byte_index_for_column_in_line` and `ShapedLine::x_for_index`.
  Background RLE quads are the cell-width arithmetic exception.
- Wide rows deliberately use `force_width = None`. Do not change that until
  the shaper advances two cells per wide glyph.
- Cursor/preedit anchors past trimmed text use `cursor_x_for_col` /
  `cell_left_x_for_col`, which bridge through `shaped_x_for_col`.
  Bare `x_for_index(byte.min(len))` loses trailing blank cells.
- Overlays with a different font size use `cell_metrics_at` for that font
  and size; inherited terminal metrics give incorrect hit testing.
- Grid rows are 1-based; viewport rows and absolute visual rows are 0-based.
  Logical-line `abs_y` is a separate, wrap-invariant space.
- When scrolled, viewport row is not grid row minus one. Keep all
  `dump_viewport*` readers' scroll-offset dispatch in sync.
- Grid-anchored overlays go through `grid_row_to_screen_row`, then
  `screen_row_to_visible`. Skip off-screen anchors, never clamp them.
  Clear cached viewport-relative hover coordinates when the viewport shifts.

## Input and Scroll Dispatch

- Reuse `reanchor_and_refresh` for scroll/settle, `settle_after_scroll`
  for relative-scroll keys, `handle_scroll_key` for shared key dispatch,
  and `send_key_bytes` for flush/snap/send/stop. Extend the owning helper
  instead of copying its sequence; a caller's `return` stays at the caller.
- `snap_to_bottom*` is only for force-to-bottom intent, not relative scroll.
- Selection-drag autoscroll may split re-anchor and refresh because it updates
  the selection endpoint between them; this is the only sanctioned split.

## Shell Marks, Navigation, and Search

- CSI and OSC scanners drain only the prefix both have consumed:
  `min(csi_consumed_upto, consumed_upto)`. Preserve cross-chunk sequences;
  rescanning emits duplicate OSC 133 marks.
- Marks store logical-line `abs_y`, not visual rows; project through
  `abs_to_screen_row` and honor eviction returning `None`.
  Keep the bounded FIFO and do not mix mark anchors with viewport-pin anchors.
- Jump focus stores monotonic `PromptMark::seq`, never indices or screen rows.
  Preserve the fallback, wrap, and alignment contracts in the reference.
  Manual scroll or PTY input intentionally clears jump focus.
- Search matches use absolute screen rows and 1-based inclusive columns.
  Preserve focus by `(row, start_col)`, not row alone; reset on query changes.
  Invalid regex sets the error state; zero-width matches are skipped.
- While the search overlay is active, printable commits update its query,
  not the PTY. Preserve its key context and the documented paint order.

## Field-access rules

- `TerminalView` fields are `pub(super)` — accessible only within `view/`.
- External crates (app) use the `pub` methods only:
  `new`, `new_with_input`, `feed_output_bytes`, `queue_output_bytes`,
  `resize_terminal`, `set_search_query`, `search_step`, `clear_search`,
  `jump_to_prompt`, `jump_to_command`, `search_state`,
  `set_dim_amount` / `dim_amount` (inactive-pane dim; see `layout.rs`),
  `has_selection`, `copy_selection`.

---

## Paint-Scope Metrics

Paint-scope state must not leak into events. In this crate, **never call
`window.text_style()` or `window.rem_size()` directly in `view/`.**
Two places legitimately do, and only those:

| File | Why it's allowed |
|---|---|
| `text_metrics.rs::cell_metrics_at` (and siblings) | the primitive that consults the GPUI text system once per measurement |
| `element/prepaint.rs::prepaint` | runs inside the paint walk, where `window.text_style()` returns the real per-pane font |

Every other site — mouse, IME, resize, keyboard, viewport refresh —
goes through `TerminalView::cell_layout(window)`. That method reads
`state.font` / `state.font_size` / `state.{vertical,horizontal}_spacing`
and feeds them into `cell_metrics_at`, so paint and events both see
the same numbers.

The CI script `scripts/lint-paint-scope.sh` greps for direct calls
outside the whitelist and fails the build on any new violation. It
is part of the [local checks](../../../../CONTRIBUTING.md#local-checks) and CI.

## State boundary — `TerminalView` vs `TerminalViewState`

`TerminalView` (the GPUI entity) holds only resources that *require*
GPUI:

  * `session: TerminalSession` — VT model
  * `state: TerminalViewState` — shared paint+event data (this file)
  * `input: Option<TerminalInput>` — PTY sender closure
  * `focus_handle: FocusHandle` + three `Subscription` slots
  * `autoscroll_task: Option<gpui::Task<()>>`
  * `line_layouts` / `line_layout_key` — paint-time shape cache

Everything else (font metrics, viewport snapshot, search, selection,
IME, hover, jump anchors, flash deadlines, scrollbar drag state)
lives in `state::TerminalViewState`. The struct carries no GPUI
context types; pure unit tests in `state.rs::tests` exercise the
state machine without `TestAppContext`. Adding a new shared field
goes there, with a getter on `TerminalView` if external callers need
read access.
