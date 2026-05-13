# render_file_viewer/ module layout

Pane-area file viewer renderers. Replaces the terminal area inside the
focused pane while a file or diff is open; the underlying PTY keeps
running because removing the view from the render tree does not affect
Entity lifetime — `pane_file_view::PaneFileView` retains the state.

The only public entry point is
`pub(in crate::workspace) fn render_pane_file_viewer`. Every other
helper is `pub(super)` (visible across this module) or private to its
file.

## File responsibilities

| File | Responsibility |
|------|----------------|
| `mod.rs` | `render_pane_file_viewer` top-level layout (toolbar + body + scrollbar + search panel + hint bar) and sub-module declarations |
| `toolbar.rs` | `render_file_viewer_toolbar` (path label + Raw/Preview/Changes mode tabs + diff stats + status badge + close button); `mode_tab` pill helper |
| `search_panel.rs` | `render_search_panel` — floating top-right overlay with a `gpui_component::Input` (via `crate::ui::input`), prev/next/clear/close controls, match counter |
| `scrollbar.rs` | `file_viewer_scrollbar` — visual scroll thumb only (no drag-to-scroll). Sized from `content_h`: Raw/Changes use `LINE_H * total_rows`, Preview uses `viewport_h + max_offset` |
| `body.rs` | `render_file_viewer_body` (kind-routing) + private `render_raw_body`, `render_diff_body`, `search_row_bg`, `footer_row` |
| `virtual_list.rs` | `virtual_range` (pure `(scroll_y, viewport_h, line_h, overscan) → (start, end)`) + inline tests for the math |
| `content_element.rs` | `FileViewerContentElement` — custom GPUI `Element` that shapes text in `prepaint` so the pixel↔byte mapping is exact, then wires `MouseDown`/`MouseMove` in `paint` to drive char-level `CharSelection`. The viewer's only piece of paint-walk state |
| `row.rs` | `diff_visual_row` (hunk header, non-selectable), `diff_selectable_row` (Context/Added/Removed/NoNewline; defers content cell to `FileViewerContentElement`), private `row_style` |
| `markdown.rs` | `render_md_body` + private `render_md_block`/`_spans`/`_span` recursive renderers; `is_block_selected` and `block_with_selection` (block-level selection driven through `Workspace.pane_file_view.char_*`) |

## Call graph

```
render_pane_file_viewer (mod.rs)
├── render_file_viewer_toolbar    (toolbar.rs)
├── render_file_viewer_body       (body.rs)
│    ├── render_raw_body          (body.rs, virtual list)
│    │    └── FileViewerContentElement::new   (content_element.rs)
│    ├── render_diff_body         (body.rs, virtual list)
│    │    ├── diff_visual_row     (row.rs)
│    │    └── diff_selectable_row (row.rs)
│    │         └── FileViewerContentElement::new
│    └── render_md_body           (markdown.rs)
│         ├── render_md_block     (markdown.rs, recursive)
│         └── block_with_selection (markdown.rs)
├── render_search_panel           (search_panel.rs)
└── file_viewer_scrollbar         (scrollbar.rs)
```

`virtual_range` (virtual_list.rs) is reused by both raw and diff body
paths to compute the `[start, end)` window. Off-screen rows live in
top/bottom spacer divs so `overflow_y_scroll` reports the right total
height.

`PaneFileContent::LoadedMarkdown` has two render paths: Raw mode goes
through `render_raw_body` (using the parser-cached `raw_rows`); Preview
mode through `render_md_body`. Both share `char_selection` semantics
through `is_block_selected` + the byte-range row helpers.

## How to add or modify

| Change | Touch |
|--------|-------|
| New mode tab in the toolbar | `toolbar.rs` — extend the chip group + add a `set_file_view_mode` arm in `pane_file_view.rs` |
| New control on the search panel | `search_panel.rs` — append a `.child(...)` with the matching `cx.listener` action; methods on `FileViewerSearch` live in `pane_file_view.rs` |
| New `PaneFileContent` kind | `body.rs::render_file_viewer_body` (route the new variant) + a renderer in `body.rs` (if list-shaped) or a sibling file (if the rendering shape differs significantly, e.g. Markdown) |
| New diff row visual | `row.rs` — add a builder; if it needs char selection, defer the content cell to `FileViewerContentElement` |
| New Markdown block / span | `markdown.rs::render_md_block` / `render_md_span` — extend the match. Keep recursive cases routing back through `render_md_spans` for span trees |
| Tweak the virtual-list window | `virtual_list.rs::virtual_range` — add the new test case alongside the change |
| New scrollbar behaviour (drag, mini-map) | `scrollbar.rs` — keep the math driven by the same `content_h` so Preview and Raw stay accurate |

## Conventions

- **No public API beyond `render_pane_file_viewer`.** Add helpers as
  `pub(super)` and import them through `self::<file>::*` in `mod.rs`
  when the entry point needs them.
- **`FileViewerContentElement` is the only paint-walk state in this
  module.** Char-level selection on shaped wide-glyph rows requires a
  custom element; extend that one rather than introducing siblings.
- **`virtual_list.rs` stays pure** (GPUI-free aside from
  `gpui::Pixels`) so the inline tests run without a window context.
- **Split a file only when it crosses the G1 600-line sibling budget.**
  Until then, prefer growing an existing file by responsibility.

See the parent `crates/app/src/CLAUDE.md` for the workspace-wide
conventions (G1–G9 maintenance guardrails, persistence schema,
modal/keybinding extension recipes).
