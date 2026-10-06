//! App-side UI palette: design tokens, role tokens, and chrome metrics.
//!
//! Hues are degrees [0, 360] via the local [`hsla`] — deliberately the same
//! contract as `daruda_terminal::ux::theme`, so colours copy between the two.

use gpui::Hsla;

// Preserve direct palette imports while numeric definitions live in one place.
pub use super::metrics::*;

const fn hsla(h_degrees: f32, s: f32, l: f32, a: f32) -> Hsla {
    Hsla {
        h: h_degrees / 360.0,
        s,
        l,
        a,
    }
}

// Derive a variant from its parent token instead of repeating the H/S/L literal.
pub const fn with_alpha(c: Hsla, a: f32) -> Hsla {
    Hsla {
        h: c.h,
        s: c.s,
        l: c.l,
        a,
    }
}

pub const fn with_lightness(c: Hsla, l: f32) -> Hsla {
    Hsla {
        h: c.h,
        s: c.s,
        l,
        a: c.a,
    }
}

// ── Design tokens (DESIGN.md §Colors) ───────────────────────────────────────
// CANVAS is the exact HSL of `#070809` so the theme survives its hex
// round-trip. EDITOR_SURFACE sits one rung above it so syntax colours clear
// the contrast floor. The light ladder matches `daruda_light.json`.

pub const CANVAS: Hsla = hsla(210.0, 0.125, 0.0314, 1.0);
pub const SURFACE_1: Hsla = hsla(210.0, 0.063, 0.063, 1.0);
pub const SURFACE_2: Hsla = hsla(210.0, 0.048, 0.082, 1.0);
pub const SURFACE_3: Hsla = hsla(210.0, 0.040, 0.098, 1.0);
pub const SURFACE_4: Hsla = hsla(210.0, 0.046, 0.128, 1.0);
pub const EDITOR_SURFACE: Hsla = hsla(220.0, 0.10, 0.05, 1.0);
pub const LIGHT_CANVAS: Hsla = hsla(210.0, 0.20, 250.0 / 255.0, 1.0);
pub const LIGHT_SURFACE_1: Hsla = hsla(220.0, 1.0 / 6.0, 237.0 / 255.0, 1.0);
pub const LIGHT_SURFACE_2: Hsla = hsla(225.0, 4.0 / 23.0, 232.0 / 255.0, 1.0);
pub const LIGHT_SURFACE_3: Hsla = hsla(222.0, 5.0 / 31.0, 224.0 / 255.0, 1.0);
pub const LIGHT_INK: Hsla = hsla(222.0, 0.14, 0.12, 1.0);
pub const HAIRLINE: Hsla = hsla(223.0, 0.091, 0.151, 1.0);
pub const INK: Hsla = hsla(0.0, 0.0, 0.97, 1.0);
pub const TEXT_BODY: Hsla = hsla(218.0, 0.089, 0.847, 1.0);
pub const TEXT_MUTE: Hsla = hsla(218.0, 0.064, 0.569, 1.0);
pub const TEXT_SUBTLE: Hsla = hsla(218.0, 0.053, 0.5, 1.0);
pub const ACCENT: Hsla = hsla(233.8, 0.563, 0.596, 1.0);
pub const ACCENT_HOVER: Hsla = hsla(233.8, 1.0, 0.755, 1.0);
pub const ACCENT_MUTED: Hsla = hsla(237.6, 0.45, 0.216, 1.0);
pub const ACCENT_FG: Hsla = hsla(0.0, 0.0, 1.0, 1.0);
pub const TRANSPARENT: Hsla = hsla(0.0, 0.0, 0.0, 0.0);

// Semantic status colours stay desaturated (DESIGN); the SIGNAL_* set is the
// high-chroma variant for gauges, incident pills, and running indicators.
pub const SUCCESS: Hsla = hsla(147.3, 0.406, 0.488, 1.0); // #4aaf78
pub const WARNING: Hsla = hsla(39.5, 0.600, 0.578, 1.0); // #d4a853
pub const ERROR: Hsla = hsla(0.0, 0.674, 0.627, 1.0); // #e06060
pub const SIGNAL_GREEN: Hsla = hsla(135.0, 0.59, 0.49, 1.0);
pub const SIGNAL_YELLOW: Hsla = hsla(50.0, 1.0, 0.52, 1.0);
pub const SIGNAL_ORANGE: Hsla = hsla(33.0, 1.0, 0.52, 1.0);
pub const SIGNAL_RED: Hsla = hsla(4.0, 1.0, 0.62, 1.0);

// Agent action states (Cursor timeline palette); the `_LIGHT` pair is
// darkened for text on light panes.
pub const AGENT_READING: Hsla = hsla(144.6, 0.374, 0.680, 1.0);
pub const AGENT_EDITING: Hsla = hsla(265.7, 0.325, 0.704, 1.0);
pub const AGENT_READING_LIGHT: Hsla = with_lightness(AGENT_READING, 0.31);
pub const AGENT_EDITING_LIGHT: Hsla = with_lightness(AGENT_EDITING, 0.43);

pub const GIT_STAGED: Hsla = SUCCESS; // #4aaf78
pub const GIT_MODIFIED: Hsla = WARNING; // #d4a853
pub const GIT_UNTRACKED: Hsla = TEXT_MUTE; // #8a8f98
pub const GIT_RENAMED: Hsla = hsla(215.6, 0.509, 0.657, 1.0); // #7b9fd4
pub const DIFF_ADD_BG: Hsla = with_alpha(SUCCESS, 0.12);
pub const DIFF_ADD_FG: Hsla = SUCCESS;
pub const DIFF_DEL_BG: Hsla = with_alpha(ERROR, 0.12);
pub const DIFF_DEL_FG: Hsla = ERROR;
pub const DIFF_HUNK: Hsla = TEXT_SUBTLE; // #797e86
pub const SELECTION_BG: Hsla = with_alpha(PRIMARY, 0.28);

// ── Role tokens ─────────────────────────────────────────────────────────────
// Feature code reads roles, not raw tokens: PRIMARY is the one chromatic
// accent, and text uses only the four cool-tinted tones (DESIGN §Don'ts bans
// neutral grays). LINK is not PRIMARY because accent text is only ~4.1:1.

pub const BG_BASE: Hsla = CANVAS;
pub const BG_EDITOR: Hsla = EDITOR_SURFACE;
pub const BG_PANEL: Hsla = SURFACE_1;
pub const BG_RAISED: Hsla = SURFACE_2;
pub const BG_FLOAT: Hsla = SURFACE_4;
pub const BG_HOVER: Hsla = SURFACE_2;
pub const BG_ACTIVE: Hsla = SURFACE_3;
pub const BORDER: Hsla = HAIRLINE;
pub const TEXT_PRIMARY: Hsla = INK;
pub const TEXT_TERTIARY: Hsla = TEXT_MUTE;
pub const PRIMARY: Hsla = ACCENT;
pub const LINK: Hsla = hsla(224.6, 0.905, 0.738, 1.0);
pub const OVERLAY_HOVER: Hsla = hsla(0.0, 0.0, 1.0, 0.04);
pub const OVERLAY_SELECTED: Hsla = hsla(0.0, 0.0, 1.0, 0.06);
pub const OVERLAY_ACTIVE: Hsla = hsla(0.0, 0.0, 1.0, 0.08);
pub const OVERLAY_PROMINENT: Hsla = hsla(0.0, 0.0, 1.0, 0.10);
// Neutral overlays: white lifts a dark surface, black recesses a light one.
pub const OVERLAY_WHITE: Hsla = hsla(0.0, 0.0, 1.0, 1.0);
pub const OVERLAY_BLACK: Hsla = hsla(0.0, 0.0, 0.0, 1.0);

// ── Title bar and window chrome ─────────────────────────────────────────────
// App-drawn window controls (off macOS) are narrower than Windows' 46px
// because the title bar is only 28px tall.

pub const TITLE_BAR_HEIGHT: f32 = 28.0;
pub const TRAFFIC_LIGHT_X: f32 = 8.0;
pub const TRAFFIC_LIGHT_Y: f32 = (TITLE_BAR_HEIGHT - TRAFFIC_LIGHT_SIZE) / 2.0;
pub const TRAFFIC_LIGHT_SIZE: f32 = 12.0;
pub const TRAFFIC_LIGHT_GAP: f32 = 8.0;
pub const TRAFFIC_LIGHT_WIDTH: f32 = 70.0;
pub const CLIENT_CHROME_INSET: f32 = 8.0;
pub const WINDOW_CONTROL_W: f32 = 38.0;
pub const WINDOW_CONTROL_GLYPH_SIZE: f32 = 11.0;
pub const BUTTON_HEIGHT: f32 = 28.0;
pub const CONTROL_DANGER_HOVER_ALPHA: f32 = 0.18;
pub const RENDER_MIN_DIM: f32 = 1.0;
pub const RESIZE_HANDLE_HIT_PX: f32 = 3.0;

// ── Tab bar and pane header ─────────────────────────────────────────────────

pub const TAB_BAR_HEIGHT: f32 = 32.0;
pub const TAB_FONT_SIZE: f32 = FONT_SIZE_MD;
pub const TAB_MIN_WIDTH: f32 = 80.0;
pub const TAB_MAX_WIDTH: f32 = 220.0;
pub const TAB_GAP: f32 = GAP_SM;
pub const TAB_PAD_LEFT: f32 = PAD_LG;
pub const TAB_PAD_RIGHT: f32 = PAD_XS;
pub const TAB_PAD_Y: f32 = GAP_XS;
pub const TAB_MARGIN_X: f32 = 1.0;
pub const TAB_STATUS_DOT_SIZE: f32 = 6.0;
pub const TAB_STATUS_DOT_GAP: f32 = GAP_SM;
pub const NEW_TAB_FONT_SIZE: f32 = FONT_SIZE_MD;
pub const NEW_TAB_MARGIN_X: f32 = GAP_XS;
pub const AGENT_MENU_FLAT_MAX: usize = 5;
pub const PANE_HEADER_HEIGHT: f32 = 28.0;
pub const PANE_HEADER_FONT_SIZE: f32 = FONT_SIZE_SM;
pub const PANE_HEADER_CWD_FONT_SIZE: f32 = FONT_SIZE_XS;
pub const PANE_HEADER_PAD_X: f32 = PAD_STANDARD;
pub const PANE_HEADER_GAP: f32 = GAP_SM;
pub const PANE_HEADER_INNER_GAP: f32 = GAP_STANDARD;
pub const DIM_GRAY_LEVEL: f32 = 0.3; // = daruda_terminal's, so panes gray alike

// ── Status bar ──────────────────────────────────────────────────────────────
// Chips must fit the 24px bar without growing it. PAD_X is wider than PAD_LG
// because the bar meets macOS's rounded bottom corners.

pub const STATUS_BAR_HEIGHT: f32 = 24.0;
pub const STATUS_BAR_FONT_SIZE: f32 = FONT_SIZE_SM;
pub const STATUS_BAR_PAD_X: f32 = 24.0;
pub const STATUS_BAR_GAP: f32 = GAP_LG;
pub const STATUS_BAR_COMPACT_WIDTH: f32 = 720.0;
pub const STATUS_BAR_ICON_ONLY_WIDTH: f32 = 480.0;
pub const STATUS_BAR_PROJECT_DOT: Hsla = hsla(180.0, 0.55, 0.55, 1.0);
pub const STATUS_BAR_PROJECT_DOT_SIZE: f32 = 6.0;
pub const STATUS_BAR_DETACHED_BG: Hsla = with_lightness(WARNING, 0.22);
pub const STATUS_BAR_DETACHED_TEXT: Hsla = with_lightness(WARNING, 0.72);
pub const STATUS_BAR_DETACHED_FONT_SIZE: f32 = FONT_SIZE_XS;
pub const STATUS_BAR_DETACHED_PAD_X: f32 = 5.0;
pub const STATUS_BAR_DETACHED_PAD_Y: f32 = 1.0;
pub const STATUS_BAR_DETACHED_RADIUS: f32 = RADIUS_XS;
pub const STATUS_BAR_ACCOUNT_HEIGHT: f32 = 18.0;
pub const STATUS_BAR_ACCOUNT_PAD_X: f32 = 6.0;
pub const STATUS_BAR_ACCOUNT_RADIUS: f32 = RADIUS_XS;
pub const STATUS_BAR_AGENT_ICON_SIZE: f32 = 12.0;
pub const STATUS_BAR_USAGE_CHIP_GAP: f32 = GAP_SM;
// Fixed so the dropdown's gauges share one scale; `PopupMenu` sizes to content.
pub const STATUS_BAR_USAGE_ROW_WIDTH: f32 = 180.0;
pub const STATUS_BAR_USAGE_ROW_GAP: f32 = GAP_XS;

// ── Docks ───────────────────────────────────────────────────────────────────

pub const DOCK_VIEW_TAB_PAD_X: f32 = PAD_LG;
pub const DOCK_VIEW_TAB_FONT_SIZE: f32 = FONT_SIZE_SM;
pub const DOCK_TAB_WIDTH: f32 = 36.0;
pub const DOCK_NAV_ROW_HEIGHT: f32 = 34.0;
pub const DOCK_NAV_ROW_GAP: f32 = 3.0;
pub const DOCK_NAV_PAD_X: f32 = PAD_STANDARD;
pub const DOCK_NAV_PAD_Y: f32 = 12.0;
pub const DOCK_PAGE_PAD: f32 = 24.0;
pub const DOCK_PAGE_MAX_WIDTH: f32 = 960.0;
pub const DOCK_TREE_ROW_HEIGHT: f32 = 28.0;
pub const DOCK_TREE_ROW_PAD_Y: f32 = 2.0;
pub const DOCK_TREE_ROW_LINE_MIN_H: f32 = DOCK_TREE_ROW_HEIGHT - 2.0 * DOCK_TREE_ROW_PAD_Y;
pub const DOCK_SECTION_HEADER_PAD_Y: f32 = 9.0;
pub const DOCK_SECTION_DIVIDER_PAD_T: f32 = PAD_XS;
pub const DOCK_SECTION_CHEVRON_SIZE: f32 = 12.0;
pub const DOCK_LIBRARY_ROW_PAD_Y: f32 = 10.0;
pub const DOCK_LIBRARY_ROW_GAP: f32 = PAD_STANDARD;
pub const DOCK_LIBRARY_ICON_SIZE: f32 = 14.0;
pub const DOCK_LIBRARY_ICON_MT: f32 = 3.0;
pub const DOCK_LIBRARY_DESC_MT: f32 = GAP_XS;
pub const DOCK_PANEL_FOOTER_HEIGHT: f32 = 36.0;
pub const DOCK_PANEL_FOOTER_PAD_X: f32 = 14.0;
pub const DOCK_PANEL_FOOTER_ICON_SIZE: f32 = 12.0;
pub const DOCK_PLACEHOLDER_FONT_SIZE: f32 = FONT_SIZE_MD;
pub const DOCK_TOGGLE_ICON_SIZE: f32 = 14.0;
pub const DOCK_FOOTER_PAD: f32 = 4.0;
pub const DOCK_ICON_GROUP_MR: f32 = PAD_STANDARD;
pub const DOCK_ICON_GROUP_GAP: f32 = GAP_XS;
pub const PANEL_BODY_PAD_X: f32 = PAD_STANDARD;
pub const PANEL_BODY_PAD_Y: f32 = PAD_STANDARD;
pub const PANEL_BODY_GAP: f32 = GAP_STANDARD;

// Dock sizes. The right default fits all five tab labels in the widest
// locale. Bottom presets are `TAB_BAR_HEIGHT + 2*PANEL_BODY_PAD_Y +
// N*BUTTON_WIDGET_HEIGHT + (N-1)*PANEL_BODY_GAP`, one per macro row count.
pub const DOCK_LEFT_DEFAULT_W: f32 = 250.0;
pub const DOCK_LEFT_MIN_W: f32 = 220.0;
pub const DOCK_LEFT_MAX_W: f32 = 400.0;
pub const DOCK_RIGHT_DEFAULT_W: f32 = 290.0;
pub const DOCK_RIGHT_MIN_W: f32 = 220.0;
pub const DOCK_RIGHT_MAX_W: f32 = 500.0;
pub const DOCK_BOTTOM_DEFAULT_H: f32 = 76.0;
pub const DOCK_BOTTOM_MIN_H: f32 = 76.0;
pub const DOCK_BOTTOM_MAX_H: f32 = 500.0;
pub const DOCK_BOTTOM_ROW_PRESET_1_H: f32 = 76.0;
pub const DOCK_BOTTOM_ROW_PRESET_2_H: f32 = 114.0;
pub const DOCK_BOTTOM_ROW_PRESET_3_H: f32 = 152.0;
// One `Size::Small` input line (1.25 rem × 16px), independent of the terminal font.
pub const DOCK_BOTTOM_INPUT_EXTRA_LINE_H: f32 = 20.0;
pub const DOCK_BOTTOM_INPUT_TEXT_PAD_H: f32 = INPUT_TEXTAREA_PAD_Y * 2.0;
pub const DOCK_BOTTOM_INPUT_ACTION_ROW_H: f32 = BUTTON_HEIGHT + INPUT_PANEL_BUTTON_GAP;

// ── Bottom dock input and macro tiles ───────────────────────────────────────

pub const INPUT_PANEL_SECTION_GAP: f32 = GAP_LG;
pub const INPUT_PANEL_BUTTON_GAP: f32 = GAP_SM;
pub const INPUT_TEXTAREA_PAD_X: f32 = 12.0;
pub const INPUT_TEXTAREA_PAD_Y: f32 = 8.0;
pub const INPUT_PANEL_MIN_H: f32 = 48.0;
pub const INPUT_PANEL_FLOATING_BAR_H: f32 = 32.0;
pub const BUTTON_WIDGET_HEIGHT: f32 = 32.0;
pub const BUTTON_WIDGET_TILE_WIDTH: f32 = 96.0;
pub const BUTTON_WIDGET_ICON_SIZE: f32 = 32.0;
pub const BUTTON_WIDGET_ADD_BORDER_W: f32 = 1.0;
pub const BUTTON_WIDGET_FONT_SIZE: f32 = FONT_SIZE_MD;
pub const BUTTON_WIDGET_PAD_X: f32 = PAD_LG;
pub const BUTTON_WIDGET_RADIUS: f32 = RADIUS_SM;

// ── Drag and drop ───────────────────────────────────────────────────────────
// Every valid drop target derives from one tint by alpha; a rejected target
// is a faint red that reads as "not here", not as an error.

pub const DROP_TARGET_TINT: Hsla = hsla(210.0, 0.50, 0.30, 1.0);
pub const LANE_DROP_TARGET_BG: Hsla = with_alpha(DROP_TARGET_TINT, 0.35);
pub const INPUT_PANEL_DROP_TARGET_BG: Hsla = with_alpha(DROP_TARGET_TINT, 0.20);
pub const TERMINAL_DROP_TARGET_BG: Hsla = with_alpha(DROP_TARGET_TINT, 0.30);
pub const LANE_DROP_TARGET_REJECTED_BG: Hsla = hsla(0.0, 0.55, 0.32, 0.20);
pub const LANE_DRAG_GHOST_PAD_Y: f32 = PAD_XS;
pub const DRAG_PILL_CURSOR_OFFSET: f32 = 4.0;

// ── Lanes and project tree (left dock) ──────────────────────────────────────
// A lane's inset lands its label on the project name and centres its status
// cell under the folder glyph. Group presets are stored as hex on
// `SerializedGroup::color`, so the header decodes them without a lookup.

pub const LANE_ROW_PAD_X: f32 = PAD_LG;
pub const LANE_ROW_GAP: f32 = GAP_SM;
pub const LANE_ROW_RADIUS: f32 = RADIUS_SM;
pub const LANE_ROOT_GAP: f32 = GAP_STANDARD;
pub const LANE_TREE_MARGIN_X: f32 = PAD_STANDARD;
pub const LANE_LIST_GAP_Y: f32 = 0.0;
pub const LANE_INDENT_STEP: f32 = 8.0;
pub const LANE_ACTIVE_BORDER_W: f32 = 2.0;
pub const LANE_LABEL_FONT_SIZE: f32 = FONT_SIZE_MD;
pub const LANE_LABEL_GAP: f32 = GAP_STANDARD;
pub const LANE_SUB_FONT_SIZE: f32 = FONT_SIZE_XS;
pub const LANE_SUBLABEL_GAP: f32 = GAP_STANDARD;
pub const LANE_SECTION_PAD_Y: f32 = PAD_SM;
pub const LANE_PLACEHOLDER_PAD: f32 = 12.0;
pub const LANE_PLACEHOLDER_GIT_INIT_MT: f32 = PAD_XS;
pub const LANE_PLACEHOLDER_LINE_GAP: f32 = GAP_STANDARD;
pub const LANE_UNREAD_DOT_SIZE: f32 = 6.0;
pub const LANE_UNREAD_DOT_RADIUS: f32 = RADIUS_XS;
pub const LANE_BRANCH_CHIP_PAD_X: f32 = PAD_XS;
pub const LANE_PROJECT_ICON_SIZE: f32 = 14.0;
pub const LANE_PROJECT_NAME_INSET: f32 = LANE_ROW_PAD_X + LANE_PROJECT_ICON_SIZE + LANE_LABEL_GAP;
pub const LANE_ROW_INSET_L: f32 =
    LANE_PROJECT_NAME_INSET - LANE_ACTIVE_BORDER_W - STATUS_INDICATOR_CELL_WIDTH - LANE_ROW_GAP;
pub const LANE_GROUP_LABEL_FONT_SIZE: f32 = FONT_SIZE_SM;
pub const LANE_GROUP_COLOR_DOT_SIZE: f32 = 8.0;
pub const LANE_GROUP_COLOR_DOT_RADIUS: f32 = RADIUS_SM;
pub const LANE_GROUP_OUTLINE_W: f32 = 1.0;
pub const LANE_GROUP_OUTLINE_PAD: f32 = 3.0;
pub const LANE_GROUP_OUTLINE_RADIUS: f32 = RADIUS_MD;
pub const LANE_GROUP_OUTLINE_MARGIN_Y: f32 = GAP_XS;
pub const GROUP_PRESET_RED: &str = "#f87171";
pub const GROUP_PRESET_ORANGE: &str = "#fb923c";
pub const GROUP_PRESET_YELLOW: &str = "#facc15";
pub const GROUP_PRESET_LIME: &str = "#a3e635";
pub const GROUP_PRESET_GREEN: &str = "#4ade80";
pub const GROUP_PRESET_TEAL: &str = "#2dd4bf";
pub const GROUP_PRESET_CYAN: &str = "#22d3ee";
pub const GROUP_PRESET_BLUE: &str = "#60a5fa";
pub const GROUP_PRESET_INDIGO: &str = "#818cf8";
pub const GROUP_PRESET_PURPLE: &str = "#a78bfa";
pub const GROUP_PRESET_PINK: &str = "#f472b6";

// ── Agent status indicator ──────────────────────────────────────────────────
// One shared `StatusPulseClock` ticks every badge at ~4 fps. Kept low because
// it repaints every window with an animating session, backgrounded ones too
// (Pitfall #10).

pub const STATUS_INDICATOR_SIZE: f32 = 16.0;
pub const STATUS_INDICATOR_BADGE_SIZE: f32 = 12.0;
pub const STATUS_INDICATOR_CELL_WIDTH: f32 = STATUS_INDICATOR_SIZE + GAP_XS;
pub const STATUS_INDICATOR_TICK_MS: u64 = 250;
pub const STATUS_INDICATOR_RING_CENTER_ALPHA: f32 = 0.15;
pub const STATUS_INDICATOR_PULSE_OPACITY_MIN: f32 = 0.4;
pub const STATUS_INDICATOR_DOT_GRID_RATIO: f32 = 0.6;
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
pub const STATUS_FAILED_DARK: Hsla = ERROR;
pub const STATUS_CONNECTING_DARK: Hsla = hsla(220.0, 0.09, 0.65, 1.0);
pub const STATUS_BADGES_ROW_GAP: f32 = 5.0;
pub const STATUS_BADGES_ROW_TOP_MARGIN: f32 = 3.0;
pub const STATUS_BADGES_LABEL_FONT_SIZE: f32 = FONT_SIZE_XS;
pub const STATUS_BADGES_LABEL_GAP: f32 = GAP_STANDARD;
pub const STATUS_BADGE_ACTIVE_OUTLINE: Hsla = hsla(0.0, 0.0, 1.0, 0.85);
pub const STATUS_BADGE_ACTIVE_OUTER_PAD: f32 = GAP_XS;
pub const STATUS_BADGE_TOOLTIP_SESSION_PREFIX_LEN: usize = 8;

// ── Modals, banners, popups ─────────────────────────────────────────────────

pub const MODAL_BACKDROP_ALPHA: f32 = 0.50;
pub const MODAL_PANEL_RADIUS: f32 = RADIUS_LG;
pub const MODAL_PANEL_WIDTH: f32 = 420.0;
pub const MODAL_PANEL_GAP: f32 = PAD_LG;
pub const MODAL_TITLE_FONT_SIZE: f32 = FONT_SIZE_LG;
pub const MODAL_BODY_FONT_SIZE: f32 = FONT_SIZE_MD;
pub const MODAL_INPUT_PAD: f32 = PAD_STANDARD;
pub const MODAL_BUTTON_PAD_X: f32 = PAD_XL;
pub const MODAL_BUTTON_PAD_Y: f32 = PAD_SM;
pub const MODAL_BUTTON_RADIUS: f32 = RADIUS_SM;
pub const MODAL_FOOTER_GAP: f32 = GAP_LG;
pub const MODAL_FOOTER_MARGIN_TOP: f32 = PAD_SM;
pub const MODAL_NOTES_TEXTAREA_MIN_H: f32 = 120.0;
pub const MODAL_RADIO_W: f32 = 14.0;
pub const FORM_MODAL_WIDE: f32 = 900.0;
pub const FORM_MODAL_SECTION_GAP: f32 = 12.0;
pub const FORM_MODAL_SPLIT_GAP: f32 = 16.0;
pub const ERROR_MODAL_WIDTH: f32 = 640.0;
pub const ERROR_MODAL_BODY_FONT_SIZE: f32 = FONT_SIZE_SM;
pub const ERROR_MODAL_BODY_PAD: f32 = PAD_LG;
pub const ERROR_MODAL_BODY_MAX_H: f32 = 360.0;
pub const POPUP_MENU_DEPLOY_EDGE_MARGIN: f32 = 8.0;
pub const BANNER_ERROR_BG: Hsla = with_alpha(ERROR, 0.10);
pub const BANNER_ERROR_TEXT: Hsla = with_lightness(ERROR, 0.70);
pub const BANNER_WARNING_BG: Hsla = with_alpha(WARNING, 0.10);
pub const BANNER_WARNING_TEXT: Hsla = with_lightness(WARNING, 0.70);
pub const BANNER_INFO_BG: Hsla = with_alpha(PRIMARY, 0.10);
pub const BANNER_INFO_TEXT: Hsla = with_lightness(PRIMARY, 0.75);
pub const BANNER_SUCCESS_BG: Hsla = with_alpha(SUCCESS, 0.10);
pub const BANNER_SUCCESS_TEXT: Hsla = with_lightness(SUCCESS, 0.65);

// ── Toasts ──────────────────────────────────────────────────────────────────

pub const TOAST_RADIUS: f32 = RADIUS_LG;
pub const TOAST_PAD_X: f32 = 16.0;
pub const TOAST_PAD_Y: f32 = PAD_LG;
pub const TOAST_FONT_SIZE: f32 = FONT_SIZE_LG;
pub const TOAST_TITLE_FONT_SIZE: f32 = FONT_SIZE_MD;
pub const TOAST_GAP: f32 = 12.0;
pub const TOAST_MIN_W: f32 = 240.0;
pub const TOAST_MAX_W: f32 = 480.0;
pub const TOAST_STACK_GAP: f32 = GAP_SM;
pub const TOAST_STACK_BOTTOM_PAD: f32 = PAD_XS;
pub const TOAST_SEVERITY_BAR_W: f32 = 3.0;
pub const TOAST_REPEAT_FONT_SIZE: f32 = FONT_SIZE_XS;
pub const TOAST_REPEAT_PAD_X: f32 = PAD_SM;
pub const TOAST_REPEAT_PAD_Y: f32 = GAP_XS;

// ── Command palette and pickers ─────────────────────────────────────────────
// MAX_HEIGHT is derived from MAX_VISIBLE: a taller list shows rows the
// keyboard cannot reach, a shorter one clips rows it can. The focus rule is
// needed because the focused row's tint alone (L 8.2% vs 9.8%) is too faint.

pub const PALETTE_WIDTH: f32 = 500.0;
pub const PALETTE_TOP_OFFSET: f32 = 40.0;
pub const PALETTE_RADIUS: f32 = RADIUS_LG;
pub const PALETTE_INPUT_PAD_X: f32 = 12.0;
pub const PALETTE_INPUT_PAD_Y: f32 = PAD_STANDARD;
pub const PALETTE_QUERY_FONT_SIZE: f32 = FONT_SIZE_LG;
pub const PALETTE_ENTRY_PAD_X: f32 = 12.0;
pub const PALETTE_ENTRY_PAD_Y: f32 = PAD_SM;
pub const PALETTE_ENTRY_FONT_SIZE: f32 = FONT_SIZE_LG;
pub const PALETTE_SHORTCUT_FONT_SIZE: f32 = FONT_SIZE_SM;
pub const PALETTE_ROW_LINE_H: f32 = (PALETTE_ENTRY_FONT_SIZE * 1.618_034) as i32 as f32;
pub const PALETTE_ROW_H: f32 = PALETTE_ROW_LINE_H + 2.0 * PALETTE_ENTRY_PAD_Y;
pub const PALETTE_MAX_VISIBLE: usize = 12;
pub const PALETTE_MAX_HEIGHT: f32 = PALETTE_MAX_VISIBLE as f32 * PALETTE_ROW_H;
pub const PALETTE_FOCUS_BORDER_W: f32 = LANE_ACTIVE_BORDER_W;
pub const PALETTE_EMPTY_PAD_Y: f32 = 16.0;

// ── Settings ────────────────────────────────────────────────────────────────
// Dependent rows dim to the vendored button's disabled tone while their
// parent switch is off.

pub const SETTINGS_SIDEBAR_W: f32 = 208.0;
pub const SETTINGS_SIDEBAR_BG: Hsla = with_alpha(CANVAS, 0.18);
pub const SETTINGS_SIDEBAR_PAD_Y: f32 = PAD_SM;
pub const SETTINGS_SIDEBAR_ROW_PAD_X: f32 = PAD_XL;
pub const SETTINGS_SIDEBAR_ROW_PAD_Y: f32 = PAD_SM;
pub const SETTINGS_CONTENT_MAX_W: f32 = 800.0;
pub const SETTINGS_CONTENT_PAD: f32 = 24.0;
pub const SETTINGS_GROUP_GAP: f32 = 28.0;
pub const SETTINGS_CARD_PAD: f32 = 16.0;
pub const SETTINGS_ROW_PAD_Y: f32 = 18.0;
pub const SETTINGS_ROW_MIN_H: f32 = 68.0;
pub const SETTINGS_ROW_GAP: f32 = 24.0;
pub const SETTINGS_LABEL_MIN_W: f32 = 180.0;
pub const SETTINGS_CONTROL_W: f32 = 240.0;
pub const SETTINGS_NUMBER_W: f32 = 96.0;
pub const SETTINGS_ACTIVE_BORDER: f32 = 2.0;
pub const SETTINGS_DEPENDENT_INDENT: f32 = 16.0;
pub const SETTINGS_DEPENDENT_OFF_OPACITY: f32 = 0.5;
pub const SETTINGS_PLUGIN_MASTER_W: f32 = 220.0;
pub const SETTINGS_PLUGIN_LABEL_W: f32 = 110.0;
pub const SETTINGS_AGENT_ENV_ROWS_MIN: usize = 2;
pub const SETTINGS_AGENT_ENV_ROWS_MAX: usize = 6;
pub const SETTINGS_AGENT_CARD_ICON_SIZE: f32 = 20.0;

// ── Landing view and main-area empty state ──────────────────────────────────

pub const WELCOME_TITLE_FONT_SIZE: f32 = 28.0;
pub const WELCOME_VERSION_FONT_SIZE: f32 = FONT_SIZE_LG;
pub const WELCOME_HEADING_FONT_SIZE: f32 = FONT_SIZE_LG;
pub const WELCOME_PANEL_WIDTH: f32 = 420.0;
pub const WELCOME_PANEL_PAD: f32 = 40.0;
pub const WELCOME_GAP: f32 = 16.0;
pub const WELCOME_GAP_TIGHT: f32 = GAP_SM;
pub const WELCOME_GAP_LOOSE: f32 = GAP_LG;
pub const MAIN_EMPTY_STATE_ICON_SIZE: f32 = 36.0;
pub const MAIN_EMPTY_STATE_TITLE_FONT_SIZE: f32 = FONT_SIZE_LG;
pub const MAIN_EMPTY_STATE_BODY_FONT_SIZE: f32 = FONT_SIZE_MD;
pub const MAIN_EMPTY_STATE_GAP: f32 = 12.0;
pub const MAIN_EMPTY_STATE_BODY_MAX_W: f32 = 360.0;

// ── Scrollbars ──────────────────────────────────────────────────────────────

pub const SCROLLBAR_W: f32 = 4.0;
pub const SCROLLBAR_MARGIN_R: f32 = GAP_XS;
pub const SCROLLBAR_MIN_THUMB_H: f32 = 24.0;
pub const SCROLLBAR_THUMB: Hsla = hsla(0.0, 0.0, 1.0, 0.25);
pub const SCROLLBAR_THUMB_HOVER: Hsla = hsla(0.0, 0.0, 1.0, 0.45);
pub const SCROLLBAR_TRACK_BG: Hsla = hsla(0.0, 0.0, 1.0, 0.04);
pub const SCROLL_AREA_GUTTER: f32 = 10.0;

// ── Git changes view ────────────────────────────────────────────────────────

pub const GIT_SECTION_FONT_SIZE: f32 = 10.5;
pub const GIT_HEADER_PAD_X: f32 = PAD_LG;
pub const GIT_HEADER_PAD_Y: f32 = PAD_SM;
pub const GIT_DIR_HEADER_FONT_SIZE: f32 = FONT_SIZE_XS;
pub const GIT_FILE_ROW_HEIGHT: f32 = 28.0;
pub const GIT_FILE_ROW_PAD_X: f32 = PAD_LG;
pub const GIT_FILE_ROW_GAP: f32 = GAP_STANDARD;
pub const GIT_STATUS_CHAR_W: f32 = 14.0;
pub const GIT_STAGE_CHECKBOX_CHECKED_BG: Hsla = ACCENT;
pub const GIT_BADGE_FONT_SIZE: f32 = FONT_SIZE_XS;
pub const GIT_BADGE_PILL_RADIUS: f32 = 9.0;
pub const GIT_BADGE_PILL_PAD_X: f32 = PAD_SM;
pub const GIT_BADGE_PILL_PAD_Y: f32 = 0.0;
pub const GIT_BADGE_PILL_MIN_W: f32 = 16.0;
pub const GIT_BADGE_ARROW_SIZE: f32 = 9.0;
pub const GIT_BADGE_GAP: f32 = GAP_SM;
pub const GIT_BADGE_ARROW_NUM_GAP: f32 = 1.0;
pub const GIT_COMMIT_PAD: f32 = PAD_STANDARD;
pub const GIT_COMMIT_BUTTON_GAP: f32 = GAP_STANDARD;
pub const GIT_COMMIT_FOOTER_H: f32 = 128.0;
pub const GIT_REMOTE_BTN_GAP: f32 = GAP_SM;

// ── Files tree ──────────────────────────────────────────────────────────────

pub const FILES_ROW_HEIGHT: f32 = 24.0;
pub const FILES_ROW_PAD_X: f32 = PAD_STANDARD;
pub const FILES_ROW_GAP: f32 = GAP_SM;
pub const FILES_ROW_FONT_SIZE: f32 = FONT_SIZE_MD;
pub const FILES_INDENT_PX: f32 = PAD_XL;
pub const FILES_CHEVRON_W: f32 = CONTROL_ICON_SIZE;
pub const FILES_ICON_W: f32 = 16.0;
pub const DISCLOSURE_CHEVRON_W: f32 = CONTROL_TARGET_SIZE;

// ── File viewer and diff ────────────────────────────────────────────────────
// Diff colours alias the DESIGN diff tokens so file-viewer and git-changes
// diffs match. The active-line band is a neutral overlay so it reads on both
// the UI-themed viewer and the terminal-derived agent-chat diff surface.

pub const FILE_VIEWER_HEADER_H: f32 = 32.0;
pub const FILE_VIEWER_HEADER_PAD_X: f32 = PAD_LG;
pub const FILE_VIEWER_HEADER_FONT_SIZE: f32 = FONT_SIZE_SM;
pub const FILE_VIEWER_TOOLBAR_GAP: f32 = GAP_SM;
pub const FILE_VIEWER_FONT_SIZE: f32 = FONT_SIZE_MD;
pub const FILE_VIEWER_LINE_H_RATIO: f32 = 1.7;
pub const FILE_VIEWER_LINE_NO_W: f32 = 50.0;
pub const FILE_VIEWER_LINE_NO_PAD_R: f32 = PAD_STANDARD;
pub const FILE_VIEWER_SCROLL_ORIGIN_X: f32 = 0.0;
pub const EDITOR_ACTIVE_LINE_ALPHA: f32 = 0.10;
pub const FILE_DIFF_ADD_BG: Hsla = DIFF_ADD_BG;
pub const FILE_DIFF_DEL_BG: Hsla = DIFF_DEL_BG;
pub const FILE_DIFF_ADD_TEXT: Hsla = DIFF_ADD_FG;
pub const FILE_DIFF_DEL_TEXT: Hsla = DIFF_DEL_FG;
pub const FILE_DIFF_HUNK_TEXT: Hsla = DIFF_HUNK;
pub const FILE_DIFF_HUNK_CTX_TEXT: Hsla = hsla(220.0, 0.20, 0.45, 1.0);
pub const FILE_DIFF_WORD_ADD_BG: Hsla = with_alpha(SUCCESS, 0.30);
pub const FILE_DIFF_WORD_DEL_BG: Hsla = with_alpha(ERROR, 0.30);
pub const FILE_DIFF_STAT_GAP: f32 = GAP_SM;
pub const FILE_DIFF_STAT_ADD: Hsla = DIFF_ADD_FG;
pub const FILE_DIFF_STAT_DEL: Hsla = DIFF_DEL_FG;
pub const FILE_DIFF_STAT_FONT_SIZE: f32 = FONT_SIZE_SM;
pub const FILE_VIEWER_SEARCH_PANEL_W: f32 = 380.0;
pub const FILE_VIEWER_SEARCH_PANEL_H: f32 = 36.0;
pub const FILE_VIEWER_SEARCH_PAD_X: f32 = 12.0;
pub const FILE_VIEWER_SEARCH_MARGIN_R: f32 = 16.0;
pub const FILE_VIEWER_SEARCH_MARGIN_T: f32 = PAD_STANDARD;
pub const FILE_VIEWER_SEARCH_FONT_SIZE: f32 = FONT_SIZE_MD;
pub const FILE_VIEWER_SEARCH_ITEM_GAP: f32 = GAP_SM;
pub const FILE_VIEWER_SEARCH_COUNTER_SIZE: f32 = 11.0;
pub const FILE_VIEWER_SEARCH_BTN_ML: f32 = PAD_XS;
pub const FILE_VIEWER_SEARCH_EMPTY: Hsla = hsla(0.0, 0.50, 0.65, 1.0);
pub const FILE_VIEWER_SEARCH_MATCH_BG: Hsla = hsla(43.0, 0.70, 0.30, 0.45);
pub const FILE_VIEWER_SEARCH_FOCUSED_BG: Hsla = hsla(43.0, 0.85, 0.48, 0.60);

// ── Markdown ────────────────────────────────────────────────────────────────
// `MD_*` serve the file viewer's own `MdBlock` renderer. `MD_VIEW_*` serve
// the agent chat's `crate::ui::markdown` as multiples of the text size; its
// muted alpha matches the chat fg ramp so a quote lands on `pane-fg-muted`.

pub const MD_H1_FONT_SCALE: f32 = 18.0 / FILE_VIEWER_FONT_SIZE;
pub const MD_H2_FONT_SCALE: f32 = 15.0 / FILE_VIEWER_FONT_SIZE;
pub const MD_H3_FONT_SCALE: f32 = 13.0 / FILE_VIEWER_FONT_SIZE;
pub const MD_H4_FONT_SCALE: f32 = 1.0;
pub const MD_H2_COLOR: Hsla = hsla(0.0, 0.0, 0.92, 1.0);
pub const MD_HEADING_MARGIN_TOP: f32 = 12.0;
pub const MD_BODY_PAD_X: f32 = 24.0;
pub const MD_BODY_PAD_Y: f32 = 16.0;
pub const MD_BLOCK_GAP: f32 = GAP_LG;
pub const MD_BLOCK_RADIUS: f32 = RADIUS_XS;
pub const MD_BLOCK_MARGIN_Y: f32 = PAD_XS;
pub const MD_CODE_BLOCK_RADIUS: f32 = RADIUS_MD;
pub const MD_CODE_BLOCK_PAD_X: f32 = 12.0;
pub const MD_CODE_BLOCK_PAD_Y: f32 = PAD_STANDARD;
pub const MD_BLOCKQUOTE_BORDER_W: f32 = 3.0;
pub const MD_BLOCKQUOTE_PAD_L: f32 = 12.0;
pub const MD_RULE_H: f32 = 1.0;
pub const MD_STRIKETHROUGH_H: f32 = 1.0;
pub const MD_LIST_INDENT: f32 = 16.0;
pub const MD_LIST_ITEM_GAP: f32 = GAP_XS;
pub const MD_LIST_ROW_GAP: f32 = GAP_STANDARD;
pub const MD_FOOTNOTE_COLOR: Hsla = hsla(209.0, 0.45, 0.60, 1.0);
pub const MD_TABLE_CELL_PAD_X: f32 = PAD_LG;
pub const MD_TABLE_CELL_PAD_Y: f32 = 5.0;
pub const MD_TABLE_CELL_MIN_W: f32 = 60.0;
pub const MD_IMAGE_MAX_HEIGHT: f32 = 600.0;
pub const MD_INLINE_IMAGE_HEIGHT: f32 = FILE_VIEWER_FONT_SIZE * 1.3;
pub const MD_VIEW_LINE_HEIGHT: f32 = 1.6;
pub const MD_VIEW_PARAGRAPH_GAP: f32 = 1.25;
pub const MD_VIEW_MUTED_ALPHA: f32 = AGENT_CHAT_FG_MUTED_ALPHA;
pub const MERMAID_LIGHTBOX_VIEWPORT_FRACTION: f32 = 0.9;

// ── Agent chat pane ─────────────────────────────────────────────────────────
// The pane mirrors the terminal palette, so its fills and edges are neutral
// overlays (`theme::agent_chat_tint`) rather than UI colours, and links are
// clamped to stay legible on a background the UI theme never saw.

pub const AGENT_CHAT_MSG_FONT_SIZE: f32 = FONT_SIZE_LG;
pub const AGENT_CHAT_FG_BRIGHTEN: f32 = 0.24;
pub const AGENT_CHAT_FG_MUTED_ALPHA: f32 = 0.62;
pub const AGENT_CHAT_FG_SUBTLE_ALPHA: f32 = 0.5;
pub const AGENT_CHAT_USER_TINT: Hsla = with_alpha(PRIMARY, 0.22);
pub const AGENT_CHAT_CARD_TINT_ALPHA: f32 = 0.05;
pub const AGENT_CHAT_HOVER_TINT_ALPHA: f32 = 0.03;
pub const AGENT_CHAT_CARD_BORDER_ALPHA: f32 = 0.12;
pub const PANE_LINK_MIN_L_ON_DARK: f32 = 0.74;
pub const PANE_LINK_MAX_L_ON_LIGHT: f32 = 0.34;
pub const AGENT_CHAT_PAD_X: f32 = PAD_STANDARD;
pub const AGENT_CHAT_PAD_Y: f32 = PAD_XS;
pub const AGENT_CHAT_MSG_GAP: f32 = GAP_XS;
pub const AGENT_CHAT_LIST_GAP: f32 = GAP_LG;
pub const AGENT_CHAT_TURN_GAP: f32 = PAD_XL;
pub const AGENT_CHAT_SUMMARY_GAP: f32 = GAP_STANDARD;
pub const AGENT_CHAT_TRAILING_GAP: f32 = GAP_STANDARD;
pub const AGENT_CHAT_BOUNDARY_GAP: f32 = GAP_STANDARD;
pub const AGENT_CHAT_FOLD_STATUS_INSET: f32 = AGENT_CHAT_INPUT_INNER_PAD_X + 1.0;
pub const AGENT_CHAT_HEADER_ICON_GAP: f32 = GAP_STANDARD;
pub const AGENT_CHAT_HEADER_ICON_SIZE: f32 = 16.0;
pub const AGENT_CHAT_COMMAND_TAG_MAX_W: f32 = 88.;
pub const AGENT_CHAT_INPUT_INNER_PAD_X: f32 = PAD_STANDARD;
pub const AGENT_CHAT_INPUT_INNER_PAD_Y: f32 = PAD_XS;
pub const AGENT_CHAT_INPUT_RADIUS: f32 = RADIUS_SM;
pub const AGENT_CHAT_DIAGRAM_PAD: f32 = PAD_STANDARD;
pub const AGENT_CHAT_DIAGRAM_GAP: f32 = GAP_LG;
pub const AGENT_CHAT_SCROLL_BOTTOM_SLACK: f32 = 24.0;
pub const AGENT_CHAT_SCROLL_BTN_INSET: f32 = 12.0;
pub const AGENT_CHAT_PLAN_MAX_H: f32 = 168.0;

// Embed row caps are load-bearing: an editor paints every row inside its
// own bounds, so an unbounded embed costs O(output) per paint. The inline
// diff fallback instead drops lines past its cap (one div per line).
pub const AGENT_CHAT_EMBED_MAX_ROWS: usize = 12;
pub const AGENT_CHAT_DIFF_EMBED_MAX_ROWS: usize = 300;
pub const AGENT_CHAT_DIFF_FALLBACK_MAX_ROWS: usize = 1000;

// Activity Bar. Chip edges are controls held to 3:1 (DESIGN.md §Readability):
// 0.34 over `#1e1e1e`, 0.42 over `#f9fafb`. The compact breakpoint is
// font-dependent; read it via `theme::agent_chat_compact_options_w`.
pub const AGENT_CHAT_CONTROL_BORDER_ALPHA_ON_DARK: f32 = 0.34;
pub const AGENT_CHAT_CONTROL_BORDER_ALPHA_ON_LIGHT: f32 = 0.42;
pub const AGENT_CHAT_BAR_CONTROL_GAP: f32 = GAP_SM;
pub const AGENT_CHAT_METER_GAP: f32 = GAP_LG;
pub const AGENT_CHAT_METER_FONT_RATIO: f32 = 12.0 / 13.0;
pub const AGENT_CHAT_OPTIONS_CLUSTER_W: f32 = 200.0;
pub const AGENT_CHAT_TITLE_MIN_W: f32 = 180.0;
pub const AGENT_CHAT_COMPACT_OPTIONS_W: f32 =
    AGENT_CHAT_TITLE_MIN_W + AGENT_CHAT_OPTIONS_CLUSTER_W + 2.0 * AGENT_CHAT_PAD_X;

pub const AGENT_QUEUE_STRIP_MAX_H: f32 = 120.0;
pub const AGENT_QUEUE_STRIP_GAP: f32 = GAP_SM;
pub const AGENT_QUEUE_STRIP_FONT_SIZE: f32 = FONT_SIZE_MD;
pub const AGENT_QUEUE_STRIP_ROW_RADIUS: f32 = RADIUS_SM;
pub const AGENT_QUEUE_STRIP_ROW_PAD_X: f32 = PAD_SM;
pub const AGENT_QUEUE_STRIP_ROW_PAD_Y: f32 = PAD_XS;
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

// ── Transcript editor popovers ──────────────────────────────────────────────
// Sizes are outer boxes (the `Popover` border is subtracted). Text roles and
// tracks are fractions, so a larger configured font scales them together.

pub const TRANSCRIPT_EDITOR_RULES_PANEL_W: f32 = 454.0;
pub const TRANSCRIPT_EDITOR_RULES_PANEL_MAX_H: f32 = 640.0;
pub const TRANSCRIPT_EDITOR_MIN_PANEL_H: f32 = 320.0;
pub const TRANSCRIPT_EDITOR_PANEL_BORDER: f32 = 1.0;
pub const TRANSCRIPT_EDITOR_TRIGGER_GAP: f32 = 4.0;
pub const TRANSCRIPT_EDITOR_WINDOW_MARGIN: f32 = 12.0;
pub const TRANSCRIPT_EDITOR_HEADER_PAD_TOP: f32 = 14.0;
pub const TRANSCRIPT_EDITOR_HEADER_PAD_BOTTOM: f32 = 10.0;
pub const TRANSCRIPT_EDITOR_PAD_X: f32 = 16.0;
pub const TRANSCRIPT_EDITOR_BODY_PAD_Y: f32 = 16.0;
pub const TRANSCRIPT_EDITOR_TABS_PAD_X: f32 = 14.0;
pub const TRANSCRIPT_EDITOR_SECTION_GAP: f32 = 16.0;
pub const TRANSCRIPT_EDITOR_BAND_PAD_Y: f32 = 12.0;
pub const TRANSCRIPT_EDITOR_FOOTER_PAD_X: f32 = 14.0;
pub const TRANSCRIPT_EDITOR_FOOTER_PAD_Y: f32 = 10.0;
pub const TRANSCRIPT_EDITOR_FOOTER_MIN_H: f32 = 48.0;
pub const TRANSCRIPT_EDITOR_BODY_RATIO: f32 = 12.0 / 13.0;
pub const TRANSCRIPT_EDITOR_AUX_RATIO: f32 = 11.0 / 13.0;
pub const TRANSCRIPT_EDITOR_AUX_ICON: f32 = 13.0;
pub const TRANSCRIPT_EDITOR_LABEL_TRACK: f32 = 0.42;
pub const TRANSCRIPT_EDITOR_PHASE_TRACK: f32 = 0.29;
pub const TRANSCRIPT_EDITOR_PHASE_GUTTER: f32 = 8.0;
pub const TRANSCRIPT_EDITOR_ROW_MIN_H: f32 = 34.0;
pub const TRANSCRIPT_EDITOR_CELL_MIN_H: f32 = 28.0;
pub const TRANSCRIPT_EDITOR_CELL_PAD_X: f32 = 8.0;
pub const TRANSCRIPT_EDITOR_PRESET_MIN_H: f32 = 28.0;
pub const TRANSCRIPT_EDITOR_CHOICE_MIN_H: f32 = 32.0;
pub const TRANSCRIPT_EDITOR_CHOICE_MIN_W: f32 = 52.0;
pub const TRANSCRIPT_EDITOR_CHOICE_GAP: f32 = 4.0;
pub const TRANSCRIPT_EDITOR_PARENT_ROW_MIN_H: f32 = 32.0;
pub const TRANSCRIPT_EDITOR_CHILD_ROW_MIN_H: f32 = 28.0;
pub const TRANSCRIPT_EDITOR_NEST_INDENT: f32 = 20.0;

// ── Right dock panels ───────────────────────────────────────────────────────

pub const RIGHT_PANEL_PAD_X: f32 = PAD_LG;
pub const RIGHT_PANEL_PAD_Y: f32 = PAD_STANDARD;
pub const RIGHT_PANEL_ROW_GAP: f32 = GAP_LG;
pub const RIGHT_PANEL_SECTION_GAP: f32 = GAP_LG;
pub const RIGHT_PANEL_BODY_FONT_SIZE: f32 = FONT_SIZE_MD;
pub const RIGHT_PANEL_LABEL_FONT_SIZE: f32 = FONT_SIZE_XS;
pub const RIGHT_PANEL_TASK_INDICATOR_W: f32 = 14.0;
pub const RIGHT_PANEL_TASK_DOT_SIZE_PX: f32 = 8.0;
pub const RIGHT_PANEL_TASK_PULSE_PERIOD_SEC: f32 = 1.5;
pub const RIGHT_PANEL_TASK_PULSE_MIN_ALPHA: f32 = 0.35;
pub const RIGHT_PANEL_TASK_PULSE_MAX_ALPHA: f32 = 1.0;
pub const RIGHT_PANEL_TASK_DURATION_FONT_SIZE: f32 = FONT_SIZE_SM;
pub const RIGHT_PANEL_TASK_FAILURE_FONT_SIZE: f32 = FONT_SIZE_XS;
pub const RIGHT_PANEL_TASK_SESSION_GAP: f32 = GAP_SM;
pub const RIGHT_PANEL_STATUS_PILL_PADDING_X_PX: f32 = PAD_STANDARD;
pub const RIGHT_PANEL_STATUS_PILL_RADIUS_PX: f32 = RADIUS_SM;
pub const RIGHT_PANEL_STATUS_PILL_BG_ALPHA: f32 = 0.12;
pub const STATUS_PILL_PAD_Y: f32 = PAD_XS;
pub const STATUS_PILL_DOT_SIZE: f32 = 8.0;
pub const STATUS_PILL_GAP: f32 = GAP_STANDARD;
pub const TASK_EDIT_HEADER_H: f32 = 40.0;
pub const TASK_EDIT_FOOTER_H: f32 = 48.0;
pub const TASK_EDIT_MAX_WIDTH: f32 = 800.0;
pub const TASK_EDIT_PREVIEW_MIN_H: f32 = 240.0;
pub const TASK_EDIT_PROMPT_ROWS: usize = 10;
pub const TASK_EDIT_NOTES_ROWS: usize = 4;
pub const LIST_ROW_RADIUS: f32 = RADIUS_SM;
pub const LIST_ROW_PAD_X: f32 = PAD_STANDARD;
pub const LIST_ROW_PAD_Y: f32 = PAD_XS;
pub const LIST_ROW_GAP: f32 = GAP_SM;
pub const FONT_FAMILY_MONOSPACE: &str = "Menlo";

// Usage tab, modelled on the Übersicht `claude-usage` widget.
pub const USAGE_SECTION_ICON_SIZE: f32 = 16.0;
pub const USAGE_HEADER_GAP: f32 = GAP_LG;
pub const USAGE_TITLE_FONT_SIZE: f32 = FONT_SIZE_MD;
pub const USAGE_PLAN_BADGE_FONT_SIZE: f32 = FONT_SIZE_XS;
pub const USAGE_PLAN_BADGE_PAD_X: f32 = PAD_SM;
pub const USAGE_PLAN_BADGE_PAD_Y: f32 = GAP_XS;
pub const USAGE_PLAN_BADGE_RADIUS: f32 = RADIUS_LG;
pub const USAGE_ACCENT_CHIP_BG: Hsla = ACCENT;
pub const USAGE_ACCENT_CHIP_FG: Hsla = ACCENT_FG;
pub const USAGE_BLOCK_GAP: f32 = 24.0;
pub const USAGE_CARD_GAP: f32 = USAGE_BLOCK_GAP;
pub const USAGE_GAUGE_PERCENT_FONT_SIZE: f32 = 18.0;
pub const GAUGE_BAR_HEIGHT: f32 = 8.0;
pub const GAUGE_BAR_RADIUS: f32 = RADIUS_SM;
pub const USAGE_CHART_BAR_MAX_HEIGHT: f32 = 40.0;
pub const USAGE_CHART_BAR_MIN_HEIGHT: f32 = 3.0;
pub const USAGE_CHART_BAR_GAP: f32 = GAP_SM;
pub const USAGE_CHART_BAR_RADIUS: f32 = RADIUS_SM;
pub const USAGE_CHART_LABEL_GAP: f32 = GAP_XS;
pub const USAGE_CHART_LABEL_FONT_SIZE: f32 = FONT_SIZE_XS;
pub const USAGE_CHART_BAR_TODAY: Hsla = ACCENT;
pub const USAGE_CHART_BAR_OTHER: Hsla = ACCENT_MUTED;

// Skills and MCP tabs.
pub const SKILL_HEADER_GAP: f32 = GAP_STANDARD;
pub const SKILL_PLUGIN_INDENT: f32 = 24.0;
pub const SKILL_PLUGIN_GROUP_PAD_Y: f32 = PAD_SM;
pub const SKILL_BADGE_FONT_SIZE: f32 = FONT_SIZE_XS;
pub const SKILL_BADGE_PAD_X: f32 = PAD_XS;
pub const SKILL_BADGE_PAD_Y: f32 = 0.0;
pub const SKILL_BADGE_RADIUS: f32 = RADIUS_XS;
pub const SKILL_AUX_CHIP_BG: Hsla = hsla(0.0, 0.0, 0.20, 0.85);
pub const MCP_HEADER_GAP: f32 = GAP_STANDARD;
pub const MCP_BADGE_RADIUS: f32 = RADIUS_XS;
pub const MCP_INDICATOR_MALFORMED: Hsla = hsla(14.0, 0.70, 0.55, 1.0);
pub const MCP_MALFORMED_BADGE_BG: Hsla = hsla(14.0, 0.50, 0.40, 0.30);
pub const MCP_MALFORMED_BADGE_TEXT: Hsla = with_lightness(MCP_INDICATOR_MALFORMED, 0.85);

// ── Flow graph pane ─────────────────────────────────────────────────────────
// The canvas clips a node to its declared size and scales the box but not its
// text, so a card drops rows below the density widths (screen px after zoom)
// instead of clipping. Framing stops at the zoom where an id no longer fits.

pub const FLOW_GRAPH_NODE_W: f32 = 250.0;
pub const FLOW_GRAPH_NODE_H: f32 = 112.0;
pub const FLOW_GRAPH_CARD_PAD: f32 = PAD_LG;
pub const FLOW_GRAPH_CARD_RADIUS: f32 = RADIUS_MD;
pub const FLOW_GRAPH_CARD_ROW_GAP: f32 = PAD_XS;
pub const FLOW_GRAPH_ID_FONT_SIZE: f32 = FONT_SIZE_LG;
pub const FLOW_GRAPH_META_FONT_SIZE: f32 = FONT_SIZE_SM;
pub const FLOW_GRAPH_CHIP_FONT_SIZE: f32 = FONT_SIZE_XS;
pub const FLOW_GRAPH_CHIP_PAD_X: f32 = PAD_SM;
pub const FLOW_GRAPH_CHIP_RADIUS: f32 = RADIUS_SM;
pub const FLOW_GRAPH_POLICY_GLYPH_SIZE: f32 = FONT_SIZE_XS;
pub const FLOW_GRAPH_POLICY_GLYPH_GAP: f32 = 2.0;
pub const FLOW_GRAPH_DENSITY_FULL_W: f32 = 200.0;
pub const FLOW_GRAPH_DENSITY_COMPACT_W: f32 = 110.0;
pub const FLOW_GRAPH_MARKER_FONT_SIZE: f32 = FONT_SIZE_XS;
pub const FLOW_GRAPH_KIND_DOT: f32 = 6.0;
pub const FLOW_GRAPH_FRAME_MARGIN: f32 = 0.08;
pub const FLOW_GRAPH_FRAME_MIN_ZOOM: f32 = 0.2;
pub const FLOW_INSPECTOR_W: f32 = 280.0;
pub const FLOW_INSPECTOR_GAP: f32 = PAD_LG;
pub const FLOW_INSPECTOR_PAD: f32 = PAD_LG;
pub const FLOW_INSPECTOR_PROMPT_ROWS: usize = 5;
pub const FLOW_TOOLBAR_INSET: f32 = PAD_LG;
pub const FLOW_TOOLBAR_GAP: f32 = GAP_SM;

// Run status colours. PINNED (nothing happened) and ISSUE (not runnable at
// all, which can coexist with the last run's green) get their own hues so
// they never share one with a run outcome.
pub const FLOW_GRAPH_BACKGROUND: Hsla = CANVAS;
pub const FLOW_GRAPH_EDGE: Hsla = hsla(218.0, 0.089, 0.35, 1.0);
pub const FLOW_GRAPH_STATUS_PENDING: Hsla = HAIRLINE;
pub const FLOW_GRAPH_STATUS_RUNNING: Hsla = ACCENT;
pub const FLOW_GRAPH_STATUS_PASSED: Hsla = SUCCESS;
pub const FLOW_GRAPH_STATUS_RETRIED: Hsla = WARNING;
pub const FLOW_GRAPH_STATUS_FAILED: Hsla = ERROR;
pub const FLOW_GRAPH_STATUS_PINNED: Hsla = hsla(186.0, 0.50, 0.52, 1.0);
pub const FLOW_GRAPH_ISSUE: Hsla = hsla(280.0, 0.45, 0.62, 1.0);

// Terminal canvas default.
pub const TERMINAL_BG: Hsla = hsla(0.0, 0.0, 0.0, 1.0);
