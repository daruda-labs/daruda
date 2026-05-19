//! App-side UI palette — workspace chrome, docks, status bar, docks.
//!
//! Lives in the `app` crate (not `daruda_terminal::ux::theme`) because
//! it describes colours / metrics for daruda-bespoke chrome widgets:
//! tab bar, dock view tabs, worktrees list, status bar, dock
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

// ============================================================================
// Workspace chrome — title bar, tab bar, status bar, docks
// ============================================================================

/// Title bar background.
pub const TITLE_BAR_BG: Hsla = hsla(0.0, 0.0, 0.10, 1.0);

/// Tab bar background.
pub const TAB_BAR_BG: Hsla = hsla(0.0, 0.0, 0.12, 1.0);

/// Tab bar bottom hairline. Matches dock `gpui_component::TabBar`'s
/// `cx.theme().border` (which `apply_daruda_palette` binds to
/// `MODAL_PANEL_BORDER`, lightness 0.30) so the terminal tab bar and
/// the dock tab strip share the same separator color.
pub const TAB_BAR_BORDER: Hsla = hsla(0.0, 0.0, 0.30, 1.0);

/// Active tab background.
pub const TAB_ACTIVE_BG: Hsla = hsla(0.0, 0.0, 0.235, 1.0);

/// Active tab and highlighted-element text color (white).
pub const TAB_ACTIVE_TEXT: Hsla = hsla(0.0, 0.0, 1.0, 1.0);

/// Focused pane header text color (white).
pub const PANE_HEADER_FOCUSED_TEXT: Hsla = hsla(0.0, 0.0, 1.0, 1.0);

/// Inactive tab background.
pub const TAB_INACTIVE_BG: Hsla = hsla(0.0, 0.0, 0.165, 1.0);

/// Inactive tab hover background.
pub const TAB_INACTIVE_HOVER_BG: Hsla = hsla(0.0, 0.0, 0.20, 1.0);

/// Inactive tab text color.
pub const TAB_INACTIVE_TEXT: Hsla = hsla(0.0, 0.0, 0.65, 1.0);

/// Status bar background.
pub const STATUS_BAR_BG: Hsla = hsla(0.0, 0.0, 0.10, 1.0);

/// Status bar top border.
pub const STATUS_BAR_BORDER: Hsla = hsla(0.0, 0.0, 0.20, 1.0);

/// Status bar error text color (used by transient failure messages).
pub const STATUS_BAR_ERROR: Hsla = hsla(0.0, 0.7, 0.55, 1.0);

/// Small dot in the right section of the status bar that lights up
/// when the workspace's project layer (`<config_dir>/daruda/projects/...`)
/// has a `config.toml` on disk. Cyan-ish accent so it reads as
/// informational, not as an alert.
pub const STATUS_BAR_PROJECT_DOT: Hsla = hsla(180.0, 0.55, 0.55, 1.0);

/// Inline "detached" chip background and text — shown next to the
/// project/branch label when the active git worktree is on a detached
/// HEAD. Amber-leaning so it signals *attention* without rising to
/// the red-error tier; the state is uncommon but not destructive.
pub const STATUS_BAR_DETACHED_BG: Hsla = hsla(35.0, 0.45, 0.22, 1.0);
pub const STATUS_BAR_DETACHED_TEXT: Hsla = hsla(35.0, 0.85, 0.72, 1.0);

/// Dock panel background.
pub const DOCK_BG: Hsla = hsla(0.0, 0.0, 0.12, 1.0);

/// Dock border (separates dock from center content).
pub const DOCK_BORDER: Hsla = hsla(0.0, 0.0, 0.20, 1.0);

/// Dock header text color.
pub const DOCK_HEADER_TEXT: Hsla = hsla(0.0, 0.0, 0.55, 1.0);

/// Dock placeholder text color (empty panel message).
pub const DOCK_PLACEHOLDER_TEXT: Hsla = hsla(0.0, 0.0, 0.35, 1.0);

/// Macro / widget button background (text & icon modes share this).
pub const BUTTON_WIDGET_BG: Hsla = hsla(0.0, 0.0, 0.20, 1.0);

/// Macro / widget button background on hover.
pub const BUTTON_WIDGET_BG_HOVER: Hsla = hsla(0.0, 0.0, 0.28, 1.0);

/// Macro / widget button label / icon color.
pub const BUTTON_WIDGET_TEXT: Hsla = hsla(0.0, 0.0, 0.85, 1.0);

/// Background of a panel tab while a draggable tab hovers over it
/// (drop target highlight). Same hue as button-widget hover for
/// visual consistency.
pub const PANEL_TAB_DROP_TARGET_BG: Hsla = hsla(0.0, 0.0, 0.28, 1.0);

/// Muted text (status bar labels, close buttons, dim UI).
pub const MUTED_TEXT: Hsla = hsla(0.0, 0.0, 0.55, 1.0);

/// Faint text (secondary info like cwd in status bar).
pub const FAINT_TEXT: Hsla = hsla(0.0, 0.0, 0.45, 1.0);

/// Background for disabled interactive items (buttons, menu rows).
pub const DISABLED_ITEM_BG: Hsla = hsla(0.0, 0.0, 0.30, 1.0);
/// Text color for disabled interactive items.
pub const DISABLED_ITEM_TEXT: Hsla = hsla(0.0, 0.0, 0.45, 1.0);

/// Tab/pane close button hover background (destructive red).
pub const CLOSE_BUTTON_HOVER_BG: Hsla = hsla(0.0, 0.55, 0.55, 1.0);

/// Accent green — shared base for action labels, active-state borders,
/// and the derived constants below (`TOAST_ACTION_TEXT`,
/// `KEYSTROKE_INPUT_BORDER_ACTIVE`, `RIGHT_PANEL_TASK_RUNNING_COLOR`).
pub const ACCENT_GREEN: Hsla = hsla(135.0, 0.55, 0.55, 1.0);

/// Dock toggle icon inactive color.
pub const DOCK_ICON_INACTIVE: Hsla = hsla(0.0, 0.0, 0.45, 1.0);

/// Dock toggle icon inactive hover color.
pub const DOCK_ICON_HOVER: Hsla = hsla(0.0, 0.0, 0.70, 1.0);

/// Dock toggle icon active background.
pub const DOCK_ICON_ACTIVE_BG: Hsla = hsla(0.0, 0.0, 0.235, 1.0);

/// Dock view tab strip — inactive tab text color.
pub const DOCK_VIEW_TAB_INACTIVE: Hsla = hsla(0.0, 0.0, 0.50, 1.0);

/// Dock view tab strip — active tab text color.
pub const DOCK_VIEW_TAB_ACTIVE: Hsla = hsla(0.0, 0.0, 0.95, 1.0);

/// Dock view tab strip — active underline accent.
pub const DOCK_VIEW_TAB_ACCENT: Hsla = hsla(210.0, 0.70, 0.60, 1.0);

/// Dock view tab strip — hover background.
pub const DOCK_VIEW_TAB_HOVER_BG: Hsla = hsla(0.0, 0.0, 0.18, 1.0);

/// Dock view tab strip — horizontal padding per tab (px).
pub const DOCK_VIEW_TAB_PAD_X: f32 = 10.0;

/// Dock view tab strip — font size (px).
pub const DOCK_VIEW_TAB_FONT_SIZE: f32 = 11.0;

/// Dock view tab strip — active underline thickness (px).
pub const DOCK_VIEW_TAB_ACCENT_H: f32 = 2.0;

// Worktrees list (left dock Worktrees view)
/// Worktrees list — row height (px). Compact preset (Premium Card redesign).
pub const WORKTREE_ROW_HEIGHT: f32 = 30.0;
/// Worktrees list — horizontal padding (px).
pub const WORKTREE_ROW_PAD_X: f32 = 10.0;
/// Worktrees list — unread marker tint (warning).
pub const WORKTREE_UNREAD: Hsla = hsla(30.0, 0.80, 0.60, 1.0);
/// Worktrees list — inactive row hover background.
pub const WORKTREE_ROW_HOVER_BG: Hsla = hsla(0.0, 0.0, 0.18, 1.0);
/// Worktrees list — primary label font size (px).
pub const WORKTREE_LABEL_FONT_SIZE: f32 = 12.0;
/// Worktrees list — secondary (path / status) font size (px).
pub const WORKTREE_SUB_FONT_SIZE: f32 = 10.0;
/// Worktrees list — section header font size (px).
pub const WORKTREE_SECTION_HEADER_FONT_SIZE: f32 = 10.5;
/// Worktrees list — section header top/bottom padding (px).
pub const WORKTREE_SECTION_PAD_Y: f32 = 6.0;
/// Worktrees list — placeholder (non-git info box) padding (px).
pub const WORKTREE_PLACEHOLDER_PAD: f32 = 12.0;
/// Context menu max width (px) — wider items get truncated by overflow_hidden.
pub const CTX_MENU_MAX_WIDTH: f32 = 200.0;
/// Drag ghost row vertical padding (px) — space above/below label in the
/// floating preview that follows the cursor during a worktree drag.
pub const WORKTREE_DRAG_GHOST_PAD_Y: f32 = 4.0;
/// Highlight color applied to a drop target row while a worktree is being
/// dragged over it.
pub const WORKTREE_DROP_TARGET_BG: Hsla = hsla(210.0, 0.50, 0.30, 0.35);
/// Rejection tint applied when the in-flight payload cannot land on
/// the hovered row (cross-project worktree drag, group dropped on a
/// grouped project, etc.). Desaturated red at low alpha so it reads
/// as "not here" rather than as a hard error.
pub const WORKTREE_DROP_TARGET_REJECTED_BG: Hsla = hsla(0.0, 0.55, 0.32, 0.20);
/// Unread indicator dot diameter (px).
pub const WORKTREE_UNREAD_DOT_SIZE: f32 = 6.0;
/// Unread indicator dot corner radius (px).
pub const WORKTREE_UNREAD_DOT_RADIUS: f32 = 3.0;
/// Gap between label elements within the primary label row (px).
pub const WORKTREE_LABEL_GAP: f32 = 6.0;
/// Gap between elements in the sub-label row (px).
pub const WORKTREE_SUBLABEL_GAP: f32 = 6.0;
/// Gap between the body and the × remove button within a row (px).
pub const WORKTREE_ROW_GAP: f32 = 8.0;
/// Top margin of the "git init" affordance inside the non-git placeholder (px).
pub const WORKTREE_PLACEHOLDER_GIT_INIT_MT: f32 = 4.0;
/// Gap between lines inside the non-git info placeholder (px).
pub const WORKTREE_PLACEHOLDER_LINE_GAP: f32 = 6.0;
/// Diameter of the optional color dot rendered in a group header (px).
pub const WORKTREE_GROUP_COLOR_DOT_SIZE: f32 = 8.0;
/// Corner radius of the group color dot (px). Half the size to render a circle.
pub const WORKTREE_GROUP_COLOR_DOT_RADIUS: f32 = 4.0;

// ----------------------------------------------------------------------------
// Premium Card surface tokens (Worktrees redesign)
// ----------------------------------------------------------------------------

/// Worktrees card — outer corner radius (px).
pub const WORKTREE_CARD_RADIUS: f32 = 10.0;
/// Worktrees card — vertical gap between adjacent cards (px).
pub const WORKTREE_CARD_GAP: f32 = 6.0;
/// Worktrees card — inner horizontal padding (px).
pub const WORKTREE_CARD_PAD_X: f32 = 10.0;
/// Worktrees card — inner vertical padding (px).
pub const WORKTREE_CARD_PAD_Y: f32 = 6.0;
/// Worktrees row — corner radius applied to hover/active background fills
/// so the highlight reads as a rounded chip instead of a hard rectangle.
pub const WORKTREE_ROW_RADIUS: f32 = 6.0;
/// Worktrees card — horizontal outer margin so cards don't hug the dock
/// edges; gives the surface visible left/right breathing room.
pub const WORKTREE_CARD_MARGIN_X: f32 = 8.0;
/// Worktrees list — vertical gap between adjacent worktree rows inside
/// a project block so consecutive rows don't read as a single block.
pub const WORKTREE_LIST_GAP_Y: f32 = 3.0;
/// Worktrees card — border width (px).
pub const WORKTREE_CARD_BORDER_W: f32 = 1.0;
/// Worktrees card — base background. Sits a step above `DOCK_BG` (#0a)
/// so the card edge is visible without leaning on the border alone.
pub const WORKTREE_CARD_BG: Hsla = hsla(0.0, 0.0, 0.13, 1.0);
/// Worktrees card — border at 8% alpha. Higher than the original 4%
/// so the card outline reads on the dark dock surface.
pub const WORKTREE_CARD_BORDER: Hsla = hsla(0.0, 0.0, 1.0, 0.08);
/// Worktrees card — hover background (one tonal step above the base).
pub const WORKTREE_CARD_HOVER_BG: Hsla = hsla(0.0, 0.0, 0.16, 1.0);
/// Worktrees active row — subtle bg only (no left bar / no glow).
pub const WORKTREE_ROW_ACTIVE_BG: Hsla = hsla(0.0, 0.0, 1.0, 0.08);
/// Group label font size (px) — uppercase eyebrow.
pub const WORKTREE_GROUP_LABEL_FONT_SIZE: f32 = 11.0;

// ============================================================================
// Migrated from daruda_terminal::ux::theme (Phase 1 follow-up)
// ============================================================================

/// Modal backdrop dim alpha (0..1).
pub const MODAL_BACKDROP_ALPHA: f32 = 0.50;
/// Modal panel background.
pub const MODAL_PANEL_BG: Hsla = hsla(0.0, 0.0, 0.14, 1.0);
/// Modal panel border.
pub const MODAL_PANEL_BORDER: Hsla = hsla(0.0, 0.0, 0.30, 1.0);
/// Modal panel corner radius (px).
pub const MODAL_PANEL_RADIUS: f32 = 8.0;
/// Modal panel width (px).
pub const MODAL_PANEL_WIDTH: f32 = 420.0;
/// Modal panel inner padding (px).
pub const MODAL_PANEL_PAD: f32 = 16.0;
/// Modal panel top offset from window top (px).
pub const MODAL_TOP_OFFSET: f32 = 140.0;
/// Modal title font size (px).
pub const MODAL_TITLE_FONT_SIZE: f32 = 14.0;
/// Modal body font size (px).
pub const MODAL_BODY_FONT_SIZE: f32 = 12.0;
/// Modal input background.
pub const MODAL_INPUT_BG: Hsla = hsla(0.0, 0.0, 0.08, 1.0);
/// Modal input border.
pub const MODAL_INPUT_BORDER: Hsla = hsla(0.0, 0.0, 0.28, 1.0);
/// Border color shown around any focused text input — daruda `TextInput`
/// and the `gpui_component::input::Input` we use for the markdown
/// prompt / notes editors. Wiring both through a single constant keeps
/// the focus highlight consistent across surfaces; `ui/theme.rs` also
/// maps `gpui_component::Theme::ring` to this so any other Sizable
/// widget (Select, DatePicker, …) inherits the same focus chrome.
pub const INPUT_FOCUS_BORDER: Hsla = hsla(0.0, 0.0, 0.95, 1.0);
/// Modal input padding (px).
pub const MODAL_INPUT_PAD: f32 = 8.0;
/// Modal error text color.
pub const MODAL_ERROR_TEXT: Hsla = hsla(0.0, 0.70, 0.65, 1.0);
/// Modal primary button background.
pub const MODAL_PRIMARY_BG: Hsla = hsla(210.0, 0.70, 0.55, 1.0);
/// Modal primary button hover background.
pub const MODAL_PRIMARY_HOVER_BG: Hsla = hsla(210.0, 0.70, 0.65, 1.0);
/// Modal secondary (cancel) button text.
pub const MODAL_SECONDARY_TEXT: Hsla = hsla(0.0, 0.0, 0.75, 1.0);
/// Modal button padding X (px).
pub const MODAL_BUTTON_PAD_X: f32 = 14.0;
/// Modal button padding Y (px).
pub const MODAL_BUTTON_PAD_Y: f32 = 6.0;
/// Modal button corner radius (px).
pub const MODAL_BUTTON_RADIUS: f32 = 4.0;
/// Primary text color inside modals + the text-input caret. White
/// against the dark `MODAL_PANEL_BG` / `MODAL_INPUT_BG`.
pub const MODAL_TEXT_PRIMARY: Hsla = hsla(0.0, 0.0, 1.0, 1.0);
/// Vertical gap between the rows inside a modal panel (title / body /
/// input / error / footer).
pub const MODAL_PANEL_GAP: f32 = 10.0;
/// Gap between the buttons in a modal footer (Cancel | Confirm).
pub const MODAL_FOOTER_GAP: f32 = 8.0;
/// Top margin of the footer row inside the panel.
pub const MODAL_FOOTER_MARGIN_TOP: f32 = 6.0;
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
/// Selection highlight background in text inputs (semi-transparent blue).
pub const INPUT_SELECTION_BG: Hsla = hsla(210.0, 0.70, 0.55, 0.35);
/// Width of the TextArea scrollbar thumb (px).
pub const TEXTAREA_SCROLLBAR_W: f32 = 4.0;
/// Gap between thumb and the right edge (px).
pub const TEXTAREA_SCROLLBAR_PAD: f32 = 2.0;
/// Minimum scrollbar thumb height so it stays clickable (px).
pub const TEXTAREA_SCROLLBAR_MIN_H: f32 = 20.0;
/// Scrollbar thumb color.
pub const TEXTAREA_SCROLLBAR_THUMB: Hsla = hsla(0.0, 0.0, 0.70, 0.40);
/// Side length of the square modal checkbox (px).
pub const MODAL_CHECKBOX_SIZE: f32 = 14.0;
/// Corner radius of the modal checkbox (px).
pub const MODAL_CHECKBOX_RADIUS: f32 = 3.0;
/// Font size of the ✓ checkmark inside the checkbox (px).
pub const MODAL_CHECKBOX_TICK_SIZE: f32 = 10.0;
/// Banner background — error severity. Red hue, low alpha so the
/// underlying `MODAL_PANEL_BG` shows through.
pub const BANNER_ERROR_BG: Hsla = hsla(0.0, 0.70, 0.55, 0.10);
/// Banner text + icon color — error severity.
pub const BANNER_ERROR_TEXT: Hsla = hsla(0.0, 0.70, 0.70, 1.0);
/// Banner background — warning severity. Amber hue.
pub const BANNER_WARNING_BG: Hsla = hsla(40.0, 0.80, 0.55, 0.10);
/// Banner text + icon color — warning severity.
pub const BANNER_WARNING_TEXT: Hsla = hsla(40.0, 0.80, 0.70, 1.0);
/// Banner background — info severity. Blue hue, matches the modal
/// primary-button family.
pub const BANNER_INFO_BG: Hsla = hsla(210.0, 0.70, 0.55, 0.10);
/// Banner text + icon color — info severity.
pub const BANNER_INFO_TEXT: Hsla = hsla(210.0, 0.70, 0.75, 1.0);
/// Banner background — success severity. Green hue.
pub const BANNER_SUCCESS_BG: Hsla = hsla(140.0, 0.55, 0.45, 0.10);
/// Banner text + icon color — success severity.
pub const BANNER_SUCCESS_TEXT: Hsla = hsla(140.0, 0.55, 0.65, 1.0);
/// Horizontal padding inside a banner (px).
pub const BANNER_PAD_X: f32 = 10.0;
/// Vertical padding inside a banner (px).
pub const BANNER_PAD_Y: f32 = 6.0;
/// Banner corner radius (px).
pub const BANNER_RADIUS: f32 = 6.0;
/// Gap between the icon glyph and the message text (px).
pub const BANNER_GAP: f32 = 6.0;
/// Fixed width of the label column in settings form rows (px).
pub const SETTINGS_LABEL_W: f32 = 120.0;
/// Settings window scrollbar thumb width (px).
pub const SETTINGS_SCROLLBAR_W: f32 = 4.0;
/// Settings window scrollbar right margin (px).
pub const SETTINGS_SCROLLBAR_MARGIN_R: f32 = 2.0;
/// Settings window scrollbar minimum thumb height (px).
pub const SETTINGS_SCROLLBAR_MIN_THUMB_H: f32 = 24.0;
/// Settings window scrollbar thumb color.
pub const SETTINGS_SCROLLBAR_THUMB: Hsla = hsla(0.0, 0.0, 1.0, 0.25);
/// Settings window scrollbar thumb hover color.
pub const SETTINGS_SCROLLBAR_THUMB_HOVER: Hsla = hsla(0.0, 0.0, 1.0, 0.45);
/// Right-dock panel scrollbar thumb width (px).
pub const RIGHT_PANEL_SCROLLBAR_W: f32 = 4.0;
/// Right-dock panel scrollbar right margin (px).
pub const RIGHT_PANEL_SCROLLBAR_MARGIN_R: f32 = 2.0;
/// Right-dock panel scrollbar minimum thumb height (px).
pub const RIGHT_PANEL_SCROLLBAR_MIN_THUMB_H: f32 = 24.0;
/// Right-dock panel scrollbar thumb color.
pub const RIGHT_PANEL_SCROLLBAR_THUMB: Hsla = hsla(0.0, 0.0, 1.0, 0.25);
/// Right-dock panel scrollbar thumb hover color.
pub const RIGHT_PANEL_SCROLLBAR_THUMB_HOVER: Hsla = hsla(0.0, 0.0, 1.0, 0.45);
/// Width of the section-nav sidebar (px). Sized to fit `Claude Status`
/// (the longest builtin nav label) at the body font size with comfort.
pub const SETTINGS_SIDEBAR_W: f32 = 168.0;
/// Dock background — slightly darker than the panel body so the
/// active row's highlight reads cleanly.
pub const SETTINGS_SIDEBAR_BG: Hsla = hsla(0.0, 0.0, 0.0, 0.18);
/// Vertical padding inside the left dock list.
pub const SETTINGS_SIDEBAR_PAD_Y: f32 = 6.0;
/// Per-row horizontal padding inside the left dock.
pub const SETTINGS_SIDEBAR_ROW_PAD_X: f32 = 14.0;
/// Per-row vertical padding inside the left dock.
pub const SETTINGS_SIDEBAR_ROW_PAD_Y: f32 = 6.0;
/// Active row background — same family as MODAL_PANEL_BG accent.
pub const SETTINGS_SIDEBAR_ROW_ACTIVE_BG: Hsla = hsla(0.0, 0.0, 1.0, 0.10);
/// Hover-state row background.
pub const SETTINGS_SIDEBAR_ROW_HOVER_BG: Hsla = hsla(0.0, 0.0, 1.0, 0.06);
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
/// Command palette panel background.
pub const PALETTE_BG: Hsla = hsla(0.0, 0.0, 0.145, 1.0);
/// Command palette border.
pub const PALETTE_BORDER: Hsla = hsla(0.0, 0.0, 0.33, 1.0);
/// Command palette input separator border.
pub const PALETTE_INPUT_BORDER: Hsla = hsla(0.0, 0.0, 0.27, 1.0);
/// Command palette focused entry background.
pub const PALETTE_FOCUSED_BG: Hsla = hsla(0.0, 0.0, 0.235, 1.0);
/// Command palette normal entry text.
pub const PALETTE_ENTRY_TEXT: Hsla = hsla(0.0, 0.0, 0.75, 1.0);
/// Command palette shortcut hint text.
pub const PALETTE_SHORTCUT_TEXT: Hsla = hsla(0.0, 0.0, 0.45, 1.0);
/// Command palette "no matching" text.
pub const PALETTE_EMPTY_TEXT: Hsla = hsla(0.0, 0.0, 0.40, 1.0);
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
pub const GIT_FILE_ROW_PAD_X: f32 = 10.0;
/// Gap between status char and file name within a row (px).
pub const GIT_FILE_ROW_GAP: f32 = 6.0;
/// Status char column width (px).
pub const GIT_STATUS_CHAR_W: f32 = 14.0;
/// Git status badge font size shown in the Worktrees list (px).
pub const GIT_BADGE_FONT_SIZE: f32 = 10.0;
/// Git status badge text color (muted gray).
pub const GIT_BADGE_TEXT: Hsla = hsla(0.0, 0.0, 0.50, 1.0);
/// Staged-file status char color (green).
pub const GIT_STAGED_COLOR: Hsla = hsla(135.0, 0.55, 0.55, 1.0);
/// Unstaged-modified status char color (amber).
pub const GIT_UNSTAGED_COLOR: Hsla = hsla(45.0, 0.75, 0.60, 1.0);
/// Untracked ("??") status char color (muted green).
pub const GIT_UNTRACKED_COLOR: Hsla = hsla(135.0, 0.35, 0.50, 1.0);
/// Selected file-row background.
pub const GIT_FILE_ROW_SELECTED_BG: Hsla = hsla(0.0, 0.0, 0.22, 1.0);
/// Hovered (unselected) file-row background.
pub const GIT_FILE_ROW_HOVER_BG: Hsla = hsla(0.0, 0.0, 0.18, 1.0);
/// Diff panel background.
pub const GIT_DIFF_PANEL_BG: Hsla = hsla(0.0, 0.0, 0.09, 1.0);
/// Diff panel top border.
pub const GIT_DIFF_BORDER: Hsla = hsla(0.0, 0.0, 0.22, 1.0);
/// Diff added-line background (dark green).
pub const GIT_DIFF_ADD_BG: Hsla = hsla(135.0, 0.55, 0.12, 1.0);
/// Diff removed-line background (dark red).
pub const GIT_DIFF_DEL_BG: Hsla = hsla(0.0, 0.55, 0.12, 1.0);
/// Diff added-line text (light green).
pub const GIT_DIFF_ADD_TEXT: Hsla = hsla(135.0, 0.65, 0.70, 1.0);
/// Diff removed-line text (light red).
pub const GIT_DIFF_DEL_TEXT: Hsla = hsla(0.0, 0.65, 0.70, 1.0);
/// Diff context / header text (muted gray).
pub const GIT_DIFF_CONTEXT_TEXT: Hsla = hsla(0.0, 0.0, 0.45, 1.0);
/// Diff hunk-header line background (muted blue).
pub const GIT_DIFF_HUNK_BG: Hsla = hsla(210.0, 0.25, 0.18, 1.0);
/// Diff line font size (px) — compact monospace.
pub const GIT_DIFF_FONT_SIZE: f32 = 10.0;
/// Diff line horizontal padding (px).
pub const GIT_DIFF_LINE_PAD_X: f32 = 6.0;
/// Diff panel max visible height before truncation note (px).
pub const GIT_DIFF_MAX_HEIGHT: f32 = 320.0;
/// Maximum diff lines rendered (guards against huge diffs).
pub const GIT_DIFF_MAX_LINES: usize = 120;
/// Stage checkbox box size (px).
pub const GIT_STAGE_CHECKBOX_SIZE: f32 = 13.0;
/// Stage checkbox border radius (px).
pub const GIT_STAGE_CHECKBOX_RADIUS: f32 = 2.0;
/// Stage checkbox border color.
pub const GIT_STAGE_CHECKBOX_BORDER: Hsla = hsla(0.0, 0.0, 0.38, 1.0);
/// Stage checkbox background when staged (green tint).
pub const GIT_STAGE_CHECKBOX_CHECKED_BG: Hsla = hsla(135.0, 0.50, 0.30, 1.0);
/// Stage checkbox background when unstaged.
pub const GIT_STAGE_CHECKBOX_UNCHECKED_BG: Hsla = hsla(0.0, 0.0, 0.14, 1.0);
/// Tick glyph font size inside the stage checkbox (px).
pub const GIT_STAGE_CHECKBOX_TICK_SIZE: f32 = 9.0;
/// Git Changes header padding X (px).
pub const GIT_HEADER_PAD_X: f32 = 10.0;
/// Git Changes header padding Y (px).
pub const GIT_HEADER_PAD_Y: f32 = 6.0;
/// Directory group header vertical padding (px).
pub const GIT_DIR_HEADER_PAD_Y: f32 = 2.0;
/// Directory group header font size (px).
pub const GIT_DIR_HEADER_FONT_SIZE: f32 = 10.0;
/// Refresh icon size in the Git Changes header (px).
pub const GIT_REFRESH_ICON_SIZE: f32 = 16.0;
/// Commit footer top-border color.
pub const GIT_COMMIT_BORDER: Hsla = hsla(0.0, 0.0, 0.22, 1.0);
/// Commit footer inner padding (px).
pub const GIT_COMMIT_PAD: f32 = 8.0;
/// Gap between Commit and Push buttons (px).
pub const GIT_COMMIT_BUTTON_GAP: f32 = 6.0;
/// Total height of the commit footer panel (textarea + floating button bar).
/// Sized to show ~4-5 lines of commit message text.
pub const GIT_COMMIT_FOOTER_H: f32 = 128.0;
/// Commit message text area height (px) — kept for reference; layout uses GIT_COMMIT_FOOTER_H.
pub const GIT_COMMIT_INPUT_HEIGHT: f32 = 64.0;
/// Commit button horizontal padding (px).
pub const GIT_COMMIT_BTN_PAD_X: f32 = 8.0;
/// Commit button vertical padding (px).
pub const GIT_COMMIT_BTN_PAD_Y: f32 = 4.0;
/// Commit button corner radius (px).
pub const GIT_COMMIT_BTN_RADIUS: f32 = 4.0;
/// Dropdown arrow button horizontal padding (px).
pub const GIT_COMMIT_DROP_PAD_X: f32 = 5.0;
/// Gap between remote action buttons (Fetch / Push) (px).
pub const GIT_REMOTE_BTN_GAP: f32 = 4.0;
/// Gap between the text area and the action button group in InputPanel (px).
pub const INPUT_PANEL_SECTION_GAP: f32 = 6.0;
/// Gap between buttons in the InputPanel action group (px).
pub const INPUT_PANEL_BUTTON_GAP: f32 = 6.0;
/// Minimum height of the TextArea inside InputPanel (px).
pub const INPUT_PANEL_MIN_H: f32 = 48.0;
/// Height of the floating action bar overlaid at the bottom of an InputPanel
/// with `ActionsFloating` layout (px). Matches Zed git_panel footer_size.
pub const INPUT_PANEL_FLOATING_BAR_H: f32 = 32.0;
/// Command palette max visible entries.
pub const PALETTE_MAX_VISIBLE: usize = 12;
/// Command palette corner radius (px).
pub const PALETTE_RADIUS: f32 = 8.0;
/// Command palette query input text (bright white).
pub const PALETTE_QUERY_TEXT: Hsla = hsla(0.0, 0.0, 1.0, 1.0);
/// Command palette focused entry text (bright white).
pub const PALETTE_FOCUSED_TEXT: Hsla = hsla(0.0, 0.0, 1.0, 1.0);
/// Tab label font size (px).
pub const TAB_FONT_SIZE: f32 = 12.0;
/// Tab close button font size (px).
pub const TAB_CLOSE_FONT_SIZE: f32 = 11.0;
/// Tab minimum width (px).
pub const TAB_MIN_WIDTH: f32 = 80.0;
/// Tab maximum width (px).
pub const TAB_MAX_WIDTH: f32 = 220.0;
/// New-tab button font size (px).
pub const NEW_TAB_FONT_SIZE: f32 = 12.0;
/// Pane header font size (px).
pub const PANE_HEADER_FONT_SIZE: f32 = 11.0;
/// Pane header cwd basename font size (px).
pub const PANE_HEADER_CWD_FONT_SIZE: f32 = 10.0;
/// Pane header close button font size (px).
pub const PANE_HEADER_CLOSE_FONT_SIZE: f32 = 10.0;
/// Status bar font size (px).
pub const STATUS_BAR_FONT_SIZE: f32 = 11.0;
/// Status bar horizontal padding (px).
pub const STATUS_BAR_PAD_X: f32 = 10.0;
/// Dock panel header font size (px).
pub const DOCK_HEADER_FONT_SIZE: f32 = 11.0;
/// Dock panel header horizontal padding (px).
pub const DOCK_HEADER_PAD_X: f32 = 10.0;
/// Dock placeholder message font size (px).
pub const DOCK_PLACEHOLDER_FONT_SIZE: f32 = 12.0;
/// Dock toggle icon size (px).
pub const DOCK_ICON_SIZE: f32 = 13.0;
/// Dock toggle icon button width (px).
pub const DOCK_ICON_BUTTON_W: f32 = 24.0;
/// Dock toggle icon button height (px).
pub const DOCK_ICON_BUTTON_H: f32 = 20.0;
/// Dock toggle icon button corner radius (px).
pub const DOCK_ICON_BUTTON_RADIUS: f32 = 4.0;
/// Dock toggle icon group right margin (px).
pub const DOCK_ICON_GROUP_MR: f32 = 8.0;
/// Side length of a `button_chip` (px). Single-glyph chip buttons
/// (`+`, `1`, `2`, `3`) sized to a uniform square so adjacent chips
/// read as a row of equal-weight controls regardless of glyph width.
/// Sized to sit inside the 28-px tab bar with comfortable margin.
pub const BUTTON_CHIP_SIZE: f32 = 20.0;
/// Panel body horizontal padding (px).
pub const PANEL_BODY_PAD_X: f32 = 8.0;
/// Panel body vertical padding (px).
pub const PANEL_BODY_PAD_Y: f32 = 8.0;
/// Gap between widgets in flex_wrap layout (px).
pub const PANEL_BODY_GAP: f32 = 6.0;
/// Highlight overlay applied to the terminal input panel body while a file
/// (internal `PathDrag` or Finder `ExternalPaths`) is dragged over it.
pub const INPUT_PANEL_DROP_TARGET_BG: Hsla = hsla(210.0, 0.50, 0.30, 0.20);
/// Highlight overlay painted over a terminal pane while a file is dragged
/// over it. Slightly more opaque than the input panel variant so it remains
/// visible against the terminal background.
pub const TERMINAL_DROP_TARGET_BG: Hsla = hsla(210.0, 0.50, 0.30, 0.30);
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
pub const BUTTON_WIDGET_FONT_SIZE: f32 = 12.0;
/// Macro button horizontal padding for text mode (px).
pub const BUTTON_WIDGET_PAD_X: f32 = 10.0;
/// Macro button corner radius (px).
pub const BUTTON_WIDGET_RADIUS: f32 = 4.0;
/// Pane header height (px).
pub const PANE_HEADER_HEIGHT: f32 = 20.0;
/// Row height for file tree entries (px).
pub const FILES_ROW_HEIGHT: f32 = 22.0;
/// Outer horizontal padding for each row (px).
pub const FILES_ROW_PAD_X: f32 = 8.0;
/// Pixel offset added per directory depth level — visual indent.
pub const FILES_INDENT_PX: f32 = 14.0;
/// Width of the chevron column (px).
pub const FILES_CHEVRON_W: f32 = 14.0;
/// Width of the icon column to the right of the chevron (px).
pub const FILES_ICON_W: f32 = 16.0;
/// Gap between chevron / icon / name (px).
pub const FILES_ROW_GAP: f32 = 4.0;
/// Font size for the file tree row name (px).
pub const FILES_ROW_FONT_SIZE: f32 = 12.0;
/// Width of the dock scrollbar thumb (px). Shared by every left-dock
/// left-dock view (Files, Git Changes, Worktrees) so the scrollbars feel
/// consistent.
pub const DOCK_SCROLLBAR_W: f32 = 6.0;
/// Right margin between the thumb and the panel edge (px).
pub const DOCK_SCROLLBAR_MARGIN_R: f32 = 2.0;
/// Minimum thumb height so very long lists keep a draggable target (px).
pub const DOCK_SCROLLBAR_MIN_THUMB_H: f32 = 24.0;
/// Scrollbar thumb fill — same alpha curve as the file viewer's thumb
/// so the dock scrollbars feel consistent.
pub const DOCK_SCROLLBAR_THUMB: Hsla = hsla(0.0, 0.0, 1.0, 0.25);
pub const DOCK_SCROLLBAR_THUMB_HOVER: Hsla = hsla(0.0, 0.0, 1.0, 0.45);
/// Focused pane header background.
pub const PANE_HEADER_FOCUSED_BG: Hsla = hsla(0.0, 0.0, 0.176, 1.0);
/// Unfocused pane header background.
pub const PANE_HEADER_UNFOCUSED_BG: Hsla = hsla(0.0, 0.0, 0.12, 1.0);
/// Pane header cwd text color.
pub const PANE_HEADER_CWD_TEXT: Hsla = hsla(0.0, 0.0, 0.45, 1.0);
/// Pane header horizontal padding (px).
pub const PANE_HEADER_PAD_X: f32 = 8.0;
/// Pane header item gap (px).
pub const PANE_HEADER_GAP: f32 = 4.0;
/// Pane header title/cwd inner gap (px).
pub const PANE_HEADER_INNER_GAP: f32 = 6.0;
/// Pane header close button width/height (px).
pub const PANE_HEADER_CLOSE_W: f32 = 16.0;
/// Pane header close button height (px).
pub const PANE_HEADER_CLOSE_H: f32 = 14.0;
/// Pane header close button corner radius (px).
pub const PANE_HEADER_CLOSE_RADIUS: f32 = 3.0;
/// Tab cell inner gap (px).
pub const TAB_GAP: f32 = 4.0;
/// Tab cell left padding (px).
pub const TAB_PAD_LEFT: f32 = 10.0;
/// Tab cell right padding (px).
pub const TAB_PAD_RIGHT: f32 = 4.0;
/// Tab cell vertical padding (px).
pub const TAB_PAD_Y: f32 = 2.0;
/// Tab cell horizontal margin (px).
pub const TAB_MARGIN_X: f32 = 1.0;
/// Tab close button width/height (px).
pub const TAB_CLOSE_W: f32 = 16.0;
/// Tab close button corner radius (px).
pub const TAB_CLOSE_RADIUS: f32 = 3.0;
/// New-tab button horizontal padding (px).
pub const NEW_TAB_PAD_X: f32 = 8.0;
/// New-tab button vertical padding (px).
pub const NEW_TAB_PAD_Y: f32 = 4.0;
/// New-tab button horizontal margin (px).
pub const NEW_TAB_MARGIN_X: f32 = 2.0;
/// New-tab button corner radius (px).
pub const NEW_TAB_RADIUS: f32 = 4.0;
/// Dock toggle icon group inner gap (px).
pub const DOCK_ICON_GROUP_GAP: f32 = 2.0;
/// Status bar item gap (px).
pub const STATUS_BAR_GAP: f32 = 8.0;
/// Command palette input padding X (px).
pub const PALETTE_INPUT_PAD_X: f32 = 12.0;
/// Command palette input padding Y (px).
pub const PALETTE_INPUT_PAD_Y: f32 = 8.0;
/// Command palette query font size (px).
pub const PALETTE_QUERY_FONT_SIZE: f32 = 14.0;
/// Command palette max list height (px).
pub const PALETTE_MAX_HEIGHT: f32 = 360.0;
/// Command palette entry padding X (px).
pub const PALETTE_ENTRY_PAD_X: f32 = 12.0;
/// Command palette entry padding Y (px).
pub const PALETTE_ENTRY_PAD_Y: f32 = 6.0;
/// Command palette entry label font size (px).
pub const PALETTE_ENTRY_FONT_SIZE: f32 = 13.0;
/// Command palette shortcut font size (px).
pub const PALETTE_SHORTCUT_FONT_SIZE: f32 = 11.0;
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
pub const STATUS_BAR_DETACHED_FONT_SIZE: f32 = 10.0;
pub const STATUS_BAR_DETACHED_PAD_X: f32 = 5.0;
pub const STATUS_BAR_DETACHED_PAD_Y: f32 = 1.0;
pub const STATUS_BAR_DETACHED_RADIUS: f32 = 3.0;
/// Agent activity log entry font size (px).
pub const AGENT_LOG_FONT_SIZE: f32 = 12.0;
/// Agent activity log icon column width (px).
pub const AGENT_LOG_ICON_W: f32 = 18.0;
/// Agent activity log entry horizontal gap (px).
pub const AGENT_LOG_GAP: f32 = 6.0;
/// Agent activity log container horizontal padding (px).
pub const AGENT_LOG_PAD_X: f32 = 8.0;
/// Agent activity log container vertical padding (px).
pub const AGENT_LOG_PAD_Y: f32 = 4.0;
/// Agent activity log entry vertical gap (px).
pub const AGENT_LOG_ENTRY_GAP: f32 = 2.0;
/// Agent activity log pinned status top margin (px).
pub const AGENT_LOG_STATUS_MT: f32 = 4.0;
/// Agent activity log pinned status padding X (px).
pub const AGENT_LOG_STATUS_PAD_X: f32 = 8.0;
/// Agent activity log pinned status padding Y (px).
pub const AGENT_LOG_STATUS_PAD_Y: f32 = 4.0;
/// Agent chat message label font size (px).
pub const AGENT_CHAT_LABEL_FONT_SIZE: f32 = 11.0;
/// Agent chat message body font size (px).
pub const AGENT_CHAT_MSG_FONT_SIZE: f32 = 13.0;
/// Agent chat message gap (px).
pub const AGENT_CHAT_MSG_GAP: f32 = 2.0;
/// Agent chat message list gap (px).
pub const AGENT_CHAT_LIST_GAP: f32 = 8.0;
/// Agent chat container padding X (px).
pub const AGENT_CHAT_PAD_X: f32 = 8.0;
/// Agent chat container padding Y (px).
pub const AGENT_CHAT_PAD_Y: f32 = 4.0;
/// Agent chat input area padding X (px).
pub const AGENT_CHAT_INPUT_PAD_X: f32 = 8.0;
/// Agent chat input area padding Y (px).
pub const AGENT_CHAT_INPUT_PAD_Y: f32 = 6.0;
/// Agent chat input box inner padding X (px).
pub const AGENT_CHAT_INPUT_INNER_PAD_X: f32 = 8.0;
/// Agent chat input box inner padding Y (px).
pub const AGENT_CHAT_INPUT_INNER_PAD_Y: f32 = 4.0;
/// Agent chat input box corner radius (px).
pub const AGENT_CHAT_INPUT_RADIUS: f32 = 4.0;
/// Agent task list entry font size (px).
pub const AGENT_TASK_FONT_SIZE: f32 = 12.0;
/// Agent task list icon column width (px).
pub const AGENT_TASK_ICON_W: f32 = 14.0;
/// Agent task list entry gap (px).
pub const AGENT_TASK_GAP: f32 = 6.0;
/// Agent task list entry padding X (px).
pub const AGENT_TASK_PAD_X: f32 = 8.0;
/// Agent task list entry padding Y (px).
pub const AGENT_TASK_PAD_Y: f32 = 4.0;
/// Agent task list container padding Y (px).
pub const AGENT_TASK_LIST_PAD_Y: f32 = 4.0;
/// Agent activity log pinned status text color (bright white).
pub const AGENT_LOG_STATUS_TEXT: Hsla = hsla(0.0, 0.0, 1.0, 1.0);
/// Agent chat — user message body text color (bright white).
pub const AGENT_CHAT_USER_TEXT: Hsla = hsla(0.0, 0.0, 1.0, 1.0);
/// Agent task list — Running task icon foreground (bright white).
pub const AGENT_TASK_RUNNING_FG: Hsla = hsla(0.0, 0.0, 1.0, 1.0);
/// Agent task list — Running task icon background (bright white).
pub const AGENT_TASK_RUNNING_BG: Hsla = hsla(0.0, 0.0, 1.0, 1.0);
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
/// Visible line color for pane dividers (the 1px bar between two
/// panes in a split). Dock boundaries use `DOCK_BORDER` instead —
/// keep the two distinct so reskinning dock vs pane splits stays
/// orthogonal.
pub const PANE_DIVIDER_BG: Hsla = hsla(0.0, 0.0, 0.25, 1.0);
/// Welcome screen background.
pub const WELCOME_BG: Hsla = hsla(0.0, 0.0, 0.10, 1.0);
/// Welcome screen title font size (px).
pub const WELCOME_TITLE_FONT_SIZE: f32 = 28.0;
/// Welcome screen version font size (px).
pub const WELCOME_VERSION_FONT_SIZE: f32 = 14.0;
/// Welcome screen section heading font size (px).
pub const WELCOME_HEADING_FONT_SIZE: f32 = 13.0;
/// Welcome screen button font size (px).
pub const WELCOME_BUTTON_FONT_SIZE: f32 = 14.0;
/// Welcome screen button background.
pub const WELCOME_BUTTON_BG: Hsla = hsla(0.0, 0.0, 0.18, 1.0);
/// Welcome screen button hover background.
pub const WELCOME_BUTTON_HOVER_BG: Hsla = hsla(0.0, 0.0, 0.25, 1.0);
/// Welcome screen button border.
pub const WELCOME_BUTTON_BORDER: Hsla = hsla(0.0, 0.0, 0.28, 1.0);
/// Welcome screen recent entry font size (px).
pub const WELCOME_RECENT_FONT_SIZE: f32 = 13.0;
/// Welcome screen recent entry hover background.
pub const WELCOME_RECENT_HOVER_BG: Hsla = hsla(0.0, 0.0, 0.18, 1.0);
/// Welcome screen panel width (px).
pub const WELCOME_PANEL_WIDTH: f32 = 420.0;
/// Welcome screen panel padding (px).
pub const WELCOME_PANEL_PAD: f32 = 40.0;
/// Welcome screen item gap (px).
pub const WELCOME_GAP: f32 = 16.0;
/// Welcome screen button padding X (px).
pub const WELCOME_BUTTON_PAD_X: f32 = 16.0;
/// Welcome screen button padding Y (px).
pub const WELCOME_BUTTON_PAD_Y: f32 = 10.0;
/// Welcome screen button corner radius (px).
pub const WELCOME_BUTTON_RADIUS: f32 = 6.0;
/// Welcome screen recent entry padding X (px).
pub const WELCOME_RECENT_PAD_X: f32 = 12.0;
/// Welcome screen recent entry padding Y (px).
pub const WELCOME_RECENT_PAD_Y: f32 = 8.0;
/// Welcome screen recent entry corner radius (px).
pub const WELCOME_RECENT_RADIUS: f32 = 4.0;
/// Welcome screen primary text color (titles, version, button text — bright white).
pub const WELCOME_TEXT: Hsla = hsla(0.0, 0.0, 1.0, 1.0);
/// Welcome screen tight inner gap (px) — used between heading + label.
pub const WELCOME_GAP_TIGHT: f32 = 4.0;
/// Welcome screen loose inner gap (px) — used between recent entry rows.
pub const WELCOME_GAP_LOOSE: f32 = 8.0;
/// File viewer main background.
pub const FILE_VIEWER_BG: Hsla = hsla(0.0, 0.0, 0.08, 1.0);
/// File viewer toolbar background.
pub const FILE_VIEWER_HEADER_BG: Hsla = hsla(0.0, 0.0, 0.13, 1.0);
/// File viewer toolbar bottom border.
pub const FILE_VIEWER_HEADER_BORDER: Hsla = hsla(0.0, 0.0, 0.22, 1.0);
/// File viewer toolbar text (path label).
pub const FILE_VIEWER_HEADER_TEXT: Hsla = hsla(0.0, 0.0, 0.80, 1.0);
/// File viewer toolbar height (px).
pub const FILE_VIEWER_HEADER_H: f32 = 28.0;
/// File viewer toolbar horizontal padding (px).
pub const FILE_VIEWER_HEADER_PAD_X: f32 = 10.0;
/// File viewer toolbar font size (px).
pub const FILE_VIEWER_HEADER_FONT_SIZE: f32 = 11.0;
/// File viewer close button font size (px).
pub const FILE_VIEWER_CLOSE_FONT_SIZE: f32 = 14.0;
/// File viewer close button hover color.
pub const FILE_VIEWER_CLOSE_HOVER: Hsla = hsla(0.0, 0.0, 1.0, 1.0);
/// File viewer body font size (px).
pub const FILE_VIEWER_FONT_SIZE: f32 = 12.0;
/// File viewer body text color.
pub const FILE_VIEWER_TEXT: Hsla = hsla(0.0, 0.0, 0.82, 1.0);
/// File viewer line number column width (px).
pub const FILE_VIEWER_LINE_NO_W: f32 = 50.0;
/// Selection highlight background in the file viewer.
pub const FILE_VIEWER_SELECTION_BG: Hsla = hsla(220.0, 0.35, 0.22, 1.0);
/// File viewer line number text color.
pub const FILE_VIEWER_LINE_NO_TEXT: Hsla = hsla(0.0, 0.0, 0.35, 1.0);
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
pub const FILE_VIEWER_TOOLBAR_GAP: f32 = 6.0;
/// Active mode tab background in file viewer toolbar.
pub const FILE_VIEWER_TAB_ACTIVE_BG: Hsla = hsla(0.0, 0.0, 0.25, 1.0);
/// Inactive mode tab text in file viewer toolbar.
pub const FILE_VIEWER_TAB_TEXT: Hsla = hsla(0.0, 0.0, 0.65, 1.0);
/// Active mode tab text in file viewer toolbar.
pub const FILE_VIEWER_TAB_ACTIVE_TEXT: Hsla = hsla(0.0, 0.0, 1.0, 1.0);
/// Mode tab horizontal padding (px).
pub const FILE_VIEWER_TAB_PAD_X: f32 = 8.0;
/// Mode tab vertical padding (px).
pub const FILE_VIEWER_TAB_PAD_Y: f32 = 3.0;
/// Mode tab corner radius (px).
pub const FILE_VIEWER_TAB_RADIUS: f32 = 4.0;
/// Diff added-line background.
pub const FILE_DIFF_ADD_BG: Hsla = hsla(135.0, 0.40, 0.15, 1.0);
/// Diff removed-line background.
pub const FILE_DIFF_DEL_BG: Hsla = hsla(0.0, 0.45, 0.15, 1.0);
/// Diff added-line text color.
pub const FILE_DIFF_ADD_TEXT: Hsla = hsla(135.0, 0.60, 0.70, 1.0);
/// Diff removed-line text color.
pub const FILE_DIFF_DEL_TEXT: Hsla = hsla(0.0, 0.60, 0.70, 1.0);
/// Diff hunk-header background.
pub const FILE_DIFF_HUNK_BG: Hsla = hsla(220.0, 0.20, 0.18, 1.0);
/// Diff hunk-header text color.
pub const FILE_DIFF_HUNK_TEXT: Hsla = hsla(220.0, 0.40, 0.60, 1.0);
/// Diff hunk-header top/bottom border color.
pub const FILE_DIFF_HUNK_BORDER: Hsla = hsla(220.0, 0.25, 0.30, 1.0);
/// Vertical padding added above and below the hunk-header content (px).
pub const FILE_DIFF_HUNK_PADDING_Y: f32 = 5.0;
/// Diff context line text color.
pub const FILE_DIFF_CTX_TEXT: Hsla = hsla(0.0, 0.0, 0.60, 1.0);
/// Line number right padding in the raw file view (px).
pub const FILE_VIEWER_LINE_NO_PAD_R: f32 = 8.0;
/// Line number right padding in the diff view dual-column (px).
pub const FILE_VIEWER_DIFF_LINE_NO_PAD_R: f32 = 4.0;
/// Diff marker (`+`/`-`/` `) column width (px).
pub const FILE_VIEWER_DIFF_MARKER_W: f32 = 10.0;
/// Vertical divider between two diff columns (side-by-side mode).
pub const FILE_VIEWER_DIVIDER: Hsla = hsla(0.0, 0.0, 0.20, 1.0);
/// Scrollbar track width (px).
pub const FILE_VIEWER_SCROLLBAR_W: f32 = 6.0;
/// Scrollbar track right margin (px).
pub const FILE_VIEWER_SCROLLBAR_MARGIN_R: f32 = 2.0;
/// Minimum scrollbar thumb height (px).
pub const FILE_VIEWER_SCROLLBAR_MIN_THUMB_H: f32 = 24.0;
/// Scrollbar thumb color.
pub const FILE_VIEWER_SCROLLBAR_THUMB: Hsla = hsla(0.0, 0.0, 1.0, 0.25);
/// Scrollbar thumb hover color.
pub const FILE_VIEWER_SCROLLBAR_THUMB_HOVER: Hsla = hsla(0.0, 0.0, 1.0, 0.45);
/// Hunk header trailing context text color (function name / class name, dim).
pub const FILE_DIFF_HUNK_CTX_TEXT: Hsla = hsla(220.0, 0.20, 0.45, 1.0);
/// syntect theme name used for diff syntax highlighting.
pub const FILE_VIEWER_SYNTAX_THEME: &str = "base16-ocean.dark";
/// Gap between `@@ -N +M @@` and its trailing context text (px).
pub const FILE_DIFF_HUNK_CTX_GAP_X: f32 = 8.0;
/// Gap between `+N` and `-N` in the diff stat badge (px).
pub const FILE_DIFF_STAT_GAP: f32 = 4.0;
/// Diff stat added-lines count color (+N).
pub const FILE_DIFF_STAT_ADD: Hsla = hsla(133.0, 0.60, 0.55, 1.0);
/// Diff stat removed-lines count color (-N).
pub const FILE_DIFF_STAT_DEL: Hsla = hsla(0.0, 0.60, 0.55, 1.0);
/// Diff stat and file-status badge font size (px).
pub const FILE_DIFF_STAT_FONT_SIZE: f32 = 11.0;
/// Word-level diff insertion highlight background (stronger than line bg).
pub const FILE_DIFF_WORD_ADD_BG: Hsla = hsla(135.0, 0.60, 0.27, 1.0);
/// Word-level diff deletion highlight background (stronger than line bg).
pub const FILE_DIFF_WORD_DEL_BG: Hsla = hsla(0.0, 0.60, 0.27, 1.0);
/// Search panel height (px).
pub const FILE_VIEWER_SEARCH_PANEL_H: f32 = 36.0;
/// Search panel horizontal padding (px).
pub const FILE_VIEWER_SEARCH_PAD_X: f32 = 12.0;
/// Search input text color.
pub const FILE_VIEWER_SEARCH_TEXT: Hsla = hsla(0.0, 0.0, 0.90, 1.0);
/// Match counter color ("3/12").
pub const FILE_VIEWER_SEARCH_COUNT: Hsla = hsla(0.0, 0.0, 0.55, 1.0);
/// "No matches" label color (same value as SEARCH_LABEL_EMPTY).
pub const FILE_VIEWER_SEARCH_EMPTY: Hsla = hsla(0.0, 0.50, 0.65, 1.0);
/// Non-focused match row highlight background.
pub const FILE_VIEWER_SEARCH_MATCH_BG: Hsla = hsla(43.0, 0.70, 0.30, 0.45);
/// Focused match row highlight background.
pub const FILE_VIEWER_SEARCH_FOCUSED_BG: Hsla = hsla(43.0, 0.85, 0.48, 0.60);
/// Right margin between the search panel and the window edge (px).
pub const FILE_VIEWER_SEARCH_MARGIN_R: f32 = 16.0;
/// Top margin between the toolbar and the search panel (px).
pub const FILE_VIEWER_SEARCH_MARGIN_T: f32 = 8.0;
/// Fixed width of the search panel (px) — matches the terminal search bar.
pub const FILE_VIEWER_SEARCH_PANEL_W: f32 = 380.0;
/// Search panel font size (px).
pub const FILE_VIEWER_SEARCH_FONT_SIZE: f32 = 12.0;
/// Gap between items inside the search panel (px).
pub const FILE_VIEWER_SEARCH_ITEM_GAP: f32 = 8.0;
/// Search input area horizontal padding (px).
pub const FILE_VIEWER_SEARCH_INPUT_PAD_X: f32 = 8.0;
/// Search input area vertical padding (px).
pub const FILE_VIEWER_SEARCH_INPUT_PAD_Y: f32 = 3.0;
/// Search input area corner radius (px).
pub const FILE_VIEWER_SEARCH_INPUT_RADIUS: f32 = 4.0;
/// Match counter font size inside the input area (px).
pub const FILE_VIEWER_SEARCH_COUNTER_SIZE: f32 = 11.0;
/// Cursor indicator width (px).
pub const FILE_VIEWER_SEARCH_CURSOR_W: f32 = 1.0;
/// Cursor indicator height (px).
pub const FILE_VIEWER_SEARCH_CURSOR_H: f32 = 14.0;
/// Button horizontal padding (px).
pub const FILE_VIEWER_SEARCH_BTN_PAD_X: f32 = 6.0;
/// Left margin of the close button to visually separate it from nav buttons (px).
pub const FILE_VIEWER_SEARCH_BTN_ML: f32 = 4.0;
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
/// H1 heading text color (bright white).
pub const MD_H1_COLOR: Hsla = hsla(0.0, 0.0, 0.97, 1.0);
/// H2 heading text color.
pub const MD_H2_COLOR: Hsla = hsla(0.0, 0.0, 0.92, 1.0);
/// H3 heading text color.
pub const MD_H3_COLOR: Hsla = hsla(0.0, 0.0, 0.85, 1.0);
/// H4–H6 heading text color (same as body).
pub const MD_H4_COLOR: Hsla = FILE_VIEWER_TEXT;
/// Inline code background.
pub const MD_CODE_INLINE_BG: Hsla = hsla(0.0, 0.0, 0.18, 1.0);
/// Inline code text color.
pub const MD_CODE_INLINE_TEXT: Hsla = hsla(29.0, 0.55, 0.72, 1.0);
/// Code block background.
pub const MD_CODE_BLOCK_BG: Hsla = hsla(0.0, 0.0, 0.11, 1.0);
/// Code block border color.
pub const MD_CODE_BLOCK_BORDER: Hsla = hsla(0.0, 0.0, 0.22, 1.0);
/// Code block corner radius (px).
pub const MD_CODE_BLOCK_RADIUS: f32 = 6.0;
/// Code block horizontal padding (px).
pub const MD_CODE_BLOCK_PAD_X: f32 = 12.0;
/// Code block vertical padding (px).
pub const MD_CODE_BLOCK_PAD_Y: f32 = 8.0;
/// Blockquote left border color.
pub const MD_BLOCKQUOTE_BORDER: Hsla = hsla(0.0, 0.0, 0.35, 1.0);
/// Blockquote text color (dimmed).
pub const MD_BLOCKQUOTE_TEXT: Hsla = hsla(0.0, 0.0, 0.60, 1.0);
/// Blockquote left border width (px).
pub const MD_BLOCKQUOTE_BORDER_W: f32 = 3.0;
/// Blockquote left padding (px).
pub const MD_BLOCKQUOTE_PAD_L: f32 = 12.0;
/// Horizontal rule color.
pub const MD_RULE_COLOR: Hsla = hsla(0.0, 0.0, 0.28, 1.0);
/// Horizontal rule height (px).
pub const MD_RULE_H: f32 = 1.0;
/// Link text color.
pub const MD_LINK_COLOR: Hsla = hsla(209.0, 0.70, 0.65, 1.0);
/// Strikethrough text color.
pub const MD_STRIKETHROUGH_COLOR: Hsla = hsla(0.0, 0.0, 0.50, 1.0);
/// List bullet color.
pub const MD_LIST_BULLET_COLOR: Hsla = hsla(0.0, 0.0, 0.50, 1.0);
/// List item indentation (px).
pub const MD_LIST_INDENT: f32 = 16.0;
/// Outer horizontal padding for the Markdown viewer body (px).
pub const MD_BODY_PAD_X: f32 = 24.0;
/// Outer vertical padding for the Markdown viewer body (px).
pub const MD_BODY_PAD_Y: f32 = 16.0;
/// Vertical gap between top-level blocks (px).
pub const MD_BLOCK_GAP: f32 = 8.0;
/// Extra top margin for headings (px).
pub const MD_HEADING_MARGIN_TOP: f32 = 12.0;
/// Task list checked item bullet color (green).
pub const MD_TASK_CHECKED_COLOR: Hsla = hsla(137.0, 0.55, 0.55, 1.0);
/// Footnote reference / definition text color.
pub const MD_FOOTNOTE_COLOR: Hsla = hsla(209.0, 0.45, 0.60, 1.0);
/// Footnote font size (slightly smaller than body, px).
pub const MD_FOOTNOTE_FONT_SIZE: f32 = 11.0;
/// Inline / block HTML passthrough text color (dim).
pub const MD_HTML_COLOR: Hsla = hsla(0.0, 0.0, 0.42, 1.0);
/// HTML passthrough font size (px).
pub const MD_HTML_FONT_SIZE: f32 = 11.0;
/// Table border color.
pub const MD_TABLE_BORDER: Hsla = hsla(0.0, 0.0, 0.28, 1.0);
/// Table header row background.
pub const MD_TABLE_HEADER_BG: Hsla = hsla(0.0, 0.0, 0.16, 1.0);
/// Even body row background.
pub const MD_TABLE_ROW_BG_EVEN: Hsla = hsla(0.0, 0.0, 0.11, 1.0);
/// Odd body row background.
pub const MD_TABLE_ROW_BG_ODD: Hsla = hsla(0.0, 0.0, 0.13, 1.0);
/// Horizontal cell padding (px).
pub const MD_TABLE_CELL_PAD_X: f32 = 10.0;
/// Vertical cell padding (px).
pub const MD_TABLE_CELL_PAD_Y: f32 = 5.0;
/// Minimum width of a table cell (px).
pub const MD_TABLE_CELL_MIN_W: f32 = 60.0;
/// Gap between list items (px).
pub const MD_LIST_ITEM_GAP: f32 = 2.0;
/// Gap between the bullet/number and item text in a list row (px).
pub const MD_LIST_ROW_GAP: f32 = 6.0;
/// Corner radius for inline code, selection highlight, and table container (px).
pub const MD_BLOCK_RADIUS: f32 = 3.0;
/// Vertical margin above/below standalone block elements (Rule, Table) (px).
pub const MD_BLOCK_MARGIN_Y: f32 = 4.0;
/// Horizontal padding for inline code (px).
pub const MD_CODE_INLINE_PAD_X: f32 = 3.0;
/// Minimum dimension for a divider / header lane (px) — 1 device pixel.
/// Used as `min_w` / `min_h` inside flex layouts so the lane always
/// has a clickable line even when its surrounding area collapses.
pub const RENDER_MIN_DIM: f32 = 1.0;
/// Background for the toast pill.
pub const TOAST_BG: Hsla = hsla(0.0, 0.0, 0.18, 0.97);
/// 1-px border around the toast pill.
pub const TOAST_BORDER: Hsla = hsla(0.0, 0.0, 0.32, 1.0);
/// Corner radius of the toast pill (px).
pub const TOAST_RADIUS: f32 = 8.0;
/// Primary text color in toasts.
pub const TOAST_TEXT: Hsla = hsla(0.0, 0.0, 0.90, 1.0);
/// Action-button label color (accent green).
pub const TOAST_ACTION_TEXT: Hsla = ACCENT_GREEN;
/// Horizontal padding inside the toast pill (px).
pub const TOAST_PAD_X: f32 = 16.0;
/// Vertical padding inside the toast pill (px).
pub const TOAST_PAD_Y: f32 = 10.0;
/// Message font size (px).
pub const TOAST_FONT_SIZE: f32 = 13.0;
/// Gap between message and action button (px).
pub const TOAST_GAP: f32 = 12.0;
/// Minimum width of the toast pill (px).
pub const TOAST_MIN_W: f32 = 240.0;
/// Maximum width of the toast pill (px).
pub const TOAST_MAX_W: f32 = 480.0;
/// Distance from the window bottom edge (px).
pub const TOAST_BOTTOM_MARGIN: f32 = 24.0;
/// Info-level toast accent — neutral cyan, matches left-dock info banners.
pub const TOAST_TINT_INFO: Hsla = hsla(200.0, 0.55, 0.65, 1.0);
/// Warning-level toast accent — amber, matches search "no matches" tone.
pub const TOAST_TINT_WARNING: Hsla = hsla(38.0, 0.85, 0.60, 1.0);
/// Error-level toast accent — destructive red, matches close-button hover.
pub const TOAST_TINT_ERROR: Hsla = hsla(0.0, 0.70, 0.60, 1.0);
/// Secondary text inside the toast (message / context). Slightly dimmer
/// than [`TOAST_TEXT`] so the title row reads first.
pub const TOAST_TEXT_DIM: Hsla = hsla(0.0, 0.0, 0.70, 1.0);
/// Repeat-counter chip background (×N badge). Subtle so it doesn't
/// dominate the title.
pub const TOAST_REPEAT_BG: Hsla = hsla(0.0, 0.0, 0.28, 1.0);
/// Vertical gap between stacked toasts.
pub const TOAST_STACK_GAP: f32 = 4.0;
/// Padding below the toast stack before the status bar starts.
pub const TOAST_STACK_BOTTOM_PAD: f32 = 4.0;
/// Title-row line height fudge so the text vertically centers in the
/// pill at the chosen font size.
pub const TOAST_TITLE_FONT_SIZE: f32 = 12.0;
/// Repeat-counter font size — smaller than the title.
pub const TOAST_REPEAT_FONT_SIZE: f32 = 10.0;
/// Minimum pixel width for the leading severity bar.
pub const TOAST_SEVERITY_BAR_W: f32 = 3.0;
/// Horizontal padding inside the `×N` repeat-counter chip.
pub const TOAST_REPEAT_PAD_X: f32 = 6.0;
/// Vertical padding inside the `×N` repeat-counter chip.
pub const TOAST_REPEAT_PAD_Y: f32 = 2.0;
/// Modal panel width for the error-report details body — wider than the
/// default `MODAL_PANEL_WIDTH` because the plain-text rendering carries
/// stack frames and source-chain entries that wrap awkwardly at 420 px.
pub const ERROR_MODAL_WIDTH: f32 = 640.0;
/// Body monospace font size. Slightly smaller than the modal title so a
/// long backtrace fits without horizontal scroll.
pub const ERROR_MODAL_BODY_FONT_SIZE: f32 = 11.0;
/// Body container — dim panel inset behind the monospace text so it
/// reads as quoted output rather than free-flowing copy.
pub const ERROR_MODAL_BODY_BG: Hsla = hsla(0.0, 0.0, 0.08, 1.0);
/// Body container border.
pub const ERROR_MODAL_BODY_BORDER: Hsla = hsla(0.0, 0.0, 0.22, 1.0);
/// Padding inside the body container.
pub const ERROR_MODAL_BODY_PAD: f32 = 10.0;
/// Maximum body height — beyond this the container scrolls.
pub const ERROR_MODAL_BODY_MAX_H: f32 = 360.0;
/// Default terminal canvas background (solid black).
pub const TERMINAL_BG: Hsla = hsla(0.0, 0.0, 0.0, 1.0);
/// Track background (subtle, nearly transparent fill).
pub const TERMINAL_SCROLLBAR_TRACK_BG: Hsla = hsla(0.0, 0.0, 1.0, 0.04);
/// Corner radius of the scrollbar thumb (px).
pub const TERMINAL_SCROLLBAR_THUMB_RADIUS: f32 = 3.0;
/// Container background (idle).
pub const KEYSTROKE_INPUT_BG: Hsla = hsla(0.0, 0.0, 0.14, 1.0);
/// Container border (idle).
pub const KEYSTROKE_INPUT_BORDER: Hsla = hsla(0.0, 0.0, 0.28, 1.0);
/// Container border when recording.
pub const KEYSTROKE_INPUT_BORDER_ACTIVE: Hsla = ACCENT_GREEN;
/// Container corner radius (px).
pub const KEYSTROKE_INPUT_RADIUS: f32 = 6.0;
/// Horizontal padding inside the container (px).
pub const KEYSTROKE_INPUT_PAD_X: f32 = 8.0;
/// Vertical padding inside the container (px).
pub const KEYSTROKE_INPUT_PAD_Y: f32 = 5.0;
/// Minimum width so the recording hint has room (px).
pub const KEYSTROKE_INPUT_MIN_W: f32 = 140.0;
/// Badge background (individual key pill).
pub const KEYSTROKE_BADGE_BG: Hsla = hsla(0.0, 0.0, 0.24, 1.0);
/// Badge border.
pub const KEYSTROKE_BADGE_BORDER: Hsla = hsla(0.0, 0.0, 0.38, 1.0);
/// Badge text color.
pub const KEYSTROKE_BADGE_TEXT: Hsla = hsla(0.0, 0.0, 0.88, 1.0);
/// Badge corner radius (px).
pub const KEYSTROKE_BADGE_RADIUS: f32 = 4.0;
/// Badge horizontal padding (px).
pub const KEYSTROKE_BADGE_PAD_X: f32 = 6.0;
/// Badge vertical padding (px).
pub const KEYSTROKE_BADGE_PAD_Y: f32 = 2.0;
/// Badge font size (px).
pub const KEYSTROKE_BADGE_FONT_SIZE: f32 = 12.0;
/// Gap between key badges in a sequence (px).
pub const KEYSTROKE_BADGE_GAP: f32 = 4.0;
/// Muted hint text (placeholder / recording hint).
pub const KEYSTROKE_HINT_TEXT: Hsla = hsla(0.0, 0.0, 0.45, 1.0);
/// Font size for container hint text (px).
pub const KEYSTROKE_INPUT_FONT_SIZE: f32 = 12.0;
/// Popover panel background.
pub const POPOVER_BG: Hsla = hsla(0.0, 0.0, 0.16, 0.98);
/// Popover 1-px border.
pub const POPOVER_BORDER: Hsla = hsla(0.0, 0.0, 0.30, 1.0);
/// Corner radius of the popover panel (px).
pub const POPOVER_RADIUS: f32 = 6.0;
/// Vertical padding above/below the item list inside the panel (px).
pub const POPOVER_LIST_PAD_Y: f32 = 4.0;
/// Horizontal padding for each item row (px).
pub const POPOVER_ITEM_PAD_X: f32 = 12.0;
/// Vertical padding for each item row (px).
pub const POPOVER_ITEM_PAD_Y: f32 = 6.0;
/// Item font size (px).
pub const POPOVER_ITEM_FONT_SIZE: f32 = 13.0;
/// Item text color.
pub const POPOVER_ITEM_TEXT: Hsla = hsla(0.0, 0.0, 0.88, 1.0);
/// Item hover/keyboard-cursor background.
pub const POPOVER_ITEM_HOVER_BG: Hsla = hsla(0.0, 0.0, 1.0, 0.08);
/// Item text color when destructive (delete/remove actions).
pub const POPOVER_ITEM_DANGER_TEXT: Hsla = hsla(0.0, 0.65, 0.60, 1.0);
/// Separator line color inside the popover.
pub const POPOVER_SEPARATOR: Hsla = hsla(0.0, 0.0, 1.0, 0.08);
/// Minimum width of the popover panel (px).
pub const POPOVER_MIN_WIDTH: f32 = 140.0;
/// Height of the separator rule inside the popover (px).
pub const POPOVER_SEPARATOR_HEIGHT: f32 = 1.0;
/// Edge margin kept between the popover panel and the window boundary (px).
pub const POPOVER_SNAP_MARGIN: f32 = 8.0;
/// Offset from the cursor hotspot to the top-left corner of the drag pill (px).
/// Applied as padding inside the transparent ghost wrapper so the pill appears
/// just below and to the right of the cursor regardless of where the user
/// clicked within the source row.
pub const DRAG_PILL_CURSOR_OFFSET: f32 = 4.0;
/// Leading indicator footprint on the worktree row. All four states
/// share the same 3×3 dot-grid shape; only color/animation differ.
pub const STATUS_INDICATOR_SIZE: f32 = 16.0;
/// Sub-row per-session badge footprint (Phase D).
pub const STATUS_INDICATOR_BADGE_SIZE: f32 = 12.0;
/// Width of the cell that holds the indicator inside the worktree row,
/// inserted between the active-row accent bar and the body.
pub const STATUS_INDICATOR_CELL_WIDTH: f32 = 22.0;
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
pub const STATUS_BADGES_LABEL_FONT_SIZE: f32 = 10.0;
pub const STATUS_BADGES_LABEL_GAP: f32 = 6.0;
pub const STATUS_BADGE_ACTIVE_OUTLINE: Hsla = hsla(0.0, 0.0, 1.0, 0.85);
pub const STATUS_BADGE_ACTIVE_OUTLINE_PX: f32 = 1.0;
/// Pixels added to the badge frame to host the outline without
/// occluding the inner colour.
pub const STATUS_BADGE_ACTIVE_OUTER_PAD: f32 = 2.0;
/// Number of leading characters of the session_id shown in the
/// per-badge tooltip (followed by `…`). 8 chars give the user enough
/// to disambiguate between concurrent sessions while staying compact
/// in the sub-row's narrow space.
pub const STATUS_BADGE_TOOLTIP_SESSION_PREFIX_LEN: usize = 8;
pub const CLAUDE_BANNER_BG: Hsla = hsla(210.0, 1.0, 0.68, 0.08);
pub const CLAUDE_BANNER_BORDER: Hsla = hsla(210.0, 1.0, 0.68, 0.20);
pub const CLAUDE_BANNER_HOVER_BG: Hsla = hsla(210.0, 1.0, 0.68, 0.14);
pub const CLAUDE_BANNER_TEXT: Hsla = hsla(0.0, 0.0, 0.75, 1.0);
pub const CLAUDE_BANNER_ICON: Hsla = hsla(210.0, 1.0, 0.68, 1.0);
pub const CLAUDE_BANNER_PAD_X: f32 = 12.0;
pub const CLAUDE_BANNER_PAD_Y: f32 = 8.0;
pub const CLAUDE_BANNER_GAP: f32 = 8.0;
pub const CLAUDE_BANNER_RADIUS: f32 = 6.0;
pub const CLAUDE_BANNER_FONT_SIZE: f32 = 11.0;
pub const CLAUDE_BANNER_MARGIN_X: f32 = 8.0;
pub const CLAUDE_BANNER_MARGIN_Y: f32 = 6.0;
/// Outer horizontal padding for right-panel content rows (px).
pub const RIGHT_PANEL_PAD_X: f32 = 10.0;
/// Outer vertical padding between right-panel sections (px).
pub const RIGHT_PANEL_PAD_Y: f32 = 6.0;
/// Vertical gap between two siblings inside a single right-panel row (px).
pub const RIGHT_PANEL_ROW_GAP: f32 = 8.0;
/// Vertical padding inside a single session row in the Usage tab (px).
pub const RIGHT_PANEL_ROW_PAD_Y: f32 = 3.0;
/// Font size for the right-panel summary + session rows (px).
/// Matches `AGENT_CHAT_MSG_FONT_SIZE` so the four right-panel tabs feel
/// part of the same typographic family as the original chat panel.
pub const RIGHT_PANEL_BODY_FONT_SIZE: f32 = 12.0;
/// Font size for inline section labels ("Total", "in", "out", "cache").
pub const RIGHT_PANEL_LABEL_FONT_SIZE: f32 = 10.0;
/// Cost-amount color on the Usage tab. An amber accent so the
/// dollar-cost cell stands out against the muted token-count text
/// without yelling like a warning would.
pub const RIGHT_PANEL_COST_COLOR: Hsla = hsla(40.0, 0.65, 0.65, 1.0);
/// Maximum displayed width of the worktree-label cell in a session
/// row before it truncates with `…` (px). The cell's hard cap keeps
/// a long branch name from pushing the token + cost cells off-screen
/// when the right dock is narrow; below this width the metrics group
/// wraps onto a second line via `flex_wrap` instead.
pub const RIGHT_PANEL_WT_MAX_W: f32 = 140.0;
/// Fixed width allocated to the leading state-indicator glyph so titles
/// across rows align on a single column (px).
pub const RIGHT_PANEL_TASK_INDICATOR_W: f32 = 14.0;
/// Horizontal gap between adjacent action buttons in a task row (px).
pub const RIGHT_PANEL_TASK_BUTTON_GAP: f32 = 4.0;
/// Vertical padding for the Tasks tab header (filter + [+ New]) (px).
pub const RIGHT_PANEL_TASK_HEADER_PAD_Y: f32 = 6.0;
/// Background for a task row while the pointer hovers it.
pub const RIGHT_PANEL_TASK_HOVER_BG: Hsla = hsla(0.0, 0.0, 1.0, 0.04);
/// Background for the currently expanded task row (R-16).
pub const RIGHT_PANEL_TASK_SELECTED_BG: Hsla = hsla(0.0, 0.0, 1.0, 0.06);
/// Subtle muted color used for the secondary state label
/// ("Running", "Done (Stop)", …) shown next to the title.
pub const RIGHT_PANEL_TASK_STATE_TEXT: Hsla = hsla(0.0, 0.0, 0.55, 1.0);
/// Title-text color for a single task row.
pub const RIGHT_PANEL_TASK_TITLE_TEXT: Hsla = hsla(0.0, 0.0, 0.85, 1.0);
/// Maximum number of characters of an `Error` task message echoed
/// inline next to the row title before it's truncated with `…`.
pub const RIGHT_PANEL_TASK_ERROR_TRUNCATE: usize = 30;
/// Indicator color for `Running` rows — matches the agent's working
/// accent so a running task reads as live, not just queued.
pub const RIGHT_PANEL_TASK_RUNNING_COLOR: Hsla = ACCENT_GREEN;
/// Indicator color for `Done` rows.
pub const RIGHT_PANEL_TASK_DONE_COLOR: Hsla = hsla(0.0, 0.0, 0.65, 1.0);
/// Indicator color for `Error` rows.
pub const RIGHT_PANEL_TASK_ERROR_COLOR: Hsla = hsla(0.0, 0.55, 0.62, 1.0);
/// Indicator color for `Cancelled` rows — same muted gray as "Done"
/// but read as "no longer relevant" via the `⊘` glyph.
pub const RIGHT_PANEL_TASK_CANCELLED_COLOR: Hsla = hsla(0.0, 0.0, 0.45, 1.0);
/// Indicator color for `Backlog` rows.
pub const RIGHT_PANEL_TASK_BACKLOG_COLOR: Hsla = hsla(0.0, 0.0, 0.50, 1.0);
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
/// Color for the inline duration text (`2m 14s`) shown after the
/// status pill on every non-`Backlog` row.
pub const RIGHT_PANEL_TASK_DURATION_TEXT: Hsla = hsla(0.0, 0.0, 0.60, 1.0);
/// Font size for the inline duration text (px). One step below
/// [`RIGHT_PANEL_BODY_FONT_SIZE`] so it reads as auxiliary metadata,
/// not as a primary action surface.
pub const RIGHT_PANEL_TASK_DURATION_FONT_SIZE: f32 = 11.0;
/// Color of the `failures N/M` counter shown when the task's session
/// is over [`crate::ux::strings::RIGHT_PANEL_TASK_FAILURE_DISPLAY_THRESHOLD`]
/// tool-use failures. Amber so it reads as a warning hint without
/// claiming the row has already escalated to `Error`.
pub const RIGHT_PANEL_TASK_FAILURE_TEXT: Hsla = hsla(40.0, 0.65, 0.65, 1.0);
/// Font size for the `failures N/M` counter (px).
pub const RIGHT_PANEL_TASK_FAILURE_FONT_SIZE: f32 = 10.0;
/// Color for the session-status glyph rendered next to the
/// 8-char session-id badge (`⟳` / `●` / `⚠`).
pub const RIGHT_PANEL_TASK_SESSION_STATUS_TEXT: Hsla = hsla(0.0, 0.0, 0.65, 1.0);
/// Tint for the `⚠` glyph when the session status is
/// `NeedsAttention` — drawn slightly warmer than the muted default so
/// the row reads as "needs the user" even out of the corner of the eye.
pub const RIGHT_PANEL_TASK_SESSION_NEEDS_ATTENTION_TEXT: Hsla = hsla(40.0, 0.70, 0.65, 1.0);
/// Horizontal gap between the session-id badge and its trailing
/// status glyph (`⟳` / `●` / `⚠`) (px).
pub const RIGHT_PANEL_TASK_SESSION_GAP: f32 = 4.0;
/// 1 px error border drawn around the branch input on the TaskEdit
/// pane while `BranchValidation::Invalid` — error border around the
/// input plus inline reason text below. Tone matches
/// [`RIGHT_PANEL_TASK_ERROR_COLOR`] so the row indicator and the
/// invalid field read as the same visual class.
pub const TASK_EDIT_BRANCH_INVALID_BORDER: Hsla = RIGHT_PANEL_TASK_ERROR_COLOR;
/// Width of the error border around the branch input (px).
pub const TASK_EDIT_BRANCH_INVALID_BORDER_W: f32 = 1.0;
/// Corner radius matched to the embedded `TextInput` so the error
/// border doesn't show as a square halo around a rounded widget (px).
pub const TASK_EDIT_BRANCH_INVALID_RADIUS: f32 = 4.0;
/// Vertical height of the status-pill trigger button. Matches the
/// `gpui_component::Button::xsmall()` baseline so the pill aligns
/// flush with the duration / failure cells in the same row.
pub const RIGHT_PANEL_STATUS_PILL_HEIGHT_PX: f32 = 20.0;
/// Horizontal padding inside the pill — slightly wider than the
/// default xsmall padding so the chevron `▾` has room to breathe and
/// the state label stays visually centered.
pub const RIGHT_PANEL_STATUS_PILL_PADDING_X_PX: f32 = 8.0;
/// Gap between the state label and the trailing chevron / between
/// adjacent inline children when callers compose the pill manually.
pub const RIGHT_PANEL_STATUS_PILL_GAP_PX: f32 = 4.0;
/// Corner radius for the pill background. A small radius keeps the
/// row's vertical rhythm intact while still reading as a button.
pub const RIGHT_PANEL_STATUS_PILL_RADIUS_PX: f32 = 4.0;
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
/// Default fill behind a `Badge`.
pub const BADGE_BG: Hsla = hsla(0.0, 0.0, 0.18, 1.0);
/// Default border around a `Badge`.
pub const BADGE_BORDER: Hsla = hsla(0.0, 0.0, 0.30, 1.0);
/// Default text color inside a `Badge`.
pub const BADGE_TEXT: Hsla = hsla(0.0, 0.0, 0.85, 1.0);
/// Corner radius (px). Slightly rounded so the badge reads as one
/// unit but not pill-shaped.
pub const BADGE_RADIUS: f32 = 3.0;
/// Horizontal padding inside a `Badge` (px).
pub const BADGE_PAD_X: f32 = 5.0;
/// Vertical padding inside a `Badge` (px).
pub const BADGE_PAD_Y: f32 = 1.0;
/// Font size inside a `Badge` (px). Slightly smaller than body text.
pub const BADGE_FONT_SIZE: f32 = 11.0;
/// Default color for `ui::Divider`. Sits ~20% lighter than
/// `DOCK_BG` (`l = 0.12`) so a 1px line stays subtle but readable
/// against dock chrome; callers that want a louder separator pass
/// `MUTED_TEXT` or a custom value via `Divider::color`.
pub const DIVIDER_DEFAULT_COLOR: Hsla = hsla(0.0, 0.0, 0.32, 1.0);
/// Inset margin applied when `Divider::inset()` is set (px).
pub const DIVIDER_INSET: f32 = 6.0;
/// Dash + gap lengths used by `Divider::horizontal_dashed()` and
/// `Divider::vertical_dashed()` (px).
pub const DIVIDER_DASH_LEN: f32 = 4.0;
pub const DIVIDER_DASH_GAP: f32 = 2.0;
/// `< 50% utilization` — operational. Also the green status pill.
pub const GAUGE_GREEN: Hsla = hsla(135.0, 0.59, 0.49, 1.0);
/// `50% ≤ utilization < 80%` — degraded. Also the minor-incident
/// pill.
pub const GAUGE_YELLOW: Hsla = hsla(50.0, 1.0, 0.52, 1.0);
/// `≥ 80% utilization` — saturated. Also the critical-incident pill.
pub const GAUGE_RED: Hsla = hsla(4.0, 1.0, 0.62, 1.0);
/// Major-incident pill (between minor and critical). Distinct from
/// `GAUGE_YELLOW` so the eye can tell "yellow yellow yellow ORANGE
/// red" apart at a glance.
pub const STATUS_ORANGE: Hsla = hsla(33.0, 1.0, 0.52, 1.0);
/// Bar height (px) for the 5-hour / 7-day gauges. 8 px reads as a
/// "ribbon" rather than a thick block — same visual weight as
/// macOS download progress bars.
pub const GAUGE_BAR_HEIGHT: f32 = 8.0;
/// Corner radius for the gauge bar's outer rounded rectangle. Half
/// the height — pure pill shape.
pub const GAUGE_BAR_RADIUS: f32 = 4.0;
/// Track color (the unfilled portion of the gauge). Sits at the
/// same lightness as `DIVIDER_DEFAULT_COLOR` so the gauge background
/// melts into the dock chrome but the filled portion still pops.
pub const GAUGE_TRACK_BG: Hsla = hsla(0.0, 0.0, 0.18, 1.0);
/// Vertical padding (px) of the status row.
pub const STATUS_PILL_PAD_Y: f32 = 4.0;
/// Diameter (px) of the small dot rendered inside the status row.
pub const STATUS_PILL_DOT_SIZE: f32 = 8.0;
/// Gap (px) between the dot and the label inside the status row.
pub const STATUS_PILL_GAP: f32 = 6.0;
/// Dim text color used for placeholder gauges and the
/// "status unavailable" pill. Uses the same lightness as the
/// existing label color but at a lower alpha to communicate
/// "we don't know".
pub const RIGHT_PANEL_DIM_TEXT: Hsla = hsla(0.0, 0.0, 0.55, 1.0);
/// Background tint for the "user + model" (default) invocation badge.
/// Green leans approachable — the most common skill state.
pub const SKILL_BADGE_BOTH_BG: Hsla = hsla(140.0, 0.50, 0.40, 0.18);
pub const SKILL_BADGE_BOTH_TEXT: Hsla = hsla(140.0, 0.55, 0.75, 1.0);
/// User-only — the user can invoke but the model can't auto-pick.
pub const SKILL_BADGE_USER_ONLY_BG: Hsla = hsla(210.0, 0.55, 0.45, 0.18);
pub const SKILL_BADGE_USER_ONLY_TEXT: Hsla = hsla(210.0, 0.55, 0.78, 1.0);
/// Model-only — set up to be auto-invoked but hidden from the user.
pub const SKILL_BADGE_MODEL_ONLY_BG: Hsla = hsla(280.0, 0.50, 0.45, 0.18);
pub const SKILL_BADGE_MODEL_ONLY_TEXT: Hsla = hsla(280.0, 0.50, 0.80, 1.0);
/// Disabled — both flags off, the skill is dormant.
pub const SKILL_BADGE_DISABLED_BG: Hsla = hsla(0.0, 0.0, 0.30, 0.30);
pub const SKILL_BADGE_DISABLED_TEXT: Hsla = hsla(0.0, 0.0, 0.55, 1.0);
/// "📎 N" chip background — surfaces auxiliary file presence.
pub const SKILL_AUX_CHIP_BG: Hsla = hsla(0.0, 0.0, 0.20, 0.85);
pub const SKILL_AUX_CHIP_TEXT: Hsla = hsla(0.0, 0.0, 0.78, 1.0);
/// Section heading colour ("Project" / "Personal").
pub const SKILL_SECTION_HEADER_TEXT: Hsla = hsla(0.0, 0.0, 0.62, 1.0);
/// Skill name (mono) colour — same brightness as RIGHT_PANEL body.
pub const SKILL_NAME_TEXT: Hsla = hsla(0.0, 0.0, 0.95, 1.0);
/// Description / metadata text — slightly dimmer than the name.
pub const SKILL_META_TEXT: Hsla = hsla(0.0, 0.0, 0.72, 1.0);
/// Hover background for skill rows.
pub const SKILL_ROW_HOVER_BG: Hsla = hsla(0.0, 0.0, 0.18, 1.0);
pub const SKILL_ROW_RADIUS: f32 = 4.0;
pub const SKILL_ROW_PAD_X: f32 = 8.0;
pub const SKILL_ROW_PAD_Y: f32 = 4.0;
pub const SKILL_ROW_GAP: f32 = 4.0;
/// Vertical padding for skill rows rendered inside a plugin
/// accordion. Smaller than `SKILL_ROW_PAD_Y` so the group reads as
/// a dense list rather than another full-size section.
pub const SKILL_PLUGIN_ROW_PAD_Y: f32 = 2.0;
/// Extra left padding for plugin-scope skill rows. Plugin rows sit
/// under a per-plugin sub-header; this indent makes the
/// header → skill hierarchy obvious without drawing rules.
pub const SKILL_PLUGIN_INDENT: f32 = 24.0;
pub const SKILL_HEADER_GAP: f32 = 6.0;
pub const SKILL_SECTION_GAP: f32 = 8.0;
/// Badge / chip metrics. Shared across the four invocation badges +
/// the aux chip so they line up at the trailing edge.
pub const SKILL_BADGE_FONT_SIZE: f32 = 10.0;
pub const SKILL_BADGE_PAD_X: f32 = 5.0;
pub const SKILL_BADGE_PAD_Y: f32 = 1.0;
pub const SKILL_BADGE_RADIUS: f32 = 3.0;
/// Empty-state helper text colour.
pub const SKILL_EMPTY_TEXT: Hsla = hsla(0.0, 0.0, 0.50, 1.0);
/// Vertical spacing between MCP server rows. Same value as the Skills
/// tab so the two tabs read as belonging to the same panel.
pub const MCP_ROW_GAP: f32 = 4.0;
/// Vertical gap between the section header and the first row inside it.
pub const MCP_SECTION_GAP: f32 = 8.0;
/// Horizontal gap between elements in the row's main line
/// (indicator / transport / command-preview).
pub const MCP_HEADER_GAP: f32 = 6.0;
/// Indicator dot diameter (px). Clickable hit-target — keep ≥ 8.
pub const MCP_INDICATOR_SIZE: f32 = 8.0;
/// Indicator colours — one for each toggle state + malformed flag.
pub const MCP_INDICATOR_ENABLED: Hsla = hsla(130.0, 0.65, 0.50, 1.0);
pub const MCP_INDICATOR_DISABLED: Hsla = hsla(0.0, 0.0, 0.40, 1.0);
pub const MCP_INDICATOR_MALFORMED: Hsla = hsla(14.0, 0.70, 0.55, 1.0);
/// Hover background for a row — reuses the Skills tab value so both
/// tabs feel identical under the cursor.
pub const MCP_ROW_HOVER_BG: Hsla = hsla(0.0, 0.0, 1.0, 0.04);
/// Section heading text colour.
pub const MCP_SECTION_HEADER_TEXT: Hsla = hsla(0.0, 0.0, 0.62, 1.0);
/// Body text colour for the command/url preview line.
pub const MCP_ROW_BODY_TEXT: Hsla = hsla(0.0, 0.0, 0.70, 1.0);
/// Args / env / headers second-line colour — slightly dimmer than the
/// primary preview so it sits visually below the command line.
pub const MCP_ROW_META_TEXT: Hsla = hsla(0.0, 0.0, 0.55, 1.0);
/// Empty-state helper text colour. Mirrors `SKILL_EMPTY_TEXT`.
pub const MCP_EMPTY_TEXT: Hsla = hsla(0.0, 0.0, 0.50, 1.0);
/// Transport label badge — neutral tint + small radius.
pub const MCP_TRANSPORT_BADGE_BG: Hsla = hsla(0.0, 0.0, 1.0, 0.06);
pub const MCP_TRANSPORT_BADGE_TEXT: Hsla = hsla(0.0, 0.0, 0.75, 1.0);
pub const MCP_BADGE_FONT_SIZE: f32 = 10.0;
pub const MCP_BADGE_PAD_X: f32 = 5.0;
pub const MCP_BADGE_PAD_Y: f32 = 1.0;
pub const MCP_BADGE_RADIUS: f32 = 3.0;
/// Malformed chip — same shape as the transport badge, warning hue.
pub const MCP_MALFORMED_BADGE_BG: Hsla = hsla(14.0, 0.50, 0.40, 0.30);
pub const MCP_MALFORMED_BADGE_TEXT: Hsla = hsla(14.0, 0.70, 0.85, 1.0);
/// Disabled chip — dimmer text shown alongside `○` indicator.
pub const MCP_DISABLED_BADGE_TEXT: Hsla = hsla(0.0, 0.0, 0.45, 1.0);
