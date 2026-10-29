# Status icon themes

Settings → Workspace → Sidebar → Status icon theme selects the artwork used by
left-dock lane and session indicators, including the compact rail. Dots and
Icons are bundled; Dots is the default. Custom themes inherit either and
override individual states.
Selections apply immediately and survive restart; activity, permission, elapsed
time and completion decisions remain owned by the sessions.

## Create a user theme

1. Choose **Open theme folder** beside the selector. This opens the active
   profile's `status-icon-themes` directory, beside `config.toml`. With an explicit
   `DARUDA_DATA_DIR`, it is `<DARUDA_DATA_DIR>/status-icon-themes`.
2. Copy [the Soft Ice example](soft-ice/theme.json)
   and its two SVG files into a child directory, for example `soft-ice/`.
3. Edit `theme.json` and the SVG files. Saved changes are discovered automatically,
   including while Settings is open. Choose `Soft Ice · user/soft-ice`.

The JSON `id`, rather than the folder or display name, identifies a theme. Keep
it stable when publishing updates. Each package has this layout:

```text
status-icon-themes/
  soft-ice/
    theme.json
    tool-0.svg
    tool-1.svg
```

```json
{
  "version": 1,
  "id": "soft-ice",
  "name": "Soft Ice",
  "extends": "icons",
  "states": {
    "executing_tool": {
      "frames": ["tool-0.svg", "tool-1.svg"],
      "ticks_per_frame": 2,
      "colors": { "dark": "#81cfe6", "light": "#256780" }
    }
  }
}
```

`extends` is `icons` (default) or `dots`. Missing states, frames and colors
inherit the selected base. A colors-only override preserves the base animation.
A single frame makes that state static. Colors must provide both dark and light
`#rrggbb` values; SVG artwork is monochrome and tinted with the selected color.
Use opacity inside SVG for dimmer elements.

| State key | Meaning | Multiple frames allowed |
|---|---|---|
| `disconnected` | Agent chat before connection; gray plug in Icons | No |
| `connecting` | Connection preparation or handshake | Yes |
| `working` | Busy session, including live background subagents | Yes |
| `executing_tool` | Tool execution detail while busy | Yes |
| `needs_attention` | Waiting for user confirmation | Yes |
| `idle` | Connected and waiting | No |
| `completed` | Unseen successful result | No |
| `errored` | Unseen execution error | No |
| `failed` | Failed session | No |

The application shares one 250ms animation clock. `ticks_per_frame` defaults to
1 and accepts 1–40; 2 means each frame holds for 500ms. A sequence loops. Themes
cannot start timers, change status priority or keep a completed session busy.
Static frames do not request animation pulses. The normal artwork boxes remain
16px and 12px; verify both, in dark and light appearances.

SVG files must be UTF-8, self-contained shapes (`svg`, `g`, `path`, `rect`,
`circle`, `ellipse`, `line`, `polyline`, `polygon`, `title`, `desc`). Prefer
`viewBox="0 0 16 16"` and outlined paths. Scripts, text, styles, images, external
references and `url(...)` references are rejected. Frame paths must be relative
`.svg` paths within the package, including after symlink resolution. The loader
validates SVG before publishing a theme and caches loaded bytes; rendering does
not read files. File edits change the renderer cache key.

Limits: 128 KiB per manifest, 64 KiB per SVG, 32 frames per state and 128 installed
themes. IDs contain 1–80 ASCII letters, digits, hyphens or underscores. Names
contain 1–80 characters. Unknown manifest fields and state keys are rejected.

## Distribute a plugin theme

An installed Claude plugin can provide the same packages under its install root:

```text
my-plugin/
  .claude-plugin/plugin.json
  daruda/
    status-icon-themes/
      soft-ice/
        theme.json
        tool-0.svg
        tool-1.svg
```

Daruda discovers **user-scope installations** from the existing installed-plugin
manifest. No skill or executable hook is required for a theme. Marketplace
previews, project-scope and local-scope installations do not become global UI
providers. Existing plugin installation/removal watching updates the catalog;
Daruda never edits the plugin's installed files.

The saved selection is `plugin/<plugin-id>@<marketplace>/<theme-id>`. User themes
use `user/<theme-id>`, so they cannot replace a bundled theme or collide with a
plugin's ID. Duplicate IDs in one namespace are reported; the first directory
in sorted order is retained. To customize an installed theme, copy its package
to the user folder and select the user copy.

## Missing or invalid themes

A removed or invalid selected theme falls back to Icons. Settings retains and
shows the unavailable ID so reinstalling or fixing the package restores the
selection automatically. Invalid packages are omitted without hiding valid
neighbors; details are logged under `status_icon_theme`. The saved preference
is never silently rewritten. Old `dots` / `icons` configuration values continue
to load unchanged.
