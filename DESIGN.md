# DESIGN.md — daruda

> Design language for daruda: a macOS multi-agent terminal multiplexer.
> Synthesised from Linear (cool dark canvas + accent scarcity), Raycast (keycap patterns + pill-tab chrome), Warp (terminal typography), and Cursor (AI timeline palette).

---

## Philosophy

**"Cool precision."** The terminal is the hero; daruda's chrome must recede.

Three rules:
1. **Cool dark everywhere.** Near-pure black with a faint blue tint — the deepest dark surface. Never neutral gray, never warm brown. The optional `daruda_light` preset inverts lightness but keeps the cool tint (see Elevation & Depth → Light theme).
2. **Accent through scarcity.** One chromatic accent (`#5e6ad2` lavender-blue). Used only for: active lane indicator, focus ring, primary button, Claude active badge.
3. **Elevation without shadow.** Depth comes from the 4-step surface ladder and 1 px hairline borders. Drop shadows are forbidden in application chrome.

**Key characteristics** (the quick-scan identity):
- **Compact, dense chrome.** Dev tool, not a marketing page — UI text lives at 10–13px, spacing snaps to a 2px-step scale. The chrome is meant to disappear behind the terminal.
- **Cool near-black canvas** (`#010102`), faint blue tint, dark-first. Light is a sanctioned cool-tinted variant, not a neutral-gray flip.
- **Single lavender-blue accent**, rationed to ≤ 3–4 visible elements; the `2px accent` left border is the primary "selected" signal.
- **Surface-ladder + hairline depth.** No shadows anywhere in chrome.
- **Semantic color is reserved for state** — Claude lifecycle, agent activity, git status, diff — never decoration.
- **Syntax highlighting is a separate axis** — selectable, readability-tuned, light/dark-paired (see Syntax Highlighting).
- **Terminal pane is sacred:** `canvas` background, never wrapped in a card.

---

## Colors

```yaml
colors:
  # Canvas — cool near-black (Linear-inspired, faint blue tint)
  canvas:         "#070809"                   # Window frame base: title/status bar, welcome, active terminal tab, active-state recesses. Lifted off pure black so the frame isn't harsher than the lifted content; cool tint matches the surface ladder (hue 210), not navy (see §Readability).
  editor-surface: "#0b0c0e"                   # File viewer + diff code area — one rung above canvas (see §Readability)
  surface-1:      "#0f1011"                   # Dock panels, sidebar, card backgrounds
  surface-2:      "#141516"                   # Hover row, active tab, MacroKey resting state
  surface-3:      "#18191a"                   # Active/pressed row, input background
  surface-4:      "#1f2022"                   # Popover, context menu, tooltip
  hairline:      "#23252a"                    # 1px borders everywhere
  hairline-soft: "rgba(255,255,255,0.06)"    # Faint overlay borders (popover edges)

  # Accent — single lavender-blue (Linear)
  accent:        "#5e6ad2"   # Active lane indicator, focus ring, primary CTA, Claude running
  accent-hover:  "#828fff"   # Hovered accent elements
  accent-muted:  "#1e2050"   # Low-opacity accent fill (badge background)
  accent-fg:     "#ffffff"   # Text/icon on solid accent background
  link:          "#809ff9"   # Clickable *text*: markdown links, diff header file path (7.4:1 — accent is too dark as text)

  # Text — cool light-gray scale (Linear-inspired)
  ink:           "#f7f8f8"   # Primary: active labels, tab titles, focused row
  body:          "#d0d6e0"   # Default: descriptions, inactive labels, body text
  mute:          "#8a8f98"   # Tertiary: timestamps, dock section headers, placeholders
  subtle:        "#797e86"   # Lowest-emphasis *readable* copy: line numbers, hints, metadata, shortcuts, empty states (≥4.5:1 on surface-1). Also the disabled-control tier, where inertness is carried by bg + loss of hover, not by this color alone.

  # Semantic — Claude lane states (badge on WorktreeRow)
  claude-active: "#f0a020"   # Claude is running in this lane (vivid gold — distinct from warning)
  claude-done:   "#4aaf78"   # Last Claude session completed (intentionally same as success)
  claude-error:  "#e06060"   # Last session errored (intentionally same as error)

  # Semantic — agent action states (Cursor timeline palette, dark-adapted)
  agent-thinking: "#8faacc"  # Steel blue — Thinking / planning
  agent-reading:  "#8fcca8"  # Mint green — Reading files / context
  agent-editing:  "#b09bcc"  # Lavender — Writing / editing output
  agent-running:  "#ccaa6e"  # Gold — Executing tool / running command
  agent-idle:     "#8a8f98"  # = mute — no active session

  # Semantic — git status
  git-staged:    "#4aaf78"   # Green — staged for commit
  git-modified:  "#d4a853"   # Amber — modified, not staged
  git-untracked: "#8a8f98"   # Mute — new file, not tracked
  git-deleted:   "#e06060"   # Red — deleted file
  git-renamed:   "#7b9fd4"   # Blue — renamed / moved
  git-conflict:  "#e06060"   # Red — merge conflict (same as error)

  # Semantic — diff line level
  diff-add-bg:   "rgba(74, 175, 120, 0.12)"   # Green tint row
  diff-add-fg:   "#4aaf78"                     # Added line marker/text
  diff-del-bg:   "rgba(224, 96, 96, 0.12)"    # Red tint row
  diff-del-fg:   "#e06060"                     # Deleted line marker/text
  diff-hunk:     "#62666d"                     # @@ section header

  # Semantic — text/terminal selection
  selection-bg:  "rgba(94, 106, 210, 0.28)"   # accent at 28% — inputs, terminal, file viewer
  selection-fg:  "#f7f8f8"                     # = ink

  # Semantic — general UI
  success:       "#4aaf78"
  warning:       "#d4a853"   # Muted amber — distinct from claude-active
  error:         "#e06060"
```

**Color usage rules:**
- `accent` appears on at most 3–4 elements visible at one time. Never use it as a panel fill.
- `claude-active` / `claude-done` / `claude-error` are badge-only colors — never applied to text or large surfaces.
- `canvas` is the only valid background for the terminal pane. The terminal is not a card.
- `editor-surface` (not `canvas`) backs the file viewer and diff code area — code renders on a gentle dark-gray, never on pure-black `canvas` (see §Readability).

---

## Typography

Two-role system: system sans for UI chrome, monospace for terminal content.

```yaml
typography:
  # UI chrome — Inter (bundled, 400/500/600) or -apple-system fallback
  ui-xs:          { size: 10px, weight: 400, lineHeight: 14px, letterSpacing:  0.2px }
  ui-xs-strong:   { size: 10px, weight: 500, lineHeight: 14px, letterSpacing:  0.2px }
  ui-sm:          { size: 11px, weight: 400, lineHeight: 16px, letterSpacing:  0.1px }
  ui-sm-strong:   { size: 11px, weight: 500, lineHeight: 16px, letterSpacing:  0.1px }
  ui-md:          { size: 12px, weight: 400, lineHeight: 18px, letterSpacing:  0    }
  ui-md-strong:   { size: 12px, weight: 500, lineHeight: 18px, letterSpacing:  0    }
  ui-lg:          { size: 13px, weight: 400, lineHeight: 20px, letterSpacing: -0.1px }
  ui-lg-strong:   { size: 13px, weight: 500, lineHeight: 20px, letterSpacing: -0.1px }

  # Dock / section headers — ALL-CAPS with positive tracking
  label:    { size: 10px, weight: 600, lineHeight: 12px, letterSpacing: 0.9px, transform: uppercase }

  # Terminal output — user-configurable, this is the default
  terminal: { family: "JetBrains Mono, SF Mono, Menlo, monospace", size: 13px, weight: 400, lineHeight: 20px }
```

**Font families:**
| Role | Family | Notes |
|------|--------|-------|
| UI chrome | `Inter` (bundled 400/500/600) | Fallback: `-apple-system, "SF Pro Text", system-ui` |
| Terminal output | `JetBrains Mono` (default) | User-overridable via config |
| Code display | Same as terminal | File viewer, task pane inline code |

**Principles:**
- UI text: 10–13px only. Dense terminal-app chrome — never 14px+ in docks or toolbars.
- `label` (ALL-CAPS, positive tracking) is reserved for dock section headers and group names only.
- Negative letter-spacing only at 13px (`ui-lg`, `ui-lg-strong`).
- Inter font features (`calt`, `kern`, `liga`) — enable if GPUI's text shaper exposes per-font feature flags.

---

## Readability

Dark-mode text legibility is a first-class constraint, not an afterthought. The
rules below come from contrast research (WCAG 2.x, BOIA, Material dark theme) and
hold for **all** text the user reads for more than a glance — UI chrome, terminal
output, and syntax-highlighted code.

**Contrast floors** (WCAG, against the surface the text sits on):

| Text role | Min ratio | Notes |
|-----------|-----------|-------|
| Body / normal-size text | **4.5:1** (AA) | The hard floor for anything read continuously. |
| Large text (≥18px, or ≥14px bold) | 3:1 (AA-large) | Headings, large labels only. |
| Comfortable target | **7:1** (AAA) | Where daruda aims for body copy (`ink`, `body`). |
| UI component edges / icons | 3:1 | Borders, focus rings, glyph-only affordances. |

**Avoid both extremes — contrast is a tuned band, not "more is better":**
- **Never pure-black background under text.** `#000`-ish backgrounds make bright
  glyphs *halate* (bloom) and leave afterimages, especially on OLED. No reading
  surface sits on pure black: code renders on `editor-surface` (`#0b0c0e`), the
  terminal on its `#1e1e1e` preset, and even the window base `canvas` is lifted
  to `#070809` so the frame isn't the harshest thing on screen.
- **Never pure-white text.** A near-white (`ink #f7f8f8`) at ~13–18:1 is the
  ceiling; literal `#fff` on near-black overshoots into glare. Tone down, don't max out.

**Saturation on dark surfaces:**
- High-chroma colors *vibrate* against dark backgrounds. Soften toward pastel —
  this is why the `daruda` syntax palette pulls `function`/`string_special` out of
  the near-gray band rather than using raw, fully-saturated source-theme values.
- `accent` (`#5e6ad2`) clears 3:1 as a UI element but only ~4.05:1 as text on
  `surface-1`, and 3.45:1 on the `surface-4` float rung — **do not use `accent`
  for small body text** (links, inline labels). Use it for fills, focus
  rings, icons, and ≥14px-bold emphasis only. Clickable text takes the `link` token
  (`#809ff9`, 7.4:1) instead.

**Contrast is a scarce resource (syntax highlighting):**
- Don't color every token. Lean on structure (a string's contents can't be confused
  with code) and spend color only where ambiguity is real. (tonsky, "everyone is
  getting syntax highlighting wrong".)
- Carry meaning on **non-color channels** where you can: the `daruda` palette uses
  keyword **bold**, escape **bold**, comment *italic* so a token stays distinguishable
  even at low chroma — and stays legible for color-blind users.
- Comments are intentionally the dimmest token (~4.0:1 on `editor-surface` — riding
  the floor); that is the *floor*, not a target to push below. Note lifting the editor
  background off pure-black slightly *lowered* the comment ratio (4.28 → 4.01), so the
  pair is now tuned to sit right at ~4.0. If a syntax color dips under ~4:1, lift it
  (see Tokyo Night's `comment` raised to `#6b74a3`).

**When you change a color or background, re-check the pair.** Lifting a dark
background *lowers* the contrast of every light token on it; darkening text on a
fixed surface lowers it too. Compute the ratio for the actual fg/bg pair — never
eyeball it. The four sanctioned text tones (`ink`/`body`/`mute`/`subtle`) are tuned
against `surface-1`; `subtle` (`#797e86`) sits right at 4.5:1 — don't darken it.

---

## Spacing

Base unit: **4px**.

```yaml
# daruda uses a dense 2px-step scale (palette PAD_*/GAP_* tokens),
# not a doubling scale — the chrome is compact terminal UI.
spacing:
  px:   1px    # Hairline separators
  xxs:  2px    # GAP_XS — tightest gap / badge padding
  xs:   4px    # PAD_XS, GAP_SM — icon gutter, tight chip padding
  sm:   6px    # PAD_SM, GAP_STANDARD — compact row / gap
  md:   8px    # PAD_STANDARD, GAP_LG — standard row / dock inner padding
  lg:   10px   # PAD_LG — panel inner padding, dock view tab padding
  xl:   14px   # PAD_XL — settings rows, modal buttons, wide controls
# Larger values (16 / 24) exist only as feature-specific
# constants (modal/card/welcome padding), not part of the shared scale.
```

**Fixed heights:**

| Element | Height |
|---------|--------|
| TitleBar | 28px |
| TabBar | 32px |
| DockSwitcher (tab strip) | 32px |
| ViewSwitcher (dock tabs) | 32px |
| Tree rows (WorktreeRow, ProjectHeader, GroupHeader) | 24px |
| StatusBar | 22px |
| MacroKey (text mode) | 32px |
| MacroKey (icon mode) | 40px |
| BottomDock (default) | 120px |
| BottomDock (minimum) | 80px |
| BottomDock (maximum) | 320px |
| Button (standard) | 28px |
| Input (standard) | 28px |

---

## Elevation & Depth

No drop shadows. Depth = surface color ladder + 1px hairline borders.

| Level | Token | Hex | Border | Use |
|-------|-------|-----|--------|-----|
| 0 — Base | `canvas` | `#010102` | — | Window background; terminal pane |
| 1 — Panel | `surface-1` | `#0f1011` | `1px hairline` | LeftDock, RightDock, BottomDock chrome |
| 2 — Card | `surface-2` | `#141516` | `1px hairline` | Hovered row, MacroKey resting, active tab |
| 3 — Raised | `surface-3` | `#18191a` | `1px hairline` | Active/pressed row, input field background |
| 4 — Float | `surface-4` | `#1f2022` | `1px hairline` + drop shadow | Everything that floats: popover, dropdown + context menu, tooltip, autocomplete, dialog, command palette, toast |

**Active indicator:** selected lane / focused pane uses a `2px solid accent` left border, not a background fill change alone.

**Level 4 is the one rung that also carries a shadow**, and it is the sole
exception to the no-shadow rule below. One surface step (`surface-4` over a
`surface-1` panel is 1.17:1) cannot on its own say "this is detached and
dismissable", and in light mode the float surface is nearer its base still.
Every floating surface reads a single token (`float_panel_bg`), so they cannot
drift apart: `Theme::popover` is the vendor's only float slot and the dialog,
the tooltip and every menu are wired to it.

### Light theme (`daruda_light`)

The optional light preset inverts the ladder's lightness but **keeps the
cool blue identity** — every surface is a faint-blue near-white, never a
neutral gray (mirroring how `canvas` is a cool near-black, not pure black).
Tint deepens slightly as surfaces darken; accent, status, and diff colors
are unchanged from dark.

| Level | Hex | Use |
|-------|-----|-----|
| 0 — Base | `#f9fafb` | Window / editor / terminal background |
| 1 — Panel | `#eaecf0` | Docks, sidebar, tab strip |
| 2 — Card | `#e4e6ec` | Title bar, hovered row, active tab |
| 3 — Raised | `#dbdee5` | Active/pressed row, button-widget surfaces |
| 4 — Float | `#f6f6f8` | Popover / modal panel (raised above base) |

Text on light stays a near-neutral dark scale; muted/subtle tiers carry a
faint cool tint, and only **surfaces** otherwise carry it. Re-darken the
dark grays rather than just inverting lightness — a gray that reads on
near-black collapses on near-white. Floor: **muted ≳ 4.5:1, subtle ≳ 3:1**
against its surface. Input fields remain pure `#ffffff`.

---

## Border Radius

```yaml
radius:
  none: 0px      # Pane separators, full-bleed surfaces
  xs:   2px      # Inline chips, keycap shortcut labels
  sm:   4px      # Small non-circular indicators, MacroKey cards
  md:   6px      # Buttons, text inputs, tab pills
  lg:   8px      # Dock panels, cards, modals, toasts
  pill: 9999px   # Claude state badges (circular dots), status pills, toggle chips
```

Most app chrome sits at `sm` (4px) or `md` (6px). Dock panels and modals use `lg` (8px). No radius exceeds 8px in application chrome.

---

## Components

### TitleBar

```
background:    canvas
height:        28px
border-bottom: 1px hairline
padding:       0 lg (0 16px)
```

- macOS traffic-light buttons at standard macOS position (left).
- App title centered: `ui-md-strong`, `body` color.
- No extra controls in TitleBar — title and window management only.

---

### LeftDock / RightDock

```
background:   surface-1
width:        220px (default, user-resizable, min 160px)
border-right: 1px hairline  (LeftDock)
border-left:  1px hairline  (RightDock)
```

**ViewSwitcher (icon tab strip at top)**
```
tab-default:  background transparent,  icon mute,    height 32px, padding 0 sm
tab-active:   background surface-2,    icon accent,  border-bottom 2px solid accent
tab-hover:    background surface-2,    icon body
```
- Icons only (16px), centered horizontally. Tooltip shows tab name on hover.
- Active tab underlined with `2px accent` — not a filled background alone.

---

### WorktreesView

**GroupHeader (collapsible accordion)**
```
height:       24px
padding:      0 sm (8px) — left edge flush with dock
background:   transparent
hover:        surface-2
text:         ui-sm-strong, body; expanded → ink
```
- Disclosure triangle `▶ / ▼` in `mute`. Color dot: 8×8px `radius.pill` at the group accent color.
- Bottom: `1px hairline` when expanded, separating from children.

**ProjectHeader**
```
height:       24px
padding:      0 sm, indent 8px (total 16px from dock edge)
text:         ui-sm-strong, body
hover:        surface-2
```

**WorktreeRow (Lane)**
```
height:       24px
padding:      0 sm, indent 16px (total 24px from dock edge)
text:         ui-sm, body
hover:        surface-2
active-bg:    surface-2 + 2px left border in accent
```

**Drag-and-Drop visual states:**
```
dragging item:     surface-2 bg, 0.6 opacity, 1px accent border, radius sm
drop target row:   2px solid accent outline around the row
drop placeholder:  2px accent horizontal line between rows (insertion point)
invalid target:    2px solid error outline
```

**Claude state badge** (right-aligned on WorktreeRow):
```
size:    6px × 6px circle
radius:  pill
colors:
  running  → claude-active  (#f0a020)  — Claude is executing
  done     → claude-done    (#4aaf78)  — last session completed
  error    → claude-error   (#e06060)  — last session errored
  idle     → no badge
```

---

### TabBar

```
background:    surface-1
height:        32px
border-bottom: 1px hairline
padding:       0
```

**Tab item**
```
tab-default: background transparent,  text mute,   padding 0 md (0 12px)
tab-active:  background canvas,       text ink,    border-bottom 2px solid accent
tab-hover:   background surface-2,   text body
tab-modified: accent dot (4px) before the title text
tab-close:   16px × 16px ×, mute — visible on tab hover only
```

- Active tab drops to `canvas` background to visually connect with the terminal pane below.
- Tab title: `ui-sm-strong`. Modified indicator: 4px `accent` dot prepended to title.

**Tab overflow (too many tabs)**
```
overflow:        horizontal scroll, no scrollbar visible
left-fade:       16px linear gradient canvas → transparent (masks clipped tabs)
right-fade:      16px linear gradient transparent → canvas
overflow-button: none — scroll by trackpad swipe or drag only
min-tab-width:   80px (title truncates with "…" below this width)
```

---

### PaneTree / Pane

**Pane separator (split mode)**
```
1px solid hairline — the separator IS the border, no resize handle chrome visible at rest
Resize handle: 4px wide, appears on hover (cursor: col-resize / row-resize)
```

**PaneHeader (visible in split mode only)**
```
background:    surface-1
height:        24px
border-bottom: 1px hairline
text:          ui-sm, mute — lane/file name  (ui-sm minimum for legibility in narrow panes)
```
- Focused pane: PaneHeader `surface-2` background, text `body`.
- Unfocused pane: PaneHeader `surface-1` background, text `mute`.

**TerminalPane**
```
background:  canvas        ← terminal renders directly on canvas — no card wrapping
font:        terminal (JetBrains Mono 13px / 20px line-height)
cursor:      accent color, block or bar per user preference
selection:   accent at 25% opacity
scrollbar:   2px, surface-3 thumb, transparent track — visible only on scroll
```

**FileViewPane / TaskEditPane**
```
background:  surface-1
padding:     lg (16px)
text:        ui-md, ink / body
toolbar:     surface-2, border-bottom 1px hairline, height 32px
```

---

### AgentChatPane

Unlike every other component above, this pane does **not** paint on the surface
ladder. Its background mirrors the resolved *terminal* palette
(`theme::agent_chat_bg`, from `[colors]` + `[theme].terminal_preset`), so a chat
pane and a terminal pane split side by side read as one surface. `ui_preset` and
`terminal_preset` are independent config keys, so nothing inside the pane may
take a colour from the UI theme without checking it against a background the UI
theme has never seen — a fixed `hairline` is invisible on it, and `accent` has
no verified contrast on it at all.

Everything on the pane therefore derives from the pane's own background
(`PaneSurfaceTokens`), and those derived tokens are what the rest of this entry
names:

```yaml
pane-bg:           terminal background, verbatim
pane-fg:           terminal foreground, lifted toward white by 0.24·(1 − bg.l)
pane-fg-muted:     pane-fg at 62% over pane-bg   # secondary labels, chip text
pane-fg-subtle:    pane-fg at 50% over pane-bg   # metadata, working indicator
pane-tint:         neutral overlay at 5%         # hover fill
pane-active-tint:  neutral overlay at 12%        # selected fill
pane-border-tint:  neutral overlay at 12%        # card edges — tool cards, code blocks
pane-control-edge: neutral overlay at 34% / 42%  # resting edge of an interactive control
```

`neutral overlay` is white over a dark background and black over a light one, so
the ladder inverts with the user's terminal preset instead of breaking on it.

`pane-control-edge` is deliberately far heavier than `pane-border-tint`, and the
split is the whole point: **a card's edge is decoration, a control's edge is what
identifies it as a control**, which §Readability holds to 3:1. At the card's 12%
a chip edge measures 1.44:1 on the default `#1e1e1e` preset. It carries two
alphas because darkening a near-white surface buys less contrast than lightening
a near-black one — 34% reaches 3.11:1 over `#1e1e1e`, 42% reaches 3.02:1 over
`#f9fafb`. Every shipped terminal preset is dark (`#002b36` through `#2e3440`),
so the light alpha exists for a user-supplied `[colors] background`, not for a
preset. A custom background at mid lightness can still fall under the floor —
a standing limit of deriving chrome from a user-supplied colour, not a bug in
these numbers.

**Activity Bar (pane toolbar, always present)**

```
background:    pane-bg (inherited — the bar is not a card)
height:        28px at the default chrome size (content 20px + xs padding, top
               and bottom), + the 1px border. Not fixed like TitleBar/TabBar —
               it grows with `font.agent_chat_size`.
border-bottom: 1px pane-border-tint    ← not hairline; see above
padding:       xs md (4px 8px)
gap:           xxs (2px) between the bar's three zones;
               xs (4px) between controls inside the right zone — bordered chips
               two pixels apart read as one segmented strip, not three controls
text:          agent-chat size (config `font.agent_chat_size`, default 13px = ui-lg)
```

| Zone | Content | Style |
|------|---------|-------|
| Left | Agent icon (16px) + session title, ellipsized | icon `pane-fg-muted`, title `pane-fg` |
| Centre | Context-window meter (`53k / 200k`) | `pane-fg-muted`, detail + cost in tooltip |
| Right | Transcript controls | see below |

- The bar renders even while the conversation is empty or still connecting, so
  the reading-width toggle is never unreachable.
- A press inside the right zone stops there. These controls say how the pane is
  *displayed*, not that the user is engaging with what is in it, so they must
  not reach the pane wrapper's "activate this pane" handler.

**Activity Bar — control vocabulary**

Two classes, and the split is by whether the control carries a word:

```
chip (carries a value — Fold / Filter / Recent steps)
  background:    transparent
  border:        1px pane-control-edge    ← always on, at rest; ≥3:1
  border-radius: sm (4px)
  height:        20px
  padding:       0 xs (0 4px)
  text:          agent-chat size, pane-fg-muted

  hover:     background pane-tint
  selected:  background pane-active-tint   (axis is off its default)
  disabled:  no chip is ever disabled

icon button (carries a glyph — expand all / collapse all / reading width / view options)
  background:    transparent
  border:        none
  size:          20px square

  hover:     background pane-tint
  selected:  background pane-active-tint
  disabled:  no hover, no press; glyph drops to `mute` at 50% (the vendored
             button's own disabled tone — a UI-theme colour on a pane-mirrored
             surface, see Known Gaps)
```

- **A chip gets a border and a glyph does not.** The chips sit next to the
  context meter, which is static text in `pane-fg-muted` at the same size — a
  borderless word is indistinguishable from a readout. A glyph needs no frame to
  read as a control, and boxing three of them adds frames without information.
  (Same reasoning, same fix as StatusBar's pill buttons.)
- **Selected means "off its default" — or "my popover is open".** The two share
  one fill, because the vendored `Popover` forces `selected` on its trigger
  while open (`popover.rs`, `selected || is_open`). So `Fold: Auto` marks itself
  while its panel is up and unmarks on close. Harmless in practice — the panel
  you are looking at states the value — but it means the fill is only a
  reliable "off default" signal when the popover is *shut*, which is when it
  matters. The Recent-steps chip is the odd one out: it opens a menu, not a
  popover, so it never marks itself for being open.
- Chip copy is `Label: Value`. The label is constant and the value is the state;
  keeping both means a chip still reads on its own, out of the row.

**Activity Bar — responsive behaviour**

The one breakpoint in the pane, measured on the **pane root** (not the
transcript list, so an empty conversation lands in the same layout as a full
one):

Both parts are *text* widths, so the threshold scales with
`font.agent_chat_size`; 596px is its value at the 13px default. Widths below are
quoted at that size.

| Pane width | Right zone | Where the values are |
|------------|-----------|----------------------|
| > 596px | three chips + three icon buttons | in the chip labels |
| ≤ 596px | one `View options` gear + three icon buttons | in the gear's tooltip; the gear is `selected` when any axis is off its default (and, like the chips, while its popover is open) |

596px is **derived, not chosen**: `title floor (180) + control cluster (400) +
2 × md padding (16)`. The cluster budget is what the three chips measure at
their widest realistic values; the title floor is the point below which the
title ellipsizes to a few words and stops identifying the session, which is the
bar's primary job. Move either part and the breakpoint moves with it — a
hand-set number drifts away from the thing it is supposed to describe. The two
parts are text measurements, so they are also scaled by
`font.agent_chat_size / 13` before the comparison: a fixed pixel breakpoint
reads as derived while silently assuming one font size, and at 20px chrome the
spelled-out chips would outgrow the budget while the bar stayed wide.

Collapsing costs the chips their labels, so the gear must repay both of their
jobs: `selected` for the at-a-glance signal, and a tooltip that spells out all
three values verbatim (not just the adjusted ones — a reader asking "what is
this pane showing me" wants the whole answer).

**Options panel (the Fold and Filter chips, and the gear)**

Two of the three chips open a panel; the **Recent-steps chip opens a dropdown
menu** of checked items instead, because that axis is a single choice from a
short list and a menu is the right control for it. Its panel body exists only as
the combined popover's third tab.

The panel is app chrome, not pane surface — it floats, so it takes the level-4
rung and its chrome comes from the shared `Popover`:

```
background:    surface-4  (float_panel_bg)
border:        1px hairline, opaque (1.06:1 — see Known Gaps)
border-radius: sm (4px) — small attached overlays; large floating surfaces
               (dialog, palette, toast) take lg (8px)
shadow:        yes (level-4 exception)
width:         240px single-axis · 430px fold-rule editor and combined panel
max-height:    520px
section-heading: agent-chat size, subtle
row-gap:       xs (4px); nested rows indent 20px
footer:        Fold and Filter only — one ghost reset ("Reset to Auto preset" /
               "Show everything"), disabled when that axis is already default.
               Recent steps needs none: picking `All` in its radio list *is* the
               reset.
```

The panel body scales with `font.agent_chat_size`, not the UI type ladder, even
though it is app chrome — it is read alongside the pane it configures. That is a
deliberate exception to §Typography's 10–13px band, which the pane's own
user-set size can leave in either direction.

- **Both dimensions cap at 80% of the window**, so the panel can never grow past
  the frame it opens in. Width is capped as well as height because the editor is
  430px and daruda's minimum window is not much wider. Overlapping the docks
  beside the pane is *not* what the cap is for — a popover anchored in a narrow
  pane covers its neighbours the way a context menu does, and that is fine.
- The combined panel adds a segmented tab strip (`Fold` / `Filter` /
  `Recent steps`) above the body. The Fold and Filter tabs show exactly what
  their chips open on a wide bar; the Recent-steps tab is the only place its
  radio panel appears at all.
- **The segmented strip keeps the `hairline` frame (1.19:1), not the chip's 3:1
  control edge.** The 3:1 floor applies where the edge is the *only* thing
  separating a control from adjacent non-interactive text — the Activity Bar
  chip's case. Inside the strip the ≥4.5:1 label and the accent-filled selected
  segment identify both the control and its state, so the frame is refinement.
  The old accent outline cleared 3:1 as an edge, but only by spending accent as
  a 3.45:1 label on every unselected segment — under the 4.5 floor — across up
  to 36 elements at once.
- **A filter with nothing checked is the unfiltered state**, not a selection of
  nothing: the chip reads `All`, "Show everything" greys out, and clearing the
  last box restores the whole transcript. A pane showing only prompts and
  permissions is not a state worth being able to reach.

**Disclosure rows inside the transcript**

Rows that reveal hidden content state **what clicking them does**, never a bare
count — a count alone reads as "still hidden" after the reveal.

```
text:  agent-chat size, pane-fg-muted
```

**A content fold and a window boundary are different controls.** The filter's
placeholder (`Show 12 filtered rows` → `Hide them again`) is a disclosure like
every other row in the transcript: the chevron carries its state and the
`▶ / ▼` glyph is right, because what it holds is *rows*. Only the shape differs
from the boundary below — the copy rule is the same one, and for the same
reason: **an expanded reveal row names no count.** The rows are on screen, so a
number restates them, and `Hide 12 filtered rows` parses as a description of the
current state in exactly the state where it is false. The extent of the cut is
the *collapsed* label's job, which the reader saw one click ago.

The tail window's row is not that. Nothing collapses into it; it marks the edge
of the step range the pane is showing, and it used to wear the same chevron and
the same muted text as the step bars beside it — so the least distinguishable
thing on the row was its own open/closed state, and `Hide 6 earlier steps` reads
as a *description* ("6 earlier steps, hidden") exactly when those steps are on
screen. It therefore takes its own shape:

```
closed:   ─────────  N earlier steps ⌄  ─────────    (label centred between two rules)
open:     ──  last N steps only ⌃  ───────────────    (stub left, label anchored)

rule:     1px pane-border-tint
chevron:  DisclosureAxis::Vertical (⌄ / ⌃) — ▶ / ▼ is the step bars' glyph
label:    agent-chat size, pane-fg-muted
```

Three rules follow from it:

- **The states differ in layout, not only in a glyph.** Centred-between-rules vs
  left-anchored-after-a-stub is legible at a glance; a 12 px chevron flip in a
  dense transcript is not.
- **The open label names the window it returns to, not the steps it re-hides.**
  `Collapse to the last 5 steps` can only be read as an action, and it surfaces
  the number the Activity Bar's `Recent steps` chip is set to — the anchor the
  word "earlier" is relative to and which the row never used to state.
- **Rows outside the kept range carry a `1px pane-border-tint` left rail**,
  tying them back to the boundary above them. The rail says "outside the range",
  not "the boundary revealed me" — a step with a running tool stays surfaced
  through a *shut* boundary, and that is exactly the row that needs explaining.
  No dimming: the reader just asked to see these.

The boundary is the one row in the transcript that is not a `FoldRow`; both
shapes live in `render/fold_header.rs` so the chevron stays in one file
(`scripts/lint-fold-header.sh`).

**BottomDock chip row (session controls, for reference)**

The mode / model / effort chips under an agent pane live in the BottomDock, not
this bar, but they share the `Label: Value` vocabulary. One rule differs and is
deliberate: **the host localizes what the host owns and passes through what the
agent owns.** A boolean option's `On` / `Off` comes from i18n; a select option's
name and choice labels arrive from the ACP adapter and render verbatim, so a
Korean UI still shows `Model: Sonnet`. Translating them would mean a per-agent
table of protocol ids in the host, which is exactly what keeps a new agent's
unfamiliar option from needing UI code.

---

### BottomDock

```
background:   surface-1
border-top:   1px hairline
```

**DockSwitcher (tab strip at top of BottomDock)**
```
tab-default: background transparent, text mute,   height 32px, padding 0 md
tab-active:  background surface-2,   text ink,    border-top 2px solid accent
tab-hover:   background surface-2,   text body
```

**TerminalInputDock**
```
background: surface-1
padding:    sm (8px)
```

- Textarea:
  ```
  background:    surface-3
  border:        1px hairline
  border-radius: md (6px)
  focus-border:  1px solid accent
  text:          ui-md, ink
  placeholder:   ui-md, mute
  padding:       sm md (8px 12px)
  ```
- Submit button: `background accent, text accent-fg, radius md, height 28px, padding 0 md`
- Secondary action buttons: `background surface-2, text body, radius md, height 28px`

**MacroDock — grid layout**
```
columns:       user-configurable (default 4)
gap:           xs (4px)
empty-slot:    surface-1, border 1px hairline (dashed), radius sm — shows "+" icon in mute
```

**MacroDock — MacroKey**

Text-mode card:
```
background:       surface-2
border:           1px hairline
border-radius:    sm (4px)
height:           32px
padding:          xs sm (4px 8px)
layout:           label left, shortcut chip right
text (label):     ui-sm-strong, body

hover:
  background:     surface-3
  text (label):   ink

active (pressed):
  background:     surface-3
  border-color:   accent
```

Keyboard shortcut chip (Raycast keycap pattern):
```
background:    surface-3
border:        1px hairline
border-radius: xs (2px)
padding:       0 xxs (0 2px)
height:        16px
text:          ui-xs, mute
symbols:       ⌘ ⌥ ⌃ ⇧ — render as glyphs, not text
```

Icon-mode card:
```
height:   40px
padding:  xs (4px)
icon:     20px, centered, body color
hover:    icon ink
```

---

### StatusBar

```
background:    canvas
height:        22px
border-top:    1px hairline
padding:       0 sm (0 8px)
```

| Zone | Content | Style |
|------|---------|-------|
| Left | Active project / lane path | `ui-xs`, `body` |
| Center | Build / git status | `ui-xs`, `mute` |
| Right | Claude model · token count · error indicator | `ui-xs`, `mute` |

- Error: small `error`-colored dot + `ui-xs error` colored text in the right zone.
- Claude running: `claude-active` dot next to model name.

---

### ToastView

```
background:    surface-4  (float_panel_bg)
border:        1px hairline
border-radius: lg (8px)
padding:       sm md (8px 12px)
max-width:     360px
text:          ui-sm, ink
```

Severity left border (3px):
```
error:   3px solid error   (#e06060)
warning: 3px solid warning (#d4a853)
success: 3px solid success (#4aaf78)
info:    3px solid accent  (#5e6ad2)
```

- Slides in from bottom-right of the window.
- Max 3 toasts stacked; `xs` (4px) gap between.
- Dismiss button: `×` at `ui-sm mute`, top-right of toast.

**Auto-dismiss timing:**
```
info:    3s then fade out
success: 3s then fade out
warning: 8s then fade out  (user should notice)
error:   manual dismiss only  (must be explicitly closed)
```

**Toast action button** (optional, right of message body):
```
background: surface-3
text:       accent, ui-sm-strong
radius:     md (6px)
height:     22px
padding:    0 sm
```

---

### ModalView

```
overlay:       canvas at 50% opacity (scrim)
card:
  background:    surface-4  (float_panel_bg — the same rung as popovers)
  border:        1px hairline
  border-radius: lg (8px)
  shadow:        yes (level-4 exception)
  padding:       xl (24px)
  max-width:     480px
```

- Title: `ui-lg-strong`, `ink`.
- Body: `ui-md`, `body`.
- Primary button: `accent` fill, `accent-fg` text, `radius md`.
- Secondary button: `surface-3` fill, `body` text, `radius md`.
- Destructive button: `error` fill, `#ffffff` text, `radius md`.
- Button row: right-aligned, `xs` (4px) gap between buttons.

---

## AgentStatusBadge

Agent status indicator using the Cursor AI timeline palette, adapted for cool dark backgrounds.

**Badge anatomy:**
```
[dot 6px] [label ui-xs-strong]
dot:   radius pill, color = state color
label: ui-xs-strong, color = state color
bg:    transparent (no background — inherits row color)
```

| State | Dot color | Animation |
|-------|-----------|-----------|
| Idle | — | no badge |
| Thinking | `agent-thinking` #8faacc | slow pulse (2s) |
| Reading | `agent-reading` #8fcca8 | slow pulse |
| Editing | `agent-editing` #b09bcc | slow pulse |
| Running tool | `agent-running` #ccaa6e | fast pulse (0.8s) |
| Done | `claude-done` #4aaf78 | static, auto-hides after 3s |
| Error | `claude-error` #e06060 | static, manual dismiss |

**Pulse animation:**
```
slow:  opacity 1.0 → 0.4 → 1.0, duration 2s, ease-in-out, infinite
fast:  opacity 1.0 → 0.3 → 1.0, duration 0.8s, ease-in-out, infinite
```

Separate from the WorktreeRow Claude state badge (6px dot only). AgentStatusBadge is used in the RightDock detail view.

---

## Git & Diff Colors

**GitChangesView — file status rows**

```
row-height:   24px
padding:      0 sm (8px)
icon-size:    12px, left of filename
filename:     ui-sm, ink
status-icon:  color = git status color (see below)
```

| Status | Icon color | Token |
|--------|-----------|-------|
| Staged (M) | `git-staged` (#4aaf78) | green |
| Modified (M) | `git-modified` (#d4a853) | amber |
| Untracked (?) | `git-untracked` (#8a8f98) | gray |
| Deleted (D) | `git-deleted` (#e06060) | red |
| Renamed (R) | `git-renamed` (#7b9fd4) | blue |
| Conflict (C) | `git-conflict` (#e06060) | red (bold icon) |

**Staged / Unstaged section header:**
```
text:         label token (ALL-CAPS, 10px, 0.9px tracking)
color:        mute (#8a8f98)
padding:      sm sm (8px top, 8px left)
border-bottom: 1px hairline
```

**Diff viewer (inline / FileViewPane):**
```
added line:
  background: diff-add-bg   (rgba green 12%)
  line-number: diff-add-fg  (#4aaf78)
  "+" gutter marker: diff-add-fg

deleted line:
  background: diff-del-bg   (rgba red 12%)
  line-number: diff-del-fg  (#e06060)
  "-" gutter marker: diff-del-fg

hunk header (@@ line):
  background: surface-2 (#141516)
  text:       diff-hunk (#62666d), ui-xs, italic
  border-left: 2px solid hairline

unchanged line:
  background: canvas (#010102)
  text:       body (#d0d6e0)
```

---

## Syntax Highlighting

Code readability is its **own design axis**, deliberately decoupled from the
brand palette above. The brand accent governs *chrome*; the syntax palette
governs *code legibility* — figure/ground against body text, signal/noise
(comments recede, structure stands out), token-class separation, and
colour-vision-deficiency safety. The two are tuned independently and never
borrowed from each other.

**Selectable, not fixed.** The syntax palette is chosen via
`config.file_viewer.syntax_theme` and applied live (no Save). 14 curated
families ship; the dropdown shows the family name (light/dark variant is
chosen automatically — see below), and the default is **Daruda**, a
readability-tuned palette built for the `canvas` background.

| Family (config slug) | Origin |
|---|---|
| **Daruda** (`daruda`, default) | daruda's own readability-tuned palette |
| One (`one-dark`) · Tokyo Night · Catppuccin · Dracula · GitHub · Material · Monokai · Nord · Gruvbox · Solarized · Ayu · Night Owl · Darcula (IntelliJ) | ported from the named editor themes, verified against their authoritative sources |

**One source feeds both render paths.** `palette::syntax_theme_of(palette,
is_light)` produces a `SyntaxTheme`; it seeds the raw editor (through
`gpui_component`'s `highlight_theme`) and the diff view (through injected
spans) identically. `bucket_for_capture()` is the single tree-sitter
capture → semantic-bucket map — colour, non-color channel, and the editor's
`SyntaxColors` all read through it so the two paths can't drift.

**Light/dark auto-pairing.** A palette is not a fixed colour set — it has a
dark and a light variant, picked from the editor background's lightness
(`DarudaTheme::is_dark()`). A dark UI gets the dark variant; `daruda_light`
gets the light one (One Light, Solarized Light, Catppuccin Latte, Daruda
Light, …). Families without a published light theme fall back to Daruda
Light. This keeps syntax legible in both appearances instead of washing out
a dark palette on a near-white surface.

**Three readability rules (the Daruda default embodies them):**
1. **Figure/ground over background-contrast.** The near-black `canvas`
   already gives every token ample contrast; the real risk is low-chroma
   tokens (`function`, `string.special`) collapsing toward gray and
   disappearing *into the body text*. Meaning-bearing buckets stay at a
   chroma that reads as a distinct colour, not a tinted gray.
2. **A non-color channel carries structure.** Keyword **bold** and comment
   *italic* (and bold string escapes) encode token class without relying on
   hue alone — robust under colour-vision deficiency and low chroma. Each
   palette owns its own bold/italic per bucket.
3. **Comments recede, never vanish.** Comments are the one bucket dimmed on
   purpose (signal/noise), but never below the readability floor — lifted
   where a ported theme's own comment colour falls under it on the target
   background.

**Daruda default palette** (on `canvas`; light variant on `#f9fafb`):

| Bucket | Dark | Channel |
|---|---|---|
| keyword | `#c678dd` | **bold** |
| function | `#82aaff` | — |
| type | `#ebcb8b` | — |
| constant / number | `#d08770` | — |
| string | `#a3be8c` | — |
| string.special (escape/regex) | `#7fdbca` | **bold** |
| tag | `#bf616a` | — |
| comment | `#65737e` | *italic* |
| default (variable/operator/punct) | `#c0c5ce` | — |

Operators, punctuation, and plain variables intentionally inherit the
default foreground — colouring them is line noise, not signal.

---

## File Viewer (FileViewPane)

```
background:      canvas (#010102)   ← same as terminal pane
header-bg:       surface-1 (#0f1011)
header-height:   32px
header-border:   border-bottom 1px hairline
```

**Gutter (line numbers):**
```
width:        44px
background:   surface-1 (#0f1011)
text:         subtle (#62666d), terminal font 12px
border-right: 1px hairline
```

**Content area:**
```
font:         terminal (JetBrains Mono 13px / 20px)
text:         ink (#f7f8f8)
selection-bg: selection-bg (rgba accent 28%)
selection-fg: selection-fg (#f7f8f8)
```

**Syntax highlight palette** (reuses agent state colors — no extra tokens):

| Token type | Color | Maps to |
|------------|-------|---------|
| keyword | `#c0a8dd` | agent-editing lavender |
| string | `#8fcca8` | agent-reading mint |
| number | `#ccaa6e` | agent-running gold |
| comment | `#62666d` | subtle |
| type | `#8faacc` | agent-thinking steel blue |
| function | `#f7f8f8` bold | ink |
| operator | `#d0d6e0` | body |
| punctuation | `#8a8f98` | mute |

Agent state colors are reused for the syntax palette to minimize total color token count.

---

## Markdown Viewer

```
background:  canvas (#010102)
padding:     lg (16px)
max-width:   680px
```

**Heading hierarchy:**

| Level | Size | Weight | Color |
|-------|------|--------|-------|
| H1 | 18px | 600 | ink (#f7f8f8) |
| H2 | 15px | 600 | ink (#f7f8f8) |
| H3 | 13px | 600 | body (#d0d6e0) |
| H4–H6 | 12px | 500 | mute (#8a8f98) |

**Inline elements:**
```
inline-code:
  background: surface-2 (#141516)
  text:       #ccaa6e  (= agent-running gold)
  font:       terminal, 12px
  padding:    0 xs (0 4px)
  radius:     xs (2px)

link:
  color:      link (#809ff9)   -- NOT accent: ~4.1:1 fails as body text
  underline:  on hover only

strong:       ink (#f7f8f8), weight 600
em:           body (#d0d6e0), italic
```

**Block elements:**
```
code-block:
  background:   surface-1 (#0f1011)
  border:       1px hairline
  border-left:  3px solid accent (#5e6ad2)
  radius:       sm (4px)
  padding:      sm md (8px 12px)
  font:         terminal, 13px
  lang-label:   ui-xs, mute, top-right corner

blockquote:
  background:   transparent
  border-left:  3px solid hairline (#23252a)
  text:         mute (#8a8f98), italic
  padding-left: md (12px)
  margin:       sm 0

hr:
  border:       1px solid hairline (#23252a)

table:
  header-bg:    surface-2 (#141516)
  row-bg:       surface-1 (#0f1011)
  border:       1px hairline
  cell-padding: xs sm (4px 8px)
```

---

## Skill / MCP Badge

Small badges used in the RightDock SkillsView / ToolsView.

**Skill badge:**
```
height:       18px
padding:      0 xs (0 4px)
radius:       xs (2px)
font:         ui-xs (10px)
```

| State | bg | text |
|-------|-----|------|
| Active | `accent-muted` (#1e2050) | `accent` (#5e6ad2) |
| Loaded | `surface-2` (#141516) | `body` (#d0d6e0) |
| Invoked | `agent-running` 12% opacity | `agent-running` (#ccaa6e) |
| Error | `error` 12% opacity | `error` (#e06060) |

**MCP transport badge:**
```
height:       16px
padding:      0 xxs (0 2px)
radius:       xs (2px)
font:         ui-xs (10px), weight 500
```

| Transport | bg | text |
|-----------|-----|------|
| stdio | `surface-3` | `mute` |
| SSE | `agent-thinking` 15% opacity | `agent-thinking` (#8faacc) |
| HTTP | `agent-reading` 15% opacity | `agent-reading` (#8fcca8) |

Both badges use `ui-xs` text — not `label`, no ALL-CAPS.

---

## State Reference

| State | Visual treatment |
|-------|-----------------|
| Focus ring | `1px solid accent` border (gpui_component `theme.ring`) |
| Selected row (lane/tab) | `surface-2` background + `2px accent` left border |
| Hover | One step up the surface ladder from resting state |
| Disabled | `mute` text, no background change, no hover effect |
| Loading / spinner | `accent` color, 16px spinning icon |
| Drag item (dragging) | `surface-2` background, `0.6` opacity, `accent` border 1px |
| Drop target (valid) | `2px solid accent` outline on the target row/area |
| Drop target (invalid) | `2px solid error` outline |
| Drop placeholder | 2px `accent` horizontal line between rows |
| Text selected | `selection-bg` (accent 28%) background |
| Input focused | Border changes from `hairline` → `1px solid accent` |
| Pane focused | PaneHeader background `surface-1` → `surface-2` |
| Tab active | Background drops to `canvas`; `2px accent` underline |
| Claude running | `claude-active` (#f0a020) badge on WorktreeRow + StatusBar dot |
| Claude done | `claude-done` (#4aaf78) badge on WorktreeRow |
| Claude error | `claude-error` (#e06060) badge on WorktreeRow + error toast |
| Agent thinking | `agent-thinking` (#8faacc) dot + label, slow pulse |
| Agent reading | `agent-reading` (#8fcca8) dot + label, slow pulse |
| Agent editing | `agent-editing` (#b09bcc) dot + label, slow pulse |
| Agent running tool | `agent-running` (#ccaa6e) dot + label, fast pulse |
| Git staged | `git-staged` (#4aaf78) icon in GitChangesView |
| Git modified | `git-modified` (#d4a853) icon in GitChangesView |
| Git untracked | `git-untracked` (#8a8f98) icon in GitChangesView |

---

## Do's

- Keep terminal pane background at `canvas` — the terminal is not a card.
- Use `accent` for at most 3–4 visible elements simultaneously. The budget counts *emphasis*; a selected segment's fill inside a segmented control is state, not emphasis, and is exempt — the doc already blesses accent as the selection signal. **Bound the exemption by the worst case actually shipped:** the agent-chat fold editor shows 12 at once (12 three-way strips, one fill each). That is the licensed ceiling, not an open door. Unselected siblings are never exempt — no accent label, no accent border (accent as text measures 4.05:1 on `surface-1` and 3.45:1 on the `surface-4` float rung, both under the 4.5:1 floor).
- Build depth with the surface ladder; the step between levels is the signal, not a shadow.
- Keep all UI text in the 10–13px range. Dense chrome is appropriate here.
- Use `label` (ALL-CAPS, positive tracking) only for dock section headers and group names.
- Place keyboard shortcut chips (Raycast keycap pattern) on MacroKey cards, right-aligned.
- Apply the `2px accent` left border as the primary "selected" signal; background lift alone is too subtle.
- Use `radius.pill` for Claude state badges and status pills only.
- Treat the syntax palette as its own axis: tune it for code legibility (figure/ground, non-color channel, CVD), independent of the brand accent (see Syntax Highlighting).

## Don'ts

- Don't ship a neutral-gray light theme. The `daruda_light` preset is the only sanctioned light mode and its surfaces stay faint-blue cool-tinted (see Elevation & Depth → Light theme) — the cool identity holds in both appearances.
- Don't add drop shadows to any application chrome element — **except** level-4 floating surfaces (popover, menu, tooltip, dialog, palette, toast), where the ladder alone cannot carry detachment. Nothing anchored in the layout gets one.
- Don't use `accent` as a panel or large-surface background fill.
- Don't use border radius > `lg` (8px) on any in-app element.
- Don't add chromatic colors outside the defined semantic palette.
- Don't render UI labels below 10px or above 13px in docks/toolbars.
- Don't wrap the terminal pane in a card — `canvas` direct, always.
- Don't use `error` / `warning` / `success` colors for decoration; reserve them for genuine state.
- Don't use warm-tinted or neutral-gray surfaces — the cool blue-black tone is the identity.
- Don't borrow the brand accent (or agent-state colors) for syntax tokens, or vice versa — chrome and code legibility are tuned on separate axes.
- Don't ship a syntax palette that only works on dark — every family pairs a light variant (or falls back to Daruda Light) so light mode stays legible.
- Don't render code on pure-black `canvas` — use `editor-surface`; pure black halates bright glyphs (see Readability).
- Don't use pure `#fff` text or `accent` for small body text — both miss the readability band (see Readability).
- Don't change a text or surface color without re-computing the fg/bg contrast ratio for the affected pair — never eyeball it.

---

## Iteration Guide

How to apply this doc when changing daruda's UI:

1. **Colors and pixel metrics are tokens, never inline.** Every value here maps to a named constant in `daruda_terminal::ux::theme` (terminal) or `app/src/ui/theme/palette.rs` (chrome) — change the constant, not the call site. Theme-variant values (light/dark chrome) live in `assets/themes/daruda_{dark,light}.json`.
2. **Reach for surface change before chrome.** When something needs emphasis, move it one step on the surface ladder or add the `2px accent` border — don't add a shadow, a second accent, or a gradient.
3. **State colors are off-limits for decoration.** `error`/`warning`/`success`/`claude-*`/`agent-*` mean a genuine state. If you want "a nice color," you don't — use a surface step.
4. **Syntax ≠ chrome.** Touch the syntax palette only for code legibility, through `palette::syntax_theme_of` + `bucket_for_capture`; never reuse the brand accent there or vice versa.
5. **Both appearances or none.** Any new chrome surface needs a light-theme value in `daruda_light.json`, cool-tinted (faint blue), not neutral gray. Any new syntax family needs a light variant or a documented fallback.
6. **Stay in the type ladder** (Inter 400 / 500 / 600 at 10–13px for chrome; `label` is 600 uppercase). No UI label below 10px or above 13px. The agent-chat pane and its option panels are the sanctioned exception — they follow the user's `font.agent_chat_size`.
7. **A breakpoint names its parts or doesn't exist.** Responsive thresholds are derived from the widths they arbitrate (see AgentChatPane), and a text-derived part scales with its font. A bare pixel constant reads as a decision and behaves as a guess.

## Known Gaps

- **No hover-state spec for most components.** daruda documents resting / active / focused / selected; hover is "one step up the surface ladder" generically and not enumerated per component. AgentChatPane's Activity Bar is the exception — its two control classes enumerate resting / hover / selected / disabled — because that pane's colours come from the terminal palette rather than the ladder, so "one step up" names nothing there.
- **Light theme is documentation-light.** The surface ladder is defined (Elevation & Depth → Light theme), but per-component light values live only in `daruda_light.json`, not re-tabulated here.
- **Motion is barely specified.** Only the badge pulse cadences (slow/fast) and the absence of hover transitions are stated; there is no easing/duration token system.
- **Responsive/window-resize behavior is mostly implicit.** daruda is a single-window desktop app; dock collapse and min-width behavior are driven by code, not a breakpoint table. AgentChatPane's Activity Bar is the one documented breakpoint, and it is documented because it is *derived* (title floor + control-cluster budget, both font-scaled) rather than dialled in.
- **`hairline-soft` is documented but not implemented.** §Colors lists it for faint overlay borders; no code token exists, so every floating surface edges in the opaque `hairline` — 1.06:1 on `surface-4`. The shadow, not the edge, is what separates a float from what it covers. Either add the token or drop it from §Colors.
- **Radius on floating surfaces splits by size, not by a stated rule.** Small attached overlays (popover, menu, tooltip, autocomplete) take `t.radius` = 4px; large ones (dialog, palette, toast) take `t.radius_lg` = 8px. Both come from global slots shared with non-floating widgets, so neither can move independently.
- **The 3:1 component-edge floor is enforced on pane-local surfaces only.** App-chrome control edges ride the global `hairline` at ~1.2:1. AgentChatPane argues the distinction (an edge must clear 3:1 when it is the *only* thing marking a control apart from neighbouring text), but the distinction is applied by hand at each site, not by a token that makes the wrong choice hard.
- **§Border Radius's `md` row is aspirational.** It assigns 6px to "Buttons, text inputs, tab pills"; every shipped button constant is `RADIUS_SM` (4px). New controls should match their neighbours at 4px until the table is reconciled.
- **Disabled controls on pane-local surfaces take a UI-theme colour.** The vendored button's disabled tone is `mute` at 50% regardless of the surface, so a light terminal preset under a dark UI theme (or the reverse) gets an unverified pair. Every other state on those controls derives from the pane.
- **Custom user palettes are out of scope.** The syntax palette is a curated set; there is no user-defined-palette format documented.
