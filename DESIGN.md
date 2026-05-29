# DESIGN.md — daruda

> Design language for daruda: a macOS multi-agent terminal multiplexer.
> Synthesised from Linear (cool dark canvas + accent scarcity), Raycast (keycap patterns + pill-tab chrome), Warp (terminal typography), and Cursor (AI timeline palette).

---

## Philosophy

**"Cool precision."** The terminal is the hero; daruda's chrome must recede.

Three rules:
1. **Cool dark everywhere.** Near-pure black with a faint blue tint — the deepest dark surface. Never neutral gray, never warm brown.
2. **Accent through scarcity.** One chromatic accent (`#5e6ad2` lavender-blue). Used only for: active lane indicator, focus ring, primary button, Claude active badge.
3. **Elevation without shadow.** Depth comes from the 4-step surface ladder and 1 px hairline borders. Drop shadows are forbidden in application chrome.

---

## Colors

```yaml
colors:
  # Canvas — cool near-black (Linear-inspired, faint blue tint)
  canvas:        "#010102"                    # Window background; terminal pane background
  surface-1:     "#0f1011"                    # Dock panels, sidebar, card backgrounds
  surface-2:     "#141516"                    # Hover row, active tab, MacroKey resting state
  surface-3:     "#18191a"                    # Active/pressed row, input background
  surface-4:     "#1f2022"                    # Popover, context menu, tooltip
  hairline:      "#23252a"                    # 1px borders everywhere
  hairline-soft: "rgba(255,255,255,0.06)"    # Faint overlay borders (popover edges)

  # Accent — single lavender-blue (Linear)
  accent:        "#5e6ad2"   # Active lane indicator, focus ring, primary CTA, Claude running
  accent-hover:  "#828fff"   # Hovered accent elements
  accent-muted:  "#1e2050"   # Low-opacity accent fill (badge background)
  accent-fg:     "#ffffff"   # Text/icon on solid accent background

  # Text — cool light-gray scale (Linear-inspired)
  ink:           "#f7f8f8"   # Primary: active labels, tab titles, focused row
  body:          "#d0d6e0"   # Default: descriptions, inactive labels, body text
  mute:          "#8a8f98"   # Tertiary: timestamps, dock section headers, placeholders
  subtle:        "#62666d"   # Disabled: greyed-out controls, lowest emphasis

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
| 4 — Float | `surface-4` | `#1f2022` | `1px hairline-soft` | Popover, context menu, tooltip |

**Active indicator:** selected lane / focused pane uses a `2px solid accent` left border, not a background fill change alone.

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
  focus-border:  2px solid accent
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
background:    surface-4
border:        1px hairline-soft
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
  background:    surface-2
  border:        1px hairline
  border-radius: lg (8px)
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
  color:      accent (#5e6ad2)
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
| Focus ring | `2px solid accent` outline, `2px` offset |
| Selected row (lane/tab) | `surface-2` background + `2px accent` left border |
| Hover | One step up the surface ladder from resting state |
| Disabled | `mute` text, no background change, no hover effect |
| Loading / spinner | `accent` color, 16px spinning icon |
| Drag item (dragging) | `surface-2` background, `0.6` opacity, `accent` border 1px |
| Drop target (valid) | `2px solid accent` outline on the target row/area |
| Drop target (invalid) | `2px solid error` outline |
| Drop placeholder | 2px `accent` horizontal line between rows |
| Text selected | `selection-bg` (accent 28%) background |
| Input focused | Border changes from `hairline` → `2px solid accent` |
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
- Use `accent` for at most 3–4 visible elements simultaneously.
- Build depth with the surface ladder; the step between levels is the signal, not a shadow.
- Keep all UI text in the 10–13px range. Dense chrome is appropriate here.
- Use `label` (ALL-CAPS, positive tracking) only for dock section headers and group names.
- Place keyboard shortcut chips (Raycast keycap pattern) on MacroKey cards, right-aligned.
- Apply the `2px accent` left border as the primary "selected" signal; background lift alone is too subtle.
- Use `radius.pill` for Claude state badges and status pills only.
- Reuse agent state colors for syntax highlight — no extra tokens needed.

## Don'ts

- Don't introduce a light mode.
- Don't add drop shadows to any application chrome element.
- Don't use `accent` as a panel or large-surface background fill.
- Don't use border radius > `lg` (8px) on any in-app element.
- Don't add chromatic colors outside the defined semantic palette.
- Don't render UI labels below 10px or above 13px in docks/toolbars.
- Don't wrap the terminal pane in a card — `canvas` direct, always.
- Don't use `error` / `warning` / `success` colors for decoration; reserve them for genuine state.
- Don't use warm-tinted or neutral-gray surfaces — the cool blue-black tone is the identity.
