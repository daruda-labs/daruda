use gpui::SharedString;

use crate::surface::strings as s;
use crate::workspace::main_area::pane_tree::{PaneId, SplitDirection};
use crate::workspace::main_area::tab_ops::NewPaneKind;

use super::context::{LaneAccess, PaneMenuContext, PaneMenuKind, PaneRole, SEND_SELECTION_LIMIT};
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

        if let Some(link) = ctx.click.as_ref().and_then(|click| click.link.clone()) {
            if link.openable {
                let url = link.url.clone();
                entries.push(item(
                    s::ctx_open_link(),
                    ItemState::Enabled,
                    Activate::Op(Box::new(move |ws, _window, cx| {
                        ws.open_pane_menu_link(url.clone(), cx);
                    })),
                ));
            }
            entries.push(item(
                s::ctx_copy_link_address(),
                ItemState::Enabled,
                Activate::Clipboard(link.url),
            ));
            entries.push(MenuEntry::Separator);
        }

        entries.push(item(
            s::menu_copy(),
            state_if(ctx.selection.is_some(), None),
            Activate::Action(Box::new(daruda_terminal::view::Copy)),
        ));
        entries.push(item(
            s::menu_paste(),
            ItemState::Enabled,
            Activate::Action(Box::new(daruda_terminal::view::Paste)),
        ));
        entries.push(item(
            s::menu_select_all(),
            ItemState::Enabled,
            Activate::Action(Box::new(daruda_terminal::view::SelectAll)),
        ));
        entries.push(MenuEntry::Separator);
        entries.push(item(
            s::menu_clear_buffer(),
            ItemState::Enabled,
            Activate::Action(Box::new(daruda_terminal::view::ClearBuffer)),
        ));
        entries.push(item(
            s::menu_copy_last_command_output(),
            ItemState::Enabled,
            Activate::Action(Box::new(daruda_terminal::view::CopyLastCommandOutput)),
        ));
        entries.push(item(
            s::menu_scroll_to_bottom(),
            ItemState::Enabled,
            Activate::Action(Box::new(daruda_terminal::view::ScrollToBottom)),
        ));

        entries.extend(send_selection_entries(
            ctx,
            s::ctx_send_selection_to_agent_chat(),
        ));

        entries.push(MenuEntry::Separator);
        match &ctx.kind {
            PaneMenuKind::Terminal { annotation_range } => {
                let pane_id = ctx.pane_id;
                if let Some(range) = *annotation_range {
                    entries.push(item(
                        s::terminal_annotation_action_add(),
                        ItemState::Enabled,
                        Activate::Op(Box::new(move |ws, window, cx| {
                            ws.open_annotation_dialog_for_create(pane_id, range, window, cx);
                        })),
                    ));
                } else {
                    entries.push(disabled_item(
                        s::terminal_annotation_action_add(),
                        Some(s::terminal_annotation_action_add_disabled_tooltip().into()),
                    ));
                }
            }
            PaneMenuKind::AgentChat { .. } | PaneMenuKind::Other => {}
        }
        if let Some(mark_id) = ctx.click.as_ref().and_then(|click| click.annotation) {
            let pane_id = ctx.pane_id;
            entries.push(item(
                s::terminal_annotation_action_delete(),
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
        let mut entries = Vec::new();

        // Clipboard-by-value, not an action: the chat's selection is cleared
        // by the left-click that confirms the item, so the text has to come
        // from the snapshot.
        entries.push(match ctx.selection.as_ref() {
            Some(text) => item(
                s::menu_copy(),
                ItemState::Enabled,
                Activate::Clipboard(text.to_string()),
            ),
            None => disabled_item(s::menu_copy(), None),
        });

        entries.extend(send_selection_entries(
            ctx,
            s::ctx_send_selection_to_terminal(),
        ));

        entries.push(MenuEntry::Separator);
        if matches!(&ctx.kind, PaneMenuKind::AgentChat { busy: true }) {
            let pane_id = ctx.pane_id;
            entries.push(item(
                s::ctx_stop(),
                ItemState::Enabled,
                Activate::Op(Box::new(move |ws, _window, cx| {
                    ws.cancel_agent_turn_if_active(pane_id, cx);
                })),
            ));
        }
        let pane_id = ctx.pane_id;
        entries.push(item(
            s::menu_scroll_to_bottom(),
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
        LaneAccess::Inaccessible => ItemState::Disabled(Some(s::ctx_lane_inaccessible().into())),
    };

    let mut entries = vec![
        split_item(
            s::ctx_split_terminal_horizontal(),
            split_state.clone(),
            NewPaneKind::Terminal,
            SplitDirection::Horizontal,
        ),
        split_item(
            s::ctx_split_terminal_vertical(),
            split_state.clone(),
            NewPaneKind::Terminal,
            SplitDirection::Vertical,
        ),
        split_item(
            s::ctx_split_agent_chat_horizontal(),
            split_state.clone(),
            NewPaneKind::AgentChat,
            SplitDirection::Horizontal,
        ),
        split_item(
            s::ctx_split_agent_chat_vertical(),
            split_state,
            NewPaneKind::AgentChat,
            SplitDirection::Vertical,
        ),
    ];

    if let PaneRole::InSplit { zoomed } = &ctx.role {
        let pane_id = ctx.pane_id;
        let label = if *zoomed {
            s::ctx_unzoom_pane()
        } else {
            s::ctx_zoom_pane()
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
        PaneRole::Solo => s::ctx_close_tab(),
        PaneRole::InSplit { .. } => s::ctx_close_pane(),
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

fn send_selection_entries(ctx: &PaneMenuContext, label: String) -> Vec<MenuEntry> {
    let Some(text) = ctx.selection.clone() else {
        return Vec::new();
    };
    if ctx.send_targets.is_empty() {
        return Vec::new();
    }

    let state = if text.len() > SEND_SELECTION_LIMIT {
        ItemState::Disabled(Some(s::ctx_selection_too_large().into()))
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
mod tests {
    use daruda_terminal::session::interval_tree::{IntervalTree, LineCoord, LineRange, MarkId};
    use daruda_terminal::view::TerminalLink;

    use super::super::context::{ClickInfo, SendTarget};
    use super::super::spec::{MenuItemSpec, normalize_entries};
    use super::*;

    fn noop() -> Activate {
        Activate::Op(Box::new(|_, _, _| {}))
    }

    /// A real `MarkId` from the real allocator — the type is opaque by design,
    /// so the test stands up a throwaway tree rather than fabricating one.
    fn some_mark_id() -> MarkId {
        let mut tree: IntervalTree<()> = IntervalTree::new();
        tree.insert(line_range(), ())
    }

    fn line_range() -> LineRange {
        LineRange::new(
            LineCoord::Viewport { abs_y: 1 },
            LineCoord::Viewport { abs_y: 1 },
        )
    }

    fn labels(entries: &[MenuEntry]) -> Vec<String> {
        entries
            .iter()
            .filter_map(|entry| match entry {
                MenuEntry::Item(spec) => Some(spec.label().to_string()),
                MenuEntry::Submenu { label, .. } => Some(label.to_string()),
                MenuEntry::Separator => None,
            })
            .collect()
    }

    fn find<'a>(entries: &'a [MenuEntry], label: &str) -> Option<&'a MenuItemSpec> {
        entries.iter().find_map(|entry| match entry {
            MenuEntry::Item(spec) if spec.label().as_ref() == label => Some(spec),
            _ => None,
        })
    }

    fn submenu<'a>(entries: &'a [MenuEntry], label: &str) -> Option<&'a Vec<MenuEntry>> {
        entries.iter().find_map(|entry| match entry {
            MenuEntry::Submenu {
                label: found,
                entries,
            } if found.as_ref() == label => Some(entries),
            _ => None,
        })
    }

    fn base(kind: PaneMenuKind) -> PaneMenuContext {
        PaneMenuContext {
            pane_id: 1,
            role: PaneRole::Solo,
            lane: LaneAccess::Accessible,
            selection: None,
            click: None,
            send_targets: Vec::new(),
            kind,
        }
    }

    fn terminal_context(role: PaneRole) -> PaneMenuContext {
        PaneMenuContext {
            role,
            ..base(PaneMenuKind::Terminal {
                annotation_range: None,
            })
        }
    }

    fn targets(count: usize) -> Vec<SendTarget> {
        (0..count)
            .map(|index| SendTarget {
                pane_id: (index + 10) as u64,
                label: SharedString::from(format!("target-{index}")),
            })
            .collect()
    }

    // -- common tail ------------------------------------------------------

    #[test]
    fn solo_close_label_is_close_tab_and_zoom_is_absent() {
        let entries = compose(&terminal_context(PaneRole::Solo));
        let labels = labels(&entries);
        assert!(labels.contains(&s::ctx_close_tab()));
        assert!(!labels.contains(&s::ctx_zoom_pane()));
        assert!(!labels.contains(&s::ctx_unzoom_pane()));
    }

    #[test]
    fn split_role_adds_zoom_and_close_pane() {
        let entries = compose(&terminal_context(PaneRole::InSplit { zoomed: false }));
        let labels = labels(&entries);
        assert!(labels.contains(&s::ctx_zoom_pane()));
        assert!(labels.contains(&s::ctx_close_pane()));
    }

    #[test]
    fn zoomed_split_uses_unzoom_label() {
        let entries = compose(&terminal_context(PaneRole::InSplit { zoomed: true }));
        let labels = labels(&entries);
        assert!(labels.contains(&s::ctx_unzoom_pane()));
    }

    #[test]
    fn inaccessible_lane_disables_every_split_entry() {
        let ctx = PaneMenuContext {
            lane: LaneAccess::Inaccessible,
            ..terminal_context(PaneRole::Solo)
        };
        let entries = compose(&ctx);
        for label in [
            s::ctx_split_terminal_horizontal(),
            s::ctx_split_terminal_vertical(),
            s::ctx_split_agent_chat_horizontal(),
            s::ctx_split_agent_chat_vertical(),
        ] {
            let spec = find(&entries, &label).expect("split entry present");
            assert!(spec.is_disabled(), "{label} should be disabled");
        }
        // Closing is still reachable — only splitting needs the lane.
        assert!(
            !find(&entries, &s::ctx_close_tab())
                .expect("close entry present")
                .is_disabled()
        );
    }

    #[test]
    fn tab_controls_never_appear_in_a_pane_menu() {
        // Tab operations live on the tab bar only; a pane menu acts on the
        // pane the user right-clicked. Guards the scope decision.
        for ctx in [
            terminal_context(PaneRole::Solo),
            terminal_context(PaneRole::InSplit { zoomed: false }),
            base(PaneMenuKind::AgentChat { busy: true }),
            base(PaneMenuKind::Other),
        ] {
            let labels = labels(&compose(&ctx));
            for forbidden in [
                s::ctx_new_tab(),
                s::ctx_close_other_tabs(),
                s::ctx_close_tabs_to_right(),
                s::ctx_move_tab_left(),
                s::ctx_move_tab_right(),
            ] {
                assert!(
                    !labels.contains(&forbidden),
                    "{forbidden} must not appear in a pane menu"
                );
            }
        }
    }

    #[test]
    fn other_pane_kinds_get_the_common_tail_only() {
        let entries = compose(&base(PaneMenuKind::Other));
        let labels = labels(&entries);
        assert_eq!(
            labels,
            vec![
                s::ctx_split_terminal_horizontal(),
                s::ctx_split_terminal_vertical(),
                s::ctx_split_agent_chat_horizontal(),
                s::ctx_split_agent_chat_vertical(),
                s::ctx_close_tab(),
            ]
        );
    }

    // -- terminal head ----------------------------------------------------

    #[test]
    fn no_selection_disables_copy_and_drops_send() {
        let ctx = PaneMenuContext {
            send_targets: targets(1),
            ..terminal_context(PaneRole::Solo)
        };
        let entries = compose(&ctx);
        assert!(
            find(&entries, &s::menu_copy())
                .expect("copy present")
                .is_disabled()
        );
        assert!(!labels(&entries).contains(&s::ctx_send_selection_to_agent_chat()));
    }

    #[test]
    fn one_send_target_is_flat_and_many_fold_into_a_submenu() {
        let one = PaneMenuContext {
            selection: Some(SharedString::from("payload")),
            send_targets: targets(1),
            ..terminal_context(PaneRole::Solo)
        };
        let entries = compose(&one);
        assert!(find(&entries, &s::ctx_send_selection_to_agent_chat()).is_some());
        assert!(submenu(&entries, &s::ctx_send_selection_to_agent_chat()).is_none());

        let many = PaneMenuContext {
            send_targets: targets(2),
            ..one
        };
        let entries = compose(&many);
        let nested =
            submenu(&entries, &s::ctx_send_selection_to_agent_chat()).expect("submenu present");
        assert_eq!(labels(nested), vec!["target-0", "target-1"]);
    }

    #[test]
    fn oversized_selection_disables_send() {
        let ctx = PaneMenuContext {
            selection: Some(SharedString::from("x".repeat(SEND_SELECTION_LIMIT + 1))),
            send_targets: targets(1),
            ..terminal_context(PaneRole::Solo)
        };
        let entries = compose(&ctx);
        assert!(
            find(&entries, &s::ctx_send_selection_to_agent_chat())
                .expect("send entry present")
                .is_disabled()
        );
    }

    #[test]
    fn no_click_drops_link_and_delete_annotation() {
        let entries = compose(&terminal_context(PaneRole::Solo));
        let labels = labels(&entries);
        assert!(!labels.contains(&s::ctx_open_link()));
        assert!(!labels.contains(&s::ctx_copy_link_address()));
        assert!(!labels.contains(&s::terminal_annotation_action_delete()));
        // Add annotation is always listed; without a single-line selection it
        // is disabled and says why.
        assert!(
            find(&entries, &s::terminal_annotation_action_add())
                .expect("add annotation present")
                .is_disabled()
        );
    }

    #[test]
    fn link_and_annotation_can_be_offered_together() {
        // A mark spans a line range while a link spans cells, so the same
        // click can land on both. Guards against collapsing ClickInfo back
        // into one exclusive enum.
        let ctx = PaneMenuContext {
            click: Some(ClickInfo {
                link: Some(TerminalLink {
                    url: "https://example.com".to_string(),
                    openable: true,
                }),
                annotation: Some(some_mark_id()),
            }),
            ..terminal_context(PaneRole::Solo)
        };
        let labels = labels(&compose(&ctx));
        assert!(labels.contains(&s::ctx_open_link()));
        assert!(labels.contains(&s::terminal_annotation_action_delete()));
    }

    #[test]
    fn non_openable_link_offers_copy_only() {
        let ctx = PaneMenuContext {
            click: Some(ClickInfo {
                link: Some(TerminalLink {
                    url: "javascript:alert(1)".to_string(),
                    openable: false,
                }),
                annotation: None,
            }),
            ..terminal_context(PaneRole::Solo)
        };
        let labels = labels(&compose(&ctx));
        assert!(!labels.contains(&s::ctx_open_link()));
        assert!(labels.contains(&s::ctx_copy_link_address()));
    }

    #[test]
    fn single_line_selection_enables_add_annotation() {
        let ctx = PaneMenuContext {
            kind: PaneMenuKind::Terminal {
                annotation_range: Some(line_range()),
            },
            ..terminal_context(PaneRole::Solo)
        };
        let entries = compose(&ctx);
        assert!(
            !find(&entries, &s::terminal_annotation_action_add())
                .expect("add annotation present")
                .is_disabled()
        );
    }

    // -- agent chat head --------------------------------------------------

    #[test]
    fn stop_appears_only_while_a_turn_is_running() {
        let idle = labels(&compose(&base(PaneMenuKind::AgentChat { busy: false })));
        assert!(!idle.contains(&s::ctx_stop()));

        let busy = labels(&compose(&base(PaneMenuKind::AgentChat { busy: true })));
        assert!(busy.contains(&s::ctx_stop()));
    }

    #[test]
    fn agent_chat_omits_terminal_only_editing_entries() {
        let labels = labels(&compose(&base(PaneMenuKind::AgentChat { busy: false })));
        assert!(!labels.contains(&s::menu_paste()));
        assert!(!labels.contains(&s::menu_select_all()));
        assert!(!labels.contains(&s::menu_clear_buffer()));
    }

    // -- separators -------------------------------------------------------

    #[test]
    fn separators_are_normalized() {
        let entries = normalize_entries(vec![
            MenuEntry::Separator,
            item("A", ItemState::Enabled, noop()),
            MenuEntry::Separator,
            MenuEntry::Separator,
            item("B", ItemState::Enabled, noop()),
            MenuEntry::Separator,
        ]);
        assert!(matches!(entries.first(), Some(MenuEntry::Item(_))));
        assert!(matches!(entries.last(), Some(MenuEntry::Item(_))));
        assert!(
            !entries
                .windows(2)
                .any(|pair| matches!(pair, [MenuEntry::Separator, MenuEntry::Separator]))
        );
    }

    #[test]
    fn empty_submenus_are_dropped() {
        let entries = normalize_entries(vec![
            item("A", ItemState::Enabled, noop()),
            MenuEntry::Submenu {
                label: SharedString::from("empty"),
                entries: vec![MenuEntry::Separator],
            },
        ]);
        assert_eq!(labels(&entries), vec!["A".to_string()]);
    }

    #[test]
    fn composed_menus_never_start_or_end_with_a_separator() {
        for ctx in [
            terminal_context(PaneRole::Solo),
            terminal_context(PaneRole::InSplit { zoomed: true }),
            base(PaneMenuKind::AgentChat { busy: true }),
            base(PaneMenuKind::Other),
        ] {
            let entries = compose(&ctx);
            assert!(!matches!(entries.first(), Some(MenuEntry::Separator)));
            assert!(!matches!(entries.last(), Some(MenuEntry::Separator)));
        }
    }
}
