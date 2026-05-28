# view/ module layout

TerminalView's GPUI rendering, input handling, and event dispatch.

> This document — like every CLAUDE.md in the repo — is maintained in English.
> Translate any new content into English before committing.

## File responsibilities

### TerminalView · UI layer

| File | Responsibility |
|------|----------------|
| `mod.rs` | `TerminalView` struct, constructors, external API, `Render` impl, IME state, keybinding registration |
| `element/mod.rs` | `TerminalTextElement`, `Element` impl, prepaint / paint, per-section `build_*` methods |
| `viewport.rs` | viewport refresh, dirty-row tracking, scroll deltas, side effects, word / line-range helpers |
| `selection.rs` | `ByteSelection`, `CellAnchor`, `BlockRect`, block-selection logic (GPUI-free pure types) |
| `search_bar.rs` | `SearchBarLayout`, `render_search_bar` UI, `recompute_search_matches` engine integration |
| `jump.rs` | `PromptJump`, `next_prompt_index`, prompt / command jumping, scroll helpers |
| `tests.rs` | unit tests (jumping, selection, block, viewport, IME, font hash, coordinate conversion) |

### Input / events

| File | Responsibility |
|------|----------------|
| `input.rs` | `on_key_down`, `EntityInputHandler` (IME), Hangul jamo re-composition |
| `mouse.rs` | mouse click / drag / scroll, URL hover, SGR mouse reporting |
| `actions.rs` | Copy / Paste / SelectAll / Zoom / Fullscreen + Search / PromptJump / CommandJump actions |
| `jamo.rs` | Hangul jamo composition table + client-side re-composition |

### Domain (GPUI-free pure logic)

| File | Responsibility |
|------|----------------|
| `selection.rs` | selection types + pure block-selection logic (`ByteSelection`, `CellAnchor`, `Side`, `BlockRect`, `block_rect_from_anchors`, `block_copy_text`, `block_selection_quads`, `pixel_to_cell_anchor`) |
| `url.rs` | URL detection (`url_at_byte_index`, `url_range_at_column_in_line`) |
| `search.rs` | literal / regex search, pure engine |
| `bg_merge.rs` | background style-run RLE merge (`merge_bg_runs` → `BgSpan`) |
| `viewport_pin.rs` | absolute-line anchor for viewport top — `ViewportPin`, `AbsLineIndex` |

### Shared helpers (split by role)

| File | Responsibility |
|------|----------------|
| `text_metrics.rs` | cell-size measurement + column / byte / pixel conversion (`cell_metrics`, `cell_metrics_at`, `byte_index_for_column_in_line`, `shaped_pixel_range_for_cols`) |
| `text_edit.rs` | UTF-8 char-boundary-safe string editing (`step_char_left/right`, `nth_char_byte`, `clamp_to_char_boundary`) |
| `style.rs` | cell-style flags + TextRun + colors (`CELL_STYLE_FLAG_*`, `TextRunKey`, `font_for_flags`, `color_for_key`, `text_run_for_key`, `hsla_from_rgb`, `cursor_color_for_background`) |
| `box_drawing.rs` | Unicode box-drawing glyphs (`BOX_DIR_*`, `box_drawing_mask`, `line_has_box_drawing`, `box_drawing_quads_for_char`) |
| `overlay.rs` | shared viewport-overlay helpers (`screen_row_to_visible`, `flash_overlay_if_active`) |

### External constant modules (outside this directory)

Do not write magic numbers, colors, or UI text inline inside `view/` — go
through the modules below:

| Target | Module |
|--------|--------|
| Escape bytes + reply builders | `crate::ansi` (`PTY_BRACKETED_PASTE_START`, `cpr_reply`, `osc`) |
| CSI / OSC / FTCS parameter codes | `crate::vt_codes` (`CsiMode`, `OSC_FTCS`, `FtcsCommand`) |
| Parser buffer capacities | `crate::vt_limits` (`PROMPT_MARKS_CAP`, etc.) |
| Overlay text, key-context names, timing `Duration`s | `crate::ux::strings` (`SEARCH_NO_MATCHES`, `PROMPT_JUMP_FLASH`) |
| `Hsla` colors, pixel metrics | `crate::ux::theme` (`SEARCH_PANEL_BG`, `SEARCH_PANEL_RADIUS`) |

Exceptions: the `cell_width * col` arithmetic in background RLE quads, and
pixel fields kept inside a `struct` such as `SearchBarLayout` (padding /
layout tuning where that struct is the single source of truth). Both fall
under the SSoT rule in `CLAUDE.md § 6`.

## Data flow

```
Key event → input.rs (on_key_down / EntityInputHandler)
          → mod.rs (commit_text / send_input_parts)
          → search overlay active: append to query in search_bar.rs
          → PTY

PTY output → mod.rs (queue_output_bytes / feed_output_bytes)
           → viewport.rs (feed_output_bytes_to_session / reconcile_dirty)
           → TerminalSession (ghostty_vt) + OSC 133 parse_tail scan
           → element/mod.rs (prepaint / paint → Metal rendering)

Cmd+Shift+↑↓ → actions.rs → jump.rs (next_prompt_index → scroll)
Cmd+F        → actions.rs → search_bar.rs (render_search_bar / recompute_matches)
```

## Field-access rules

- `TerminalView` fields are `pub(super)` — accessible only within `view/`.
- External crates (app) use the `pub` methods only:
  `new`, `new_with_input`, `feed_output_bytes`, `queue_output_bytes`,
  `resize_terminal`, `set_search_query`, `search_step`, `clear_search`,
  `jump_to_prompt`, `jump_to_command`, `search_state`.

---

## Render-side pixel calculation rules

**Column → pixel conversion must go through
`text_metrics::shaped_pixel_range_for_cols`.**

- GPUI's `shape_line` does not apply `force_width` when the line contains
  wide characters (CJK / emoji), so `cell_width * col` misses the real
  glyph position by ≥ 1 cell.
- Helper path: `byte_index_for_column_in_line` →
  `ShapedLine::x_for_index`.
- Applies to: search highlight, URL hover underline, every column-based
  overlay.
- Exception: background RLE quads color cells one at a time, so the
  cell-width arithmetic is fine there (switch to the helper if accuracy
  issues appear).

```rust
// ❌ Drifts on lines that contain wide characters
let x = origin.x + px(cell_width * (start_col - 1) as f32);

// ✅ Real glyph boundaries
let (x_start, x_end) = shaped_pixel_range_for_cols(
    shaped, line_text, start_col, end_col
)?;
```

When an overlay uses a font size different from the terminal default (e.g.
the 13 px search bar), always call
`cell_metrics_at(window, font, font_size)`. Calling the default
`cell_metrics` derives `cell_w` from the inherited size, so click-to-cursor
mapping accumulates error as the text grows longer.

---

## Coordinate spaces — grid row vs viewport row vs absolute screen row

Three vertical row spaces coexist and are all bare integers, so the
compiler will **not** catch a mix-up:

| Space | Range | Source |
|-------|-------|--------|
| **Grid row** | `1..=rows` (1-indexed) | `session.cursor_position()`, ghostty's live grid |
| **Viewport row** | `0..rows` | what is painted right now (`viewport_lines[i]`) |
| **Absolute screen row** | `0..total_rows()` | unified `line_buffer` scrollback + grid; what `viewport_row_offset()` and prompt-mark `abs_y` live in |

**The rule:** when the user has scrolled into history (`scroll_offset > 0`)
the viewport shows scrollback, so **grid row `r` is no longer at viewport
row `r - 1`**. Any read addressed by a viewport-relative `row`, and any
paint anchored to a *grid* coordinate, must translate through the
scroll-offset dispatch — never assume `viewport_row == grid_row - 1`.

- **Reading a viewport row's content** → `session.dump_viewport*` already
  dispatch on `scroll_offset` (text, style runs, full dump). Keep the
  three in lockstep — adding a new `*_viewport_row*` reader on
  `TerminalSession` means copying that `if self.scroll_offset == 0 { … }`
  branch, not pass-through to `self.terminal.*` (the cause of the Claude
  Code "input box afterimage": one sibling skipped the branch).
- **Painting a grid-anchored overlay** (cursor, IME preedit) → map the
  grid row to an absolute screen row with
  `overlay::grid_row_to_screen_row(grid_row, total_rows, rows)`, then to a
  visible viewport row with `overlay::screen_row_to_visible(.., viewport_top, rows)`.
  A `None` result means the anchor scrolled out of view — **skip the
  paint**, don't clamp it onto unrelated scrollback. Prompt marks and
  search highlights already follow this via `screen_row_to_visible`.
- **Painting a viewport-anchored overlay** (URL hover underline) → the
  cached `row` *is* a 0-indexed viewport row, so the dispatch lives at
  the cache, not at paint: clear it whenever the viewport shifts.
  `update_hovered_url` only fires on mouse-move, so without that clear a
  keyboard scroll / PTY output would leave the underline painted onto
  whatever happens to occupy that row next. `schedule_viewport_refresh`
  resets `state.hovered_url` for this reason; the next mouse-move
  re-derives it.

---

## Pitfall #8 entry point — `cell_layout` is the only door

Root CLAUDE.md §8 ("paint-scope state must not leak into events")
reduces in this crate to a single rule: **never call
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
is part of the standard CI command (see `crates/app/src/CLAUDE.md`).

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

---

## Shared helper modules — split by role

Each module has a **single responsibility**. Place new helpers in the file
that matches their domain.

### `text_metrics.rs` — measurement / conversion

| Item | Role |
|------|------|
| `cell_metrics(window, font)` | monospace cell width / height at the default font size |
| `cell_metrics_at(window, font, font_size)` | explicit font size — for overlays like the search bar |
| `byte_index_for_column_in_line(line, col)` | 1-indexed column → byte offset (handles CJK width) |
| `shaped_pixel_range_for_cols(shaped, text, start, end)` | column range → pixel range (see alignment rule above) |

### `text_edit.rs` — UTF-8-safe string editing

| Item | Role |
|------|------|
| `step_char_left(text, byte)` | move one char left (clamps to 0) |
| `step_char_right(text, byte)` | move one char right (clamps to len) |
| `nth_char_byte(text, n)` | byte offset of the nth char, len if out of range |
| `clamp_to_char_boundary(text, byte)` | round down to the nearest preceding char boundary |

### `style.rs` — cell style → GPUI render primitives

| Item | Role |
|------|------|
| `CELL_STYLE_FLAG_*` | u8 bitmask (BOLD / ITALIC / UNDERLINE / FAINT / STRIKETHROUGH) |
| `TextRunKey { fg, flags }` | cache key for grouping glyphs by identical style |
| `hsla_from_rgb` | RGB → Hsla |
| `cursor_color_for_background` | cursor color contrasted against the background (light / dark auto) |
| `font_for_flags`, `color_for_key`, `text_run_for_key` | TextRun construction |

### `box_drawing.rs` — Unicode box-drawing glyphs

| Item | Role |
|------|------|
| `BOX_DIR_LEFT/RIGHT/UP/DOWN` | direction bitmask |
| `box_drawing_mask(ch)` | glyph → (direction mask, stroke scale) |
| `line_has_box_drawing(line)` | per-line fast pre-check (Alacritty pattern) |
| `box_drawing_quads_for_char` | box-stroke quad builder |

### `overlay.rs` — shared viewport overlays

| Item | Role |
|------|------|
| `screen_row_to_visible(row, viewport_top, rows)` | absolute screen_row → viewport-relative row, `None` if out of range |
| `flash_overlay_if_active(deadline, build)` | timed overlay builder (shared by bell / wrap flashes) |

### Visibility rules

- `pub`: callable from the outer app (`cell_metrics`,
  `cell_metrics_at`).
- `pub(crate)`: usable from anywhere in the daruda crate
  (`cursor_color_for_background`, `box_drawing_mask`,
  `line_has_box_drawing`, `screen_row_to_visible`,
  `flash_overlay_if_active`, `shaped_pixel_range_for_cols`,
  `byte_index_for_column_in_line`, `step_char_*`, `nth_char_byte`,
  `clamp_to_char_boundary`).
- `pub(super)`: `element/mod.rs` only (style flag / key, font / color
  helpers, `box_drawing_quads_for_char`).

---

## prepaint section methods (`element/mod.rs::TerminalTextElement::build_*`)

The prepaint body reduces to: refresh the ShapedLine cache, call the
section methods, and assemble the returned state.

| Method | Output | Dependencies |
|--------|--------|--------------|
| `build_background_quads(bounds, line_height, cell_width, default_bg, cx)` | `Vec<PaintQuad>` | `viewport_style_runs` → `bg_merge` |
| `build_search_quads(bounds, line_height, shaped_lines, cx)` | `Vec<PaintQuad>` | search state + `shaped_pixel_range` |
| `build_prompt_marks(bounds, line_height, cx)` | `Vec<PaintQuad>` | `session.prompt_marks` + `focused_*_row` |
| `build_hover_underline(bounds, line_height, shaped_lines, run_color, cx)` | `Option<PaintQuad>` | `hovered_url` + `shaped_pixel_range` |

The inline-only prepaint sections (selection, cursor, scrollbar,
box_drawing, marked_text, bell_flash, prompt_jump_flash) rely on state
beyond grid coordinates, so they are not yet factored into methods.
Apply the same `build_*` pattern when a change motivates extraction.

---

## OSC 133 (shell integration) handling

### Parsing flow (`session.rs`)

1. `update_state_from_output` runs two scanners over `parse_tail`:
   - **CSI-mode scanner** (`\x1b[?…h/l`): mouse tracking, bracketed paste.
   - **OSC scanner** (`\x1b]…\x07` / `\x1b]…\x1b\\`): codes 0 / 2 / 7 /
     52 / 133.
2. Each scanner tracks its own `*_consumed_upto`.
3. Drain `drain_to = min(csi_consumed_upto, consumed_upto)` — only the
   prefix **both** scanners have completed is discarded. This preserves
   cross-chunk sequences.
4. OSC 133 payload (A / B / C / D / E / F) → the pure `parse_osc133_payload`
   function → `PromptMark { kind, abs_y, screen_col, exit_code }` pushed
   into a `VecDeque`.

### Invariants

- **Marks store `abs_y`, not a screen row**: captured as
  `line_buffer.overflow() + line_buffer.wrapped_row_count(cols) + (cursor_y - 1)`
  at dispatch time. Translate to a current-frame screen row via
  `TerminalSession::abs_to_screen_row` at read time — returns `None`
  when the row has been evicted from `LineBuffer`, so marks survive ring
  eviction without aliasing onto unrelated rows.
- **`prompt_marks` is a bounded FIFO** (`PROMPT_MARKS_CAP = 4096`).
- **No re-scan**: without the drain, re-scanning `parse_tail` re-emits
  duplicate OSC 133 marks. OSC 7 / 52 are idempotent so they do not
  matter; OSC 133 accumulates into the VecDeque.

### `PromptMarkKind`

| FTCS | Variant | Use |
|------|---------|-----|
| A | `PromptStart` | prompt start; `Cmd+Shift+↑/↓` jump |
| B | `CommandStart` | user input start; no visual marker |
| C | `CommandExecuted` | command execution; `Cmd+Shift+Option+↑/↓` jump |
| D | `CommandFinished` | exit + exit code; non-zero = red band |
| E | `SemanticTextStart` | command output start; drives `last_command_output_rows` |
| F | `SemanticTextEnd` | command output end |

---

## Jump navigation (prompt / command)

### Focus is identity-based (do not use indices, do not use screen rows)

- `focused_prompt: Option<u64>` / `focused_command: Option<u64>` —
  stores the focused mark's [`PromptMark::seq`] (a monotonic push-order
  identity assigned in `push_prompt_mark`).
- Tracking indices breaks on FIFO eviction; tracking a screen row
  breaks on `clear_line_buffer_and_shift_marks` (which shifts
  `abs_y` of viewport-resident marks after a `\x1b[3J` mirror). `seq`
  is the only field that is **never shifted and never resets**, so
  the highlight follows the focused mark across both — mirroring
  iTerm2's `_selectedScreenMark` weak-ref pattern.
- `next_prompt_index(sorted_starts, previous_row, viewport_top, forward)
  -> Option<PromptJump>` stays in screen-row space; `jump_to_mark`
  converts the stored `seq` → current screen row at entry and maps
  the chosen row → mark `seq` at exit.

### `PromptJump { row, wrapped }` contract

- Input: ascending sorted absolute rows, previous focused row,
  `viewport_top`, direction.
- Previous row absent from the list (evicted) → fresh-anchor fallback.
- Fresh forward: first `r >= viewport_top`. If none, first element +
  `wrapped = true`.
- Fresh backward: last `r < viewport_top`. If none, last element +
  `wrapped = true`.
- Step: `(prev + 1) % len` / `(prev + len - 1) % len`. Crossing a boundary
  sets `wrapped = true`.

### Scroll alignment modes (`config::PromptJumpScroll`)

| Mode | Behavior |
|------|----------|
| `AlwaysTop` (default) | align the target mark to the top of the viewport, even if it was already on-screen |
| `LeaveInPlace` | no-op when the target is on-screen; otherwise center it |

### Re-anchor triggers

Manual scroll (wheel, PageUp / PageDown, Home / End) or PTY input →
`schedule_viewport_refresh` → `focused_* = None`. This is a deliberate
UX clear (turn the jump highlight off when the user moves on), not a
staleness workaround — with identity-based focus the mark would
otherwise remain correctly highlighted across content changes.

### Wrap flash

`PromptJump.wrapped == true` → `state.flash.prompt_jump` set for 180 ms →
`flash_overlay_if_active` paints a 3 px band at the top.

---

## Search

### Engine (`search.rs` — GPUI-free)

- `find_literal_matches_from(lines, needle, case, row_offset)` —
  non-overlapping, ASCII-folded case handling.
- `find_regex_matches_from(lines, re, row_offset)` — `regex` crate,
  zero-width match skip.
- `compile_regex(pattern, case)` — `RegexBuilder` wrapper; invalid input
  returns `None` and sets `regex_error = true`.
- `MatchRange { row: u32 (absolute screen), start_col, end_col (1-indexed
  inclusive) }`.

### Scrollback scan

- `recompute_search_matches` → `dump_screen_row(y)` up to 10 000 rows.
- Re-scan on query change or viewport change.
- Matches carry absolute `screen_row`; convert with
  `screen_row_to_visible`.
- In `search_step`, if the focused match is off-screen, recenter with
  `scroll_to_screen_row`.

### Focus preservation (`recompute_search_matches_with(reset_focus)`)

- Query change → `reset_focus = true` → `focused = Some(0)`.
- Viewport refresh → `reset_focus = false` → remember the prior match as a
  `(row, start_col)` pair.
- **Do not match on `row` alone**: that cannot distinguish duplicate
  matches on the same row and snaps focus back to the first match.

### UI (Cmd+F overlay)

- `search_overlay: bool` + `TerminalSearch` key context.
- While active, the `commit_text` branch appends printable keys to the
  query (they do not reach the PTY).
- Top-right banner: `/{query}  {focused + 1}/{total}` /
  `no matches` / `regex error`.

---

## Paint order (`element/mod.rs::paint`)

Z-order — **do not paint in reverse**:

1. Default background (fills with `default_bg`).
2. `background_quads` (RLE-merged cell backgrounds).
3. `prompt_marks` (left-gutter band).
4. `search_quads` (match highlight).
5. `selection_quads` (selected region).
6. `shaped_lines` (text glyphs).
7. `box_drawing_quads` (Unicode box characters).
8. `marked_text_background` + `marked_text` (IME composition).
9. `cursor`.
10. `scrollbar`.
11. `hover_underline` (URL Cmd+hover).
12. `bell_flash` (translucent fill over the viewport).
13. `prompt_jump_flash` (top band on wrap).

---

## Keybindings (`mod.rs::ensure_key_bindings`)

| Key | Action |
|-----|--------|
| `tab` / `shift-tab` | Tab / TabPrev |
| `cmd-=` / `cmd--` / `cmd-0` | ZoomIn / ZoomOut / ResetZoom |
| `cmd-ctrl-f` | ToggleFullscreen |
| `cmd-k` | ClearBuffer (wipe viewport + scrollback) |
| `cmd-shift-k` | ClearScrollback (drop scrollback, keep viewport) |
| `cmd-end` | ScrollToBottom (release pin, snap to bottom) |
| `cmd-f` | SearchOpen |
| `cmd-g` / `cmd-shift-g` | SearchNext / SearchPrev |
| `escape` (search context) | SearchClose |
| `enter` (search context) | SearchNext |
| `shift-enter` (search context) | SearchPrev |
| `backspace` (search context) | SearchBackspace |
| `cmd-shift-up` / `cmd-shift-down` | PromptJumpPrev / PromptJumpNext |
| `cmd-shift-alt-up` / `cmd-shift-alt-down` | CommandJumpPrev / CommandJumpNext |
| `cmd-shift-c` | CopyLastCommandOutput |

After `SearchOpen`, the `TerminalSearch` context is active and the
keybindings above diverge accordingly.

---

## Splitting criteria

**Prefer single responsibility over file size.** If a file gains two or
more independent reasons to change, split it.

Likely future splits:

- **Adding keymap customization to `input.rs`** → `keybindings.rs`.
- **More complex mouse selection** (vi-mode, block selection) →
  `selection.rs`.
- **`jamo.rs` gaining Japanese / Chinese fallback** → `ime_fallback/`
  directory.
- **Inline prepaint sections (selection / cursor / scrollbar / box /
  marked_text) gaining their own change drivers** → extract with the same
  `build_*` pattern, and split into `element/prepaint.rs` if needed.

---

## Testing strategy

- **GPUI-free pure functions** rely on `#[cfg(test)]` module-level unit
  tests (`search.rs`, `bg_merge.rs`, `url.rs`, `jamo.rs`, portions of
  `helpers.rs`).
- **Render paths that need `ShapedLine` / `Window`** cannot be tested
  headlessly — rely on manual smoke tests.
- **Regression tests** ship with bug-fix commits:
  - `next_prompt_index::focus_row_survives_eviction_across_steps`
  - `search::multiple_matches_in_same_line_have_distinct_start_cols`
  - `feed_does_not_duplicate_osc133_when_csi_and_osc_share_a_chunk`
  - `byte_index_for_column_handles_wide_chars`
