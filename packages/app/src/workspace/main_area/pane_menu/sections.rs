use gpui::SharedString;

use crate::surface::strings as s;
use crate::workspace::main_area::pane_tree::{PaneId, SplitDirection};
use crate::workspace::main_area::tab_ops::NewPaneKind;
use daruda_content::link_target::LocalKind;

use super::context::{
    ClickLink, LaneAccess, PaneMenuContext, PaneMenuKind, PaneRole, SEND_SELECTION_LIMIT,
};
use super::spec::{
    Activate, ItemState, MenuEntry, disabled_item, item, normalize_entries, state_if,
};

trait PaneMenuSource {
    fn head(ctx: &PaneMenuContext) -> Vec<MenuEntry>;
}

struct TerminalMenu;
struct AgentChatMenu;
struct DefaultMenu;

impl PaneMenuSource for TerminalMenu {
    fn head(ctx: &PaneMenuContext) -> Vec<MenuEntry> {
        let mut entries = Vec::new();

        if let PaneMenuKind::Terminal { exited: true, .. } = ctx.kind {
            let pane_id = ctx.pane_id;
            entries.push(item(
                s::ctx::restart_shell(),
                ItemState::Enabled,
                Activate::Op(Box::new(move |ws, window, cx| {
                    ws.restart_terminal_pane(pane_id, window, cx);
                })),
            ));
            entries.push(MenuEntry::Separator);
        }

        entries.extend(link_entries(ctx));

        entries.push(item(
            s::menu::copy(),
            state_if(ctx.selection.is_some(), None),
            Activate::Action(Box::new(daruda_terminal::view::Copy)),
        ));
        entries.push(item(
            s::menu::paste(),
            ItemState::Enabled,
            Activate::Action(Box::new(daruda_terminal::view::Paste)),
        ));
        entries.push(item(
            s::menu::select_all(),
            ItemState::Enabled,
            Activate::Action(Box::new(daruda_terminal::view::SelectAll)),
        ));
        entries.push(MenuEntry::Separator);
        entries.push(item(
            s::menu::clear_buffer(),
            ItemState::Enabled,
            Activate::Action(Box::new(daruda_terminal::view::ClearBuffer)),
        ));
        entries.push(item(
            s::menu::copy_last_command_output(),
            ItemState::Enabled,
            Activate::Action(Box::new(daruda_terminal::view::CopyLastCommandOutput)),
        ));
        entries.push(item(
            s::menu::scroll_to_bottom(),
            ItemState::Enabled,
            Activate::Action(Box::new(daruda_terminal::view::ScrollToBottom)),
        ));

        entries.extend(send_selection_entries(
            ctx,
            s::ctx::send_selection_to_agent_chat(),
        ));

        entries.push(MenuEntry::Separator);
        match &ctx.kind {
            PaneMenuKind::Terminal {
                annotation_range, ..
            } => {
                let pane_id = ctx.pane_id;
                if let Some(range) = *annotation_range {
                    entries.push(item(
                        s::terminal::annotation_action_add(),
                        ItemState::Enabled,
                        Activate::Op(Box::new(move |ws, window, cx| {
                            ws.open_annotation_dialog_for_create(pane_id, range, window, cx);
                        })),
                    ));
                } else {
                    entries.push(disabled_item(
                        s::terminal::annotation_action_add(),
                        Some(s::terminal::annotation_action_add_disabled_tooltip().into()),
                    ));
                }
            }
            PaneMenuKind::AgentChat { .. } | PaneMenuKind::Other => {}
        }
        if let Some(mark_id) = ctx.click.as_ref().and_then(|click| click.annotation) {
            let pane_id = ctx.pane_id;
            entries.push(item(
                s::terminal::annotation_action_delete(),
                ItemState::Enabled,
                Activate::Op(Box::new(move |ws, _window, cx| {
                    ws.remove_annotation(pane_id, mark_id, cx);
                })),
            ));
        }

        entries
    }
}

impl PaneMenuSource for AgentChatMenu {
    fn head(ctx: &PaneMenuContext) -> Vec<MenuEntry> {
        let mut entries = link_entries(ctx);

        // Clipboard-by-value, not an action: the chat's selection is cleared
        // by the left-click that confirms the item, so the text has to come
        // from the snapshot.
        entries.push(match ctx.selection.as_ref() {
            Some(text) => item(
                s::menu::copy(),
                ItemState::Enabled,
                Activate::Clipboard(text.to_string()),
            ),
            None => disabled_item(s::menu::copy(), None),
        });

        entries.extend(send_selection_entries(
            ctx,
            s::ctx::send_selection_to_terminal(),
        ));

        entries.push(MenuEntry::Separator);
        if matches!(&ctx.kind, PaneMenuKind::AgentChat { busy: true }) {
            let pane_id = ctx.pane_id;
            entries.push(item(
                s::common::btn_stop(),
                ItemState::Enabled,
                Activate::Op(Box::new(move |ws, _window, cx| {
                    ws.cancel_agent_turn_if_active(pane_id, cx);
                })),
            ));
        }
        let pane_id = ctx.pane_id;
        entries.push(item(
            s::menu::scroll_to_bottom(),
            ItemState::Enabled,
            Activate::Op(Box::new(move |ws, _window, cx| {
                ws.scroll_agent_chat_to_bottom(pane_id, cx);
            })),
        ));

        entries
    }
}

impl PaneMenuSource for DefaultMenu {
    fn head(_ctx: &PaneMenuContext) -> Vec<MenuEntry> {
        Vec::new()
    }
}

pub(super) fn compose(ctx: &PaneMenuContext) -> Vec<MenuEntry> {
    let mut entries = match &ctx.kind {
        PaneMenuKind::Terminal { .. } => TerminalMenu::head(ctx),
        PaneMenuKind::AgentChat { .. } => AgentChatMenu::head(ctx),
        PaneMenuKind::Other => DefaultMenu::head(ctx),
    };
    if !entries.is_empty() {
        entries.push(MenuEntry::Separator);
    }
    entries.extend(common_tail(ctx));
    normalize_entries(entries)
}

fn common_tail(ctx: &PaneMenuContext) -> Vec<MenuEntry> {
    let split_state = match ctx.lane {
        LaneAccess::Accessible => ItemState::Enabled,
        LaneAccess::Inaccessible => ItemState::Disabled(Some(s::ctx::lane_inaccessible().into())),
    };

    let mut entries = vec![
        split_item(
            s::common::split_terminal_horizontal(),
            split_state.clone(),
            NewPaneKind::Terminal,
            SplitDirection::Horizontal,
        ),
        split_item(
            s::common::split_terminal_vertical(),
            split_state.clone(),
            NewPaneKind::Terminal,
            SplitDirection::Vertical,
        ),
        split_item(
            s::common::split_agent_chat_horizontal(),
            split_state.clone(),
            NewPaneKind::AgentChat,
            SplitDirection::Horizontal,
        ),
        split_item(
            s::common::split_agent_chat_vertical(),
            split_state,
            NewPaneKind::AgentChat,
            SplitDirection::Vertical,
        ),
    ];

    if let PaneRole::InSplit { zoomed } = &ctx.role {
        let pane_id = ctx.pane_id;
        let label = if *zoomed {
            s::ctx::unzoom_pane()
        } else {
            s::ctx::zoom_pane()
        };
        entries.push(MenuEntry::Separator);
        entries.push(item(
            label,
            ItemState::Enabled,
            Activate::Op(Box::new(move |ws, _window, cx| {
                ws.toggle_zoom_pane(pane_id, cx);
            })),
        ));
    }

    let pane_id = ctx.pane_id;
    let close_label = match &ctx.role {
        PaneRole::Solo => s::common::close_tab(),
        PaneRole::InSplit { .. } => s::common::close_pane(),
    };
    entries.push(MenuEntry::Separator);
    entries.push(item(
        close_label,
        ItemState::Enabled,
        Activate::Op(Box::new(move |ws, window, cx| {
            ws.request_close_pane(pane_id, window, cx);
        })),
    ));

    entries
}

fn split_item(
    label: String,
    state: ItemState,
    kind: NewPaneKind,
    direction: SplitDirection,
) -> MenuEntry {
    item(
        label,
        state,
        Activate::Op(Box::new(move |ws, window, cx| {
            ws.mutate_durable_in(window, cx, |ws, window, cx| {
                ws.split_focused_pane_kind(kind, direction, window, cx);
            });
        })),
    )
}

/// The entries the link under the click earns, plus the separator that closes
/// them off. Empty when the click landed on no link.
///
/// One builder for both menus: what a link offers follows from what it *is*
/// ([`ClickLink`]), not from which pane it was read out of.
fn link_entries(ctx: &PaneMenuContext) -> Vec<MenuEntry> {
    let Some(link) = ctx.click.as_ref().and_then(|click| click.link.clone()) else {
        return Vec::new();
    };
    let pane_id = ctx.pane_id;
    let mut entries = Vec::new();

    match &link {
        ClickLink::File { url, path, kind } => {
            let path = path.clone();
            match kind {
                // Text has two openers: the viewer, and the user's editor.
                LocalKind::Text => {
                    let url = url.clone();
                    entries.push(item(
                        s::ctx::open_link_in_file_view(),
                        ItemState::Enabled,
                        Activate::Op(Box::new(move |ws, window, cx| {
                            ws.open_pane_link(pane_id, &url, window, cx);
                        })),
                    ));
                    entries.push(item(
                        s::ctx::open_link_externally(),
                        ItemState::Enabled,
                        Activate::Op(Box::new(move |ws, _window, cx| {
                            ws.open_pane_file_externally(pane_id, path.clone(), cx);
                        })),
                    ));
                }
                // An image, a PDF, a directory: the code editor is the wrong
                // tool, so only the OS default handler is offered.
                LocalKind::Image | LocalKind::Binary | LocalKind::Directory => {
                    entries.push(item(
                        s::ctx::open_link_externally(),
                        ItemState::Enabled,
                        Activate::Op(Box::new(move |ws, _window, cx| {
                            ws.open_path_with_system_default(path.clone(), cx);
                        })),
                    ));
                }
                // Gone: nothing opens it; the address is still worth copying.
                LocalKind::Missing => {}
            }
        }
        ClickLink::Web { url } => {
            let url = url.clone();
            entries.push(item(
                s::ctx::open_link(),
                ItemState::Enabled,
                Activate::Op(Box::new(move |ws, _window, cx| {
                    ws.open_pane_menu_link(url.clone(), cx);
                })),
            ));
        }
        // Nothing can open it; its text is still worth having.
        ClickLink::Opaque { .. } => {}
    }

    entries.push(item(
        s::ctx::copy_link_address(),
        ItemState::Enabled,
        Activate::Clipboard(link.url().to_owned()),
    ));
    entries.push(MenuEntry::Separator);
    entries
}

fn send_selection_entries(ctx: &PaneMenuContext, label: String) -> Vec<MenuEntry> {
    let Some(text) = ctx.selection.clone() else {
        return Vec::new();
    };
    if ctx.send_targets.is_empty() {
        return Vec::new();
    }

    let state = if text.len() > SEND_SELECTION_LIMIT {
        ItemState::Disabled(Some(s::ctx::selection_too_large().into()))
    } else {
        ItemState::Enabled
    };

    if ctx.send_targets.len() == 1 {
        let target = ctx.send_targets[0].pane_id;
        return vec![MenuEntry::Separator, send_item(label, state, target, text)];
    }

    let entries = ctx
        .send_targets
        .iter()
        .map(|target| {
            send_item(
                target.label.clone(),
                state.clone(),
                target.pane_id,
                text.clone(),
            )
        })
        .collect();

    vec![
        MenuEntry::Separator,
        MenuEntry::Submenu {
            label: label.into(),
            entries,
        },
    ]
}

fn send_item(
    label: impl Into<SharedString>,
    state: ItemState,
    target: PaneId,
    text: SharedString,
) -> MenuEntry {
    item(
        label,
        state,
        Activate::Op(Box::new(move |ws, window, cx| {
            if !ws.send_pane_selection_to(target, text.clone(), window, cx) {
                ws.report_pane_menu_send_failed(cx);
            }
        })),
    )
}

#[cfg(test)]
mod tests;
