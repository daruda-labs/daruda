# daruda_content

GPUI-free image decoding and Mermaid rendering shared by the file viewer
and agent chat. The host resolves visual tokens; this package processes content.

## Mermaid Rendering

`merman` is a normal crates.io dependency (MIT/Apache-2.0), validated against
upstream Mermaid.js SVG baselines. It has no daruda-side vendor patch.

The alpha integration exact-pins merman, dugong, and manatee. Stable layout
releases removed APIs used by the alpha renderer, and stable merman replaces
the host-theme and text-measurement APIs. Upgrade this family in one migration,
including conformance tests and PNG inspection; remove the temporary direct
layout dependencies and Dependabot exclusions when that migration passes.

- Host theming uses renderer-level site config, not a document directive.
  `mermaid_host_theme` builds a profile from `MermaidPalette`; the UI resolves
  those palette values in `daruda_ui::mermaid_palette`.
- `visual::render_mermaid_svg` applies the profile through `with_host_theme`.
- A source declaring `theme`, `themeVariables`, or `themeCSS` in an
  `%%{init}%%` directive or leading YAML frontmatter's `config:` opts out of
  host colors via `source_declares_own_theme`. Merman's site config overrides
  document config wholesale, rather than merging per field.
- Sources tuning only layout or behavior retain host colors.
- The opt-out is color-only. Always apply `mermaid_render_profile`, including
  `htmlLabels: false` and the `ResvgSafe` pipeline. Resvg cannot paint
  `<foreignObject>` labels; omitting the profile leaves every label blank.
- Both `file_view_pane::file_content` and `daruda_chat::reconcile` use
  `visual::render_mermaid_raster`, which reaches
  `merman::render::HeadlessRenderer::render_svg_sync`.

## Shared Markdown Behavior

Interactive Markdown has two independent rendering stacks:

| Host | Parser and renderer |
|---|---|
| File viewer | `pulldown-cmark` -> workspace-owned `MdBlock` / `MdSpan` IR -> `file_view_pane/render/markdown/` |
| Agent chat | `daruda_ui::markdown` -> vendored `gpui_component::text::TextView` and its mdast parser |

A behavior intended for both hosts must be implemented in both stacks and
added to the shared rendered-conformance table in
`packages/app/src/workspace/main_area/file_view_pane/render/markdown/layout_tests.rs`.
File-viewer parser coverage lives beside `parse_markdown` and includes
semantic field states (`checked`, `loose`, ordered start), not enum variants
alone. In its renderer, `prose.rs` compiles span runs;
`inline.rs`, `block.rs`, and `image.rs` render them; `selection.rs` owns
block-selection plumbing.

Follow the [UI facade rules](../ui/AGENTS.md) and
[chat host boundary](../chat/AGENTS.md) when changing the consumers.
