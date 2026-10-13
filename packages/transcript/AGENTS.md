# daruda_transcript

How a transcript is presented: the fold matrix (`fold_mode`), the display
filter (`display_filter`), the tool categories both index by
(`tool_category`, `command_analysis`), and the editors that author them
(`editor/`). The chat pane applies these values; Settings' agent catalog
writes the defaults a pane starts on. Neither owns the other, so both depend
on this package and it depends on neither.

- Only `editor/` reaches GPUI; the model modules stay GPUI-free.
- Widgets come from `daruda_ui` directly, never `gpui_component`.
- The editors' strings live in this package's `locales/` (`transcript:`),
  generated through `strings_gen`.
