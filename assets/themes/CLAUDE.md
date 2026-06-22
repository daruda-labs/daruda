# assets/themes/ — UI theme presets (light + dark **together**)

Bundled `DarudaTheme` presets, compiled into the binary via `include_str!`
(`crates/app/src/ui/theme/mod.rs::bundled_theme_json`) and loaded at runtime
by `apply_ui_theme`. Two presets ship: `daruda_dark` (default) and
`daruda_light`.

```
daruda_dark.json     # intentionally (near-)empty — dark = compile-time defaults
daruda_light.json    # full override set — every field that differs from dark
theme.schema.json    # generated JSON-Schema for editor autocomplete (do not hand-edit)
```

## The cardinal rule: light must be COMPLETE

`DarudaTheme` is `#[serde(default)]` at the struct level. **Any field absent
from a preset JSON falls through to its compile-time dark constant** (the
`field => CONST` line in `daruda_theme_fields!` in `daruda_theme.rs`).

- `daruda_dark.json` is empty on purpose: every missing key = "use the
  built-in dark constant", which *is* the dark theme. Correct by construction.
- `daruda_light.json` is the opposite: a key it omits renders **dark** in
  light mode (a black surface or white-on-light text). The light preset must
  therefore list **every field whose light value differs from dark** — in
  practice, essentially all surfaces, text, borders, scrollbars, badges, and
  banners. (A 46-field omission was exactly the "light mode has dark patches"
  bug; don't reintroduce it.)

**When you touch one file, check the other.** Change a color's meaning → set
it in both. Add a field → give it a dark default (macro) *and* a light value.

## Commonization rule: reuse tokens, don't hand-pick per field

The macro is the **token map** — its right-hand side is a small semantic set
(`BG_BASE` / `BG_PANEL` / `BG_HOVER` / `BG_ACTIVE` / `BG_RAISED`, `TEXT_BODY`
/ `TEXT_MUTE` / `TEXT_SUBTLE`, `BORDER`, `SCROLLBAR_*`, `ACCENT`, status,
banner). In dark, every field reading a given token *is* the same color.

The five tokens that were the same in **both** themes are already collapsed to
one shared field each — `border`, `text_muted`, `text_body`, `text_subtle`,
`scrollbar_thumb` (was ~59 per-feature fields). Read those at call sites
(`theme::current(cx).text_muted`) — do **not** re-introduce a per-feature
`foo_border` / `foo_text` field when one of these fits. Only split off a new
field when the new role genuinely needs a *different* value in some theme.

**Light must preserve that collapse.** All fields that map to the same token
in the macro share **one** light value — reuse it, don't invent a slightly
different gray per field. The number of distinct light values should track
the **token count (~66)**, not the field count (~177). Concretely:

- Before adding a light value, find the field's token (`field => TOKEN` in
  `daruda_theme.rs`) and reuse the value its siblings already use.
- Don't introduce a new near-duplicate gray when an existing surface/text/
  border token's value fits. (A drift of 7 muted grays across `TEXT_MUTE`
  fields is the smell this rule kills.)

**If two fields genuinely need *different* light values, they are different
roles — split them at the token level**, i.e. give them distinct consts in
the macro, instead of silently diverging in the JSON. Surface tokens
sometimes carry such hidden roles (a selection row wants a faint accent, a
text input wants pure `#ffffff`) that happen to share a dark const; those are
the only sanctioned per-field divergences, and the right long-term fix is a
dedicated token. When in doubt, collapse — match the dark token.

## Light identity (DESIGN.md compliance)

Light is a sanctioned variant but keeps the brand's **cool** identity — see
`DESIGN.md` → Elevation & Depth → Light theme. So in `daruda_light.json`:

- **Surfaces are faint-blue cool near-whites, never neutral gray.** Mirror the
  cool light ladder in `palette.rs` — read the constants
  `LIGHT_CANVAS` → `LIGHT_SURFACE_1` → `LIGHT_SURFACE_2` → `LIGHT_SURFACE_3`,
  don't inline their hex here (it drifts from the source). All sit at hue
  ~222° with a low cool saturation; tint deepens slightly as the surface
  darkens.
- **Text is a near-neutral dark scale** — the `text_primary` / `text_body` /
  `text_muted` / `text_subtle` fields in `daruda_light.json`. Text inputs stay
  pure `#ffffff` (e.g. `modal_input_bg`). **Re-darken the dark grays, don't just
  invert lightness**: a gray that reads on near-black collapses on near-white.
  Hold a contrast floor — `text_muted` ≳ 4.5:1, `text_subtle` ≳ 3:1 against its
  surface (the `#9aa1ad` subtle = 2.06:1 washout bug is what this prevents).
- **Borders** are light cool hairlines (the `border` field); **scrollbar
  thumbs** are black-alpha (`scrollbar_thumb` = `#00000040`); **banners** are
  light-tinted bg + dark text; **accent / status / diff** colors keep their hue
  (don't gray them out).

The theme's light/dark *appearance* is detected from luminance:
`DarudaTheme::is_dark()` is `title_bar_bg.l < 0.5`. A light preset's
`title_bar_bg` must therefore be light (l ≥ 0.5) or the whole app
mis-detects as dark.

## Adding a new theme field (cross-file checklist)

1. **`daruda_theme.rs`** — add `your_field => YOUR_CONST,` to the
   `daruda_theme_fields!` macro (the dark default).
2. **`daruda_light.json`** — add `"your_field": "#<cool light value>"`. Skip
   this and light mode shows the dark const.
3. **`theme.schema.json`** — regenerate from `DarudaTheme::json_schema()` so
   editor autocomplete sees the new key.
4. **`daruda_dark.json`** — leave empty unless you deliberately pin a value
   (it tracks `DarudaTheme::default()`).
5. Prefer reading it at the call site via `theme::current(cx).your_field`
   (theme-variant), **not** a raw `theme::YOUR_CONST` (see caveat below).

## Caveat: a complete light.json is necessary, NOT sufficient

Some render sites still read a raw dark constant directly
(`theme::SURFACE_1`, `theme::CANVAS`, `theme::TEXT_PRIMARY`, …) instead of the
theme-variant `theme::current(cx).field`. Those stay **dark in light mode no
matter what this directory contains** — the deferred Phase-3 migration tail
noted in `daruda_theme.rs`. Fixing such a spot is a code change, not a JSON
change: either migrate it to a `DarudaTheme` field, or pick by
`theme::current(cx).is_dark()` (e.g. `if dark { SURFACE_1 } else { LIGHT_SURFACE_1 }`).
So when light mode shows a stray dark surface, first check whether the field
is in `daruda_light.json`; if it is, the offender is a raw-const render site.

## Verification

```bash
cargo build -p daruda                                   # include_str! parses both JSONs
cargo test -p daruda --bin daruda daruda_theme          # schema + dark-default + is_dark guards
# light-mode visual check (set ui_preset = "daruda_light" in config first):
cargo build -p daruda --features screenshot
target/debug/daruda --screenshot /tmp/light.png         # then read it back — no dark patches
```

Guard tests live in `crates/app/src/ui/theme/daruda_theme/tests.rs`:
`bundled_daruda_dark_json_matches_default` (dark stays = defaults),
`json_schema_lists_every_theme_slot` (schema lists every field). They catch
"file hand-edited but generator not re-run" — not light/dark *parity*, which
is your responsibility per the cardinal rule above.

## Conventions

- Values are `"#rrggbb"` or `"#rrggbbaa"` (alpha). No other formats.
- Keep keys valid `DarudaTheme` field names; unknown keys are silently ignored
  by serde, so a typo'd key = a silent dark fallthrough.
- `theme.schema.json` is generated — never hand-edit; regenerate it.
