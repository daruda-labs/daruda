//! App-side UI palette — workspace chrome, docks, status bar, docks.
//!
//! Lives in the `app` crate (not `daruda_terminal::ux::theme`) because
//! it describes colours / metrics for daruda-bespoke chrome widgets:
//! tab bar, dock view tabs, lanes list, status bar, dock
//! panels. The terminal-side palette (cell fg/bg, cursor, search
//! overlay, scrollback search, terminal scrollbar) stays in
//! `daruda_terminal::ux::theme` because the `view/` rendering code
//! inside that crate reads it directly.
//!
//! Sibling [`super`] (the bridge) re-projects selected entries here
//! plus terminal-side constants into `gpui_component::Theme` slots so
//! every wrapped widget (Dialog / Input / Select / …) inherits the
//! daruda tone.
//!
//! Hue convention matches `daruda_terminal::ux::theme`: every literal
//! reads in degrees [0, 360]; the local [`hsla`] helper normalizes to
//! the fractional form `gpui::Hsla` actually stores. The duplicate
//! helper here is intentional — both files speak in degrees so call
//! sites can copy a colour between them without unit conversion. This
//! does **not** violate the "no third hsla helper" rule (CLAUDE.md
//! §11) because that rule targets *unit confusion* (degrees vs.
//! fraction); both helpers use the same degree-based contract.

use gpui::Hsla;

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

/// Re-project an existing token at a new alpha — `const`-friendly so a
/// tint/overlay can derive from its opaque parent (e.g. a 12 % success
/// row fill from [`SUCCESS`]) instead of duplicating the H/S/L literal.
/// Keeps the single-source-of-truth chain intact: change the parent and
/// every alpha variant follows.
pub const fn with_alpha(c: Hsla, a: f32) -> Hsla {
    Hsla {
        h: c.h,
        s: c.s,
        l: c.l,
        a,
    }
}

/// Re-project an existing token at a new lightness — the H/S/A stay put.
/// Lets a hover/active state variant derive from its base token (e.g. a
/// brighter [`SUCCESS`] for a button's hover) instead of hand-writing a
/// parallel `hsla()` literal in the gpui_component bridge.
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

// ============================================================================
// Design tokens — primitive colour literals (single source of truth)
// ============================================================================

pub const CANVAS: Hsla = hsla(240.0, 0.333, 0.006, 1.0);
pub const SURFACE_1: Hsla = hsla(210.0, 0.063, 0.063, 1.0);
pub const SURFACE_2: Hsla = hsla(210.0, 0.048, 0.082, 1.0);
pub const SURFACE_3: Hsla = hsla(210.0, 0.040, 0.098, 1.0);
pub const HAIRLINE: Hsla = hsla(223.0, 0.091, 0.151, 1.0);
pub const ACCENT: Hsla = hsla(233.8, 0.563, 0.596, 1.0);
pub const INK: Hsla = hsla(0.0, 0.0, 0.97, 1.0);
pub const TEXT_BODY: Hsla = hsla(218.0, 0.089, 0.847, 1.0);
pub const TEXT_MUTE: Hsla = hsla(218.0, 0.064, 0.569, 1.0);
pub const TEXT_SUBTLE: Hsla = hsla(218.0, 0.053, 0.406, 1.0);

// ============================================================================
// DESIGN.md color tokens — complete palette (synced from DESIGN.md §Colors)
// ============================================================================

/// Popover, context menu, tooltip background  (#1f2022).
pub const SURFACE_4: Hsla = hsla(210.0, 0.046, 0.128, 1.0);
/// Faint overlay border — popover edges (rgba white at 6%).
pub const HAIRLINE_SOFT: Hsla = hsla(0.0, 0.0, 1.0, 0.06);
/// Hovered accent elements (#828fff).
pub const ACCENT_HOVER: Hsla = hsla(233.8, 1.0, 0.755, 1.0);
/// Low-opacity accent fill — badge background (#1e2050).
pub const ACCENT_MUTED: Hsla = hsla(237.6, 0.345, 0.216, 1.0);
/// Text/icon on solid accent background (#ffffff).
pub const ACCENT_FG: Hsla = hsla(0.0, 0.0, 1.0, 1.0);

// ---------------------------------------------------------------------------
// Claude lane states
// ---------------------------------------------------------------------------

/// Claude is running in this lane (#f0a020).
pub const CLAUDE_ACTIVE: Hsla = hsla(36.9, 0.874, 0.533, 1.0);
/// Last Claude session completed (DESIGN: intentionally = success).
pub const CLAUDE_DONE: Hsla = SUCCESS;
/// Last session errored (DESIGN: intentionally = error).
pub const CLAUDE_ERROR: Hsla = ERROR;

// ---------------------------------------------------------------------------
// Agent action states (Cursor timeline palette, dark-adapted)
// ---------------------------------------------------------------------------

/// Steel blue — Thinking / planning (#8faacc).
pub const AGENT_THINKING: Hsla = hsla(213.4, 0.375, 0.681, 1.0);
/// Mint green — Reading files / context (#8fcca8).
pub const AGENT_READING: Hsla = hsla(144.6, 0.375, 0.681, 1.0);
/// Lavender — Writing / editing output (#b09bcc).
pub const AGENT_EDITING: Hsla = hsla(265.6, 0.324, 0.704, 1.0);
/// Gold — Executing tool / running command (#ccaa6e).
pub const AGENT_RUNNING: Hsla = hsla(38.4, 0.480, 0.616, 1.0);
/// No active session (#8a8f98 = mute).
pub const AGENT_IDLE: Hsla = TEXT_MUTE;

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
pub const DIFF_HUNK: Hsla = TEXT_SUBTLE; // #62666d

// ---------------------------------------------------------------------------
// Selection
// ---------------------------------------------------------------------------

pub const SELECTION_BG: Hsla = with_alpha(PRIMARY, 0.28);
pub const SELECTION_FG: Hsla = INK;

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
pub const BG_PANEL: Hsla = SURFACE_1;
pub const BG_RAISED: Hsla = SURFACE_2;
pub const BG_HOVER: Hsla = SURFACE_2;
pub const BG_ACTIVE: Hsla = SURFACE_3;
pub const BORDER: Hsla = HAIRLINE;

// Text roles — the only four sanctioned UI text tones (DESIGN §Colors:
// ink / body / mute / subtle). All carry the cool-blue tint that is the
// design identity; feature constants must reference these rather than
// inventing neutral-gray (`hsla(0,0,L)`) literals, which DESIGN §Don'ts
// forbids ("never neutral gray").
pub const TEXT_PRIMARY: Hsla = INK; // titles, active labels, focused rows
pub const TEXT_SECONDARY: Hsla = TEXT_BODY; // body copy, descriptions, previews
pub const TEXT_TERTIARY: Hsla = TEXT_MUTE; // section headers, muted metadata
pub const TEXT_DISABLED: Hsla = TEXT_SUBTLE; // empty states, lowest emphasis

// Action role — the single chromatic accent (DESIGN §Colors: accent),
// surfaced under a clearer semantic name. Feature constants and render
// reference `PRIMARY` for primary CTA / focus ring / active indicator /
// link / info tint; [`ACCENT`] stays as the raw design-token value and
// is referenced only here.
pub const PRIMARY: Hsla = ACCENT;

// Overlay roles — white lift at fixed alphas (DESIGN: elevation via the
// surface ladder, expressed as translucent white for row hover/active
// fills that must read over any underlying surface). One step per
// intensity so every list/row reuses the same lift instead of picking a
// bespoke alpha.
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

/// Small dot in the right section of the status bar that lights up
/// when the workspace's project layer (`<config_dir>/daruda/projects/...`)
/// has a `config.toml` on disk. Cyan-ish accent so it reads as
/// informational, not as an alert.
pub const STATUS_BAR_PROJECT_DOT: Hsla = hsla(180.0, 0.55, 0.55, 1.0);

/// Inline "detached" chip background and text — shown next to the
/// project/branch label when the active git worktree is on a detached
/// HEAD. Amber-leaning so it signals *attention* without rising to
/// the red-error tier; the state is uncommon but not destructive.
pub const STATUS_BAR_DETACHED_BG: Hsla = with_lightness(WARNING, 0.22);
pub const STATUS_BAR_DETACHED_TEXT: Hsla = with_lightness(WARNING, 0.72);

/// Dock view tab strip — horizontal padding per tab (px).
pub const DOCK_VIEW_TAB_PAD_X: f32 = PAD_LG;

/// Dock view tab strip — font size (px).
pub const DOCK_VIEW_TAB_FONT_SIZE: f32 = FONT_SIZE_SM;

/// Dock view tab strip — active underline thickness (px).
pub const DOCK_VIEW_TAB_ACCENT_H: f32 = 2.0;

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
/// Context menu max width (px) — wider items get truncated by overflow_hidden.
pub const CTX_MENU_MAX_WIDTH: f32 = 200.0;
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
pub const LANE_ROW_GAP: f32 = GAP_LG;
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

/// Lanes card — outer corner radius (px).
pub const LANE_CARD_RADIUS: f32 = 10.0;
/// Lanes card — vertical gap between adjacent cards (px).
pub const LANE_CARD_GAP: f32 = GAP_STANDARD;
/// Lanes card — inner horizontal padding (px).
pub const LANE_CARD_PAD_X: f32 = PAD_LG;
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
/// Lanes card — border width (px).
pub const LANE_CARD_BORDER_W: f32 = 1.0;
/// Group label font size (px) — uppercase eyebrow.
pub const LANE_GROUP_LABEL_FONT_SIZE: f32 = FONT_SIZE_SM;

// ============================================================================
// Migrated from daruda_terminal::ux::theme (Phase 1 follow-up)
// ============================================================================

/// Modal backdrop dim alpha (0..1).
pub const MODAL_BACKDROP_ALPHA: f32 = 0.50;
/// Modal panel corner radius (px).
pub const MODAL_PANEL_RADIUS: f32 = RADIUS_LG;
/// Modal panel width (px).
pub const MODAL_PANEL_WIDTH: f32 = 420.0;
/// Modal panel inner padding (px).
pub const MODAL_PANEL_PAD: f32 = 16.0;
/// Modal panel top offset from window top (px).
pub const MODAL_TOP_OFFSET: f32 = 140.0;
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
/// Single-line text-input field height (px).
pub const MODAL_INPUT_HEIGHT: f32 = 32.0;
/// Minimum pixel height of the Notes textarea inside `EditTaskModal`.
/// The Notes field shares the same `flex_1` column as the Prompt, but
/// is biased smaller because notes are typically short — `min_h`
/// guarantees at least ~6 lines of room without letting the row
/// dominate the column.
pub const MODAL_NOTES_TEXTAREA_MIN_H: f32 = 120.0;
/// Radio-button indicator column width in the merge modal (px).
/// Wide enough to contain "●" / "○" at MODAL_BODY_FONT_SIZE.
pub const MODAL_RADIO_W: f32 = 14.0;
/// Wide form modal width (px). Used for split layouts (e.g. Skills:
/// metadata form on the left + markdown body editor on the right).
pub const FORM_MODAL_WIDE: f32 = 900.0;
/// Narrow form modal width (px). Single-column form (e.g. Tools MCP
/// server registration).
pub const FORM_MODAL_NARROW: f32 = 480.0;
/// Fixed overall height for the Tasks Create/Edit modals (px). The
/// 2-column body fills the panel; Prompt grows to `flex_1` so the
/// writing surface stays comfortable. ModalLayer caps at viewport
/// height so this is treated as a target on tall screens and a
/// scroll trigger on short ones.
pub const FORM_MODAL_HEIGHT_TASK: f32 = 640.0;
/// Vertical gap between sections inside a form modal column (px).
pub const FORM_MODAL_SECTION_GAP: f32 = 12.0;
/// Horizontal gap between the left/right columns of a split-form
/// modal body (px).
pub const FORM_MODAL_SPLIT_GAP: f32 = 16.0;
/// Bottom-edge breathing room kept clear under modal panels so a tall
/// modal — once `max_h + overflow_y_scroll` is engaged — has visible
/// margin instead of butting up against the window edge (px).
pub const MODAL_BOTTOM_MARGIN: f32 = 24.0;
/// Height of the 1px separator strip rendered between context-menu
/// item groups (px). `bg` uses `MODAL_PANEL_BORDER`.
pub const CONTEXT_MENU_SEPARATOR_H: f32 = 1.0;
/// Width of the text-input cursor caret (px). 1.5 keeps it visible
/// at the smaller modal font sizes without looking heavy.
pub const CARET_WIDTH: f32 = 1.5;
/// Underline thickness for in-progress IME composition (Hangul / CJK).
pub const IME_UNDERLINE_THICKNESS: f32 = 1.0;
/// Side length of the square modal checkbox (px).
pub const MODAL_CHECKBOX_SIZE: f32 = 14.0;
/// Corner radius of the modal checkbox (px).
pub const MODAL_CHECKBOX_RADIUS: f32 = RADIUS_XS;
/// Font size of the ✓ checkmark inside the checkbox (px).
pub const MODAL_CHECKBOX_TICK_SIZE: f32 = 10.0;
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
/// Horizontal padding inside a banner (px).
pub const BANNER_PAD_X: f32 = PAD_LG;
/// Vertical padding inside a banner (px).
pub const BANNER_PAD_Y: f32 = PAD_SM;
/// Banner corner radius (px).
pub const BANNER_RADIUS: f32 = RADIUS_MD;
/// Gap between the icon glyph and the message text (px).
pub const BANNER_GAP: f32 = GAP_STANDARD;
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
/// Minimum width for Select dropdown lists in modal/settings contexts (px).
pub const MODAL_SELECT_MIN_W: f32 = 160.0;
/// Maximum height for Select dropdown lists before they start scrolling (px).
pub const MODAL_SELECT_MAX_H: f32 = 280.0;
/// macOS traffic light X offset from window left edge.
pub const TRAFFIC_LIGHT_X: f32 = 8.0;
/// macOS traffic light Y offset from window top edge.
pub const TRAFFIC_LIGHT_Y: f32 = 6.0;
/// Width reserved for traffic lights in the title bar.
pub const TRAFFIC_LIGHT_WIDTH: f32 = 70.0;
/// Dock panel header height (px).
pub const DOCK_HEADER_HEIGHT: f32 = 28.0;
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
/// Diff line font size (px) — compact monospace.
pub const GIT_DIFF_FONT_SIZE: f32 = FONT_SIZE_XS;
/// Diff line horizontal padding (px).
pub const GIT_DIFF_LINE_PAD_X: f32 = PAD_SM;
/// Diff panel max visible height before truncation note (px).
pub const GIT_DIFF_MAX_HEIGHT: f32 = 320.0;
/// Maximum diff lines rendered (guards against huge diffs).
pub const GIT_DIFF_MAX_LINES: usize = 120;
/// Stage checkbox box size (px).
pub const GIT_STAGE_CHECKBOX_SIZE: f32 = 13.0;
/// Stage checkbox border radius (px).
pub const GIT_STAGE_CHECKBOX_RADIUS: f32 = RADIUS_XS;
/// Stage checkbox background when staged (green tint).
pub const GIT_STAGE_CHECKBOX_CHECKED_BG: Hsla = with_lightness(SUCCESS, 0.30);
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
/// Refresh icon size in the Git Changes header (px).
pub const GIT_REFRESH_ICON_SIZE: f32 = 16.0;
/// Commit footer inner padding (px).
pub const GIT_COMMIT_PAD: f32 = PAD_STANDARD;
/// Gap between Commit and Push buttons (px).
pub const GIT_COMMIT_BUTTON_GAP: f32 = GAP_STANDARD;
/// Total height of the commit footer panel (textarea + floating button bar).
/// Sized to show ~4-5 lines of commit message text.
pub const GIT_COMMIT_FOOTER_H: f32 = 128.0;
/// Commit message text area height (px) — kept for reference; layout uses GIT_COMMIT_FOOTER_H.
pub const GIT_COMMIT_INPUT_HEIGHT: f32 = 64.0;
/// Commit button horizontal padding (px).
pub const GIT_COMMIT_BTN_PAD_X: f32 = PAD_STANDARD;
/// Commit button vertical padding (px).
pub const GIT_COMMIT_BTN_PAD_Y: f32 = PAD_XS;
/// Commit button corner radius (px).
pub const GIT_COMMIT_BTN_RADIUS: f32 = RADIUS_SM;
/// Dropdown arrow button horizontal padding (px).
pub const GIT_COMMIT_DROP_PAD_X: f32 = 5.0;
/// Gap between remote action buttons (Fetch / Push) (px).
pub const GIT_REMOTE_BTN_GAP: f32 = GAP_SM;
/// Gap between the text area and the action button group in InputPanel (px).
pub const INPUT_PANEL_SECTION_GAP: f32 = GAP_STANDARD;
/// Gap between buttons in the InputPanel action group (px).
pub const INPUT_PANEL_BUTTON_GAP: f32 = GAP_STANDARD;
/// Minimum height of the TextArea inside InputPanel (px).
pub const INPUT_PANEL_MIN_H: f32 = 48.0;
/// Height of the floating action bar overlaid at the bottom of an InputPanel
/// with `ActionsFloating` layout (px). Matches Zed git_panel footer_size.
pub const INPUT_PANEL_FLOATING_BAR_H: f32 = 32.0;
/// Command palette max visible entries.
pub const PALETTE_MAX_VISIBLE: usize = 12;
/// Command palette corner radius (px).
pub const PALETTE_RADIUS: f32 = RADIUS_LG;
/// Tab label font size (px).
pub const TAB_FONT_SIZE: f32 = FONT_SIZE_MD;
/// Tab close button font size (px).
pub const TAB_CLOSE_FONT_SIZE: f32 = FONT_SIZE_SM;
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
/// Status bar horizontal padding (px).
pub const STATUS_BAR_PAD_X: f32 = PAD_LG;
/// Dock panel header font size (px).
pub const DOCK_HEADER_FONT_SIZE: f32 = FONT_SIZE_SM;
/// Dock panel header horizontal padding (px).
pub const DOCK_HEADER_PAD_X: f32 = PAD_LG;
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
/// Tab close button width/height (px).
pub const TAB_CLOSE_W: f32 = 16.0;
/// Tab close button corner radius (px).
pub const TAB_CLOSE_RADIUS: f32 = RADIUS_XS;
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
/// Command palette "no results" padding Y (px).
pub const PALETTE_EMPTY_PAD_Y: f32 = 16.0;
/// Divider drag hit area min dimension (px).
pub const DIVIDER_MIN_DIM: f32 = 1.0;
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
pub const TITLE_BAR_HEIGHT: f32 = 24.0;
/// Tab bar height (px).
pub const TAB_BAR_HEIGHT: f32 = 28.0;
/// Status bar height (px).
pub const STATUS_BAR_HEIGHT: f32 = 22.0;
/// Diameter of the project-config indicator dot.
pub const STATUS_BAR_PROJECT_DOT_SIZE: f32 = 6.0;
/// Inline detached-HEAD chip — font size, horizontal padding,
/// vertical padding, corner radius. Sized to sit flush with the
/// 22px status bar height without forcing a row-height increase.
pub const STATUS_BAR_DETACHED_FONT_SIZE: f32 = FONT_SIZE_XS;
pub const STATUS_BAR_DETACHED_PAD_X: f32 = 5.0;
pub const STATUS_BAR_DETACHED_PAD_Y: f32 = 1.0;
pub const STATUS_BAR_DETACHED_RADIUS: f32 = RADIUS_XS;
/// Agent activity log entry font size (px).
pub const AGENT_LOG_FONT_SIZE: f32 = FONT_SIZE_MD;
/// Agent activity log icon column width (px).
pub const AGENT_LOG_ICON_W: f32 = 18.0;
/// Agent activity log entry horizontal gap (px).
pub const AGENT_LOG_GAP: f32 = GAP_STANDARD;
/// Agent activity log container horizontal padding (px).
pub const AGENT_LOG_PAD_X: f32 = PAD_STANDARD;
/// Agent activity log container vertical padding (px).
pub const AGENT_LOG_PAD_Y: f32 = PAD_XS;
/// Agent activity log entry vertical gap (px).
pub const AGENT_LOG_ENTRY_GAP: f32 = GAP_XS;
/// Agent activity log pinned status top margin (px).
pub const AGENT_LOG_STATUS_MT: f32 = PAD_XS;
/// Agent activity log pinned status padding X (px).
pub const AGENT_LOG_STATUS_PAD_X: f32 = PAD_STANDARD;
/// Agent activity log pinned status padding Y (px).
pub const AGENT_LOG_STATUS_PAD_Y: f32 = PAD_XS;
/// Agent chat message label font size (px).
pub const AGENT_CHAT_LABEL_FONT_SIZE: f32 = FONT_SIZE_SM;
/// Agent chat message body font size (px).
pub const AGENT_CHAT_MSG_FONT_SIZE: f32 = FONT_SIZE_LG;
/// Agent chat message gap (px).
pub const AGENT_CHAT_MSG_GAP: f32 = GAP_XS;
/// Agent chat message list gap (px).
pub const AGENT_CHAT_LIST_GAP: f32 = GAP_LG;
/// Agent chat container padding X (px).
pub const AGENT_CHAT_PAD_X: f32 = PAD_STANDARD;
/// Agent chat container padding Y (px).
pub const AGENT_CHAT_PAD_Y: f32 = PAD_XS;
/// Agent chat input area padding X (px).
pub const AGENT_CHAT_INPUT_PAD_X: f32 = PAD_STANDARD;
/// Agent chat input area padding Y (px).
pub const AGENT_CHAT_INPUT_PAD_Y: f32 = PAD_SM;
/// Agent chat input box inner padding X (px).
pub const AGENT_CHAT_INPUT_INNER_PAD_X: f32 = PAD_STANDARD;
/// Agent chat input box inner padding Y (px).
pub const AGENT_CHAT_INPUT_INNER_PAD_Y: f32 = PAD_XS;
/// Agent chat input box corner radius (px).
pub const AGENT_CHAT_INPUT_RADIUS: f32 = RADIUS_SM;
/// Agent task list entry font size (px).
pub const AGENT_TASK_FONT_SIZE: f32 = FONT_SIZE_MD;
/// Agent task list icon column width (px).
pub const AGENT_TASK_ICON_W: f32 = 14.0;
/// Agent task list entry gap (px).
pub const AGENT_TASK_GAP: f32 = GAP_STANDARD;
/// Agent task list entry padding X (px).
pub const AGENT_TASK_PAD_X: f32 = PAD_STANDARD;
/// Agent task list entry padding Y (px).
pub const AGENT_TASK_PAD_Y: f32 = PAD_XS;
/// Agent task list container padding Y (px).
pub const AGENT_TASK_LIST_PAD_Y: f32 = PAD_XS;
/// Left dock default width (px).
pub const DOCK_LEFT_DEFAULT_W: f32 = 220.0;
/// Left dock minimum width (px).
pub const DOCK_LEFT_MIN_W: f32 = 150.0;
/// Left dock maximum width (px).
pub const DOCK_LEFT_MAX_W: f32 = 400.0;
/// Right dock default width (px).
pub const DOCK_RIGHT_DEFAULT_W: f32 = 250.0;
/// Right dock minimum width (px).
pub const DOCK_RIGHT_MIN_W: f32 = 150.0;
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

/// Fixed row height for the virtual-list renderer (px).
/// Derived so it is always ≥ FILE_VIEWER_FONT_SIZE * phi() without manual sync.
pub const FILE_VIEWER_LINE_H: f32 = FILE_VIEWER_FONT_SIZE * 1.7;
/// Rows rendered above and below the visible viewport (overscan).
pub const FILE_VIEWER_VIRTUAL_OVERSCAN: usize = 8;
/// Gap between toolbar button group items (px).
pub const FILE_VIEWER_TOOLBAR_GAP: f32 = GAP_STANDARD;
/// Mode tab horizontal padding (px).
pub const FILE_VIEWER_TAB_PAD_X: f32 = PAD_STANDARD;
/// Mode tab vertical padding (px).
pub const FILE_VIEWER_TAB_PAD_Y: f32 = 3.0;
/// Mode tab corner radius (px).
pub const FILE_VIEWER_TAB_RADIUS: f32 = RADIUS_SM;
/// Diff added-line background.
pub const FILE_DIFF_ADD_BG: Hsla = hsla(135.0, 0.40, 0.15, 1.0);
/// Diff removed-line background.
pub const FILE_DIFF_DEL_BG: Hsla = hsla(0.0, 0.45, 0.15, 1.0);
/// Diff added-line text color.
pub const FILE_DIFF_ADD_TEXT: Hsla = hsla(135.0, 0.60, 0.70, 1.0);
/// Diff removed-line text color.
pub const FILE_DIFF_DEL_TEXT: Hsla = hsla(0.0, 0.60, 0.70, 1.0);
/// Diff hunk-header text color.
pub const FILE_DIFF_HUNK_TEXT: Hsla = hsla(220.0, 0.40, 0.60, 1.0);
/// Vertical padding added above and below the hunk-header content (px).
pub const FILE_DIFF_HUNK_PADDING_Y: f32 = 5.0;
/// Line number right padding in the raw file view (px).
pub const FILE_VIEWER_LINE_NO_PAD_R: f32 = PAD_STANDARD;
/// Line number right padding in the diff view dual-column (px).
pub const FILE_VIEWER_DIFF_LINE_NO_PAD_R: f32 = PAD_XS;
/// Diff marker (`+`/`-`/` `) column width (px).
pub const FILE_VIEWER_DIFF_MARKER_W: f32 = 10.0;
/// Hunk header trailing context text color (function name / class name, dim).
pub const FILE_DIFF_HUNK_CTX_TEXT: Hsla = hsla(220.0, 0.20, 0.45, 1.0);
/// syntect theme name used for diff syntax highlighting.
pub const FILE_VIEWER_SYNTAX_THEME: &str = "base16-ocean.dark";
/// Gap between `@@ -N +M @@` and its trailing context text (px).
pub const FILE_DIFF_HUNK_CTX_GAP_X: f32 = GAP_LG;
/// Gap between `+N` and `-N` in the diff stat badge (px).
pub const FILE_DIFF_STAT_GAP: f32 = GAP_SM;
/// Diff stat added-lines count color (+N).
pub const FILE_DIFF_STAT_ADD: Hsla = hsla(133.0, 0.60, 0.55, 1.0);
/// Diff stat removed-lines count color (-N).
pub const FILE_DIFF_STAT_DEL: Hsla = hsla(0.0, 0.60, 0.55, 1.0);
/// Diff stat and file-status badge font size (px).
pub const FILE_DIFF_STAT_FONT_SIZE: f32 = FONT_SIZE_SM;
/// Word-level diff insertion highlight background (stronger than line bg).
pub const FILE_DIFF_WORD_ADD_BG: Hsla = hsla(135.0, 0.60, 0.27, 1.0);
/// Word-level diff deletion highlight background (stronger than line bg).
pub const FILE_DIFF_WORD_DEL_BG: Hsla = hsla(0.0, 0.60, 0.27, 1.0);
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
/// Search input area horizontal padding (px).
pub const FILE_VIEWER_SEARCH_INPUT_PAD_X: f32 = PAD_STANDARD;
/// Search input area vertical padding (px).
pub const FILE_VIEWER_SEARCH_INPUT_PAD_Y: f32 = 3.0;
/// Search input area corner radius (px).
pub const FILE_VIEWER_SEARCH_INPUT_RADIUS: f32 = RADIUS_SM;
/// Match counter font size inside the input area (px).
pub const FILE_VIEWER_SEARCH_COUNTER_SIZE: f32 = 11.0;
/// Cursor indicator width (px).
pub const FILE_VIEWER_SEARCH_CURSOR_W: f32 = 1.0;
/// Cursor indicator height (px).
pub const FILE_VIEWER_SEARCH_CURSOR_H: f32 = 14.0;
/// Button horizontal padding (px).
pub const FILE_VIEWER_SEARCH_BTN_PAD_X: f32 = PAD_SM;
/// Left margin of the close button to visually separate it from nav buttons (px).
pub const FILE_VIEWER_SEARCH_BTN_ML: f32 = PAD_XS;
/// Horizontal scroll origin for file viewer scroll-to-match (always 0, px).
pub const FILE_VIEWER_SCROLL_ORIGIN_X: f32 = 0.0;
/// H1 heading font size (px).
pub const MD_H1_FONT_SIZE: f32 = 22.0;
/// H2 heading font size (px).
pub const MD_H2_FONT_SIZE: f32 = 18.0;
/// H3 heading font size (px).
pub const MD_H3_FONT_SIZE: f32 = 15.0;
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
/// Distance from the window bottom edge (px).
pub const TOAST_BOTTOM_MARGIN: f32 = 24.0;
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
/// Corner radius of the scrollbar thumb (px).
pub const SCROLLBAR_THUMB_RADIUS: f32 = 3.0;

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
pub const FONT_SIZE_LG: f32 = 13.0;
pub const RADIUS_SM: f32 = 4.0;
pub const RADIUS_XS: f32 = 2.0;
pub const RADIUS_MD: f32 = 6.0;
pub const RADIUS_LG: f32 = 8.0;
/// Container corner radius (px).
pub const KEYSTROKE_INPUT_RADIUS: f32 = RADIUS_MD;
/// Horizontal padding inside the container (px).
pub const KEYSTROKE_INPUT_PAD_X: f32 = PAD_STANDARD;
/// Vertical padding inside the container (px).
pub const KEYSTROKE_INPUT_PAD_Y: f32 = 5.0;
/// Minimum width so the recording hint has room (px).
pub const KEYSTROKE_INPUT_MIN_W: f32 = 140.0;
/// Badge corner radius (px).
pub const KEYSTROKE_BADGE_RADIUS: f32 = RADIUS_SM;
/// Badge horizontal padding (px).
pub const KEYSTROKE_BADGE_PAD_X: f32 = PAD_SM;
/// Badge vertical padding (px).
pub const KEYSTROKE_BADGE_PAD_Y: f32 = GAP_XS;
/// Badge font size (px).
pub const KEYSTROKE_BADGE_FONT_SIZE: f32 = FONT_SIZE_MD;
/// Gap between key badges in a sequence (px).
pub const KEYSTROKE_BADGE_GAP: f32 = GAP_SM;
/// Font size for container hint text (px).
pub const KEYSTROKE_INPUT_FONT_SIZE: f32 = FONT_SIZE_MD;
/// Corner radius of the popover panel (px).
pub const POPOVER_RADIUS: f32 = RADIUS_MD;
/// Vertical padding above/below the item list inside the panel (px).
pub const POPOVER_LIST_PAD_Y: f32 = PAD_XS;
/// Horizontal padding for each item row (px).
pub const POPOVER_ITEM_PAD_X: f32 = 12.0;
/// Vertical padding for each item row (px).
pub const POPOVER_ITEM_PAD_Y: f32 = PAD_SM;
/// Item font size (px).
pub const POPOVER_ITEM_FONT_SIZE: f32 = FONT_SIZE_LG;
/// Minimum width of the popover panel (px).
pub const POPOVER_MIN_WIDTH: f32 = 140.0;
/// Height of the separator rule inside the popover (px).
pub const POPOVER_SEPARATOR_HEIGHT: f32 = 1.0;
/// Edge margin kept between the popover panel and the window boundary (px).
pub const POPOVER_SNAP_MARGIN: f32 = PAD_STANDARD;
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
/// One full Working-state animation cycle (head sweeps every dot once).
pub const STATUS_INDICATOR_SPINNER_PERIOD_MS: u64 = 1100;
/// One full ExecutingTool-state animation cycle (head sweeps the outer ring).
pub const STATUS_INDICATOR_RING_PERIOD_MS: u64 = 900;
/// Centre-dot alpha multiplier for the ExecutingTool ring animation.
pub const STATUS_INDICATOR_RING_CENTER_ALPHA: f32 = 0.15;
/// One full pulse cycle (NeedsAttention state) — opacity 0.4 → 1.0 → 0.4.
pub const STATUS_INDICATOR_PULSE_DURATION_MS: u64 = 1000;
/// One full Connecting-state cycle: plus (+) → cross (×) → plus,
/// with a smooth cosine cross-fade between the two patterns.
pub const STATUS_INDICATOR_CONNECTING_PERIOD_MS: u64 = 1200;
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
pub const STATUS_BADGE_ACTIVE_OUTLINE_PX: f32 = 1.0;
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
pub const CLAUDE_BANNER_ICON: Hsla = hsla(210.0, 1.0, 0.68, 1.0);
pub const CLAUDE_BANNER_BG: Hsla = with_alpha(CLAUDE_BANNER_ICON, 0.08);
pub const CLAUDE_BANNER_BORDER: Hsla = with_alpha(CLAUDE_BANNER_ICON, 0.20);
pub const CLAUDE_BANNER_HOVER_BG: Hsla = with_alpha(CLAUDE_BANNER_ICON, 0.14);
pub const CLAUDE_BANNER_PAD_X: f32 = 12.0;
pub const CLAUDE_BANNER_PAD_Y: f32 = PAD_STANDARD;
pub const CLAUDE_BANNER_GAP: f32 = GAP_LG;
pub const CLAUDE_BANNER_RADIUS: f32 = RADIUS_MD;
pub const CLAUDE_BANNER_FONT_SIZE: f32 = FONT_SIZE_SM;
pub const CLAUDE_BANNER_MARGIN_X: f32 = PAD_STANDARD;
pub const CLAUDE_BANNER_MARGIN_Y: f32 = PAD_SM;
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
/// Vertical padding inside a single session row in the Usage tab (px).
pub const RIGHT_PANEL_ROW_PAD_Y: f32 = 3.0;
/// Font size for the right-panel summary + session rows (px).
/// Matches `AGENT_CHAT_MSG_FONT_SIZE` so the four right-panel tabs feel
/// part of the same typographic family as the original chat panel.
pub const RIGHT_PANEL_BODY_FONT_SIZE: f32 = FONT_SIZE_MD;
/// Font size for inline section labels ("Total", "in", "out", "cache").
pub const RIGHT_PANEL_LABEL_FONT_SIZE: f32 = FONT_SIZE_XS;
/// Maximum displayed width of the lane-label cell in a session
/// row before it truncates with `…` (px). The cell's hard cap keeps
/// a long branch name from pushing the token + cost cells off-screen
/// when the right dock is narrow; below this width the metrics group
/// wraps onto a second line via `flex_wrap` instead.
pub const RIGHT_PANEL_WT_MAX_W: f32 = 140.0;
/// Fixed width allocated to the leading state-indicator glyph so titles
/// across rows align on a single column (px).
pub const RIGHT_PANEL_TASK_INDICATOR_W: f32 = 14.0;
/// Horizontal gap between adjacent action buttons in a task row (px).
pub const RIGHT_PANEL_TASK_BUTTON_GAP: f32 = GAP_SM;
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
/// Vertical height of the status-pill trigger button. Matches the
/// `gpui_component::Button::xsmall()` baseline so the pill aligns
/// flush with the duration / failure cells in the same row.
pub const RIGHT_PANEL_STATUS_PILL_HEIGHT_PX: f32 = 20.0;
/// Horizontal padding inside the pill — slightly wider than the
/// default xsmall padding so the chevron `▾` has room to breathe and
/// the state label stays visually centered.
pub const RIGHT_PANEL_STATUS_PILL_PADDING_X_PX: f32 = PAD_STANDARD;
/// Gap between the state label and the trailing chevron / between
/// adjacent inline children when callers compose the pill manually.
pub const RIGHT_PANEL_STATUS_PILL_GAP_PX: f32 = GAP_SM;
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
/// Inset margin applied when `Divider::inset()` is set (px).
pub const DIVIDER_INSET: f32 = 6.0;
/// Dash + gap lengths used by `Divider::horizontal_dashed()` and
/// `Divider::vertical_dashed()` (px).
pub const DIVIDER_DASH_LEN: f32 = 4.0;
pub const DIVIDER_DASH_GAP: f32 = GAP_XS;
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
