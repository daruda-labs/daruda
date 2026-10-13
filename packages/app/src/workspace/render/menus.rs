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
