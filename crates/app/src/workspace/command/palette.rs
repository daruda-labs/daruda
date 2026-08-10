//! Command palette — Cmd+Shift+P fuzzy action search overlay.
//!
//! Renders a centered input box at the top of the workspace. Typing
//! filters the action list; Enter executes the focused action; Escape
//! closes. Matches are scored by substring position (earlier = better).

use crate::ui::theme;
use gpui::{
    App, IntoElement, MouseButton, MouseDownEvent, RenderOnce, SharedString, Window, div,
    prelude::*, px,
};
use std::rc::Rc;

/// A single entry in the command palette.
#[derive(Clone)]
pub(in crate::workspace) struct PaletteEntry {
    /// Action identifier (snake_case, matches action_map).
    pub id: &'static str,
    /// Human-readable label shown in the palette.
    pub label: &'static str,
    /// Keyboard shortcut hint (displayed right-aligned).
    pub shortcut: &'static str,
}

/// All available palette entries. Ordered by usage frequency.
///
/// Per-section settings entries use the dotted form
/// `open_settings.<slug>` matching the keybinding-override syntax in
/// `surface::action_map`. The bare `open_settings` id resolves to the
/// default page (General) so an empty-arg keybinding still works.
pub(in crate::workspace) const PALETTE_ENTRIES: &[PaletteEntry] = &[
    PaletteEntry {
        id: "toggle_lane_switcher",
        label: "Switch Lane…",
        shortcut: "Cmd+P",
    },
    PaletteEntry {
        id: "run_flow",
        label: "Run Flow…",
        shortcut: "",
    },
    PaletteEntry {
        id: "validate_flow",
        label: "Check Flow…",
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings",
        label: "Settings…",
        shortcut: "Cmd+,",
    },
    PaletteEntry {
        id: "open_settings.general",
        label: "Settings: General",
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.font",
        label: "Settings: Font",
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.cursor",
        label: "Settings: Cursor",
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.shell",
        label: "Settings: Shell",
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.window",
        label: "Settings: Window",
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.terminal",
        label: "Settings: Terminal",
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.dock",
        label: "Settings: Dock",
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.clipboard",
        label: "Settings: Clipboard",
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.notifications",
        label: "Settings: Notifications",
        shortcut: "",
    },
    PaletteEntry {
        id: "open_settings.keymap",
        label: "Settings: Keymap",
        shortcut: "",
    },
    PaletteEntry {
        id: "new_tab",
        label: "New Tab",
        shortcut: "Cmd+T",
    },
    PaletteEntry {
        id: "new_task",
        label: "New Task",
        shortcut: "",
    },
    PaletteEntry {
        id: "new_agent_chat",
        label: "New Agent Chat",
        shortcut: "",
    },
    PaletteEntry {
        id: "edit_task",
        label: "Edit Task…",
        shortcut: "",
    },
    PaletteEntry {
        id: "start_task",
        label: "Start Task…",
        shortcut: "",
    },
    PaletteEntry {
        id: "cancel_task",
        label: "Cancel Task…",
        shortcut: "",
    },
    PaletteEntry {
        id: "reopen_task",
        label: "Reopen Task…",
        shortcut: "",
    },
    PaletteEntry {
        id: "retry_task",
        label: "Retry Task…",
        shortcut: "",
    },
    PaletteEntry {
        id: "delete_task",
        label: "Delete Task…",
        shortcut: "",
    },
    PaletteEntry {
        id: "close_pane",
        label: "Close Pane",
        shortcut: "Cmd+W",
    },
    PaletteEntry {
        id: "close_tab",
        label: "Close Tab",
        shortcut: "",
    },
    PaletteEntry {
        id: "split_right",
        label: "Split Right",
        shortcut: "Cmd+D",
    },
    PaletteEntry {
        id: "split_down",
        label: "Split Down",
        shortcut: "Cmd+Shift+D",
    },
    PaletteEntry {
        id: "next_tab",
        label: "Next Tab",
        shortcut: "Ctrl+Tab",
    },
    PaletteEntry {
        id: "prev_tab",
        label: "Previous Tab",
        shortcut: "Ctrl+Shift+Tab",
    },
    PaletteEntry {
        id: "toggle_left_dock",
        label: "Toggle Left Dock",
        shortcut: "Cmd+B",
    },
    PaletteEntry {
        id: "toggle_bottom_dock",
        label: "Toggle Bottom Panel",
        shortcut: "Cmd+J",
    },
    PaletteEntry {
        id: "toggle_right_dock",
        label: "Toggle Right Dock",
        shortcut: "Cmd+Shift+B",
    },
    PaletteEntry {
        id: "focus_next_pane",
        label: "Focus Next Pane",
        shortcut: "Cmd+]",
    },
    PaletteEntry {
        id: "focus_prev_pane",
        label: "Focus Previous Pane",
        shortcut: "Cmd+[",
    },
    PaletteEntry {
        id: "focus_pane_left",
        label: "Focus Pane Left",
        shortcut: "Cmd+Alt+Left",
    },
    PaletteEntry {
        id: "focus_pane_right",
        label: "Focus Pane Right",
        shortcut: "Cmd+Alt+Right",
    },
    PaletteEntry {
        id: "focus_pane_up",
        label: "Focus Pane Up",
        shortcut: "Cmd+Alt+Up",
    },
    PaletteEntry {
        id: "focus_pane_down",
        label: "Focus Pane Down",
        shortcut: "Cmd+Alt+Down",
    },
    PaletteEntry {
        id: "move_tab_left",
        label: "Move Tab Left",
        shortcut: "",
    },
    PaletteEntry {
        id: "move_tab_right",
        label: "Move Tab Right",
        shortcut: "",
    },
    PaletteEntry {
        id: "copy",
        label: "Copy",
        shortcut: "Cmd+C",
    },
    PaletteEntry {
        id: "paste",
        label: "Paste",
        shortcut: "Cmd+V",
    },
    PaletteEntry {
        id: "select_all",
        label: "Select All",
        shortcut: "Cmd+A",
    },
    PaletteEntry {
        id: "activate_lane_1",
        label: "Activate Lane 1",
        shortcut: "Cmd+Ctrl+1",
    },
    PaletteEntry {
        id: "activate_lane_2",
        label: "Activate Lane 2",
        shortcut: "Cmd+Ctrl+2",
    },
    PaletteEntry {
        id: "activate_lane_3",
        label: "Activate Lane 3",
        shortcut: "Cmd+Ctrl+3",
    },
    PaletteEntry {
        id: "activate_lane_4",
        label: "Activate Lane 4",
        shortcut: "Cmd+Ctrl+4",
    },
    PaletteEntry {
        id: "activate_lane_5",
        label: "Activate Lane 5",
        shortcut: "Cmd+Ctrl+5",
    },
    PaletteEntry {
        id: "activate_lane_6",
        label: "Activate Lane 6",
        shortcut: "Cmd+Ctrl+6",
    },
    PaletteEntry {
        id: "activate_lane_7",
        label: "Activate Lane 7",
        shortcut: "Cmd+Ctrl+7",
    },
    PaletteEntry {
        id: "activate_lane_8",
        label: "Activate Lane 8",
        shortcut: "Cmd+Ctrl+8",
    },
    PaletteEntry {
        id: "activate_lane_9",
        label: "Activate Lane 9",
        shortcut: "Cmd+Ctrl+9",
    },
    PaletteEntry {
        id: "open_folder",
        label: "Open Project\u{2026}",
        shortcut: "Cmd+O",
    },
    PaletteEntry {
        id: "new_group",
        label: "New Group",
        shortcut: "Cmd+Shift+N",
    },
    PaletteEntry {
        id: "rename_project",
        label: "Rename Project\u{2026}",
        shortcut: "Cmd+Shift+R",
    },
    PaletteEntry {
        id: "move_project_to_group",
        label: "Move Project to Group\u{2026}",
        shortcut: "Cmd+Shift+M",
    },
    PaletteEntry {
        id: "close_project",
        label: "Close Project",
        shortcut: "Cmd+Shift+W",
    },
    PaletteEntry {
        id: "show_left_dock_lanes",
        label: "Show Lanes",
        shortcut: "",
    },
    PaletteEntry {
        id: "show_left_dock_git",
        label: "Show Git Changes",
        shortcut: "",
    },
    PaletteEntry {
        id: "show_left_dock_files",
        label: "Show Files",
        shortcut: "",
    },
    PaletteEntry {
        id: "switch_right_panel_usage",
        label: "Right Panel: Usage",
        shortcut: "",
    },
    PaletteEntry {
        id: "switch_right_panel_skills",
        label: "Right Panel: Skills",
        shortcut: "",
    },
    PaletteEntry {
        id: "switch_right_panel_tools",
        label: "Right Panel: Tools",
        shortcut: "",
    },
    PaletteEntry {
        id: "switch_right_panel_tasks",
        label: "Right Panel: Tasks",
        shortcut: "",
    },
    PaletteEntry {
        id: "new_skill",
        label: "Skills: New skill",
        shortcut: "",
    },
    PaletteEntry {
        id: "refresh_git_status",
        label: "Refresh Git Status",
        shortcut: "",
    },
    PaletteEntry {
        id: "files_toggle_hidden",
        label: "Files: Toggle Hidden Files",
        shortcut: "Cmd+Shift+.",
    },
    PaletteEntry {
        id: "files_refresh",
        label: "Files: Refresh",
        shortcut: "",
    },
    PaletteEntry {
        id: "commit_changes",
        label: "Commit Changes",
        shortcut: "",
    },
    PaletteEntry {
        id: "push_changes",
        label: "Push Changes",
        shortcut: "",
    },
    PaletteEntry {
        id: "install_claude_hooks",
        label: "Claude: Install Hook Integration",
        shortcut: "",
    },
    PaletteEntry {
        id: "uninstall_claude_hooks",
        label: "Claude: Uninstall Hook Integration",
        shortcut: "",
    },
    PaletteEntry {
        id: "open_command_history",
        label: "Open Command History",
        shortcut: "Cmd+Shift+H",
    },
    PaletteEntry {
        id: "quit",
        label: "Quit",
        shortcut: "Cmd+Q",
    },
];

/// State for the command palette overlay.
#[derive(Default, Clone)]
pub(in crate::workspace) struct CommandPaletteState {
    pub is_open: bool,
    pub query: String,
    pub focused_index: usize,
}

impl CommandPaletteState {
    pub fn open(&mut self) {
        self.is_open = true;
        self.query.clear();
        self.focused_index = 0;
    }

    pub fn close(&mut self) {
        self.is_open = false;
        self.query.clear();
        self.focused_index = 0;
    }

    pub fn append(&mut self, ch: char) {
        self.query.push(ch);
        self.focused_index = 0;
    }

    pub fn backspace(&mut self) {
        self.query.pop();
        self.focused_index = 0;
    }

    /// Move the focus to a row the mouse named. Clicking is the same
    /// gesture as arrowing there and pressing Enter, so it goes through the
    /// same field rather than a second path to the same decision.
    pub fn focus(&mut self, index: usize) {
        self.focused_index = index;
    }

    pub fn move_up(&mut self) {
        if self.focused_index > 0 {
            self.focused_index -= 1;
        }
    }

    pub fn move_down(&mut self, max: usize) {
        let cap = max.min(theme::PALETTE_MAX_VISIBLE);
        if cap > 0 && self.focused_index < cap - 1 {
            self.focused_index += 1;
        }
    }

    /// Filter entries by fuzzy substring match. Returns indices into
    /// `PALETTE_ENTRIES` sorted by match quality (earlier substring
    /// position wins, then alphabetical).
    pub fn filtered_entries(&self) -> Vec<usize> {
        if self.query.is_empty() {
            return (0..PALETTE_ENTRIES.len()).collect();
        }
        let query_lower = self.query.to_ascii_lowercase();
        let mut matches: Vec<(usize, usize)> = PALETTE_ENTRIES
            .iter()
            .enumerate()
            .filter_map(|(i, entry)| {
                let label_lower = entry.label.to_ascii_lowercase();
                label_lower.find(&query_lower).map(|pos| (i, pos))
            })
            .collect();
        matches.sort_by_key(|(_, pos)| *pos);
        matches.into_iter().map(|(i, _)| i).collect()
    }

    /// Get the action id of the currently focused entry, if any.
    pub fn focused_action_id(&self) -> Option<&'static str> {
        let filtered = self.filtered_entries();
        filtered
            .get(self.focused_index)
            .map(|&i| PALETTE_ENTRIES[i].id)
    }
}

/// GPUI render-once wrapper for the command palette floating overlay.
/// Renders an empty invisible div when the palette is closed.
#[derive(IntoElement)]
pub(in crate::workspace) struct CommandPaletteOverlay {
    pub(in crate::workspace) state: CommandPaletteState,
    #[allow(clippy::type_complexity)]
    pub(in crate::workspace) on_close:
        Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>,
    /// Activate the row at this visible index. `Rc` because every row needs
    /// its own handle to it.
    #[allow(clippy::type_complexity)]
    pub(in crate::workspace) on_pick: Rc<dyn Fn(&usize, &mut Window, &mut App) + 'static>,
}

impl CommandPaletteOverlay {
    pub(in crate::workspace) fn new(
        state: CommandPaletteState,
        on_close: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
        on_pick: impl Fn(&usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            state,
            on_close: Box::new(on_close),
            on_pick: Rc::new(on_pick),
        }
    }
}

/// Full-screen absolute overlay — click-to-dismiss hit target for the
/// command palette.
fn backdrop() -> gpui::Div {
    div().absolute().size_full().top_0().left_0()
}

impl RenderOnce for CommandPaletteOverlay {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        if !self.state.is_open {
            return div().into_any_element();
        }
        let state = self.state;
        let on_close = self.on_close;
        let filtered = state.filtered_entries();

        let t = theme::current(cx);
        let input_border = t.border;
        let query_text = t.text_primary;
        let focused_bg = t.palette_focused_bg;
        let focused_text = t.text_primary;
        let entry_text = t.text_body;
        let shortcut_text = t.text_subtle;
        let empty_text = t.text_subtle;
        let panel_bg = t.palette_bg;
        let panel_border = t.border;

        let input = div()
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .px(px(theme::PALETTE_INPUT_PAD_X))
            .py(px(theme::PALETTE_INPUT_PAD_Y))
            .border_b_1()
            .border_color(input_border)
            .child(
                div()
                    .text_size(px(theme::PALETTE_QUERY_FONT_SIZE))
                    .text_color(query_text)
                    .child(if state.query.is_empty() {
                        SharedString::from("Type a command...")
                    } else {
                        SharedString::from(state.query.clone())
                    }),
            );

        let entries = div()
            .flex()
            .flex_col()
            .max_h(px(theme::PALETTE_MAX_HEIGHT))
            .overflow_hidden()
            .children(
                filtered
                    .iter()
                    .take(theme::PALETTE_MAX_VISIBLE)
                    .enumerate()
                    .map(|(vis_idx, &entry_idx)| {
                        let entry = &PALETTE_ENTRIES[entry_idx];
                        let is_focused = vis_idx == state.focused_index;

                        let on_pick = self.on_pick.clone();
                        div()
                            .cursor_pointer()
                            .on_mouse_down(
                                MouseButton::Left,
                                move |_: &MouseDownEvent, window, cx| {
                                    cx.stop_propagation();
                                    on_pick(&vis_idx, window, cx);
                                },
                            )
                            .hover(|d| d.bg(focused_bg))
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .w_full()
                            .px(px(theme::PALETTE_ENTRY_PAD_X))
                            .py(px(theme::PALETTE_ENTRY_PAD_Y))
                            .text_size(px(theme::PALETTE_ENTRY_FONT_SIZE))
                            // Reserve the same-width transparent border on
                            // unfocused rows so the label does not shift when
                            // the accent rule appears — same idiom as the lane
                            // rows in the left dock.
                            .border_l(px(theme::PALETTE_FOCUS_BORDER_W))
                            .border_color(theme::TRANSPARENT)
                            .when(is_focused, |d| {
                                d.bg(focused_bg)
                                    .text_color(focused_text)
                                    .border_color(theme::PRIMARY)
                            })
                            .when(!is_focused, |d| d.text_color(entry_text))
                            .child(div().child(entry.label))
                            .when(!entry.shortcut.is_empty(), |d| {
                                d.child(
                                    div()
                                        .text_size(px(theme::PALETTE_SHORTCUT_FONT_SIZE))
                                        .text_color(shortcut_text)
                                        .child(entry.shortcut),
                                )
                            })
                    }),
            );

        let no_results = if filtered.is_empty() {
            Some(
                div()
                    .px(px(theme::PALETTE_ENTRY_PAD_X))
                    .py(px(theme::PALETTE_EMPTY_PAD_Y))
                    .text_size(px(theme::PALETTE_ENTRY_FONT_SIZE))
                    .text_color(empty_text)
                    .child("No matching commands"),
            )
        } else {
            None
        };

        let panel = div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .mx_auto()
            .mt(px(theme::PALETTE_TOP_OFFSET))
            .w(px(theme::PALETTE_WIDTH))
            .bg(panel_bg)
            .border_1()
            .border_color(panel_border)
            .rounded(px(theme::PALETTE_RADIUS))
            .shadow_lg()
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
            })
            .child(input)
            .child(entries)
            .when_some(no_results, |el, nr| el.child(nr));

        backdrop()
            .on_mouse_down(MouseButton::Left, on_close)
            .child(panel)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_open_close_lifecycle() {
        let state = CommandPaletteState::default();
        assert!(!state.is_open);
        assert!(state.query.is_empty());

        let mut state = CommandPaletteState {
            query: "old".to_string(),
            ..Default::default()
        };
        state.open();
        assert!(state.is_open);
        assert!(state.query.is_empty());
        assert_eq!(state.focused_index, 0);

        let mut state = CommandPaletteState::default();
        state.open();
        state.append('a');
        // Set focused_index directly to test that close() resets it.
        state.focused_index = 3;
        state.close();
        assert!(!state.is_open);
        assert!(state.query.is_empty());
        assert_eq!(state.focused_index, 0);
    }

    #[test]
    fn palette_filter_cases() {
        let state = CommandPaletteState::default();
        let filtered = state.filtered_entries();
        assert_eq!(filtered.len(), PALETTE_ENTRIES.len());

        let mut state = CommandPaletteState::default();
        state.append('s');
        state.append('p');
        state.append('l');
        state.append('i');
        state.append('t');
        let filtered = state.filtered_entries();
        assert!(
            filtered.len() >= 2,
            "should match Split Right and Split Down"
        );
        for &idx in &filtered {
            assert!(
                PALETTE_ENTRIES[idx]
                    .label
                    .to_ascii_lowercase()
                    .contains("split"),
                "entry {:?} should contain 'split'",
                PALETTE_ENTRIES[idx].label
            );
        }

        let state = CommandPaletteState {
            query: "QUIT".to_string(),
            ..Default::default()
        };
        let filtered = state.filtered_entries();
        assert!(filtered.iter().any(|&i| PALETTE_ENTRIES[i].id == "quit"));

        let state = CommandPaletteState {
            query: "zzzzzzz".to_string(),
            ..Default::default()
        };
        assert!(state.filtered_entries().is_empty());
    }

    #[test]
    fn focused_action_id_returns_correct_entry() {
        let state = CommandPaletteState::default();
        // First entry in unfiltered list.
        let id = state.focused_action_id().unwrap();
        assert_eq!(id, PALETTE_ENTRIES[0].id);
    }

    #[test]
    fn palette_editing_and_focus_movement_cases() {
        let mut state = CommandPaletteState::default();
        for _ in 0..100 {
            state.move_down(3);
        }
        assert_eq!(state.focused_index, 2);

        let mut state = CommandPaletteState::default();
        state.move_up();
        assert_eq!(state.focused_index, 0);

        let mut state = CommandPaletteState::default();
        state.append('a');
        state.append('b');
        state.backspace();
        assert_eq!(state.query, "a");

        let mut state = CommandPaletteState {
            focused_index: 5,
            ..Default::default()
        };
        state.append('x');
        assert_eq!(state.focused_index, 0);
    }

    #[test]
    fn palette_entries_have_unique_ids() {
        let mut ids: Vec<&str> = PALETTE_ENTRIES.iter().map(|e| e.id).collect();
        let len = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), len, "duplicate palette entry IDs found");
    }
}
