//! App-side UI palette for workspace chrome, docks, status bar, and panels.
//!
//! Terminal colours stay in `daruda_terminal::ux::theme`; this palette is
//! re-projected into `gpui_component::Theme` by the sibling bridge. Hue literals
//! follow CLAUDE.md §9: degrees [0, 360], normalized by local [`hsla`].
//!
//! The duplicate helper is intentional and does not violate the "no third hsla
//! helper" rule (CLAUDE.md §11): both approved helpers share the same
//! degree-based contract, so colours copy between app and terminal palettes
//! without unit conversion.

use gpui::Hsla;
use gpui_component::highlighter::{SyntaxColors, ThemeStyle};

/// `const`-friendly `Hsla` constructor — hue in degrees [0, 360].
///
/// Matches the helper in `daruda_terminal::ux::theme` so colours can
/// move between the two palettes without recomputing units. See the
/// module-level docs for why this is allowed.
const fn hsla(h_degrees: f32, s: f32, l: f32, a: f32) -> Hsla {
    Hsla {
        h: h_degrees / 360.0,
        s,
        l,
        a,
    }
}

/// Re-project a token at a new alpha — `const`-friendly so a tint/overlay
/// derives from its opaque parent (e.g. a 12% success fill from [`SUCCESS`])
/// instead of duplicating the H/S/L literal, keeping the single-source chain
/// intact.
pub const fn with_alpha(c: Hsla, a: f32) -> Hsla {
    Hsla {
        h: c.h,
        s: c.s,
        l: c.l,
        a,
    }
}

/// Re-project a token at a new lightness (H/S/A stay put) — lets a
/// hover/active variant derive from its base (e.g. a brighter [`SUCCESS`] for a
/// button hover) instead of a parallel `hsla()` literal in the bridge.
pub const fn with_lightness(c: Hsla, l: f32) -> Hsla {
    Hsla {
        h: c.h,
        s: c.s,
        l,
        a: c.a,
    }
}

/// Fully transparent — a structural sentinel for slots that must paint
/// nothing (e.g. a nested accordion whose parent surface shows through).
pub const TRANSPARENT: Hsla = hsla(0.0, 0.0, 0.0, 0.0);

/// Neutral overlays (`s = 0`) for background-derived elevation. Over any
/// surface at low alpha via [`with_alpha`], white lifts a dark background and
/// black recesses a light one, so a tint reads one step off the base from its
/// lightness alone. See `theme::agent_chat_tint`.
pub const OVERLAY_WHITE: Hsla = hsla(0.0, 0.0, 1.0, 1.0);
pub const OVERLAY_BLACK: Hsla = hsla(0.0, 0.0, 0.0, 1.0);
/// Alpha for the agent-chat tool-card tint — a gentle lift so the card sits
/// one step above the pane background on any background color.
pub const AGENT_CHAT_CARD_TINT_ALPHA: f32 = 0.05;
/// Alpha for the agent-chat tool-card / code-block border — a hairline one
/// step stronger than the fill tint, drawn from the same neutral overlay so
/// the edge tracks the background instead of a fixed line color.
pub const AGENT_CHAT_CARD_BORDER_ALPHA: f32 = 0.12;
/// Alpha for the resting edge of an *interactive* control on a pane-local
/// surface (the Activity Bar's chips), white over a dark background.
///
/// Deliberately far heavier than [`AGENT_CHAT_CARD_BORDER_ALPHA`], and the
/// split is the point: a tool card's edge is decoration, while a chip's edge is
/// the thing that identifies it as a control, which DESIGN.md §Readability
/// holds to 3:1. At the card's 0.12 the chip edge measures 1.44:1 on the
/// default `#1e1e1e` preset; 0.34 puts it at 3.11:1.
pub const AGENT_CHAT_CONTROL_BORDER_ALPHA_ON_DARK: f32 = 0.34;
/// The same edge, black over a light background. Higher than its dark
/// counterpart because darkening a near-white surface buys less contrast than
/// lightening a near-black one: over `#f9fafb`, 0.34 lands at 2.36:1 and 0.42 at
/// 3.02:1.
///
/// No shipped terminal preset reaches this branch — all eight are dark — so it
/// serves a user-supplied light `[colors] background`. A single shared alpha
/// would fail one direction or the other.
pub const AGENT_CHAT_CONTROL_BORDER_ALPHA_ON_LIGHT: f32 = 0.42;
/// Alpha for the code/diff editor's current-line band. Same neutral-overlay
/// technique as [`AGENT_CHAT_CARD_TINT_ALPHA`] (white lift on dark, black
/// recess on light) rather than a fixed solid colour, so the band reads
/// reasonably on either surface the editor chrome paints on — the UI-themed
/// File-viewer surface and the agent-chat diff embed's terminal-derived
/// background — without per-instance wiring. A touch stronger than the card
/// tint since the band needs to read as "this line", not just "elevated".
pub const EDITOR_ACTIVE_LINE_ALPHA: f32 = 0.10;
/// User-message bubble fill: a translucent *accent* tint (not the neutral
/// white/black used for code) so the user's turn is set off by hue while code
/// stays quiet. Accent-hued rather than neutral is the one sanctioned
/// chromatic fill (cf. `accent-muted`, DESIGN §Accent — badge fill).
pub const AGENT_CHAT_USER_TINT: Hsla = with_alpha(PRIMARY, 0.22);

/// Mid-gray target the inactive-pane dim blends toward, matching
/// `daruda_terminal`'s `DIM_GRAY_LEVEL`. Kept in step so an inactive agent-chat
/// pane grays to the exact tone of an inactive terminal pane.
pub const DIM_GRAY_LEVEL: f32 = 0.3;

// ============================================================================
// Design tokens — primitive colour literals (single source of truth)
// ============================================================================

// Lifted off pure black so the window frame isn't harsher than its content.
// Hue/saturation match the SURFACE_* ladder (faint cyan-blue 210, not a 240
// navy which reads visibly navy at this lightness). Values are the exact HSL
// of hex #070809 so the theme survives its serialize→hex→parse round-trip.
pub const CANVAS: Hsla = hsla(210.0, 0.125, 0.0314, 1.0);
pub const SURFACE_1: Hsla = hsla(210.0, 0.063, 0.063, 1.0);
pub const SURFACE_2: Hsla = hsla(210.0, 0.048, 0.082, 1.0);
pub const SURFACE_3: Hsla = hsla(210.0, 0.040, 0.098, 1.0);
/// Code-viewing surface (file viewer + diff). One rung above `CANVAS` so the
/// editor reads as its own plane and the syntax palette clears the contrast
/// floor — see DESIGN §Readability.
pub const EDITOR_SURFACE: Hsla = hsla(220.0, 0.10, 0.05, 1.0);

// Light-theme surface ladder — cool-tinted near-whites (faint blue, never
// neutral gray), mirroring the dark ladder. Used by `apply_daruda_palette` and
// theme-variant render helpers so light mode doesn't fall through to the dark
// consts above. Matches `daruda_light.json`.
pub const LIGHT_CANVAS: Hsla = hsla(222.0, 0.10, 0.954, 1.0);
pub const LIGHT_SURFACE_1: Hsla = hsla(222.0, 0.09, 0.907, 1.0);
pub const LIGHT_SURFACE_2: Hsla = hsla(222.0, 0.085, 0.879, 1.0);
pub const LIGHT_SURFACE_3: Hsla = hsla(222.0, 0.08, 0.846, 1.0);
/// Near-black ink for text on light surfaces (inverse of `INK`). Used by
/// render sites that read a raw text const instead of a theme-variant field.
pub const LIGHT_INK: Hsla = hsla(222.0, 0.14, 0.12, 1.0);
pub const HAIRLINE: Hsla = hsla(223.0, 0.091, 0.151, 1.0);
pub const ACCENT: Hsla = hsla(233.8, 0.563, 0.596, 1.0);
pub const INK: Hsla = hsla(0.0, 0.0, 0.97, 1.0);
pub const TEXT_BODY: Hsla = hsla(218.0, 0.089, 0.847, 1.0);
pub const TEXT_MUTE: Hsla = hsla(218.0, 0.064, 0.569, 1.0);
pub const TEXT_SUBTLE: Hsla = hsla(218.0, 0.053, 0.5, 1.0);

// ============================================================================
// DESIGN.md color tokens — complete palette (synced from DESIGN.md §Colors)
// ============================================================================

/// Popover, context menu, tooltip background  (#1f2022).
pub const SURFACE_4: Hsla = hsla(210.0, 0.046, 0.128, 1.0);
/// Hovered accent elements (#828fff).
pub const ACCENT_HOVER: Hsla = hsla(233.8, 1.0, 0.755, 1.0);
/// Low-opacity accent fill — badge background (#1e2050).
pub const ACCENT_MUTED: Hsla = hsla(237.6, 0.45, 0.216, 1.0);
/// Text/icon on solid accent background (#ffffff).
pub const ACCENT_FG: Hsla = hsla(0.0, 0.0, 1.0, 1.0);

// ---------------------------------------------------------------------------
// Agent action states (Cursor timeline palette, dark-adapted)
// ---------------------------------------------------------------------------

/// Gold — Executing tool / running command (#ccaa6e).
pub const AGENT_RUNNING: Hsla = hsla(38.4, 0.480, 0.616, 1.0);

// ---------------------------------------------------------------------------
// Git status
// ---------------------------------------------------------------------------

pub const GIT_STAGED: Hsla = SUCCESS; // #4aaf78
pub const GIT_MODIFIED: Hsla = WARNING; // #d4a853
pub const GIT_UNTRACKED: Hsla = TEXT_MUTE; // #8a8f98
pub const GIT_DELETED: Hsla = ERROR; // #e06060
pub const GIT_RENAMED: Hsla = hsla(215.6, 0.509, 0.657, 1.0); // #7b9fd4
pub const GIT_CONFLICT: Hsla = ERROR; // #e06060

// ---------------------------------------------------------------------------
// Diff line-level
// ---------------------------------------------------------------------------

pub const DIFF_ADD_BG: Hsla = with_alpha(SUCCESS, 0.12);
pub const DIFF_ADD_FG: Hsla = SUCCESS;
pub const DIFF_DEL_BG: Hsla = with_alpha(ERROR, 0.12);
pub const DIFF_DEL_FG: Hsla = ERROR;
pub const DIFF_HUNK: Hsla = TEXT_SUBTLE; // #797e86

// ---------------------------------------------------------------------------
// Selection
// ---------------------------------------------------------------------------

pub const SELECTION_BG: Hsla = with_alpha(PRIMARY, 0.28);

// ---------------------------------------------------------------------------
// Semantic — general UI
// ---------------------------------------------------------------------------

pub const SUCCESS: Hsla = hsla(147.3, 0.406, 0.488, 1.0); // #4aaf78
pub const WARNING: Hsla = hsla(39.5, 0.600, 0.578, 1.0); // #d4a853
pub const ERROR: Hsla = hsla(0.0, 0.674, 0.627, 1.0); // #e06060

// ---------------------------------------------------------------------------
// Signal palette — vivid status colors
// ---------------------------------------------------------------------------

// Higher-chroma than the muted SUCCESS/WARNING/ERROR set so threshold
// gauges, incident pills, and the running-task indicator visually pop.
// Kept as their own tokens rather than reusing the semantic set, which
// DESIGN intentionally keeps desaturated.
pub const SIGNAL_GREEN: Hsla = hsla(135.0, 0.59, 0.49, 1.0);
pub const SIGNAL_YELLOW: Hsla = hsla(50.0, 1.0, 0.52, 1.0);
pub const SIGNAL_ORANGE: Hsla = hsla(33.0, 1.0, 0.52, 1.0);
pub const SIGNAL_RED: Hsla = hsla(4.0, 1.0, 0.62, 1.0);

// ============================================================================
// Role tokens — semantic aliases over design tokens
// ============================================================================

// Surface roles — elevation ladder (DESIGN §Elevation).
pub const BG_BASE: Hsla = CANVAS;
/// Surface for code-rendering panes (file viewer + diff). Distinct from
/// `BG_BASE` so the editor reads on a gentle dark-gray, not pure black.
pub const BG_EDITOR: Hsla = EDITOR_SURFACE;
pub const BG_PANEL: Hsla = SURFACE_1;
pub const BG_RAISED: Hsla = SURFACE_2;
/// Background for anything that floats above the workspace — popovers, menus,
/// tooltips, dialogs, the command palette, toasts. One rung above every panel
/// they can cover, so "floating" is carried by the surface ladder and not by
/// the drop shadow alone (DESIGN.md §Elevation level 4).
pub const BG_FLOAT: Hsla = SURFACE_4;
pub const BG_HOVER: Hsla = SURFACE_2;
pub const BG_ACTIVE: Hsla = SURFACE_3;
pub const BORDER: Hsla = HAIRLINE;

// Text roles — the only four sanctioned UI text tones (DESIGN §Colors:
// ink / body / mute / subtle), all carrying the cool-blue design tint.
// Feature constants must reference these, never neutral-gray (`hsla(0,0,L)`)
// literals, which DESIGN §Don'ts forbids.
pub const TEXT_PRIMARY: Hsla = INK; // titles, active labels, focused rows
pub const TEXT_SECONDARY: Hsla = TEXT_BODY; // body copy, descriptions, previews
pub const TEXT_TERTIARY: Hsla = TEXT_MUTE; // section headers, muted metadata
pub const TEXT_DISABLED: Hsla = TEXT_SUBTLE; // empty states, lowest emphasis

// Action role — the single chromatic accent (DESIGN §Colors: accent),
// surfaced under a clearer semantic name. Feature constants and render
// reference `PRIMARY` for primary CTA / focus ring / active indicator /
// info tint; [`ACCENT`] stays as the raw design-token value and
// is referenced only here.
pub const PRIMARY: Hsla = ACCENT;

/// Link role — clickable *text*: markdown links and the agent-chat diff header's
/// file path. Not [`PRIMARY`]: DESIGN §Saturation on dark surfaces bans `accent`
/// as body text (~4.1:1); this reaches ~7.4:1 on [`CANVAS`]. One token so the
/// link sites can't drift apart again (they borrowed `ACCENT` / [`GIT_RENAMED`]).
pub const LINK: Hsla = hsla(224.6, 0.905, 0.738, 1.0);

// Overlay roles — white lift at fixed alphas for row hover/active fills that
// must read over any underlying surface. One step per intensity so every
// list/row reuses the same lift instead of a bespoke alpha.
pub const OVERLAY_HOVER: Hsla = hsla(0.0, 0.0, 1.0, 0.04); // pointer hover row
pub const OVERLAY_SELECTED: Hsla = hsla(0.0, 0.0, 1.0, 0.06); // selected / expanded row
pub const OVERLAY_ACTIVE: Hsla = hsla(0.0, 0.0, 1.0, 0.08); // active row, faint border/separator
pub const OVERLAY_PROMINENT: Hsla = hsla(0.0, 0.0, 1.0, 0.10); // pill fill, settings active row

// State roles — semantic status colors are the DESIGN tokens themselves
// ([`SUCCESS`] / [`WARNING`] / [`ERROR`]) plus [`PRIMARY`] for info. Feature
// constants alias these directly; see the claude/git/diff sections below.

// ============================================================================
// Workspace chrome — title bar, tab bar, status bar, docks
// ============================================================================

/// Status-bar dot that lights up when the workspace's project layer has a
/// `config.toml` on disk. Cyan-ish accent so it reads as informational, not
/// an alert.
pub const STATUS_BAR_PROJECT_DOT: Hsla = hsla(180.0, 0.55, 0.55, 1.0);

/// Inline "detached" chip background and text — shown next to the branch
/// label on a detached HEAD. Amber-leaning so it signals attention without
/// rising to the red-error tier.
pub const STATUS_BAR_DETACHED_BG: Hsla = with_lightness(WARNING, 0.22);
pub const STATUS_BAR_DETACHED_TEXT: Hsla = with_lightness(WARNING, 0.72);

/// Dock view tab strip — horizontal padding per tab (px).
pub const DOCK_VIEW_TAB_PAD_X: f32 = PAD_LG;

/// Dock view tab strip — font size (px).
pub const DOCK_VIEW_TAB_FONT_SIZE: f32 = FONT_SIZE_SM;

// Lanes list (left dock Lanes view)
/// Lanes list — horizontal padding (px).
pub const LANE_ROW_PAD_X: f32 = PAD_LG;
/// Lanes list — primary label font size (px).
pub const LANE_LABEL_FONT_SIZE: f32 = FONT_SIZE_MD;
/// Lanes list — secondary (path / status) font size (px).
pub const LANE_SUB_FONT_SIZE: f32 = FONT_SIZE_XS;
/// Lanes list — section header font size (px).
pub const LANE_SECTION_HEADER_FONT_SIZE: f32 = 10.5;
/// Lanes list — section header top/bottom padding (px).
pub const LANE_SECTION_PAD_Y: f32 = PAD_SM;
/// Lanes list — placeholder (non-git info box) padding (px).
pub const LANE_PLACEHOLDER_PAD: f32 = 12.0;
/// Drag ghost row vertical padding (px) — space above/below label in the
/// floating preview that follows the cursor during a lane drag.
pub const LANE_DRAG_GHOST_PAD_Y: f32 = PAD_XS;
/// Shared blue tint for every valid drag-drop target highlight. The
/// three surfaces below differ only in opacity (row vs. input panel vs.
/// terminal pane), so they derive from this one base via [`with_alpha`].
pub const DROP_TARGET_TINT: Hsla = hsla(210.0, 0.50, 0.30, 1.0);
/// Highlight color applied to a drop target row while a lane is being
/// dragged over it.
pub const LANE_DROP_TARGET_BG: Hsla = with_alpha(DROP_TARGET_TINT, 0.35);
/// Rejection tint applied when the in-flight payload cannot land on
/// the hovered row (cross-project lane drag, group dropped on a
/// grouped project, etc.). Desaturated red at low alpha so it reads
/// as "not here" rather than as a hard error.
pub const LANE_DROP_TARGET_REJECTED_BG: Hsla = hsla(0.0, 0.55, 0.32, 0.20);
/// Unread indicator dot diameter (px).
pub const LANE_UNREAD_DOT_SIZE: f32 = 6.0;
/// Unread indicator dot corner radius (px).
pub const LANE_UNREAD_DOT_RADIUS: f32 = RADIUS_XS;
/// Gap between label elements within the primary label row (px).
pub const LANE_LABEL_GAP: f32 = GAP_STANDARD;
/// Gap between elements in the sub-label row (px).
pub const LANE_SUBLABEL_GAP: f32 = GAP_STANDARD;
/// Gap between the body and the × remove button within a row (px).
pub const LANE_ROW_GAP: f32 = GAP_SM;
/// Top margin of the "git init" affordance inside the non-git placeholder (px).
pub const LANE_PLACEHOLDER_GIT_INIT_MT: f32 = PAD_XS;
/// Gap between lines inside the non-git info placeholder (px).
pub const LANE_PLACEHOLDER_LINE_GAP: f32 = GAP_STANDARD;
/// Diameter of the optional color dot rendered in a group header (px).
pub const LANE_GROUP_COLOR_DOT_SIZE: f32 = 8.0;
/// Corner radius of the group color dot (px). Half the size to render a circle.
pub const LANE_GROUP_COLOR_DOT_RADIUS: f32 = RADIUS_SM;

// ----------------------------------------------------------------------------
// Premium Card surface tokens (Lanes redesign)
// ----------------------------------------------------------------------------

/// Lanes card — outer corner radius (px). Capped at `lg` (8px) per
/// DESIGN §Border Radius ("no radius exceeds 8px in application chrome").
pub const LANE_CARD_RADIUS: f32 = RADIUS_LG;
/// Lanes card — vertical gap between adjacent cards (px).
pub const LANE_CARD_GAP: f32 = GAP_STANDARD;
/// Lanes card — inner horizontal padding (px).
pub const LANE_CARD_PAD_X: f32 = PAD_SM;
/// Lanes card — inner vertical padding (px).
pub const LANE_CARD_PAD_Y: f32 = PAD_SM;
/// Lanes row — corner radius applied to hover/active background fills
/// so the highlight reads as a rounded chip instead of a hard rectangle.
pub const LANE_ROW_RADIUS: f32 = RADIUS_MD;
/// Lanes card — horizontal outer margin so cards don't hug the dock
/// edges; gives the surface visible left/right breathing room.
pub const LANE_CARD_MARGIN_X: f32 = PAD_STANDARD;
/// Lanes list — vertical gap between adjacent lane rows inside
/// a project block so consecutive rows don't read as a single block.
pub const LANE_LIST_GAP_Y: f32 = 3.0;
/// Lanes list — horizontal indent step (px). Each hierarchy level sits one
/// step deeper than its parent (lane rows one step right of their project
/// header). Applied to the lane list container, not per-row.
pub const LANE_INDENT_STEP: f32 = 8.0;
/// Lanes card — border width (px).
pub const LANE_CARD_BORDER_W: f32 = 1.0;
/// Active lane row — left accent border width (px). Renders as the primary
/// selection signal on the active lane row; inactive rows reserve the same
/// space with a transparent border so label x-position stays stable.
pub const LANE_ACTIVE_BORDER_W: f32 = 2.0;
/// Group label font size (px) — uppercase eyebrow.
pub const LANE_GROUP_LABEL_FONT_SIZE: f32 = FONT_SIZE_SM;

/// Project header — branch chip horizontal padding (px). Matches the git
/// badge pill padding so chips on the same row have consistent weight.
pub const LANE_BRANCH_CHIP_PAD_X: f32 = PAD_XS;
/// Project header — branch chip vertical padding (px). Zero keeps the chip
/// flush with the row's line-height, same as the git badge pill.
pub const LANE_BRANCH_CHIP_PAD_Y: f32 = 0.0;
/// Project header — branch chip corner radius (px). Matches `RADIUS_SM` for
/// consistency with other pill-shaped chips in the lanes list.
pub const LANE_BRANCH_CHIP_RADIUS: f32 = RADIUS_SM;
/// Project header — branch chip border width (px). 1 px hairline, same as
/// card borders across the lanes list.
pub const LANE_BRANCH_CHIP_BORDER_W: f32 = LANE_CARD_BORDER_W;

// ============================================================================
// Workspace chrome (modal, dock, banners, settings, agent panels, etc.)
// ============================================================================

/// Modal backdrop dim alpha (0..1).
pub const MODAL_BACKDROP_ALPHA: f32 = 0.50;
/// Modal panel corner radius (px).
pub const MODAL_PANEL_RADIUS: f32 = RADIUS_LG;
/// Modal panel width (px).
pub const MODAL_PANEL_WIDTH: f32 = 420.0;
/// Modal panel inner padding (px).
pub const MODAL_PANEL_PAD: f32 = 16.0;
/// Modal title font size (px).
pub const MODAL_TITLE_FONT_SIZE: f32 = FONT_SIZE_LG;
/// Modal body font size (px).
pub const MODAL_BODY_FONT_SIZE: f32 = FONT_SIZE_MD;
/// Modal input padding (px).
pub const MODAL_INPUT_PAD: f32 = PAD_STANDARD;
/// Modal button padding X (px).
pub const MODAL_BUTTON_PAD_X: f32 = PAD_XL;
/// Modal button padding Y (px).
pub const MODAL_BUTTON_PAD_Y: f32 = PAD_SM;
/// Modal button corner radius (px).
pub const MODAL_BUTTON_RADIUS: f32 = RADIUS_SM;
/// Vertical gap between the rows inside a modal panel (title / body /
/// input / error / footer).
pub const MODAL_PANEL_GAP: f32 = PAD_LG;
/// Gap between the buttons in a modal footer (Cancel | Confirm).
pub const MODAL_FOOTER_GAP: f32 = GAP_LG;
/// Top margin of the footer row inside the panel.
pub const MODAL_FOOTER_MARGIN_TOP: f32 = PAD_SM;
/// Minimum height of the Notes textarea inside `EditTaskModal` (px). Shares
/// the Prompt's `flex_1` column but is biased smaller (notes are short); this
/// guarantees ~6 lines without letting the row dominate the column.
pub const MODAL_NOTES_TEXTAREA_MIN_H: f32 = 120.0;
/// Radio-button indicator column width in the merge modal (px).
/// Wide enough to contain "●" / "○" at MODAL_BODY_FONT_SIZE.
pub const MODAL_RADIO_W: f32 = 14.0;
/// Wide form modal width (px). Used for split layouts (e.g. Skills:
/// metadata form on the left + markdown body editor on the right).
pub const FORM_MODAL_WIDE: f32 = 900.0;
/// Vertical gap between sections inside a form modal column (px).
pub const FORM_MODAL_SECTION_GAP: f32 = 12.0;
/// Horizontal gap between the left/right columns of a split-form
/// modal body (px).
pub const FORM_MODAL_SPLIT_GAP: f32 = 16.0;
/// Minimum gap kept between an imperatively-opened `PopupMenu` (System
/// B — `crate::ui::popup_menu_deferred`) and the window edge when
/// `snap_to_window_with_margin` repositions it away from a clipping
/// anchor.
pub const POPUP_MENU_DEPLOY_EDGE_MARGIN: f32 = 8.0;
/// Banner background — error severity. Red hue, low alpha so the
/// underlying `MODAL_PANEL_BG` shows through.
pub const BANNER_ERROR_BG: Hsla = with_alpha(ERROR, 0.10);
/// Banner text + icon color — error severity.
pub const BANNER_ERROR_TEXT: Hsla = with_lightness(ERROR, 0.70);
/// Banner background — warning severity. Amber hue.
pub const BANNER_WARNING_BG: Hsla = with_alpha(WARNING, 0.10);
/// Banner text + icon color — warning severity.
pub const BANNER_WARNING_TEXT: Hsla = with_lightness(WARNING, 0.70);
/// Banner background — info severity (DESIGN: info = accent).
pub const BANNER_INFO_BG: Hsla = with_alpha(PRIMARY, 0.10);
/// Banner text + icon color — info severity.
pub const BANNER_INFO_TEXT: Hsla = with_lightness(PRIMARY, 0.75);
/// Banner background — success severity.
pub const BANNER_SUCCESS_BG: Hsla = with_alpha(SUCCESS, 0.10);
/// Banner text + icon color — success severity.
pub const BANNER_SUCCESS_TEXT: Hsla = with_lightness(SUCCESS, 0.65);
/// Fixed width of the label column in settings form rows (px).
pub const SETTINGS_LABEL_W: f32 = 120.0;

/// Width of the section-nav sidebar (px). Sized to fit `Claude Status`
/// (the longest builtin nav label) at the body font size with comfort.
pub const SETTINGS_SIDEBAR_W: f32 = 168.0;
/// Dock background — slightly darker than the panel body so the
/// active row's highlight reads cleanly.
pub const SETTINGS_SIDEBAR_BG: Hsla = with_alpha(CANVAS, 0.18);
/// Vertical padding inside the left dock list.
pub const SETTINGS_SIDEBAR_PAD_Y: f32 = PAD_SM;
/// Per-row horizontal padding inside the left dock.
pub const SETTINGS_SIDEBAR_ROW_PAD_X: f32 = PAD_XL;
/// Per-row vertical padding inside the left dock.
pub const SETTINGS_SIDEBAR_ROW_PAD_Y: f32 = PAD_SM;
/// Width of the master (left) column in the Settings → Plugin
/// master-detail layout. Wide enough for `<plugin_local>` + `N skills`
/// at body font size without forcing a wrap on common Claude plugin
/// names.
pub const SETTINGS_PLUGIN_MASTER_W: f32 = 220.0;
/// Fixed label-column width inside the right-pane detail rows
/// (`Marketplace`, `Version`, `Path`, ...). Matches `SETTINGS_LABEL_W`
/// in spirit but slightly narrower since the detail labels are shorter
/// than the form-field labels.
pub const SETTINGS_PLUGIN_LABEL_W: f32 = 110.0;
/// macOS traffic light X offset from window left edge.
pub const TRAFFIC_LIGHT_X: f32 = 8.0;
/// macOS traffic light Y offset from window top edge.
pub const TRAFFIC_LIGHT_Y: f32 = 6.0;
/// Width reserved for traffic lights in the title bar.
pub const TRAFFIC_LIGHT_WIDTH: f32 = 70.0;
/// Standard button height (px) — DESIGN.md §Fixed Heights "Button (standard): 28px".
/// Applied to Submit and secondary action buttons in the bottom-dock input chrome.
pub const BUTTON_HEIGHT: f32 = 28.0;
/// Command palette panel width (px).
pub const PALETTE_WIDTH: f32 = 500.0;
/// Command palette top offset from window (px).
pub const PALETTE_TOP_OFFSET: f32 = 40.0;
/// "Staged" / "Changes" section label font size (px).
pub const GIT_SECTION_FONT_SIZE: f32 = 10.5;
/// File row height (px).
pub const GIT_FILE_ROW_HEIGHT: f32 = 28.0;
/// File row horizontal padding (px).
pub const GIT_FILE_ROW_PAD_X: f32 = PAD_LG;
/// Gap between status char and file name within a row (px).
pub const GIT_FILE_ROW_GAP: f32 = GAP_STANDARD;
/// Status char column width (px).
pub const GIT_STATUS_CHAR_W: f32 = 14.0;
/// Git status badge font size shown in the Lanes list (px).
pub const GIT_BADGE_FONT_SIZE: f32 = FONT_SIZE_XS;
/// Git badge — pill corner radius (px). Rolled-up "chip" rounding.
pub const GIT_BADGE_PILL_RADIUS: f32 = 9.0;
/// Git badge — pill horizontal padding (px).
pub const GIT_BADGE_PILL_PAD_X: f32 = PAD_SM;
/// Git badge — pill vertical padding (px).
pub const GIT_BADGE_PILL_PAD_Y: f32 = 0.0;
/// Git badge — minimum pill width (px) so a single-digit count still
/// reads as a chip instead of a tight circle.
pub const GIT_BADGE_PILL_MIN_W: f32 = 16.0;
/// Git badge — ahead/behind arrow icon size (px). Sized just below
/// the 10px badge font so the arrow + digit pair reads as one chip.
pub const GIT_BADGE_ARROW_SIZE: f32 = 9.0;
/// Git badge — gap between the ahead/behind groups and the pill (px).
pub const GIT_BADGE_GAP: f32 = GAP_SM;
/// Git badge — gap between an arrow icon and its number (px). Tighter
/// than the inter-group gap so each `↑N` reads as one unit.
pub const GIT_BADGE_ARROW_NUM_GAP: f32 = 1.0;
// Git-changes-view diff shares the file-viewer's diff palette so the two
// surfaces read identically; see the `FILE_DIFF_*` canonical definitions.
/// Stage checkbox box size (px).
pub const GIT_STAGE_CHECKBOX_SIZE: f32 = 13.0;
/// Stage checkbox border radius (px).
pub const GIT_STAGE_CHECKBOX_RADIUS: f32 = RADIUS_XS;
/// Stage checkbox background when staged (accent fill — matches the
/// gpui_component checkbox `primary` convention; theme-invariant).
pub const GIT_STAGE_CHECKBOX_CHECKED_BG: Hsla = ACCENT;
/// Tick glyph font size inside the stage checkbox (px).
pub const GIT_STAGE_CHECKBOX_TICK_SIZE: f32 = 9.0;
/// Git Changes header padding X (px).
pub const GIT_HEADER_PAD_X: f32 = PAD_LG;
/// Git Changes header padding Y (px).
pub const GIT_HEADER_PAD_Y: f32 = PAD_SM;
/// Directory group header vertical padding (px).
pub const GIT_DIR_HEADER_PAD_Y: f32 = GAP_XS;
/// Directory group header font size (px).
pub const GIT_DIR_HEADER_FONT_SIZE: f32 = FONT_SIZE_XS;
/// Commit footer inner padding (px).
pub const GIT_COMMIT_PAD: f32 = PAD_STANDARD;
/// Gap between Commit and Push buttons (px).
pub const GIT_COMMIT_BUTTON_GAP: f32 = GAP_STANDARD;
/// Total height of the commit footer panel (textarea + floating button bar).
/// Sized to show ~4-5 lines of commit message text.
pub const GIT_COMMIT_FOOTER_H: f32 = 128.0;
/// Commit message text area height (px) — kept for reference; layout uses GIT_COMMIT_FOOTER_H.
pub const GIT_COMMIT_INPUT_HEIGHT: f32 = 64.0;
/// Gap between remote action buttons (Fetch / Push) (px).
pub const GIT_REMOTE_BTN_GAP: f32 = GAP_SM;
/// Gap between the text area and the action button group in InputPanel (px).
pub const INPUT_PANEL_SECTION_GAP: f32 = GAP_STANDARD;
/// Gap between buttons in the InputPanel action group (px).
pub const INPUT_PANEL_BUTTON_GAP: f32 = GAP_STANDARD;
/// Horizontal inner padding of the bottom-dock textarea (px). Matches
/// DESIGN.md TerminalInputDock textarea spec: `padding: sm md (8px 12px)`.
pub const INPUT_TEXTAREA_PAD_X: f32 = 12.0;
/// Vertical inner padding of the bottom-dock textarea (px). Matches
/// DESIGN.md TerminalInputDock textarea spec: `padding: sm md (8px 12px)`.
pub const INPUT_TEXTAREA_PAD_Y: f32 = 8.0;
/// Minimum height of the TextArea inside InputPanel (px).
pub const INPUT_PANEL_MIN_H: f32 = 48.0;
/// Height of the floating action bar overlaid at the bottom of an InputPanel
/// with `ActionsFloating` layout (px). Matches Zed git_panel footer_size.
pub const INPUT_PANEL_FLOATING_BAR_H: f32 = 32.0;
/// Command palette max visible entries.
pub const PALETTE_MAX_VISIBLE: usize = 12;
/// Max configured-agent count that renders the `+` menu's agent entries
/// flat. Above this the menu folds them into a `New Agent Chat` submenu.
pub const AGENT_MENU_FLAT_MAX: usize = 5;
/// Command palette corner radius (px).
pub const PALETTE_RADIUS: f32 = RADIUS_LG;
/// Tab label font size (px).
pub const TAB_FONT_SIZE: f32 = FONT_SIZE_MD;
/// Tab minimum width (px).
pub const TAB_MIN_WIDTH: f32 = 80.0;
/// Tab maximum width (px).
pub const TAB_MAX_WIDTH: f32 = 220.0;
/// New-tab button font size (px).
pub const NEW_TAB_FONT_SIZE: f32 = FONT_SIZE_MD;
/// Pane header font size (px).
pub const PANE_HEADER_FONT_SIZE: f32 = FONT_SIZE_SM;
/// Pane header cwd basename font size (px).
pub const PANE_HEADER_CWD_FONT_SIZE: f32 = FONT_SIZE_XS;
/// Pane header close button font size (px).
pub const PANE_HEADER_CLOSE_FONT_SIZE: f32 = FONT_SIZE_XS;
/// Status bar font size (px).
pub const STATUS_BAR_FONT_SIZE: f32 = FONT_SIZE_SM;
/// Status bar horizontal padding (px). Wider than the shared `PAD_LG`
/// spacing token: macOS rounds the window's own bottom corners, and the
/// status bar sits flush against the bottom edge, so a bordered control
/// near the trailing edge (the account chip) needs extra clearance from
/// the true corner or its straight border reads as clipped by the curve.
pub const STATUS_BAR_PAD_X: f32 = 24.0;
/// Dock placeholder message font size (px).
pub const DOCK_PLACEHOLDER_FONT_SIZE: f32 = FONT_SIZE_MD;
/// Dock toggle icon size (px).
pub const DOCK_ICON_SIZE: f32 = 13.0;
/// Dock toggle icon button width (px).
pub const DOCK_ICON_BUTTON_W: f32 = 24.0;
/// Dock toggle icon button height (px).
pub const DOCK_ICON_BUTTON_H: f32 = 20.0;
/// Dock toggle icon button corner radius (px).
pub const DOCK_ICON_BUTTON_RADIUS: f32 = RADIUS_SM;
/// Dock toggle icon group right margin (px).
pub const DOCK_ICON_GROUP_MR: f32 = PAD_STANDARD;
/// Side length of a `button_chip` (px). Single-glyph chip buttons
/// (`+`, `1`, `2`, `3`) sized to a uniform square so adjacent chips
/// read as a row of equal-weight controls regardless of glyph width.
/// Sized to sit inside the 28-px tab bar with comfortable margin.
pub const BUTTON_CHIP_SIZE: f32 = 20.0;
/// Panel body horizontal padding (px).
pub const PANEL_BODY_PAD_X: f32 = PAD_STANDARD;
/// Panel body vertical padding (px).
pub const PANEL_BODY_PAD_Y: f32 = PAD_STANDARD;
/// Gap between widgets in flex_wrap layout (px).
pub const PANEL_BODY_GAP: f32 = GAP_STANDARD;
/// Highlight overlay applied to the terminal input panel body while a file
/// (internal `PathDrag` or Finder `ExternalPaths`) is dragged over it.
pub const INPUT_PANEL_DROP_TARGET_BG: Hsla = with_alpha(DROP_TARGET_TINT, 0.20);
/// Highlight overlay painted over a terminal pane while a file is dragged
/// over it. Slightly more opaque than the input panel variant so it remains
/// visible against the terminal background.
pub const TERMINAL_DROP_TARGET_BG: Hsla = with_alpha(DROP_TARGET_TINT, 0.30);
/// Macro button height (px) — shared by Text and Icon modes so a
/// mixed-mode grid row aligns. Text width is content-driven by default
/// but overridden to [`BUTTON_WIDGET_TILE_WIDTH`] when the tile renders
/// inside the bottom-dock grid (uniform fixed cells).
pub const BUTTON_WIDGET_HEIGHT: f32 = 32.0;
/// Text-mode macro tile width (px) when rendered in the bottom-dock
/// fixed-column grid. Long labels truncate.
pub const BUTTON_WIDGET_TILE_WIDTH: f32 = 96.0;
/// Icon-mode macro button width (px). Height is taken from
/// [`BUTTON_WIDGET_HEIGHT`] so a mixed-mode grid row aligns; set this
/// equal to `BUTTON_WIDGET_HEIGHT` for a square tile.
pub const BUTTON_WIDGET_ICON_SIZE: f32 = 32.0;
/// Bottom-dock `[+]` add-tile dashed border width (px).
pub const BUTTON_WIDGET_ADD_BORDER_W: f32 = 1.0;
/// Macro button label / icon font size (px).
pub const BUTTON_WIDGET_FONT_SIZE: f32 = FONT_SIZE_MD;
/// Macro button horizontal padding for text mode (px).
pub const BUTTON_WIDGET_PAD_X: f32 = PAD_LG;
/// Macro button corner radius (px).
pub const BUTTON_WIDGET_RADIUS: f32 = RADIUS_SM;
/// Pane header height (px).
pub const PANE_HEADER_HEIGHT: f32 = 20.0;
/// Row height for file tree entries (px).
pub const FILES_ROW_HEIGHT: f32 = 22.0;
/// Outer horizontal padding for each row (px).
pub const FILES_ROW_PAD_X: f32 = PAD_STANDARD;
/// Pixel offset added per directory depth level — visual indent.
pub const FILES_INDENT_PX: f32 = PAD_XL;
/// Width of the chevron column (px).
pub const FILES_CHEVRON_W: f32 = 14.0;
/// Hitbox width of the reusable `Disclosure` chevron (px). Decoupled from the
/// file-tree column so the two can diverge later; initially equal.
pub const DISCLOSURE_CHEVRON_W: f32 = FILES_CHEVRON_W;
/// Width of the icon column to the right of the chevron (px).
pub const FILES_ICON_W: f32 = 16.0;
/// Gap between chevron / icon / name (px).
pub const FILES_ROW_GAP: f32 = GAP_SM;
/// Font size for the file tree row name (px).
pub const FILES_ROW_FONT_SIZE: f32 = FONT_SIZE_MD;
/// Pane header horizontal padding (px).
pub const PANE_HEADER_PAD_X: f32 = PAD_STANDARD;
/// Pane header item gap (px).
pub const PANE_HEADER_GAP: f32 = GAP_SM;
/// Pane header title/cwd inner gap (px).
pub const PANE_HEADER_INNER_GAP: f32 = GAP_STANDARD;
/// Pane header close button width/height (px).
pub const PANE_HEADER_CLOSE_W: f32 = 16.0;
/// Pane header close button height (px).
pub const PANE_HEADER_CLOSE_H: f32 = 14.0;
/// Pane header close button corner radius (px).
pub const PANE_HEADER_CLOSE_RADIUS: f32 = RADIUS_XS;
/// Tab cell inner gap (px).
pub const TAB_GAP: f32 = GAP_SM;
/// Tab cell left padding (px).
pub const TAB_PAD_LEFT: f32 = PAD_LG;
/// Tab cell right padding (px).
pub const TAB_PAD_RIGHT: f32 = PAD_XS;
/// Tab cell vertical padding (px).
pub const TAB_PAD_Y: f32 = GAP_XS;
/// Tab cell horizontal margin (px).
pub const TAB_MARGIN_X: f32 = 1.0;
/// New-tab button horizontal padding (px).
pub const NEW_TAB_PAD_X: f32 = PAD_STANDARD;
/// New-tab button vertical padding (px).
pub const NEW_TAB_PAD_Y: f32 = PAD_XS;
/// New-tab button horizontal margin (px).
pub const NEW_TAB_MARGIN_X: f32 = GAP_XS;
/// New-tab button corner radius (px).
pub const NEW_TAB_RADIUS: f32 = RADIUS_SM;
/// Dock toggle icon group inner gap (px).
pub const DOCK_ICON_GROUP_GAP: f32 = GAP_XS;
/// Status bar item gap (px).
pub const STATUS_BAR_GAP: f32 = GAP_LG;
/// Command palette input padding X (px).
pub const PALETTE_INPUT_PAD_X: f32 = 12.0;
/// Command palette input padding Y (px).
pub const PALETTE_INPUT_PAD_Y: f32 = PAD_STANDARD;
/// Command palette query font size (px).
pub const PALETTE_QUERY_FONT_SIZE: f32 = FONT_SIZE_LG;
/// Command palette max list height (px).
pub const PALETTE_MAX_HEIGHT: f32 = 360.0;
/// Command palette entry padding X (px).
pub const PALETTE_ENTRY_PAD_X: f32 = 12.0;
/// Command palette entry padding Y (px).
pub const PALETTE_ENTRY_PAD_Y: f32 = PAD_SM;
/// Command palette entry label font size (px).
pub const PALETTE_ENTRY_FONT_SIZE: f32 = FONT_SIZE_LG;
/// Command palette shortcut font size (px).
pub const PALETTE_SHORTCUT_FONT_SIZE: f32 = FONT_SIZE_SM;
/// Accent rule down the left of the row Enter will act on.
///
/// DESIGN §Colors calls the 2px accent left border the primary "selected"
/// signal, and forbids accent as a panel fill. The focused row's tint alone
/// is `SURFACE_2`(L 8.2%) against `SURFACE_3`(L 9.8%) — 1.6 points apart on
/// a near-black panel, which is not a signal a person can navigate by.
pub const PALETTE_FOCUS_BORDER_W: f32 = LANE_ACTIVE_BORDER_W;
/// Command palette "no results" padding Y (px).
pub const PALETTE_EMPTY_PAD_Y: f32 = 16.0;
/// Settings window initial origin X from screen top-left (px).
pub const SETTINGS_WINDOW_ORIGIN_X: f32 = 200.0;
/// Settings window initial origin Y from screen top-left (px).
pub const SETTINGS_WINDOW_ORIGIN_Y: f32 = 100.0;
/// Settings window width (px). Was 520 (single-column form);
/// increased to fit the new section-nav sidebar to the left of the
/// body without compressing field rows.
pub const SETTINGS_WINDOW_W: f32 = 720.0;
/// Settings window height (px).
pub const SETTINGS_WINDOW_H: f32 = 680.0;
/// Title bar height (px).
pub const TITLE_BAR_HEIGHT: f32 = 28.0;
/// Tab bar height (px).
pub const TAB_BAR_HEIGHT: f32 = 28.0;
/// Status bar height (px).
pub const STATUS_BAR_HEIGHT: f32 = 24.0;
/// Diameter of the project-config indicator dot.
pub const STATUS_BAR_PROJECT_DOT_SIZE: f32 = 6.0;
/// Inline detached-HEAD chip — font size, horizontal padding,
/// vertical padding, corner radius. Sized to sit flush with the
/// 24px status bar height without forcing a row-height increase.
pub const STATUS_BAR_DETACHED_FONT_SIZE: f32 = FONT_SIZE_XS;
pub const STATUS_BAR_DETACHED_PAD_X: f32 = 5.0;
pub const STATUS_BAR_DETACHED_PAD_Y: f32 = 1.0;
pub const STATUS_BAR_DETACHED_RADIUS: f32 = RADIUS_XS;
/// Account slot chip — fixed height, horizontal padding, corner radius.
/// 18px leaves 3px of clearance above/below inside the 24px status bar
/// (same "don't force a row-height increase" constraint as the detached
/// chip above), with room for the always-on hairline border that gives
/// the dropdown trigger a resting-state outline (`ghost()` alone paints
/// no border in any state).
pub const STATUS_BAR_ACCOUNT_HEIGHT: f32 = 18.0;
pub const STATUS_BAR_ACCOUNT_PAD_X: f32 = 6.0;
pub const STATUS_BAR_ACCOUNT_RADIUS: f32 = RADIUS_XS;
/// Agent/auth-domain mark inside a status-bar pill. Two below the 14px cap
/// that `STATUS_BAR_ACCOUNT_HEIGHT` minus its border leaves, so the glyph
/// never touches the pill edge.
pub const STATUS_BAR_AGENT_ICON_SIZE: f32 = 12.0;
/// Provider mark heading a Usage tab section — sized to the section title
/// beside it rather than to the status bar's tighter row.
pub const USAGE_SECTION_ICON_SIZE: f32 = 14.0;
/// Window-width breakpoints driving `StatusBarDensity`. Below
/// `STATUS_BAR_COMPACT_WIDTH` the project/branch label abbreviates to
/// just the branch and the Ports chip drops its "Ports:" word (bare
/// count). `STATUS_BAR_ICON_ONLY_WIDTH` is the narrowest tier; segments
/// keep their labels there and shed only inter-word padding.
pub const STATUS_BAR_COMPACT_WIDTH: f32 = 720.0;
pub const STATUS_BAR_ICON_ONLY_WIDTH: f32 = 480.0;
/// Claude usage chip — gap between the chip's own spans (window label,
/// severity-coloured percent, reset countdown, chevron). Tighter than
/// `STATUS_BAR_GAP`, which separates whole segments rather than words
/// inside one pill.
pub const STATUS_BAR_USAGE_CHIP_GAP: f32 = GAP_SM;
/// Fixed width of a row in the usage chip's dropdown. `PopupMenu` sizes
/// to its content, so the gauge bars need an explicit width to share one
/// scale instead of each stretching to its own label's length.
pub const STATUS_BAR_USAGE_ROW_WIDTH: f32 = 180.0;
/// Vertical gap between a dropdown row's header line, its gauge bar, and
/// its reset caption.
pub const STATUS_BAR_USAGE_ROW_GAP: f32 = GAP_XS;
/// Agent chat font size (px) — the whole conversation pane (message bodies,
/// headers, tool titles, chrome) shares this one size. Compile-time default
/// for the config-driven `font.agent_chat_size`; the render path resolves the
/// live value via `theme::agent_chat_font_size(cx)`.
pub const AGENT_CHAT_MSG_FONT_SIZE: f32 = FONT_SIZE_LG;
/// Lightness lift applied to the terminal-mirrored agent-chat foreground so
/// chat text reads slightly brighter than the raw terminal glyph, without
/// altering the terminal itself. Applied as `l += this * (1 - bg.l)` in
/// `theme::agent_chat_fg` (scaled by background darkness), then clamped to 1.0.
/// 0.0 = exact terminal foreground. Tune here.
pub const AGENT_CHAT_FG_BRIGHTEN: f32 = 0.24;
/// Alpha for the muted (secondary) step of the agent-chat foreground ramp —
/// the terminal foreground blended toward the pane background. Tuned so that
/// over the default dark terminal background it lands near the old
/// `TEXT_MUTE` lightness. See `theme::agent_chat_fg_muted`.
pub const AGENT_CHAT_FG_MUTED_ALPHA: f32 = 0.62;
/// Alpha for the subtle (tertiary) step of the agent-chat foreground ramp.
/// See `theme::agent_chat_fg_subtle`.
pub const AGENT_CHAT_FG_SUBTLE_ALPHA: f32 = 0.5;
/// Agent chat message gap (px).
pub const AGENT_CHAT_MSG_GAP: f32 = GAP_XS;
/// Extra left gap (px) between a collapsed foldable header's label (agent name /
/// "Thinking") and its inline one-line preview, so the two read as distinct
/// rather than running together at the tight `AGENT_CHAT_MSG_GAP`.
pub const AGENT_CHAT_SUMMARY_GAP: f32 = GAP_STANDARD;
/// Agent chat message list gap (px).
pub const AGENT_CHAT_LIST_GAP: f32 = GAP_LG;
/// Gap (px) between the tail window's boundary rule and the label it frames.
/// Wider than `AGENT_CHAT_MSG_GAP`: a label that nearly touches the rule reads
/// as a broken line rather than as an inset word.
pub const AGENT_CHAT_BOUNDARY_GAP: f32 = GAP_STANDARD;
/// Length (px) of the stub rule left of the boundary label once the boundary is
/// open. The label moves from centred-between-two-rules to left-anchored, which
/// is the boundary's primary open/closed cue — a chevron alone is what the row
/// used to rely on, and it reads identically to every step bar around it.
pub const AGENT_CHAT_BOUNDARY_STUB_W: f32 = 12.0;
/// Left gap (px) between the rail marking a boundary-revealed row and the row's
/// own content.
pub const AGENT_CHAT_OUTSIDE_RAIL_GAP: f32 = GAP_STANDARD;
/// Agent chat turn-boundary gap (px) — extra space above a new user message,
/// paired with a hairline, so consecutive turns read as distinct exchanges.
pub const AGENT_CHAT_TURN_GAP: f32 = PAD_XL;
/// Poll interval (ms) for the agent-chat drag-selection autoscroll loop.
/// Mirrors the terminal's selection-autoscroll cadence (~20 Hz): fast enough
/// to feel continuous while the cursor is parked past the pane edge, slow
/// enough to stay cheap. Read by `start_selection_autoscroll`.
pub const AGENT_CHAT_AUTOSCROLL_POLL_MS: u64 = 50;
/// Maximum per-tick scroll distance (px) for the agent-chat drag-selection
/// autoscroll — caps the velocity so a cursor dragged far past the edge
/// scrolls smoothly (~3 chat lines per tick) instead of jumping pages. Read
/// by `autoscroll_step`.
pub const AGENT_CHAT_AUTOSCROLL_MAX_STEP_PX: f32 = 48.0;
/// Per-row height of any editor embedded in a tool card — the diff view and the
/// verbatim tool-output view (px). Equal to gpui's window `line_height`
/// (`Rems(1.25)` × the 16 px `rem_size` = 20 px, font-size independent — same
/// value the bottom-input auto-grow relies on). An embedded `CodeEditor` (not
/// `AutoGrow`) sizes to `relative(1.)` of its parent, which collapses to a
/// single line without a definite-height parent; a tool-card body has none, so
/// it sets an explicit `rows × this` height.
pub const AGENT_CHAT_EMBED_ROW_H: f32 = 20.0;
/// Max height (px) of an editor embedded in a tool card before it scrolls
/// internally. Load-bearing, not cosmetic: `InputState` shapes and paints only
/// the rows inside its own bounds height, so without a bound every row is
/// "visible" and the paint cost is linear in output size. 12 rows keeps a shell
/// failure unit (assert + backtrace) readable while leaving a split pane's card
/// header and the neighbouring conversation on screen.
pub const AGENT_CHAT_EMBED_MAX_H: f32 = 240.0;
/// Agent chat header: gap between the agent icon and the label/text (px).
pub const AGENT_CHAT_HEADER_ICON_GAP: f32 = GAP_STANDARD;
/// Agent chat header: agent icon square size (px).
pub const AGENT_CHAT_HEADER_ICON_SIZE: f32 = 16.0;
/// Agent chat container padding X (px).
pub const AGENT_CHAT_PAD_X: f32 = PAD_STANDARD;
/// Agent chat container padding Y (px).
pub const AGENT_CHAT_PAD_Y: f32 = PAD_XS;
/// Agent chat input box inner padding X (px).
pub const AGENT_CHAT_INPUT_INNER_PAD_X: f32 = PAD_STANDARD;
/// Agent chat input box inner padding Y (px).
pub const AGENT_CHAT_INPUT_INNER_PAD_Y: f32 = PAD_XS;
/// Agent chat input box corner radius (px).
pub const AGENT_CHAT_INPUT_RADIUS: f32 = RADIUS_SM;
/// Inner padding of the mermaid diagram card in the agent chat — breathing
/// room between the card hairline and the (transparent-canvas) diagram.
pub const AGENT_CHAT_DIAGRAM_PAD: f32 = PAD_STANDARD;
/// Vertical gap after a mermaid diagram card embedded in markdown. The custom
/// code-block renderer replaces TextView's built-in code-block spacing, so the
/// card supplies its own gap for consecutive diagrams.
pub const AGENT_CHAT_DIAGRAM_GAP: f32 = GAP_LG;
/// Fraction of the viewport the mermaid lightbox dialog may occupy.
pub const MERMAID_LIGHTBOX_VIEWPORT_FRACTION: f32 = 0.9;
/// Slack (px) for "is the agent chat scrolled to the bottom?" — within this
/// distance of the live edge still counts as at-bottom (follow mode stays on).
pub const AGENT_CHAT_SCROLL_BOTTOM_SLACK: f32 = 24.0;
/// Inset (px) of the floating scroll-to-bottom button from the pane's
/// bottom-right corner.
pub const AGENT_CHAT_SCROLL_BTN_INSET: f32 = 12.0;
/// Max height (px) of the bottom plan region's expanded checklist before it
/// scrolls internally, so a long plan can't crowd out the conversation above.
pub const AGENT_CHAT_PLAN_MAX_H: f32 = 168.0;
/// Width (px) the Activity Bar's right-hand control cluster needs when the
/// three transcript chips are spelled out — the widest realistic values
/// (`Fold: Custom`, `Filter: Reasoning + Replies`, `Recent steps: 20`) plus the
/// three icon buttons and their gaps, at [`AGENT_CHAT_MSG_FONT_SIZE`].
///
/// Text-derived, so it is a width *per unit font size*, not an absolute one:
/// [`theme::agent_chat_compact_options_w`](crate::ui::theme::agent_chat_compact_options_w)
/// scales it by the pane's configured size before comparing.
pub const AGENT_CHAT_OPTIONS_CLUSTER_W: f32 = 400.0;
/// Width (px) the Activity Bar keeps for the session title before the chips are
/// worth showing. Below this the title ellipsizes to a few words and stops
/// identifying the session, which is the bar's primary job. Text-derived and
/// font-scaled on the same terms as [`AGENT_CHAT_OPTIONS_CLUSTER_W`].
pub const AGENT_CHAT_TITLE_MIN_W: f32 = 180.0;
/// Pane width (px) at or below which transcript controls collapse into the
/// single view-options popover, at the default font size.
///
/// Derived rather than dialled in: the split is "does the spelled-out cluster
/// still leave a usable title", so it moves when either part does. A hand-set
/// number drifts away from the parts it is supposed to describe.
///
/// Both parts are text widths, so the live threshold is font-dependent; read it
/// through
/// [`theme::agent_chat_compact_options_w`](crate::ui::theme::agent_chat_compact_options_w)
/// rather than using this constant directly. It is the value at
/// [`AGENT_CHAT_MSG_FONT_SIZE`] and exists so the derivation has one home.
pub const AGENT_CHAT_COMPACT_OPTIONS_W: f32 =
    AGENT_CHAT_TITLE_MIN_W + AGENT_CHAT_OPTIONS_CLUSTER_W + 2.0 * AGENT_CHAT_PAD_X;
/// Width (px) of single-column Activity Bar popovers.
pub const AGENT_CHAT_OPTIONS_PANEL_W: f32 = 240.0;
/// Width (px) of fold-rule and combined options popovers.
pub const AGENT_CHAT_RULES_PANEL_W: f32 = 430.0;
/// Max height (px) of the fold-rule editor popover.
pub const AGENT_CHAT_RULES_PANEL_MAX_H: f32 = 520.0;
/// Largest fraction of the window an Activity Bar popover may claim, on either
/// axis. Width is capped as well as height because the fold editor is 430px and
/// the window can be narrower than that leaves room for — not to keep the panel
/// off the docks beside its pane, which it does not do and is not meant to.
pub const AGENT_CHAT_PANEL_VIEWPORT_FRACTION: f32 = 0.8;
/// Indent (px) for nested Activity Bar options.
pub const AGENT_CHAT_OPTION_NEST_INDENT: f32 = 20.0;
/// Corner radius (px) of an Activity Bar chip. `RADIUS_SM`, not the 2px
/// `radius.xs` DESIGN.md reserves for inline chips: these are toolbar controls,
/// and every button-shaped constant in the app already sits at `RADIUS_SM`
/// (`MODAL_BUTTON_RADIUS`, `BUTTON_WIDGET_RADIUS`, `DOCK_ICON_BUTTON_RADIUS`, …)
/// as do the pane's own tool cards and code blocks via
/// [`AGENT_CHAT_INPUT_RADIUS`]. Matching the neighbours beats matching
/// DESIGN.md's radius table, whose `md` row for buttons no shipped button obeys.
pub const AGENT_CHAT_CHIP_RADIUS: f32 = RADIUS_SM;
/// Gap (px) between Activity Bar controls. Wider than `AGENT_CHAT_MSG_GAP`
/// (which spaces text runs): the chips carry a hairline, and adjacent boxes two
/// pixels apart read as one segmented strip rather than three controls.
pub const AGENT_CHAT_BAR_CONTROL_GAP: f32 = GAP_SM;
/// Left dock default width (px).
pub const DOCK_LEFT_DEFAULT_W: f32 = 250.0;
/// Left dock minimum width (px).
pub const DOCK_LEFT_MIN_W: f32 = 220.0;
/// Left dock maximum width (px).
pub const DOCK_LEFT_MAX_W: f32 = 400.0;
/// Right dock default width (px). Sized so the five-tab view switcher
/// shows every label plus its overflow chevron in the widest locale —
/// measured at 250 the English "Flows" was already clipped. Narrower is
/// still allowed; the chevron is what keeps the hidden tabs reachable.
pub const DOCK_RIGHT_DEFAULT_W: f32 = 290.0;
/// Right dock minimum width (px).
pub const DOCK_RIGHT_MIN_W: f32 = 220.0;
/// Right dock maximum width (px).
pub const DOCK_RIGHT_MAX_W: f32 = 500.0;
/// Bottom dock default height (px). Sized to the single-row macro
/// preset so a freshly-opened project starts with the dock at its
/// most compact useful height — the user can expand to 2 or 3 rows
/// via the suffix menu in the tab strip.
pub const DOCK_BOTTOM_DEFAULT_H: f32 = 76.0;
/// Bottom dock minimum height (px) — sized so the single-row preset
/// (`DOCK_BOTTOM_ROW_PRESET_1_H`) is reachable both via drag and via
/// the row-preset menu in the bottom dock tab strip suffix.
pub const DOCK_BOTTOM_MIN_H: f32 = 76.0;
/// Bottom dock maximum height (px).
pub const DOCK_BOTTOM_MAX_H: f32 = 500.0;
/// Bottom dock row presets — pick the dock size that fits N rows of
/// macro tiles with the standard padding+gap geometry:
/// `TAB_BAR_HEIGHT + 2*PANEL_BODY_PAD_Y + N*BUTTON_WIDGET_HEIGHT + (N-1)*PANEL_BODY_GAP`.
pub const DOCK_BOTTOM_ROW_PRESET_1_H: f32 = 76.0;
/// Two-row preset height for the bottom dock (px).
pub const DOCK_BOTTOM_ROW_PRESET_2_H: f32 = 114.0;
/// Three-row preset height for the bottom dock (px).
pub const DOCK_BOTTOM_ROW_PRESET_3_H: f32 = 152.0;
/// Height added to the bottom dock for each extra text line when the
/// bottom input auto-grows beyond one row (px). Sized to match one
/// `Size::Small` input line: gpui `line_height` is `LINE_HEIGHT = Rems(1.25)`
/// resolved against the global `rem_size` (16 px) — 1.25 × 16 = 20 px,
/// independent of the terminal font size. `TextElement::request_layout`
/// sets `min_size.height = rows * window.line_height()` = `rows * 20 px`.
/// One line height per row — display rows 1…N (soft-wrapped), each
/// contributing one 20 px step.
pub const DOCK_BOTTOM_INPUT_EXTRA_LINE_H: f32 = 20.0;
/// Total vertical padding around the text area inside the input chrome
/// (top + bottom, px). Derived from `INPUT_TEXTAREA_PAD_Y * 2`; lifted
/// as a named constant so `bottom_dock_height_for_rows` can reference it
/// without inline arithmetic.
pub const DOCK_BOTTOM_INPUT_TEXT_PAD_H: f32 = INPUT_TEXTAREA_PAD_Y * 2.0;
/// Height contributed by the stacked action row (mode chip + Submit/Stop
/// button) inside the bottom-dock input chrome, after the layout change to
/// `flex_col` (text above, action row below). Equals `BUTTON_HEIGHT +
/// INPUT_PANEL_BUTTON_GAP` (button height + bottom gap from input chrome
/// edge).
pub const DOCK_BOTTOM_INPUT_ACTION_ROW_H: f32 = BUTTON_HEIGHT + INPUT_PANEL_BUTTON_GAP;
/// Queued-prompt strip: max height (px) before the item list scrolls
/// internally, so a long queue can't crowd out the terminal input below it.
pub const AGENT_QUEUE_STRIP_MAX_H: f32 = 120.0;
/// Queued-prompt strip: gap between the header row and item rows, and between
/// adjacent item rows (px).
pub const AGENT_QUEUE_STRIP_GAP: f32 = GAP_SM;
/// Queued-prompt strip: item row / header font size (px).
pub const AGENT_QUEUE_STRIP_FONT_SIZE: f32 = FONT_SIZE_MD;
/// Queued-prompt strip: item row corner radius (px).
pub const AGENT_QUEUE_STRIP_ROW_RADIUS: f32 = RADIUS_SM;
/// Queued-prompt strip: item row inner padding X (px).
pub const AGENT_QUEUE_STRIP_ROW_PAD_X: f32 = PAD_SM;
/// Queued-prompt strip: item row inner padding Y (px).
pub const AGENT_QUEUE_STRIP_ROW_PAD_Y: f32 = PAD_XS;
/// Width of the invisible hit target for resize handles — used by
/// both dock handles and pane dividers. Kept independent of the
/// visible boundary width so the hit zone can be widened without
/// affecting layout (handles are absolute overlays).
pub const RESIZE_HANDLE_HIT_PX: f32 = 3.0;
/// Welcome screen title font size (px).
pub const WELCOME_TITLE_FONT_SIZE: f32 = 28.0;
/// Welcome screen version font size (px).
pub const WELCOME_VERSION_FONT_SIZE: f32 = FONT_SIZE_LG;
/// Welcome screen section heading font size (px).
pub const WELCOME_HEADING_FONT_SIZE: f32 = FONT_SIZE_LG;
/// Welcome screen button font size (px).
pub const WELCOME_BUTTON_FONT_SIZE: f32 = FONT_SIZE_LG;
/// Welcome screen recent entry font size (px).
pub const WELCOME_RECENT_FONT_SIZE: f32 = FONT_SIZE_LG;
/// Welcome screen panel width (px).
pub const WELCOME_PANEL_WIDTH: f32 = 420.0;
/// Welcome screen panel padding (px).
pub const WELCOME_PANEL_PAD: f32 = 40.0;
/// Welcome screen item gap (px).
pub const WELCOME_GAP: f32 = 16.0;
/// Welcome screen button padding X (px).
pub const WELCOME_BUTTON_PAD_X: f32 = 16.0;
/// Welcome screen button padding Y (px).
pub const WELCOME_BUTTON_PAD_Y: f32 = PAD_LG;
/// Welcome screen button corner radius (px).
pub const WELCOME_BUTTON_RADIUS: f32 = RADIUS_MD;
/// Welcome screen recent entry padding X (px).
pub const WELCOME_RECENT_PAD_X: f32 = 12.0;
/// Welcome screen recent entry padding Y (px).
pub const WELCOME_RECENT_PAD_Y: f32 = PAD_STANDARD;
/// Welcome screen recent entry corner radius (px).
pub const WELCOME_RECENT_RADIUS: f32 = RADIUS_SM;
/// Welcome screen tight inner gap (px) — used between heading + label.
pub const WELCOME_GAP_TIGHT: f32 = GAP_SM;
/// Welcome screen loose inner gap (px) — used between recent entry rows.
pub const WELCOME_GAP_LOOSE: f32 = GAP_LG;
/// File viewer toolbar height (px).
pub const FILE_VIEWER_HEADER_H: f32 = 28.0;
/// File viewer toolbar horizontal padding (px).
pub const FILE_VIEWER_HEADER_PAD_X: f32 = PAD_LG;
/// File viewer toolbar font size (px).
pub const FILE_VIEWER_HEADER_FONT_SIZE: f32 = FONT_SIZE_SM;
/// File viewer close button font size (px).
pub const FILE_VIEWER_CLOSE_FONT_SIZE: f32 = FONT_SIZE_LG;
/// File viewer body font size (px).
pub const FILE_VIEWER_FONT_SIZE: f32 = FONT_SIZE_MD;
/// File viewer line number column width (px).
pub const FILE_VIEWER_LINE_NO_W: f32 = 50.0;
/// Maximum lines shown in file viewer body before truncation.
pub const FILE_VIEWER_MAX_LINES: usize = 2000;
/// Maximum bytes read from a file in Raw mode. Files larger than this are
/// truncated before line-splitting so the process never loads unbounded data.
pub const FILE_VIEWER_MAX_BYTES: usize = 5 * 1024 * 1024; // 5 MB

/// Row-height multiple over the body font size (≈ phi). Applied to the
/// config-driven editor font so the raw virtual-list row height scales
/// with `font.editor_size` instead of the fixed const.
pub const FILE_VIEWER_LINE_H_RATIO: f32 = 1.7;
/// Fixed row height for the virtual-list renderer (px).
/// Derived so it is always ≥ FILE_VIEWER_FONT_SIZE * phi() without manual sync.
pub const FILE_VIEWER_LINE_H: f32 = FILE_VIEWER_FONT_SIZE * FILE_VIEWER_LINE_H_RATIO;
/// Rows rendered above and below the visible viewport (overscan).
pub const FILE_VIEWER_VIRTUAL_OVERSCAN: usize = 8;
/// Gap between toolbar button group items (px).
pub const FILE_VIEWER_TOOLBAR_GAP: f32 = GAP_STANDARD;
/// File viewer close button horizontal padding (px).
pub const FILE_VIEWER_CLOSE_PAD_X: f32 = PAD_STANDARD;
/// File viewer toolbar button corner radius (px).
pub const FILE_VIEWER_TOOL_BUTTON_RADIUS: f32 = RADIUS_SM;
/// Icon-only file viewer toolbar button width (px).
pub const FILE_VIEWER_TOOL_BUTTON_W: f32 = 26.0;
/// Icon-only file viewer toolbar button height (px).
pub const FILE_VIEWER_TOOL_BUTTON_H: f32 = 22.0;
// Diff line colors track the canonical DESIGN §Git&Diff tokens
// (`DIFF_*`) so the file-viewer and git-changes diffs read identically
// to the spec instead of carrying a parallel green/red palette.
/// Diff added-line background (DESIGN diff-add-bg = success @ 12%).
pub const FILE_DIFF_ADD_BG: Hsla = DIFF_ADD_BG;
/// Diff removed-line background (DESIGN diff-del-bg = error @ 12%).
pub const FILE_DIFF_DEL_BG: Hsla = DIFF_DEL_BG;
/// Diff added-line text color (DESIGN diff-add-fg).
pub const FILE_DIFF_ADD_TEXT: Hsla = DIFF_ADD_FG;
/// Diff removed-line text color (DESIGN diff-del-fg).
pub const FILE_DIFF_DEL_TEXT: Hsla = DIFF_DEL_FG;
/// Diff hunk-header text color (DESIGN diff-hunk = subtle).
pub const FILE_DIFF_HUNK_TEXT: Hsla = DIFF_HUNK;
/// Line number right padding in the raw file view (px).
pub const FILE_VIEWER_LINE_NO_PAD_R: f32 = PAD_STANDARD;
/// Hunk header trailing context text color (function name / class name, dim).
pub const FILE_DIFF_HUNK_CTX_TEXT: Hsla = hsla(220.0, 0.20, 0.45, 1.0);
/// 24-bit hex → `Hsla`. Hex literals live here (the designated colour
/// home, G4-exempt), never at the call site.
fn base16(hex: u32) -> Hsla {
    gpui::rgb(hex).into()
}

/// Semantic syntax palette — one resolved [`SyntaxPalette`]'s colours,
/// shared by the raw editor (via [`editor_syntax_colors_of`], installed
/// into `gpui_component`'s `highlight_theme`) and the diff view (via
/// [`syntax_color_of`]). Fields are semantic token buckets.
pub struct SyntaxTheme {
    /// purple — keywords.
    pub keyword: Hsla,
    /// blue — functions / titles.
    pub function: Hsla,
    /// yellow — types, enums, constructors, labels, preproc, embedded.
    pub type_: Hsla,
    /// orange — constants, booleans, numbers, attributes, variants, link URIs.
    pub constant: Hsla,
    /// green — strings, literal text.
    pub string: Hsla,
    /// cyan — string escapes / regex / special symbols.
    pub string_special: Hsla,
    /// red — tags, special variables, link text.
    pub tag: Hsla,
    /// brown — doctype tags.
    pub tag_doctype: Hsla,
    /// gray — comments, hints, predictive text.
    pub comment: Hsla,
    /// default foreground (variables, operators, punctuation, …).
    pub default: Hsla,
    /// Non-color channel (R1) for keywords — bold to carry structure
    /// without relying on chroma / CVD-robust.
    pub keyword_style: TokenStyle,
    /// Non-color channel for string escapes / regex / special symbols —
    /// distinguishes them from plain strings even at low chroma.
    pub string_special_style: TokenStyle,
    /// Non-color channel for comments — italic to signal "noise".
    pub comment_style: TokenStyle,
}

/// A token's non-color rendering channel. `Default` = plain (no
/// weight / style override). Lets a palette carry bold/italic alongside
/// its colours so figure/ground survives low chroma and colour-vision
/// deficiency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TokenStyle {
    pub bold: bool,
    pub italic: bool,
}

impl TokenStyle {
    const PLAIN: Self = Self {
        bold: false,
        italic: false,
    };
    const BOLD: Self = Self {
        bold: true,
        italic: false,
    };
    const ITALIC: Self = Self {
        bold: false,
        italic: true,
    };
}

/// Selectable syntax palette — chosen independently of the brand theme
/// (background / accent). Resolved from `config.file_viewer.syntax_theme`
/// via [`SyntaxPalette::from_config_name`]; the single source of truth for
/// which colours the raw editor and the diff view both render with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SyntaxPalette {
    /// Daruda's own palette — the readability-tuned default (recommended).
    #[default]
    Daruda,
    OneDark,
    TokyoNight,
    CatppuccinMocha,
    Dracula,
    GitHubDark,
    MaterialPalenight,
    Monokai,
    Nord,
    GruvboxDark,
    SolarizedDark,
    AyuMirage,
    NightOwl,
    Darcula,
}

impl SyntaxPalette {
    /// Resolve a config string to a palette. Unknown / legacy names
    /// (including the old `base16-*` slots) fall back to the recommended
    /// [`SyntaxPalette::Daruda`] — a normal fallback, not an error.
    pub fn from_config_name(name: &str) -> Self {
        match name {
            "one-dark" => Self::OneDark,
            "tokyo-night" => Self::TokyoNight,
            "catppuccin-mocha" => Self::CatppuccinMocha,
            "dracula" => Self::Dracula,
            "github-dark" => Self::GitHubDark,
            "material-palenight" => Self::MaterialPalenight,
            "monokai" => Self::Monokai,
            "nord" => Self::Nord,
            "gruvbox-dark" => Self::GruvboxDark,
            "solarized-dark" => Self::SolarizedDark,
            "ayu-mirage" => Self::AyuMirage,
            "night-owl" => Self::NightOwl,
            "darcula" => Self::Darcula,
            _ => Self::Daruda,
        }
    }

    /// Canonical config slug — inverse of [`SyntaxPalette::from_config_name`].
    /// Lets the settings dropdown resolve a stored (possibly legacy) value to
    /// the slug that is actually selected, so the effective palette always
    /// shows as the active option.
    pub fn config_name(self) -> &'static str {
        match self {
            Self::Daruda => "daruda",
            Self::OneDark => "one-dark",
            Self::TokyoNight => "tokyo-night",
            Self::CatppuccinMocha => "catppuccin-mocha",
            Self::Dracula => "dracula",
            Self::GitHubDark => "github-dark",
            Self::MaterialPalenight => "material-palenight",
            Self::Monokai => "monokai",
            Self::Nord => "nord",
            Self::GruvboxDark => "gruvbox-dark",
            Self::SolarizedDark => "solarized-dark",
            Self::AyuMirage => "ayu-mirage",
            Self::NightOwl => "night-owl",
            Self::Darcula => "darcula",
        }
    }
}

/// Semantic colour bucket — the unit a tree-sitter capture maps to.
/// [`bucket_for_capture`] is the single place the capture grouping lives;
/// every consumer (colour, non-color channel, editor `SyntaxColors`) reads
/// through it so the grouping can't drift between paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntaxBucket {
    Keyword,
    Function,
    Type,
    Constant,
    String,
    StringSpecial,
    Tag,
    TagDoctype,
    Comment,
    Default,
}

/// Map a tree-sitter highlight capture name onto its [`SyntaxBucket`].
///
/// A dotted capture that doesn't match exactly falls back to its first
/// segment (`function.method` → `function`, `keyword.control` →
/// `keyword`), mirroring the gpui_component editor's
/// `SyntaxColors::style` resolution (`registry.rs`). Without this the
/// raw editor coloured `function.method` / `type.builtin` via the base
/// field while the diff view dropped them to `Default` — the two views
/// disagreed on methods, qualified types, and keyword sub-kinds.
/// Unrecognised captures (and the empty string) resolve to
/// [`SyntaxBucket::Default`], so every token gets an explicit bucket.
pub fn bucket_for_capture(capture: &str) -> SyntaxBucket {
    use SyntaxBucket::*;
    match capture {
        "keyword" => Keyword,
        "function" | "title" => Function,
        "type" | "enum" | "constructor" | "label" | "preproc" | "embedded" => Type,
        "constant" | "boolean" | "number" | "attribute" | "variant" | "link_uri" => Constant,
        "string" | "text.literal" => String,
        "string.escape" | "string.regex" | "string.special" | "string.special.symbol" => {
            StringSpecial
        }
        "tag" | "variable.special" | "link_text" => Tag,
        "tag.doctype" => TagDoctype,
        "comment" | "comment.doc" | "hint" | "predictive" => Comment,
        _ => match capture.split_once('.') {
            Some((prefix, _)) => bucket_for_capture(prefix),
            None => Default,
        },
    }
}

impl SyntaxTheme {
    /// Foreground colour for a bucket.
    pub fn color(&self, bucket: SyntaxBucket) -> Hsla {
        use SyntaxBucket::*;
        match bucket {
            Keyword => self.keyword,
            Function => self.function,
            Type => self.type_,
            Constant => self.constant,
            String => self.string,
            StringSpecial => self.string_special,
            Tag => self.tag,
            TagDoctype => self.tag_doctype,
            Comment => self.comment,
            Default => self.default,
        }
    }

    /// Non-color channel (bold/italic) for a bucket. Only keyword /
    /// string_special / comment carry one; the rest are plain.
    pub fn style(&self, bucket: SyntaxBucket) -> TokenStyle {
        use SyntaxBucket::*;
        match bucket {
            Keyword => self.keyword_style,
            StringSpecial => self.string_special_style,
            Comment => self.comment_style,
            _ => TokenStyle::default(),
        }
    }

    /// Foreground colour for a capture name — [`bucket_for_capture`] +
    /// [`SyntaxTheme::color`]. Used by the diff view's own highlighter.
    pub fn color_for(&self, capture: &str) -> Hsla {
        self.color(bucket_for_capture(capture))
    }

    /// Non-color channel for a capture name.
    pub fn style_for(&self, capture: &str) -> TokenStyle {
        self.style(bucket_for_capture(capture))
    }
}

/// The active syntax palette (`base16-ocean.dark`). One source feeds both
/// highlighting paths — change a colour here and the editor and diff
/// views move together.
pub fn syntax_theme() -> SyntaxTheme {
    syntax_theme_of(SyntaxPalette::Daruda, false)
}

/// The semantic syntax palette for `palette` at the editor's lightness.
/// Hex literals live here (the designated colour home, G4-exempt), never at
/// the call site. One source feeds both highlighting paths — the raw editor
/// (via [`editor_syntax_colors_of`]) and the diff view (via
/// [`syntax_color_of`]). When `is_light` the palette's light variant is used
/// (families without one fall back to Daruda Light) so syntax stays legible
/// on a light editor background.
pub fn syntax_theme_of(palette: SyntaxPalette, is_light: bool) -> SyntaxTheme {
    if is_light {
        return light_syntax_theme(palette);
    }
    match palette {
        // Daruda — readability-tuned default. function / string_special
        // lifted out of the near-gray band; keyword + escape bold and
        // comment italic add non-color channels (R1).
        SyntaxPalette::Daruda => SyntaxTheme {
            keyword: base16(0xc6_78_dd),
            function: base16(0x82_aa_ff),
            type_: base16(0xeb_cb_8b),
            constant: base16(0xd0_87_70),
            string: base16(0xa3_be_8c),
            string_special: base16(0x7f_db_ca),
            tag: base16(0xbf_61_6a),
            tag_doctype: base16(0xab_79_67),
            comment: base16(0x65_73_7e),
            default: base16(0xc0_c5_ce),
            keyword_style: TokenStyle::BOLD,
            string_special_style: TokenStyle::BOLD,
            comment_style: TokenStyle::ITALIC,
        },
        // One Dark (Atom / zed) — color-only.
        SyntaxPalette::OneDark => SyntaxTheme {
            keyword: base16(0xc6_78_dd),
            function: base16(0x61_af_ef),
            type_: base16(0xe5_c0_7b),
            constant: base16(0xd1_9a_66),
            string: base16(0x98_c3_79),
            string_special: base16(0x56_b6_c2),
            tag: base16(0xe0_6c_75),
            tag_doctype: base16(0xbe_50_46),
            comment: base16(0x5c_63_70),
            default: base16(0xab_b2_bf),
            keyword_style: TokenStyle::PLAIN,
            string_special_style: TokenStyle::PLAIN,
            comment_style: TokenStyle::PLAIN,
        },
        // Tokyo Night — color + italic comments. The original comment
        // `#565f89` only reaches ~3.4x on near-black; lifted to `#6b74a3`
        // to clear the readability floor against daruda's background.
        SyntaxPalette::TokyoNight => SyntaxTheme {
            keyword: base16(0xbb_9a_f7),
            function: base16(0x7a_a2_f7),
            type_: base16(0x0d_b9_d7),
            constant: base16(0xff9e64),
            string: base16(0x9e_ce_6a),
            string_special: base16(0x89_dd_ff),
            tag: base16(0xf7_76_8e),
            tag_doctype: base16(0xff9e64),
            comment: base16(0x6b_74_a3),
            default: base16(0xc0_ca_f5),
            keyword_style: TokenStyle::PLAIN,
            string_special_style: TokenStyle::PLAIN,
            comment_style: TokenStyle::ITALIC,
        },
        // Catppuccin Mocha — color-only.
        SyntaxPalette::CatppuccinMocha => SyntaxTheme {
            keyword: base16(0xcb_a6_f7),
            function: base16(0x89_b4_fa),
            type_: base16(0xf9_e2_af),
            constant: base16(0xfa_b3_87),
            string: base16(0xa6_e3_a1),
            string_special: base16(0xf5_c2_e7),
            tag: base16(0xf3_8b_a8),
            tag_doctype: base16(0xeb_a0_ac),
            comment: base16(0x93_99_b2),
            default: base16(0xcd_d6_f4),
            keyword_style: TokenStyle::PLAIN,
            string_special_style: TokenStyle::PLAIN,
            comment_style: TokenStyle::PLAIN,
        },
        // Dracula — high-chroma, color-only.
        SyntaxPalette::Dracula => SyntaxTheme {
            keyword: base16(0xff_79_c6),
            function: base16(0x50_fa_7b),
            type_: base16(0x8b_e9_fd),
            constant: base16(0xbd_93_f9),
            string: base16(0xf1_fa_8c),
            string_special: base16(0xff_b8_6c),
            tag: base16(0xff_55_55),
            tag_doctype: base16(0x62_72_a4),
            comment: base16(0x62_72_a4),
            default: base16(0xf8_f8_f2),
            keyword_style: TokenStyle::PLAIN,
            string_special_style: TokenStyle::PLAIN,
            comment_style: TokenStyle::PLAIN,
        },
        // GitHub Dark — neutral; escapes bold (GitHub's own convention).
        SyntaxPalette::GitHubDark => SyntaxTheme {
            keyword: base16(0xff_7b_72),
            function: base16(0xd2_a8_ff),
            type_: base16(0xff_a6_57),
            constant: base16(0x79_c0_ff),
            string: base16(0xa5_d6_ff),
            string_special: base16(0xa5_d6_ff),
            tag: base16(0x7e_e7_87),
            tag_doctype: base16(0x7e_e7_87),
            comment: base16(0x8b_94_9e),
            default: base16(0xc9_d1_d9),
            keyword_style: TokenStyle::PLAIN,
            string_special_style: TokenStyle::BOLD,
            comment_style: TokenStyle::PLAIN,
        },
        // Material Palenight — calm purples/blues; italic comments.
        SyntaxPalette::MaterialPalenight => SyntaxTheme {
            keyword: base16(0xc7_92_ea),
            function: base16(0x82_aa_ff),
            type_: base16(0xff_cb_6b),
            constant: base16(0xf7_8c_6c),
            string: base16(0xc3_e8_8d),
            string_special: base16(0x89_dd_ff),
            tag: base16(0xff_55_72),
            tag_doctype: base16(0xff_55_72),
            comment: base16(0x69_70_98),
            default: base16(0xbf_c7_d5),
            keyword_style: TokenStyle::PLAIN,
            string_special_style: TokenStyle::PLAIN,
            comment_style: TokenStyle::ITALIC,
        },
        // Monokai — the Sublime classic; color-only.
        SyntaxPalette::Monokai => SyntaxTheme {
            keyword: base16(0xf9_26_72),
            function: base16(0xa6_e2_2e),
            type_: base16(0x66_d9_ef),
            constant: base16(0xae_81_ff),
            string: base16(0xe6_db_74),
            string_special: base16(0xae_81_ff),
            tag: base16(0xf9_26_72),
            tag_doctype: base16(0xf9_26_72),
            comment: base16(0x88_84_6f),
            default: base16(0xf8_f8_f2),
            keyword_style: TokenStyle::PLAIN,
            string_special_style: TokenStyle::PLAIN,
            comment_style: TokenStyle::PLAIN,
        },
        // Nord — muted arctic cool tones; comment lifted from nord3 (too
        // dim) to nord3's brighter editor variant for the near-black bg.
        SyntaxPalette::Nord => SyntaxTheme {
            keyword: base16(0x81_a1_c1),
            function: base16(0x88_c0_d0),
            type_: base16(0x8f_bc_bb),
            constant: base16(0xb4_8e_ad),
            string: base16(0xa3_be_8c),
            string_special: base16(0xeb_cb_8b),
            tag: base16(0xbf_61_6a),
            tag_doctype: base16(0xd0_87_70),
            comment: base16(0x61_6e_88),
            default: base16(0xd8_de_e9),
            keyword_style: TokenStyle::PLAIN,
            string_special_style: TokenStyle::PLAIN,
            comment_style: TokenStyle::PLAIN,
        },
        // Gruvbox Dark — warm retro; function uses aqua to stay distinct
        // from the green strings.
        SyntaxPalette::GruvboxDark => SyntaxTheme {
            keyword: base16(0xfb_49_34),
            function: base16(0x8e_c0_7c),
            type_: base16(0xfa_bd_2f),
            constant: base16(0xd3_86_9b),
            string: base16(0xb8_bb_26),
            string_special: base16(0xfe_80_19),
            tag: base16(0xfb_49_34),
            tag_doctype: base16(0xfe_80_19),
            comment: base16(0x92_83_74),
            default: base16(0xeb_db_b2),
            keyword_style: TokenStyle::PLAIN,
            string_special_style: TokenStyle::PLAIN,
            comment_style: TokenStyle::PLAIN,
        },
        // Solarized Dark — balanced; comment lifted base01 -> base00 to
        // clear the contrast floor on the near-black background.
        SyntaxPalette::SolarizedDark => SyntaxTheme {
            keyword: base16(0x85_99_00),
            function: base16(0x26_8b_d2),
            type_: base16(0xb5_89_00),
            constant: base16(0xd3_36_82),
            string: base16(0x2a_a1_98),
            string_special: base16(0xcb4b16),
            tag: base16(0x26_8b_d2),
            tag_doctype: base16(0x58_6e_75),
            comment: base16(0x65_7b_83),
            default: base16(0x83_94_96),
            keyword_style: TokenStyle::PLAIN,
            string_special_style: TokenStyle::PLAIN,
            comment_style: TokenStyle::PLAIN,
        },
        // Ayu Mirage — modern minimal; italic comments.
        SyntaxPalette::AyuMirage => SyntaxTheme {
            keyword: base16(0xff_a6_59),
            function: base16(0xff_cd_66),
            type_: base16(0x73_d0_ff),
            constant: base16(0xdf_bf_ff),
            string: base16(0xd5_ff_80),
            string_special: base16(0x95_e6_cb),
            tag: base16(0x5c_cf_e6),
            tag_doctype: base16(0x5c_cf_e6),
            comment: base16(0x6e_7c_8f),
            default: base16(0xcc_ca_c2),
            keyword_style: TokenStyle::PLAIN,
            string_special_style: TokenStyle::PLAIN,
            comment_style: TokenStyle::ITALIC,
        },
        // Night Owl — italic keywords and comments.
        SyntaxPalette::NightOwl => SyntaxTheme {
            keyword: base16(0xc7_92_ea),
            function: base16(0x82_aa_ff),
            type_: base16(0xff_cb_8b),
            constant: base16(0xf7_8c_6c),
            string: base16(0xec_c4_8d),
            string_special: base16(0x7f_db_ca),
            tag: base16(0xca_ec_e6),
            tag_doctype: base16(0x63_77_77),
            comment: base16(0x63_77_77),
            default: base16(0xd6_de_eb),
            keyword_style: TokenStyle::ITALIC,
            string_special_style: TokenStyle::PLAIN,
            comment_style: TokenStyle::ITALIC,
        },
        // Darcula — IntelliJ default; bold keywords (its signature).
        SyntaxPalette::Darcula => SyntaxTheme {
            keyword: base16(0xcc7832),
            function: base16(0xff_c6_6d),
            type_: base16(0xa9_b7_c6),
            constant: base16(0x98_76_aa),
            string: base16(0x6a_87_59),
            string_special: base16(0xcc7832),
            tag: base16(0xe8_bf_6a),
            tag_doctype: base16(0xe8_bf_6a),
            comment: base16(0x80_80_80),
            default: base16(0xa9_b7_c6),
            keyword_style: TokenStyle::BOLD,
            string_special_style: TokenStyle::PLAIN,
            comment_style: TokenStyle::PLAIN,
        },
    }
}

/// Light-background variant for `palette`. Families with a published light
/// theme map to it; dark-only families (Monokai / Nord / Darcula) fall back
/// to Daruda Light so a light editor stays legible. Values are floored for
/// contrast on the light editor background (`#fafafa`); Daruda Light is fully
/// WCAG-AA, the ported families keep their identity at their native contrast.
fn light_syntax_theme(palette: SyntaxPalette) -> SyntaxTheme {
    match palette {
        // Daruda Light — the readability-tuned light default. Also the
        // fallback for dark-only families on a light editor.
        SyntaxPalette::Daruda
        | SyntaxPalette::Monokai
        | SyntaxPalette::Nord
        | SyntaxPalette::Darcula => SyntaxTheme {
            keyword: base16(0x7c_3a_ed),
            function: base16(0x1d_4e_d8),
            type_: base16(0xb4_53_09),
            constant: base16(0xc2_41_0c),
            string: base16(0x15_80_3d),
            string_special: base16(0x0e_74_90),
            tag: base16(0xb9_1c_1c),
            tag_doctype: base16(0x92_40_0e),
            comment: base16(0x6e_73_7d),
            default: base16(0x38_3a_42),
            keyword_style: TokenStyle::BOLD,
            string_special_style: TokenStyle::BOLD,
            comment_style: TokenStyle::ITALIC,
        },
        SyntaxPalette::OneDark => SyntaxTheme {
            keyword: base16(0xa6_26_a4),
            function: base16(0x40_78_f2),
            type_: base16(0xc1_84_01),
            constant: base16(0x98_68_01),
            string: base16(0x50_a1_4f),
            string_special: base16(0x01_84_bc),
            tag: base16(0xe4_56_49),
            tag_doctype: base16(0x83_84_8c),
            comment: base16(0x83_84_8c),
            default: base16(0x38_3a_42),
            keyword_style: TokenStyle::PLAIN,
            string_special_style: TokenStyle::PLAIN,
            comment_style: TokenStyle::PLAIN,
        },
        SyntaxPalette::TokyoNight => SyntaxTheme {
            keyword: base16(0x65_35_9d),
            function: base16(0x29_59_aa),
            type_: base16(0x00_6c_86),
            constant: base16(0x96_50_27),
            string: base16(0x38_5f_0d),
            string_special: base16(0x00_6c_86),
            tag: base16(0x8c_43_51),
            tag_doctype: base16(0x82_85_8f),
            comment: base16(0x82_85_8f),
            default: base16(0x34_3b_59),
            keyword_style: TokenStyle::PLAIN,
            string_special_style: TokenStyle::PLAIN,
            comment_style: TokenStyle::ITALIC,
        },
        SyntaxPalette::CatppuccinMocha => SyntaxTheme {
            keyword: base16(0x88_39_ef),
            function: base16(0x1e_66_f5),
            type_: base16(0xcb_81_1a),
            constant: base16(0xfb_5c_01),
            string: base16(0x40_a0_2b),
            string_special: base16(0xe6_5b_c1),
            tag: base16(0xd2_0f_39),
            tag_doctype: base16(0x7c_7f_93),
            comment: base16(0x7c_7f_93),
            default: base16(0x4c_4f_69),
            keyword_style: TokenStyle::PLAIN,
            string_special_style: TokenStyle::PLAIN,
            comment_style: TokenStyle::PLAIN,
        },
        SyntaxPalette::Dracula => SyntaxTheme {
            keyword: base16(0xa3_14_4d),
            function: base16(0x14_71_0a),
            type_: base16(0x03_6a_96),
            constant: base16(0x64_4a_c9),
            string: base16(0x84_6e_15),
            string_special: base16(0xa3_4d_14),
            tag: base16(0xcb_3a_2a),
            tag_doctype: base16(0x6c_66_4b),
            comment: base16(0x6c_66_4b),
            default: base16(0x1f_1f_1f),
            keyword_style: TokenStyle::PLAIN,
            string_special_style: TokenStyle::PLAIN,
            comment_style: TokenStyle::PLAIN,
        },
        SyntaxPalette::GitHubDark => SyntaxTheme {
            keyword: base16(0xcf_22_2e),
            function: base16(0x82_50_df),
            type_: base16(0x95_38_00),
            constant: base16(0x09_69_da),
            string: base16(0x0a_30_69),
            string_special: base16(0x0a_30_69),
            tag: base16(0x1a_7f_37),
            tag_doctype: base16(0x1a_7f_37),
            comment: base16(0x6e_77_81),
            default: base16(0x1f_23_28),
            keyword_style: TokenStyle::PLAIN,
            string_special_style: TokenStyle::BOLD,
            comment_style: TokenStyle::PLAIN,
        },
        SyntaxPalette::MaterialPalenight => SyntaxTheme {
            keyword: base16(0x7c_4d_ff),
            function: base16(0x61_82_b8),
            type_: base16(0xd0_7c_09),
            constant: base16(0xf6_61_38),
            string: base16(0x78_9d_43),
            string_special: base16(0x34_9f_a7),
            tag: base16(0xe5_39_35),
            tag_doctype: base16(0x6f_89_96),
            comment: base16(0x6f_89_96),
            default: base16(0x54_6e_7a),
            keyword_style: TokenStyle::PLAIN,
            string_special_style: TokenStyle::PLAIN,
            comment_style: TokenStyle::ITALIC,
        },
        SyntaxPalette::GruvboxDark => SyntaxTheme {
            keyword: base16(0x9d_00_06),
            function: base16(0x42_7b_58),
            type_: base16(0xb5_76_14),
            constant: base16(0x8f_3f_71),
            string: base16(0x79_74_0e),
            string_special: base16(0xaf_3a_03),
            tag: base16(0x9d_00_06),
            tag_doctype: base16(0xaf_3a_03),
            comment: base16(0x92_83_74),
            default: base16(0x3c_38_36),
            keyword_style: TokenStyle::PLAIN,
            string_special_style: TokenStyle::PLAIN,
            comment_style: TokenStyle::PLAIN,
        },
        SyntaxPalette::SolarizedDark => SyntaxTheme {
            keyword: base16(0x85_99_00),
            function: base16(0x26_8b_d2),
            type_: base16(0xb5_89_00),
            constant: base16(0xd3_36_82),
            string: base16(0x2a_a1_98),
            string_special: base16(0xcb4b16),
            tag: base16(0x26_8b_d2),
            tag_doctype: base16(0x77_89_89),
            comment: base16(0x77_89_89),
            default: base16(0x62_77_7f),
            keyword_style: TokenStyle::PLAIN,
            string_special_style: TokenStyle::PLAIN,
            comment_style: TokenStyle::PLAIN,
        },
        SyntaxPalette::AyuMirage => SyntaxTheme {
            keyword: base16(0xf0_67_06),
            function: base16(0xc1_86_00),
            type_: base16(0x2a_97_e4),
            constant: base16(0xa3_7a_cc),
            string: base16(0x76_9e_00),
            string_special: base16(0x3a_a1_7f),
            tag: base16(0x31_9c_c0),
            tag_doctype: base16(0x84_85_8a),
            comment: base16(0x84_85_8a),
            default: base16(0x5c_61_66),
            keyword_style: TokenStyle::PLAIN,
            string_special_style: TokenStyle::PLAIN,
            comment_style: TokenStyle::ITALIC,
        },
        SyntaxPalette::NightOwl => SyntaxTheme {
            keyword: base16(0x99_4c_c3),
            function: base16(0x48_76_d6),
            type_: base16(0x0c_96_9b),
            constant: base16(0xaa_09_82),
            string: base16(0xc9_67_65),
            string_special: base16(0x0c_96_9b),
            tag: base16(0x99_4c_c3),
            tag_doctype: base16(0x7c_85_9c),
            comment: base16(0x7c_85_9c),
            default: base16(0x40_3f_53),
            keyword_style: TokenStyle::ITALIC,
            string_special_style: TokenStyle::PLAIN,
            comment_style: TokenStyle::ITALIC,
        },
    }
}

/// Foreground colour for a tree-sitter highlight capture name. Unrecognised
/// captures (and the empty string) fall back to the default editor
/// foreground, so every token gets an explicit foreground. Used by the diff
/// view's own highlighter.
pub fn syntax_color(capture: &str) -> Hsla {
    syntax_color_of(SyntaxPalette::Daruda, false, capture)
}

/// Foreground colour for a capture in `palette` at the editor's lightness.
/// Thin wrapper over [`SyntaxTheme::color_for`] for call sites that hold only
/// a palette.
pub fn syntax_color_of(palette: SyntaxPalette, is_light: bool, capture: &str) -> Hsla {
    syntax_theme_of(palette, is_light).color_for(capture)
}

/// Build `gpui_component`'s per-capture [`SyntaxColors`] from
/// [`syntax_theme`] so the raw editor highlights with the exact colours
/// the diff view uses. The grouping mirrors [`syntax_color`] one-to-one;
/// every field is set explicitly so a future upstream field addition
/// fails to compile here rather than silently diverging.
pub fn editor_syntax_colors() -> SyntaxColors {
    editor_syntax_colors_of(SyntaxPalette::Daruda, false)
}

/// Build `gpui_component`'s per-capture [`SyntaxColors`] for `palette` at the
/// editor's lightness. The styled buckets (keyword / string_special /
/// comment) carry the palette's bold/italic channel; every other bucket is
/// colour-only.
pub fn editor_syntax_colors_of(palette: SyntaxPalette, is_light: bool) -> SyntaxColors {
    use SyntaxBucket::*;
    let t = syntax_theme_of(palette, is_light);
    // Colour + non-color channel for a bucket, as one `ThemeStyle`.
    let b = |bucket: SyntaxBucket| {
        let st = t.style(bucket);
        let mut ts = ThemeStyle::new(t.color(bucket));
        if st.bold {
            ts = ts.bold();
        }
        if st.italic {
            ts = ts.italic();
        }
        Some(ts)
    };
    SyntaxColors {
        keyword: b(Keyword),
        function: b(Function),
        title: b(Function),
        type_: b(Type),
        enum_: b(Type),
        constructor: b(Type),
        label: b(Type),
        preproc: b(Type),
        embedded: b(Type),
        constant: b(Constant),
        boolean: b(Constant),
        number: b(Constant),
        attribute: b(Constant),
        variant: b(Constant),
        link_uri: b(Constant),
        string: b(String),
        text_literal: b(String),
        string_escape: b(StringSpecial),
        string_regex: b(StringSpecial),
        string_special: b(StringSpecial),
        string_special_symbol: b(StringSpecial),
        tag: b(Tag),
        variable_special: b(Tag),
        link_text: b(Tag),
        tag_doctype: b(TagDoctype),
        comment: b(Comment),
        comment_doc: b(Comment),
        hint: b(Comment),
        predictive: b(Comment),
        variable: b(Default),
        property: b(Default),
        operator: b(Default),
        punctuation: b(Default),
        punctuation_bracket: b(Default),
        punctuation_delimiter: b(Default),
        punctuation_list_marker: b(Default),
        punctuation_special: b(Default),
        emphasis: b(Default),
        emphasis_strong: b(Default),
        primary: b(Default),
    }
}
/// Gap between `+N` and `-N` in the diff stat badge (px).
pub const FILE_DIFF_STAT_GAP: f32 = GAP_SM;
/// Diff stat added-lines count color (+N) — same green as the `+` marker.
pub const FILE_DIFF_STAT_ADD: Hsla = DIFF_ADD_FG;
/// Diff stat removed-lines count color (-N) — same red as the `-` marker.
pub const FILE_DIFF_STAT_DEL: Hsla = DIFF_DEL_FG;
/// Diff stat and file-status badge font size (px).
pub const FILE_DIFF_STAT_FONT_SIZE: f32 = FONT_SIZE_SM;
/// Word-level diff insertion highlight — the line bg tint at higher
/// alpha so the intra-line span reads stronger while staying in-palette.
pub const FILE_DIFF_WORD_ADD_BG: Hsla = with_alpha(SUCCESS, 0.30);
/// Word-level diff deletion highlight — the line bg tint at higher alpha.
pub const FILE_DIFF_WORD_DEL_BG: Hsla = with_alpha(ERROR, 0.30);
/// Search panel height (px).
pub const FILE_VIEWER_SEARCH_PANEL_H: f32 = 36.0;
/// Search panel horizontal padding (px).
pub const FILE_VIEWER_SEARCH_PAD_X: f32 = 12.0;
/// "No matches" label color (same value as SEARCH_LABEL_EMPTY).
pub const FILE_VIEWER_SEARCH_EMPTY: Hsla = hsla(0.0, 0.50, 0.65, 1.0);
/// Non-focused match row highlight background.
pub const FILE_VIEWER_SEARCH_MATCH_BG: Hsla = hsla(43.0, 0.70, 0.30, 0.45);
/// Focused match row highlight background.
pub const FILE_VIEWER_SEARCH_FOCUSED_BG: Hsla = hsla(43.0, 0.85, 0.48, 0.60);
/// Right margin between the search panel and the window edge (px).
pub const FILE_VIEWER_SEARCH_MARGIN_R: f32 = 16.0;
/// Top margin between the toolbar and the search panel (px).
pub const FILE_VIEWER_SEARCH_MARGIN_T: f32 = PAD_STANDARD;
/// Fixed width of the search panel (px) — matches the terminal search bar.
pub const FILE_VIEWER_SEARCH_PANEL_W: f32 = 380.0;
/// Search panel font size (px).
pub const FILE_VIEWER_SEARCH_FONT_SIZE: f32 = FONT_SIZE_MD;
/// Gap between items inside the search panel (px).
pub const FILE_VIEWER_SEARCH_ITEM_GAP: f32 = GAP_LG;
/// Match counter font size inside the input area (px).
pub const FILE_VIEWER_SEARCH_COUNTER_SIZE: f32 = 11.0;
/// Button horizontal padding (px).
pub const FILE_VIEWER_SEARCH_BTN_PAD_X: f32 = PAD_SM;
/// Left margin of the close button to visually separate it from nav buttons (px).
pub const FILE_VIEWER_SEARCH_BTN_ML: f32 = PAD_XS;
/// Horizontal scroll origin for file viewer scroll-to-match (always 0, px).
pub const FILE_VIEWER_SCROLL_ORIGIN_X: f32 = 0.0;
/// H1 heading font size (px) — DESIGN §Markdown Viewer.
pub const MD_H1_FONT_SIZE: f32 = 18.0;
/// H2 heading font size (px) — DESIGN §Markdown Viewer.
pub const MD_H2_FONT_SIZE: f32 = 15.0;
/// H3 heading font size (px) — DESIGN §Markdown Viewer.
pub const MD_H3_FONT_SIZE: f32 = 13.0;
/// H4–H6 heading font size (same as body).
pub const MD_H4_FONT_SIZE: f32 = FILE_VIEWER_FONT_SIZE;
/// H2 heading text color.
pub const MD_H2_COLOR: Hsla = hsla(0.0, 0.0, 0.92, 1.0);
/// Code block corner radius (px).
pub const MD_CODE_BLOCK_RADIUS: f32 = RADIUS_MD;
/// Code block horizontal padding (px).
pub const MD_CODE_BLOCK_PAD_X: f32 = 12.0;
/// Code block vertical padding (px).
pub const MD_CODE_BLOCK_PAD_Y: f32 = PAD_STANDARD;
/// Blockquote left border width (px).
pub const MD_BLOCKQUOTE_BORDER_W: f32 = 3.0;
/// Blockquote left padding (px).
pub const MD_BLOCKQUOTE_PAD_L: f32 = 12.0;
/// Horizontal rule height (px).
pub const MD_RULE_H: f32 = 1.0;
/// List item indentation (px).
pub const MD_LIST_INDENT: f32 = 16.0;
/// Outer horizontal padding for the Markdown viewer body (px).
pub const MD_BODY_PAD_X: f32 = 24.0;
/// Outer vertical padding for the Markdown viewer body (px).
pub const MD_BODY_PAD_Y: f32 = 16.0;
/// Vertical gap between top-level blocks (px).
pub const MD_BLOCK_GAP: f32 = GAP_LG;
/// Extra top margin for headings (px).
pub const MD_HEADING_MARGIN_TOP: f32 = 12.0;
/// Footnote reference / definition text color.
pub const MD_FOOTNOTE_COLOR: Hsla = hsla(209.0, 0.45, 0.60, 1.0);
/// Footnote font size (slightly smaller than body, px).
pub const MD_FOOTNOTE_FONT_SIZE: f32 = FONT_SIZE_SM;
/// HTML passthrough font size (px).
pub const MD_HTML_FONT_SIZE: f32 = FONT_SIZE_SM;
/// Horizontal cell padding (px).
pub const MD_TABLE_CELL_PAD_X: f32 = PAD_LG;
/// Vertical cell padding (px).
pub const MD_TABLE_CELL_PAD_Y: f32 = 5.0;
/// Minimum width of a table cell (px).
pub const MD_TABLE_CELL_MIN_W: f32 = 60.0;
/// Gap between list items (px).
pub const MD_LIST_ITEM_GAP: f32 = GAP_XS;
/// Gap between the bullet/number and item text in a list row (px).
pub const MD_LIST_ROW_GAP: f32 = GAP_STANDARD;
/// Corner radius for inline code, selection highlight, and table container (px).
pub const MD_BLOCK_RADIUS: f32 = RADIUS_XS;
/// Vertical margin above/below standalone block elements (Rule, Table) (px).
pub const MD_BLOCK_MARGIN_Y: f32 = PAD_XS;
/// Horizontal padding for inline code (px).
pub const MD_CODE_INLINE_PAD_X: f32 = 3.0;
/// Maximum rendered height of a block image / diagram in the preview (px).
/// Width fits the pane; height is capped so a tall image can't dominate.
pub const MD_IMAGE_MAX_HEIGHT: f32 = 600.0;
/// Height of an image embedded in a text line (px) — sized to the body line so
/// inline icons/badges flow with the text instead of breaking the line.
pub const MD_INLINE_IMAGE_HEIGHT: f32 = FILE_VIEWER_FONT_SIZE * 1.3;
/// Minimum dimension for a divider / header lane (px) — 1 device pixel.
/// Used as `min_w` / `min_h` inside flex layouts so the lane always
/// has a clickable line even when its surrounding area collapses.
pub const RENDER_MIN_DIM: f32 = 1.0;
/// Corner radius of the toast pill (px).
pub const TOAST_RADIUS: f32 = RADIUS_LG;
/// Horizontal padding inside the toast pill (px).
pub const TOAST_PAD_X: f32 = 16.0;
/// Vertical padding inside the toast pill (px).
pub const TOAST_PAD_Y: f32 = PAD_LG;
/// Message font size (px).
pub const TOAST_FONT_SIZE: f32 = FONT_SIZE_LG;
/// Gap between message and action button (px).
pub const TOAST_GAP: f32 = 12.0;
/// Minimum width of the toast pill (px).
pub const TOAST_MIN_W: f32 = 240.0;
/// Maximum width of the toast pill (px).
pub const TOAST_MAX_W: f32 = 480.0;
/// Vertical gap between stacked toasts.
pub const TOAST_STACK_GAP: f32 = GAP_SM;
/// Padding below the toast stack before the status bar starts.
pub const TOAST_STACK_BOTTOM_PAD: f32 = PAD_XS;
/// Title-row line height fudge so the text vertically centers in the
/// pill at the chosen font size.
pub const TOAST_TITLE_FONT_SIZE: f32 = FONT_SIZE_MD;
/// Repeat-counter font size — smaller than the title.
pub const TOAST_REPEAT_FONT_SIZE: f32 = FONT_SIZE_XS;
/// Minimum pixel width for the leading severity bar.
pub const TOAST_SEVERITY_BAR_W: f32 = 3.0;
/// Horizontal padding inside the `×N` repeat-counter chip.
pub const TOAST_REPEAT_PAD_X: f32 = PAD_SM;
/// Vertical padding inside the `×N` repeat-counter chip.
pub const TOAST_REPEAT_PAD_Y: f32 = GAP_XS;
/// Modal panel width for the error-report details body — wider than the
/// default `MODAL_PANEL_WIDTH` because the plain-text rendering carries
/// stack frames and source-chain entries that wrap awkwardly at 420 px.
pub const ERROR_MODAL_WIDTH: f32 = 640.0;
/// Body monospace font size. Slightly smaller than the modal title so a
/// long backtrace fits without horizontal scroll.
pub const ERROR_MODAL_BODY_FONT_SIZE: f32 = FONT_SIZE_SM;
/// Padding inside the body container.
pub const ERROR_MODAL_BODY_PAD: f32 = PAD_LG;
/// Maximum body height — beyond this the container scrolls.
pub const ERROR_MODAL_BODY_MAX_H: f32 = 360.0;
/// Default terminal canvas background (solid black).
pub const TERMINAL_BG: Hsla = hsla(0.0, 0.0, 0.0, 1.0);

// ============================================================================
// Unified scrollbar metrics — shared across docks, settings, textareas
// ============================================================================

/// Scrollbar thumb width (px). Shared by docks, settings, and textarea.
pub const SCROLLBAR_W: f32 = 4.0;
/// Right margin between the thumb and the panel edge (px).
pub const SCROLLBAR_MARGIN_R: f32 = GAP_XS;
/// Minimum scrollbar thumb height so it stays clickable (px).
pub const SCROLLBAR_MIN_THUMB_H: f32 = 24.0;
/// Scrollbar thumb fill.
pub const SCROLLBAR_THUMB: Hsla = hsla(0.0, 0.0, 1.0, 0.25);
/// Scrollbar thumb fill on hover.
pub const SCROLLBAR_THUMB_HOVER: Hsla = hsla(0.0, 0.0, 1.0, 0.45);
/// Scrollbar track background (subtle, nearly transparent fill).
pub const SCROLLBAR_TRACK_BG: Hsla = hsla(0.0, 0.0, 1.0, 0.04);
/// Right gutter a `ui::scroll_area` reserves beside its content so text
/// clears the draggable thumb (px).
pub const SCROLL_AREA_GUTTER: f32 = 10.0;

// ============================================================================
// Shared layout tokens — single source of truth for dimensions/metrics
// ============================================================================

pub const PAD_STANDARD: f32 = 8.0;
pub const PAD_LG: f32 = 10.0;
pub const PAD_XL: f32 = 14.0;
pub const PAD_SM: f32 = 6.0;
pub const PAD_XS: f32 = 4.0;
pub const GAP_STANDARD: f32 = 6.0;
pub const GAP_SM: f32 = 4.0;
pub const GAP_LG: f32 = 8.0;
pub const GAP_XS: f32 = 2.0;
pub const FONT_SIZE_SM: f32 = 11.0;
pub const FONT_SIZE_MD: f32 = 12.0;
pub const FONT_SIZE_XS: f32 = 10.0;
/// Smallest caption size — for dense labels that must fit a narrow
/// column (e.g. Usage tab stat / totals labels in the 220px dock).
pub const FONT_SIZE_XXS: f32 = 8.0;
pub const FONT_SIZE_LG: f32 = 13.0;
pub const RADIUS_SM: f32 = 4.0;
pub const RADIUS_XS: f32 = 2.0;
pub const RADIUS_MD: f32 = 6.0;
pub const RADIUS_LG: f32 = 8.0;

// Main-area empty-state shown when the active lane/project root is
// inaccessible (Missing / AccessDenied). Centered icon + title + body
// + Remove button stack.
pub const MAIN_EMPTY_STATE_ICON_SIZE: f32 = 36.0;
pub const MAIN_EMPTY_STATE_TITLE_FONT_SIZE: f32 = FONT_SIZE_LG;
pub const MAIN_EMPTY_STATE_BODY_FONT_SIZE: f32 = FONT_SIZE_MD;
pub const MAIN_EMPTY_STATE_GAP: f32 = 12.0;
pub const MAIN_EMPTY_STATE_BODY_MAX_W: f32 = 360.0;
/// Offset from the cursor hotspot to the top-left corner of the drag pill (px).
/// Applied as padding inside the transparent ghost wrapper so the pill appears
/// just below and to the right of the cursor regardless of where the user
/// clicked within the source row.
pub const DRAG_PILL_CURSOR_OFFSET: f32 = 4.0;
/// Leading indicator footprint on the lane row. All four states
/// share the same 3×3 dot-grid shape; only color/animation differ.
pub const STATUS_INDICATOR_SIZE: f32 = 16.0;
/// Sub-row per-session badge footprint (Phase D).
pub const STATUS_INDICATOR_BADGE_SIZE: f32 = 12.0;
/// Width of the cell that holds the indicator inside the lane row,
/// inserted between the active-row accent bar and the body.
pub const STATUS_INDICATOR_CELL_WIDTH: f32 = 22.0;
/// Status-badge animation tick interval (~4 fps). One shared
/// `StatusPulseClock` tick fires this often; every badge derives its
/// frame from the tick rather than from a per-frame `with_animation`
/// (which would repaint the whole window ~60×/s). The pump notifies
/// every window with an animating session — including backgrounded
/// ones — so the rate is kept low to bound the off-focus repaint cost
/// (Pitfall #10). The 6-frame comet still reads as motion at 4 fps.
pub const STATUS_INDICATOR_TICK_MS: u64 = 250;
/// Centre-dot alpha multiplier for the ExecutingTool ring animation.
pub const STATUS_INDICATOR_RING_CENTER_ALPHA: f32 = 0.15;
/// Pulse minimum opacity at the off-beat of the NeedsAttention cycle.
pub const STATUS_INDICATOR_PULSE_OPACITY_MIN: f32 = 0.4;
/// Diameter of one dot inside its 3×3 cell (Working state), as a ratio
/// of the cell width. Higher = chunkier dots; lower = airier grid.
pub const STATUS_INDICATOR_DOT_GRID_RATIO: f32 = 0.6;
/// Tail-end alpha multiplier for the Working-state dot grid. The head
/// dot renders at `color.a`; the dot 8 steps behind renders at
/// `color.a * STATUS_INDICATOR_DOT_GRID_TAIL_ALPHA_MIN`. Linear between.
pub const STATUS_INDICATOR_DOT_GRID_TAIL_ALPHA_MIN: f32 = 0.18;
pub const STATUS_WORKING_LIGHT: Hsla = hsla(210.0, 1.0, 0.61, 1.0);
pub const STATUS_EXECUTING_TOOL_LIGHT: Hsla = hsla(32.0, 0.95, 0.44, 1.0);
pub const STATUS_NEEDS_ATTENTION_LIGHT: Hsla = hsla(0.0, 0.72, 0.51, 1.0);
pub const STATUS_IDLE_LIGHT: Hsla = hsla(142.0, 0.71, 0.45, 1.0);
pub const STATUS_CONNECTING_LIGHT: Hsla = hsla(220.0, 0.09, 0.46, 1.0);
pub const STATUS_WORKING_DARK: Hsla = hsla(210.0, 1.0, 0.68, 1.0);
pub const STATUS_EXECUTING_TOOL_DARK: Hsla = hsla(43.0, 0.96, 0.56, 1.0);
pub const STATUS_NEEDS_ATTENTION_DARK: Hsla = hsla(0.0, 0.84, 0.60, 1.0);
pub const STATUS_IDLE_DARK: Hsla = hsla(160.0, 0.64, 0.52, 1.0);
pub const STATUS_CONNECTING_DARK: Hsla = hsla(220.0, 0.09, 0.65, 1.0);
pub const STATUS_BADGES_ROW_GAP: f32 = 5.0;
pub const STATUS_BADGES_ROW_TOP_MARGIN: f32 = 3.0;
pub const STATUS_BADGES_LABEL_FONT_SIZE: f32 = FONT_SIZE_XS;
pub const STATUS_BADGES_LABEL_GAP: f32 = GAP_STANDARD;
pub const STATUS_BADGE_ACTIVE_OUTLINE: Hsla = hsla(0.0, 0.0, 1.0, 0.85);
/// Pixels added to the badge frame to host the outline without
/// occluding the inner colour.
pub const STATUS_BADGE_ACTIVE_OUTER_PAD: f32 = GAP_XS;
/// Number of leading characters of the session_id shown in the
/// per-badge tooltip (followed by `…`). 8 chars give the user enough
/// to disambiguate between concurrent sessions while staying compact
/// in the sub-row's narrow space.
pub const STATUS_BADGE_TOOLTIP_SESSION_PREFIX_LEN: usize = 8;
/// Bright blue — the banner's icon tint and the opaque base every other
/// banner surface (bg / border / hover) derives from via [`with_alpha`].
pub const AGENT_BANNER_ICON: Hsla = hsla(210.0, 1.0, 0.68, 1.0);
pub const AGENT_BANNER_BG: Hsla = with_alpha(AGENT_BANNER_ICON, 0.08);
pub const AGENT_BANNER_BORDER: Hsla = with_alpha(AGENT_BANNER_ICON, 0.20);
pub const AGENT_BANNER_HOVER_BG: Hsla = with_alpha(AGENT_BANNER_ICON, 0.14);
pub const AGENT_BANNER_PAD_X: f32 = 12.0;
pub const AGENT_BANNER_PAD_Y: f32 = PAD_STANDARD;
pub const AGENT_BANNER_GAP: f32 = GAP_LG;
pub const AGENT_BANNER_RADIUS: f32 = RADIUS_MD;
pub const AGENT_BANNER_FONT_SIZE: f32 = FONT_SIZE_SM;
pub const AGENT_BANNER_MARGIN_X: f32 = PAD_STANDARD;
pub const AGENT_BANNER_MARGIN_Y: f32 = PAD_SM;
/// Outer horizontal padding for right-panel content rows (px).
pub const RIGHT_PANEL_PAD_X: f32 = PAD_LG;
/// Outer vertical padding between right-panel sections (px).
pub const RIGHT_PANEL_PAD_Y: f32 = PAD_SM;
/// Vertical gap between two siblings inside a single right-panel row (px).
pub const RIGHT_PANEL_ROW_GAP: f32 = GAP_LG;
/// Vertical gap between major sections in a right-dock tab body
/// (header, search, sections, list). Shared by all four right-dock
/// views via `right_dock::right_panel_body()` so section spacing is
/// uniform.
pub const RIGHT_PANEL_SECTION_GAP: f32 = GAP_LG;
/// Vertical padding inside a single right-panel body row (px).
pub const RIGHT_PANEL_ROW_PAD_Y: f32 = 3.0;
/// Font size for the right-panel body rows (px).
/// Matches `AGENT_CHAT_MSG_FONT_SIZE` so the four right-panel tabs feel
/// part of the same typographic family as the original chat panel.
pub const RIGHT_PANEL_BODY_FONT_SIZE: f32 = FONT_SIZE_MD;
/// Font size for inline labels (gauge rows, percent / reset text).
pub const RIGHT_PANEL_LABEL_FONT_SIZE: f32 = FONT_SIZE_XS;
/// Fixed width allocated to the leading state-indicator glyph so titles
/// across rows align on a single column (px).
pub const RIGHT_PANEL_TASK_INDICATOR_W: f32 = 14.0;
/// Vertical padding for a right-dock tab header row (px). Shared by the
/// Tasks / Skills / Tools headers so they sit at a uniform height.
pub const RIGHT_PANEL_HEADER_PAD_Y: f32 = PAD_SM;
/// Maximum number of characters of an `Error` task message echoed
/// inline next to the row title before it's truncated with `…`.
pub const RIGHT_PANEL_TASK_ERROR_TRUNCATE: usize = 30;
/// Diameter of the `Running`-row pulse dot rendered in the leading
/// indicator column (px). Drawn as a filled circle whose alpha
/// oscillates between [`RIGHT_PANEL_TASK_PULSE_MIN_ALPHA`] and
/// [`RIGHT_PANEL_TASK_PULSE_MAX_ALPHA`] over
/// [`RIGHT_PANEL_TASK_PULSE_PERIOD_SEC`].
pub const RIGHT_PANEL_TASK_DOT_SIZE_PX: f32 = 8.0;
/// Pulse period for the `Running`-row dot (seconds, end-to-end).
pub const RIGHT_PANEL_TASK_PULSE_PERIOD_SEC: f32 = 1.5;
/// Lower bound of the pulse alpha — the dot dims to this opacity at
/// the midpoint of each period before returning to full opacity.
pub const RIGHT_PANEL_TASK_PULSE_MIN_ALPHA: f32 = 0.35;
/// Upper bound of the pulse alpha — the dot reads as fully lit at
/// the start/end of each period.
pub const RIGHT_PANEL_TASK_PULSE_MAX_ALPHA: f32 = 1.0;
/// Live-tick interval that drives both the pulse animation and the
/// duration text on `Running` rows (ms). Chosen as a sub-second
/// interval so the pulse animates smoothly; the duration text only
/// changes once per second so 3 of every 4 ticks are visual-no-ops on
/// that column, which is cheap relative to one full panel rerender.
pub const RIGHT_PANEL_TASK_LIVE_TICK_MS: u64 = 250;
/// Font size for the inline duration text (px). One step below
/// [`RIGHT_PANEL_BODY_FONT_SIZE`] so it reads as auxiliary metadata,
/// not as a primary action surface.
pub const RIGHT_PANEL_TASK_DURATION_FONT_SIZE: f32 = FONT_SIZE_SM;
/// Font size for the `failures N/M` counter (px).
pub const RIGHT_PANEL_TASK_FAILURE_FONT_SIZE: f32 = FONT_SIZE_XS;
/// Horizontal gap between the session-id badge and its trailing
/// status glyph (`⟳` / `●` / `⚠`) (px).
pub const RIGHT_PANEL_TASK_SESSION_GAP: f32 = GAP_SM;
/// Width of the error border around the branch input (px).
pub const TASK_EDIT_BRANCH_INVALID_BORDER_W: f32 = 1.0;
/// Corner radius matched to the embedded `TextInput` so the error
/// border doesn't show as a square halo around a rounded widget (px).
pub const TASK_EDIT_BRANCH_INVALID_RADIUS: f32 = RADIUS_SM;
/// Horizontal padding inside the pill — slightly wider than the
/// default xsmall padding so the chevron `▾` has room to breathe and
/// the state label stays visually centered.
pub const RIGHT_PANEL_STATUS_PILL_PADDING_X_PX: f32 = PAD_STANDARD;
/// Corner radius for the pill background. A small radius keeps the
/// row's vertical rhythm intact while still reading as a button.
pub const RIGHT_PANEL_STATUS_PILL_RADIUS_PX: f32 = RADIUS_SM;
/// Alpha multiplier applied to the state colour for the pill's
/// background tint. The state colour stays the source of truth (no
/// per-state bg constants); the pill simply softens it so the row
/// label still leads visually.
pub const RIGHT_PANEL_STATUS_PILL_BG_ALPHA: f32 = 0.12;
/// macOS system monospace font. Used by widgets (`ui::Badge`,
/// future code-snippet labels, etc.) that need a fixed-width
/// treatment without going through the user's configurable terminal
/// font. daruda is macOS-only (see project README), so this can be
/// a literal `&'static str` rather than a runtime lookup.
pub const FONT_FAMILY_MONOSPACE: &str = "Menlo";
/// Corner radius (px). Slightly rounded so the badge reads as one
/// unit but not pill-shaped.
pub const BADGE_RADIUS: f32 = RADIUS_XS;
/// Horizontal padding inside a `Badge` (px).
pub const BADGE_PAD_X: f32 = PAD_XS;
/// Vertical padding inside a `Badge` (px).
pub const BADGE_PAD_Y: f32 = 0.0;
/// Font size inside a `Badge` (px). Slightly smaller than body text.
pub const BADGE_FONT_SIZE: f32 = FONT_SIZE_SM;
/// Bar height (px) for the 5-hour / 7-day gauges. 8 px reads as a
/// "ribbon" rather than a thick block — same visual weight as
/// macOS download progress bars.
pub const GAUGE_BAR_HEIGHT: f32 = 8.0;
/// Corner radius for the gauge bar's outer rounded rectangle. Half
/// the height — pure pill shape.
pub const GAUGE_BAR_RADIUS: f32 = RADIUS_SM;
/// Vertical padding (px) of the status row.
pub const STATUS_PILL_PAD_Y: f32 = PAD_XS;
/// Diameter (px) of the small dot rendered inside the status row.
pub const STATUS_PILL_DOT_SIZE: f32 = 8.0;
/// Gap (px) between the dot and the label inside the status row.
pub const STATUS_PILL_GAP: f32 = GAP_STANDARD;

// ---- Usage tab dashboard (widget-style cards) ----
// Modelled on the Übersicht `claude-usage` widget. All sizes here
// (G4-exempt is NOT in play — these are the designated theme home).
/// Gap between the header title and the plan badge.
pub const USAGE_HEADER_GAP: f32 = GAP_LG;
/// Title ("Claude Code") font size (px).
pub const USAGE_TITLE_FONT_SIZE: f32 = FONT_SIZE_LG;
/// Plan-badge font size (px).
pub const USAGE_PLAN_BADGE_FONT_SIZE: f32 = FONT_SIZE_XS;
/// Plan-badge horizontal padding (px).
pub const USAGE_PLAN_BADGE_PAD_X: f32 = PAD_SM;
/// Plan-badge vertical padding (px).
pub const USAGE_PLAN_BADGE_PAD_Y: f32 = GAP_XS;
/// Plan-badge corner radius (px) — reads as a pill.
pub const USAGE_PLAN_BADGE_RADIUS: f32 = RADIUS_LG;
/// Logo-chip / plan-badge background tint.
pub const USAGE_ACCENT_CHIP_BG: Hsla = ACCENT;
/// Logo-chip / plan-badge foreground.
pub const USAGE_ACCENT_CHIP_FG: Hsla = ACCENT_FG;
/// Vertical gap between stacked gauge cards (px).
pub const USAGE_CARD_GAP: f32 = GAP_LG;
/// Big utilization-percent font size on a gauge card (px).
pub const USAGE_GAUGE_PERCENT_FONT_SIZE: f32 = 18.0;
/// 7-day chart: bar height (px) the busiest day maps to.
pub const USAGE_CHART_BAR_MAX_HEIGHT: f32 = 40.0;
/// 7-day chart: minimum bar height (px) so a zero/low day still shows.
pub const USAGE_CHART_BAR_MIN_HEIGHT: f32 = 3.0;
/// 7-day chart: gap between adjacent bars (px).
pub const USAGE_CHART_BAR_GAP: f32 = GAP_SM;
/// 7-day chart: bar corner radius (px).
pub const USAGE_CHART_BAR_RADIUS: f32 = RADIUS_SM;
/// 7-day chart: gap between a bar and its weekday label (px).
pub const USAGE_CHART_LABEL_GAP: f32 = GAP_XS;
/// 7-day chart: weekday-label font size (px).
pub const USAGE_CHART_LABEL_FONT_SIZE: f32 = FONT_SIZE_XS;
/// Today's bar fill color in the 7-day chart.
pub const USAGE_CHART_BAR_TODAY: Hsla = ACCENT;
/// Non-today bar fill color in the 7-day chart.
pub const USAGE_CHART_BAR_OTHER: Hsla = ACCENT_MUTED;
/// "📎 N" chip background — surfaces auxiliary file presence.
pub const SKILL_AUX_CHIP_BG: Hsla = hsla(0.0, 0.0, 0.20, 0.85);
pub const SKILL_ROW_RADIUS: f32 = RADIUS_SM;
pub const SKILL_ROW_PAD_X: f32 = PAD_STANDARD;
pub const SKILL_ROW_PAD_Y: f32 = PAD_XS;
pub const SKILL_ROW_GAP: f32 = GAP_SM;
/// Vertical padding for skill rows rendered inside a plugin
/// accordion. Smaller than `SKILL_ROW_PAD_Y` so the group reads as
/// a dense list rather than another full-size section.
pub const SKILL_PLUGIN_ROW_PAD_Y: f32 = GAP_XS;
/// Extra left padding for plugin-scope skill rows. Plugin rows sit
/// under a per-plugin sub-header; this indent makes the
/// header → skill hierarchy obvious without drawing rules.
pub const SKILL_PLUGIN_INDENT: f32 = 24.0;
pub const SKILL_HEADER_GAP: f32 = GAP_STANDARD;
/// Badge / chip metrics. Shared across the four invocation badges +
/// the aux chip so they line up at the trailing edge.
pub const SKILL_BADGE_FONT_SIZE: f32 = FONT_SIZE_XS;
pub const SKILL_BADGE_PAD_X: f32 = PAD_XS;
pub const SKILL_BADGE_PAD_Y: f32 = 0.0;
pub const SKILL_BADGE_RADIUS: f32 = RADIUS_XS;
/// Vertical spacing between MCP server rows. Same value as the Skills
/// tab so the two tabs read as belonging to the same panel.
pub const MCP_ROW_GAP: f32 = GAP_SM;
/// Horizontal gap between elements in the row's main line
/// (indicator / transport / command-preview).
pub const MCP_HEADER_GAP: f32 = GAP_STANDARD;
/// Indicator dot diameter (px). Clickable hit-target — keep ≥ 8.
pub const MCP_INDICATOR_SIZE: f32 = 8.0;
/// Indicator colour for the malformed flag.
pub const MCP_INDICATOR_MALFORMED: Hsla = hsla(14.0, 0.70, 0.55, 1.0);
pub const MCP_BADGE_FONT_SIZE: f32 = FONT_SIZE_XS;
pub const MCP_BADGE_PAD_X: f32 = GAP_XS;
pub const MCP_BADGE_PAD_Y: f32 = 0.0;
pub const MCP_BADGE_RADIUS: f32 = RADIUS_XS;
/// Malformed chip — same shape as the transport badge, warning hue.
pub const MCP_MALFORMED_BADGE_BG: Hsla = hsla(14.0, 0.50, 0.40, 0.30);
pub const MCP_MALFORMED_BADGE_TEXT: Hsla = with_lightness(MCP_INDICATOR_MALFORMED, 0.85);

// ── Flow graph pane ────────────────────────────────────────────────────
// The vendored canvas lays a node out at the size declared on the graph
// node and clips whatever overflows, so the card size is fixed here rather
// than derived from its content. The card spends its height on four rows:
// kind chip + status badge, id, agent axes, prompt first line.

/// Flow graph node card width (px).
pub const FLOW_GRAPH_NODE_W: f32 = 250.0;
/// Flow graph node card height (px).
pub const FLOW_GRAPH_NODE_H: f32 = 112.0;
/// Inner padding of a flow graph node card (px).
pub const FLOW_GRAPH_CARD_PAD: f32 = PAD_LG;
/// Flow graph node card corner radius (px).
pub const FLOW_GRAPH_CARD_RADIUS: f32 = RADIUS_MD;
/// Gap between a flow graph card's rows (px).
pub const FLOW_GRAPH_CARD_ROW_GAP: f32 = PAD_XS;
/// The node id, the one line read at a glance.
pub const FLOW_GRAPH_ID_FONT_SIZE: f32 = FONT_SIZE_LG;
/// Agent axes and prompt preview.
pub const FLOW_GRAPH_META_FONT_SIZE: f32 = FONT_SIZE_SM;
/// The `AGENT` / `GATE` chip.
pub const FLOW_GRAPH_CHIP_FONT_SIZE: f32 = FONT_SIZE_XS;

/// The retry glyph on a card, and the gap before its count. Sized to the chip
/// text beside it rather than to an icon scale: it reads as one token with the
/// number, not as a button.
pub const FLOW_GRAPH_POLICY_GLYPH_SIZE: f32 = FONT_SIZE_XS;
pub const FLOW_GRAPH_POLICY_GLYPH_GAP: f32 = 2.0;
/// Chip padding X (px).
pub const FLOW_GRAPH_CHIP_PAD_X: f32 = PAD_SM;
/// Chip corner radius (px).
pub const FLOW_GRAPH_CHIP_RADIUS: f32 = RADIUS_SM;

// A card's *box* shrinks with zoom but its text does not — the canvas scales
// `w`/`h` and leaves the content the size the renderer asked for. So a card
// zoomed out does not become a small card, it becomes a card too small for
// what is in it. These are the widths (screen px, after zoom) at which the
// card drops a row rather than clipping one.

/// At or above this screen width a card shows everything: chip + badge, id,
/// agent axes, prompt.
pub const FLOW_GRAPH_DENSITY_FULL_W: f32 = 200.0;
/// At or above this, only the line that identifies the node: a kind dot, the
/// id, and its status.
pub const FLOW_GRAPH_DENSITY_COMPACT_W: f32 = 110.0;
/// The id alone, at this size, once a card is narrower than
/// [`FLOW_GRAPH_DENSITY_COMPACT_W`].
pub const FLOW_GRAPH_MARKER_FONT_SIZE: f32 = FONT_SIZE_XS;
/// The kind dot that replaces the chip once the chip's text would not fit.
pub const FLOW_GRAPH_KIND_DOT: f32 = 6.0;

// Framing a graph into the pane. daruda owns this rather than taking the
// vendored fit's policy, because the floor below belongs with the card
// densities above: how far out it is worth zooming is exactly the question of
// what a card still says when it gets there.

/// Fraction of the drawable left as margin on each side when framing.
pub const FLOW_GRAPH_FRAME_MARGIN: f32 = 0.08;
/// Furthest out framing will go. Below this a marker card is narrower than
/// the id it carries, so the graph stops being readable and panning is the
/// better answer.
pub const FLOW_GRAPH_FRAME_MIN_ZOOM: f32 = 0.2;

// The inspector beside the graph. Its width is what the canvas loses, so it is
// the narrowest that still holds a prompt worth reading — and the graph re-fits
// into what is left (see `flow_graph_pane::frame`).

/// Width of the node inspector.
pub const FLOW_INSPECTOR_W: f32 = 280.0;
/// Space between the inspector's rows.
pub const FLOW_INSPECTOR_GAP: f32 = PAD_LG;
/// Inset around the inspector's content.
pub const FLOW_INSPECTOR_PAD: f32 = PAD_LG;
/// Rows the prompt box shows before it scrolls. Five, measured against a short
/// pane: eight pushed the Save button past the bottom edge, and the column
/// scrolls for the rest.
pub const FLOW_INSPECTOR_PROMPT_ROWS: usize = 5;

// The buttons floating over the graph. Inset from the canvas corner rather than
// the pane's, so they clear the inspector's border.

/// Distance from the canvas's top-right corner.
pub const FLOW_TOOLBAR_INSET: f32 = PAD_LG;
/// Space between the toolbar's buttons.
pub const FLOW_TOOLBAR_GAP: f32 = PAD_SM;

/// Canvas behind the graph — the app canvas, so the pane reads as a
/// surface rather than a floating panel.
pub const FLOW_GRAPH_BACKGROUND: Hsla = CANVAS;
/// Dot grid on that canvas. Faint enough to give depth without competing
/// with the edges.
pub const FLOW_GRAPH_GRID_DOT: Hsla = SURFACE_2;
/// A `deps` edge and the port discs it joins.
pub const FLOW_GRAPH_EDGE: Hsla = hsla(218.0, 0.089, 0.35, 1.0);
/// Node card fill.
pub const FLOW_GRAPH_CARD_BG: Hsla = SURFACE_1;
/// Kind chip fill.
pub const FLOW_GRAPH_CHIP_BG: Hsla = SURFACE_2;

/// A node that has not started. Its border is the card's own hairline —
/// pending is the absence of a signal, not a signal of its own.
pub const FLOW_GRAPH_STATUS_PENDING: Hsla = HAIRLINE;
/// A node the run is on now.
pub const FLOW_GRAPH_STATUS_RUNNING: Hsla = ACCENT;
/// A node that passed.
pub const FLOW_GRAPH_STATUS_PASSED: Hsla = SUCCESS;
/// A node on a second or later attempt, or a gate running its repair.
pub const FLOW_GRAPH_STATUS_RETRIED: Hsla = WARNING;
/// A node that failed and stopped the run.
pub const FLOW_GRAPH_STATUS_FAILED: Hsla = ERROR;
/// A node whose finished output is pinned, so this run will not compute it.
/// Its own hue rather than one of the four above: nothing happened to this
/// node, which is a different kind of fact from passing or failing, and the
/// four are already spoken for.
pub const FLOW_GRAPH_STATUS_PINNED: Hsla = hsla(186.0, 0.50, 0.52, 1.0);

/// A card the engine refuses to run: it names how many rules this node breaks.
///
/// Its own hue, and its own place on the card, because it is a different axis
/// from the six above. Those say what a *run* did; this says the flow is not
/// runnable at all, and the two are true at the same time — a card still
/// wearing the last run's green can be one you have since broken. Sharing
/// `WARNING` with `RETRIED` would put both facts in one colour on one card.
pub const FLOW_GRAPH_ISSUE: Hsla = hsla(280.0, 0.45, 0.62, 1.0);

#[cfg(test)]
mod tests {
    use super::*;

    /// The editor's per-capture colours (looked up the exact way
    /// gpui_component's highlighter does, via `SyntaxColors::style`) must
    /// equal the diff view's `syntax_color` for the same capture. This is
    /// the single-source invariant: drift between the two mappings fails
    /// here.
    #[test]
    fn editor_and_diff_share_one_colour_source() {
        let editor = editor_syntax_colors();
        // Every capture whose `SyntaxColors` field is set explicitly in
        // `editor_syntax_colors_of`. Exercising all of them guards the
        // static field->bucket map against drifting from
        // `bucket_for_capture` (which the diff path uses). The field->bucket
        // mapping is palette-independent, so checking Daruda catches any
        // misassignment for all palettes.
        for capture in [
            "keyword",
            "function",
            "title",
            "type",
            "enum",
            "constructor",
            "label",
            "preproc",
            "embedded",
            "constant",
            "boolean",
            "number",
            "attribute",
            "variant",
            "link_uri",
            "string",
            "text.literal",
            "string.escape",
            "string.regex",
            "string.special",
            "string.special.symbol",
            "tag",
            "variable.special",
            "link_text",
            "tag.doctype",
            "comment",
            "comment.doc",
            "hint",
            "predictive",
            "variable",
            "property",
            "operator",
            "punctuation",
            "emphasis",
            // Dotted sub-captures with no exact field must fall back to the
            // first segment in both editor and diff views.
            "function.method",
            "function.call",
            "function.macro",
            "keyword.control",
            "keyword.function",
            "type.builtin",
            "constant.builtin",
            "variable.parameter",
            "variable.member",
            "punctuation.bracket",
        ] {
            let from_editor = editor.style(capture).and_then(|s| s.color);
            assert_eq!(
                from_editor,
                Some(syntax_color(capture)),
                "capture {capture:?}: editor highlight must match the diff palette"
            );
        }
    }

    /// Distinct semantic groups must stay visually distinct after the
    /// refactor — a guard against accidentally collapsing the palette.
    #[test]
    fn semantic_groups_are_distinct() {
        let t = syntax_theme();
        let groups = [
            t.keyword,
            t.function,
            t.type_,
            t.constant,
            t.string,
            t.string_special,
            t.tag,
            t.tag_doctype,
            t.comment,
            t.default,
        ];
        for (i, a) in groups.iter().enumerate() {
            for b in &groups[i + 1..] {
                assert_ne!(a, b, "syntax groups must be distinct colours");
            }
        }
    }

    #[test]
    fn from_config_name_maps_curated_and_falls_back() {
        assert_eq!(
            SyntaxPalette::from_config_name("daruda"),
            SyntaxPalette::Daruda
        );
        assert_eq!(
            SyntaxPalette::from_config_name("one-dark"),
            SyntaxPalette::OneDark
        );
        assert_eq!(
            SyntaxPalette::from_config_name("tokyo-night"),
            SyntaxPalette::TokyoNight
        );
        assert_eq!(
            SyntaxPalette::from_config_name("catppuccin-mocha"),
            SyntaxPalette::CatppuccinMocha
        );
        // Unknown + legacy base16 names fall back to the recommended default.
        assert_eq!(SyntaxPalette::from_config_name(""), SyntaxPalette::Daruda);
        assert_eq!(
            SyntaxPalette::from_config_name("base16-ocean.dark"),
            SyntaxPalette::Daruda
        );
        assert_eq!(
            SyntaxPalette::from_config_name("nonsense"),
            SyntaxPalette::Daruda
        );
    }

    /// Every curated palette, used to assert invariants across all of them.
    const ALL_PALETTES: [SyntaxPalette; 14] = [
        SyntaxPalette::Daruda,
        SyntaxPalette::OneDark,
        SyntaxPalette::TokyoNight,
        SyntaxPalette::CatppuccinMocha,
        SyntaxPalette::Dracula,
        SyntaxPalette::GitHubDark,
        SyntaxPalette::MaterialPalenight,
        SyntaxPalette::Monokai,
        SyntaxPalette::Nord,
        SyntaxPalette::GruvboxDark,
        SyntaxPalette::SolarizedDark,
        SyntaxPalette::AyuMirage,
        SyntaxPalette::NightOwl,
        SyntaxPalette::Darcula,
    ];

    #[test]
    fn every_palette_separates_keyword_from_default() {
        for p in ALL_PALETTES {
            for is_light in [false, true] {
                let t = syntax_theme_of(p, is_light);
                assert_ne!(
                    t.color_for("keyword"),
                    t.color_for(""),
                    "{p:?} (light={is_light}): keyword must differ from default"
                );
            }
        }
    }

    fn wcag_contrast(fg: Hsla, bg: Hsla) -> f32 {
        fn lum(c: Hsla) -> f32 {
            let rgba = gpui::Rgba::from(c);
            let f = |v: f32| {
                if v <= 0.03928 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * f(rgba.r) + 0.7152 * f(rgba.g) + 0.0722 * f(rgba.b)
        }
        let (a, b) = (lum(fg), lum(bg));
        let (hi, lo) = if a > b { (a, b) } else { (b, a) };
        (hi + 0.05) / (lo + 0.05)
    }

    #[test]
    fn light_variant_clears_contrast_on_light_bg() {
        // Daruda Light is the WCAG-AA light default; every bucket must clear
        // 4.5:1 on the light editor background (`#fafafa`).
        let bg = base16(0xfa_fa_fa);
        let t = syntax_theme_of(SyntaxPalette::Daruda, true);
        for capture in [
            "keyword", "function", "type", "constant", "string", "comment", "",
        ] {
            assert!(
                wcag_contrast(t.color_for(capture), bg) >= 4.5,
                "Daruda Light {capture:?} must clear 4.5:1 on #fafafa"
            );
        }
    }

    #[test]
    fn every_palette_round_trips_through_config_name() {
        // Each curated palette must have a config name that resolves back
        // to it (the settings dropdown relies on this).
        let names = [
            "daruda",
            "one-dark",
            "tokyo-night",
            "catppuccin-mocha",
            "dracula",
            "github-dark",
            "material-palenight",
            "monokai",
            "nord",
            "gruvbox-dark",
            "solarized-dark",
            "ayu-mirage",
            "night-owl",
            "darcula",
        ];
        assert_eq!(names.len(), ALL_PALETTES.len());
        for (name, expected) in names.iter().zip(ALL_PALETTES) {
            assert_eq!(SyntaxPalette::from_config_name(name), expected, "{name}");
        }
    }

    #[test]
    fn capture_grouping_is_shared_by_color_and_style() {
        // color_for / style_for / editor colours must all read the same
        // bucket for a capture — guard against the grouping drifting.
        let t = syntax_theme_of(SyntaxPalette::Daruda, false);
        for capture in ["keyword", "string.escape", "comment", "function", ""] {
            let bucket = bucket_for_capture(capture);
            assert_eq!(t.color_for(capture), t.color(bucket));
            assert_eq!(t.style_for(capture), t.style(bucket));
        }
    }

    #[test]
    fn daruda_carries_non_color_channels() {
        let t = syntax_theme_of(SyntaxPalette::Daruda, false);
        assert!(t.style_for("keyword").bold, "Daruda keyword is bold");
        assert!(
            t.style_for("string.escape").bold,
            "Daruda string_special is bold"
        );
        assert!(t.style_for("comment").italic, "Daruda comment is italic");
        // A color-only palette carries no non-color channel.
        let one = syntax_theme_of(SyntaxPalette::OneDark, false);
        assert_eq!(one.style_for("keyword"), TokenStyle::default());
    }
}
