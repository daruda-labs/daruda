//! Workspace popup-menu item builders and the `+` new-tab dropdown.

use crate::ui::{PopupMenu, PopupMenuItem};

use super::Workspace;

/// Builds a workspace-scoped `PopupMenuItem` that upgrades the weak
/// workspace reference and runs `f`. `PopupMenu` dismisses itself after
/// a confirmed click (`PopupMenu::confirm` always calls
/// `self.dismiss(&Cancel, ...)` post-handler — see
/// `gpui_component::menu::popup_menu`), so no `close_context_menu` call
/// is needed here.
pub(in crate::workspace) fn ws_popup_menu_item(
    ws: gpui::WeakEntity<Workspace>,
    label: impl Into<gpui::SharedString>,
    disabled: bool,
    f: impl Fn(&mut Workspace, &mut gpui::Window, &mut gpui::Context<Workspace>) + 'static,
) -> PopupMenuItem {
    PopupMenuItem::new(label)
        .on_click(move |_, window, app| {
            if let Some(w) = ws.upgrade() {
                w.update(app, |this, cx| f(this, window, cx));
            }
        })
        .disabled(disabled)
}

/// Builds a `PopupMenuItem` that writes `text` to the system clipboard.
/// PopupMenu self-dismisses, so there is no workspace state to touch.
pub(in crate::workspace) fn ws_popup_clipboard_item(
    label: impl Into<gpui::SharedString>,
    text: String,
) -> PopupMenuItem {
    PopupMenuItem::new(label).on_click(move |_, _, app| {
        app.write_to_clipboard(gpui::ClipboardItem::new_string(text.clone()));
    })
}

/// `true` when the `+` menu lists agent entries flat; `false` when it should
/// fold them into a `New Agent Chat` submenu (too many to list inline).
pub(super) fn agent_menu_is_flat(agent_count: usize) -> bool {
    agent_count <= crate::ui::theme::AGENT_MENU_FLAT_MAX
}

/// Build the `+` tab-add dropdown: New Terminal, then one agent-chat entry per
/// configured agent — flat when `agents.len() <= AGENT_MENU_FLAT_MAX`, else
/// folded into a `New Agent Chat` submenu. All handlers dispatch into
/// `Workspace` (one-way data flow) and wrap in `mutate_durable_in` so the new
/// tab is persisted. `agents` is `(id, name)` — remote-ness is resolved from
/// the lane at connect time (`resolve_session_command`), not the agent
/// picked here, so every catalog entry is always selectable.
pub(super) fn build_new_tab_menu(
    menu: PopupMenu,
    ws: &gpui::WeakEntity<Workspace>,
    agents: &[(String, String)],
    window: &mut gpui::Window,
    cx: &mut gpui::Context<PopupMenu>,
) -> PopupMenu {
    // New Terminal.
    let menu = {
        let ws = ws.clone();
        menu.item(
            PopupMenuItem::new(crate::surface::strings::common::new_terminal()).on_click(
                move |_, window, app| {
                    if let Some(w) = ws.upgrade() {
                        w.update(app, |this, cx| {
                            this.mutate_durable_in(window, cx, |ws, w, cx| ws.add_tab(w, cx));
                        });
                    }
                },
            ),
        )
    }
    .separator();

    if agent_menu_is_flat(agents.len()) {
        // Flat: New {name} Chat per agent.
        agents.iter().fold(menu, |m, (id, name)| {
            let ws = ws.clone();
            let agent_id = id.clone();
            let label = crate::surface::strings::common::new_agent_chat_named(name);
            m.item(
                PopupMenuItem::new(label)
                    .icon(crate::ui::agent_menu_icon(
                        daruda_config::agent::icon::icon_for_agent(id),
                    ))
                    .on_click(move |_, window, app| {
                        if let Some(w) = ws.upgrade() {
                            let agent_id = agent_id.clone();
                            w.update(app, |this, cx| {
                                this.mutate_durable_in(window, cx, |ws, w, cx| {
                                    ws.open_agent_chat_pane_with_agent(agent_id, w, cx)
                                });
                            });
                        }
                    }),
            )
        })
    } else {
        // Submenu: New Agent Chat ▸ { agent display name per item }.
        let agents: Vec<(String, String)> = agents.to_vec();
        let ws = ws.clone();
        menu.submenu(
            crate::surface::strings::common::new_agent_chat(),
            window,
            cx,
            move |sub, _w, _c| {
                agents.iter().fold(sub, |m, (id, name)| {
                    let ws = ws.clone();
                    let agent_id = id.clone();
                    m.item(
                        PopupMenuItem::new(gpui::SharedString::from(name.clone()))
                            .icon(crate::ui::agent_menu_icon(
                                daruda_config::agent::icon::icon_for_agent(id),
                            ))
                            .on_click(move |_, window, app| {
                                if let Some(w) = ws.upgrade() {
                                    let agent_id = agent_id.clone();
                                    w.update(app, |this, cx| {
                                        this.mutate_durable_in(window, cx, |ws, w, cx| {
                                            ws.open_agent_chat_pane_with_agent(agent_id, w, cx)
                                        });
                                    });
                                }
                            }),
                    )
                })
            },
        )
    }
}

/// Menu data is captured with the tab cell, before the menu can open.
pub(super) struct TabMenuData {
    index: usize,
    tab_id: u64,
    tab_count: usize,
    is_orchestrator: bool,
    abs_str: Option<String>,
    rel_path: Option<String>,
}

impl TabMenuData {
    pub(super) fn new(
        index: usize,
        tab_id: u64,
        tab_count: usize,
        is_orchestrator: bool,
        file_path: Option<&std::path::Path>,
        worktree_root: Option<&std::path::Path>,
    ) -> Self {
        Self {
            index,
            tab_id,
            tab_count,
            is_orchestrator,
            abs_str: file_path.map(|p| p.to_string_lossy().into_owned()),
            rel_path: file_path.and_then(|p| {
                worktree_root.and_then(|root| {
                    p.strip_prefix(root)
                        .ok()
                        .map(|r| r.to_string_lossy().into_owned())
                })
            }),
        }
    }
}

pub(super) fn tab_context_menu_items(
    ws: &gpui::WeakEntity<Workspace>,
    data: &TabMenuData,
) -> Vec<PopupMenuItem> {
    use crate::workspace::main_area::pane_tree::SplitDirection;
    use crate::workspace::main_area::tab_ops::NewPaneKind;

    let TabMenuData {
        index: i,
        tab_id,
        tab_count,
        is_orchestrator,
        abs_str,
        rel_path,
    } = data;
    let (i, tab_id, tab_count, is_orchestrator) = (*i, *tab_id, *tab_count, *is_orchestrator);
    let is_file = abs_str.is_some();
    let is_last = i + 1 >= tab_count;
    use crate::surface::strings as s;

    let mut items: Vec<PopupMenuItem> = Vec::new();
    if !is_orchestrator {
        items.push(ws_popup_menu_item(
            ws.clone(),
            s::ctx::rename_tab(),
            false,
            move |this, win, cx| this.open_rename_tab_dialog(tab_id, win, cx),
        ));
        items.push(PopupMenuItem::separator());
    }
    items.extend([
        ws_popup_menu_item(
            ws.clone(),
            s::common::close_tab(),
            false,
            move |this, win, cx| {
                this.mutate_durable_in(win, cx, |ws, win, cx| {
                    ws.request_close_tab(i, win, cx);
                });
            },
        ),
        ws_popup_menu_item(
            ws.clone(),
            s::ctx::close_other_tabs(),
            tab_count <= 1,
            move |this, win, cx| {
                this.mutate_durable_in(win, cx, |ws, win, cx| {
                    ws.close_other_tabs(i, win, cx);
                });
            },
        ),
        ws_popup_menu_item(
            ws.clone(),
            s::ctx::close_tabs_to_right(),
            is_last,
            move |this, win, cx| {
                this.mutate_durable_in(win, cx, |ws, win, cx| {
                    ws.close_tabs_to_right(i, win, cx);
                });
            },
        ),
        PopupMenuItem::separator(),
        ws_popup_menu_item(
            ws.clone(),
            s::common::move_tab_left(),
            i == 0,
            move |this, _win, cx| {
                this.mutate_durable(cx, |ws, cx| ws.move_tab_left(i, cx));
            },
        ),
        ws_popup_menu_item(
            ws.clone(),
            s::common::move_tab_right(),
            is_last,
            move |this, _win, cx| {
                this.mutate_durable(cx, |ws, cx| ws.move_tab_right(i, cx));
            },
        ),
    ]);

    // Split + New Tab — terminal tabs only.
    if !is_file {
        items.extend([
            PopupMenuItem::separator(),
            ws_popup_menu_item(
                ws.clone(),
                s::common::split_terminal_horizontal(),
                false,
                move |this, win, cx| {
                    this.split_tab_from_menu(
                        i,
                        NewPaneKind::Terminal,
                        SplitDirection::Horizontal,
                        win,
                        cx,
                    );
                },
            ),
            ws_popup_menu_item(
                ws.clone(),
                s::common::split_terminal_vertical(),
                false,
                move |this, win, cx| {
                    this.split_tab_from_menu(
                        i,
                        NewPaneKind::Terminal,
                        SplitDirection::Vertical,
                        win,
                        cx,
                    );
                },
            ),
            PopupMenuItem::separator(),
            ws_popup_menu_item(
                ws.clone(),
                s::common::split_agent_chat_horizontal(),
                false,
                move |this, win, cx| {
                    this.split_tab_from_menu(
                        i,
                        NewPaneKind::AgentChat,
                        SplitDirection::Horizontal,
                        win,
                        cx,
                    );
                },
            ),
            ws_popup_menu_item(
                ws.clone(),
                s::common::split_agent_chat_vertical(),
                false,
                move |this, win, cx| {
                    this.split_tab_from_menu(
                        i,
                        NewPaneKind::AgentChat,
                        SplitDirection::Vertical,
                        win,
                        cx,
                    );
                },
            ),
            PopupMenuItem::separator(),
            ws_popup_menu_item(
                ws.clone(),
                crate::surface::strings::common::new_tab(),
                false,
                |this, win, cx| {
                    this.mutate_durable_in(win, cx, |ws, win, cx| {
                        ws.add_tab(win, cx);
                    });
                },
            ),
        ]);
    }

    // File-pane-specific items
    if is_file {
        items.push(PopupMenuItem::separator());
        if let Some(abs) = abs_str.clone() {
            items.push(ws_popup_clipboard_item(s::ctx::copy_file_path(), abs));
        }
        if let Some(rel) = rel_path.clone() {
            items.push(ws_popup_clipboard_item(s::ctx::copy_relative_path(), rel));
        }
        items.push(ws_popup_menu_item(
            ws.clone(),
            s::ctx::close_file_viewer(),
            false,
            move |this, win, cx| {
                this.mutate_durable_in(win, cx, |ws, win, cx| {
                    ws.request_close_tab(i, win, cx);
                });
            },
        ));
    }

    items
}
