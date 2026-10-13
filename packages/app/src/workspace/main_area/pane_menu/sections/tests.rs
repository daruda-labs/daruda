use daruda_terminal::session::interval_tree::{IntervalTree, LineCoord, LineRange, MarkId};

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
            exited: false,
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
    assert!(labels.contains(&s::common::close_tab()));
    assert!(!labels.contains(&s::ctx::zoom_pane()));
    assert!(!labels.contains(&s::ctx::unzoom_pane()));
}

#[test]
fn split_role_adds_zoom_and_close_pane() {
    let entries = compose(&terminal_context(PaneRole::InSplit { zoomed: false }));
    let labels = labels(&entries);
    assert!(labels.contains(&s::ctx::zoom_pane()));
    assert!(labels.contains(&s::common::close_pane()));
}

#[test]
fn zoomed_split_uses_unzoom_label() {
    let entries = compose(&terminal_context(PaneRole::InSplit { zoomed: true }));
    let labels = labels(&entries);
    assert!(labels.contains(&s::ctx::unzoom_pane()));
}

#[test]
fn inaccessible_lane_disables_every_split_entry() {
    let ctx = PaneMenuContext {
        lane: LaneAccess::Inaccessible,
        ..terminal_context(PaneRole::Solo)
    };
    let entries = compose(&ctx);
    for label in [
        s::common::split_terminal_horizontal(),
        s::common::split_terminal_vertical(),
        s::common::split_agent_chat_horizontal(),
        s::common::split_agent_chat_vertical(),
    ] {
        let spec = find(&entries, &label).expect("split entry present");
        assert!(spec.is_disabled(), "{label} should be disabled");
    }
    // Closing is still reachable — only splitting needs the lane.
    assert!(
        !find(&entries, &s::common::close_tab())
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
            s::common::new_tab(),
            s::ctx::close_other_tabs(),
            s::ctx::close_tabs_to_right(),
            s::common::move_tab_left(),
            s::common::move_tab_right(),
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
            s::common::split_terminal_horizontal(),
            s::common::split_terminal_vertical(),
            s::common::split_agent_chat_horizontal(),
            s::common::split_agent_chat_vertical(),
            s::common::close_tab(),
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
        find(&entries, &s::menu::copy())
            .expect("copy present")
            .is_disabled()
    );
    assert!(!labels(&entries).contains(&s::ctx::send_selection_to_agent_chat()));
}

#[test]
fn one_send_target_is_flat_and_many_fold_into_a_submenu() {
    let one = PaneMenuContext {
        selection: Some(SharedString::from("payload")),
        send_targets: targets(1),
        ..terminal_context(PaneRole::Solo)
    };
    let entries = compose(&one);
    assert!(find(&entries, &s::ctx::send_selection_to_agent_chat()).is_some());
    assert!(submenu(&entries, &s::ctx::send_selection_to_agent_chat()).is_none());

    let many = PaneMenuContext {
        send_targets: targets(2),
        ..one
    };
    let entries = compose(&many);
    let nested =
        submenu(&entries, &s::ctx::send_selection_to_agent_chat()).expect("submenu present");
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
        find(&entries, &s::ctx::send_selection_to_agent_chat())
            .expect("send entry present")
            .is_disabled()
    );
}

#[test]
fn no_click_drops_link_and_delete_annotation() {
    let entries = compose(&terminal_context(PaneRole::Solo));
    let labels = labels(&entries);
    assert!(!labels.contains(&s::ctx::open_link()));
    assert!(!labels.contains(&s::ctx::copy_link_address()));
    assert!(!labels.contains(&s::terminal::annotation_action_delete()));
    // Add annotation is always listed; without a single-line selection it
    // is disabled and says why.
    assert!(
        find(&entries, &s::terminal::annotation_action_add())
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
            link: Some(ClickLink::Web {
                url: "https://example.com".to_string(),
            }),
            annotation: Some(some_mark_id()),
        }),
        ..terminal_context(PaneRole::Solo)
    };
    let labels = labels(&compose(&ctx));
    assert!(labels.contains(&s::ctx::open_link()));
    assert!(labels.contains(&s::terminal::annotation_action_delete()));
}

#[test]
fn non_openable_link_offers_copy_only() {
    let ctx = PaneMenuContext {
        click: Some(ClickInfo {
            link: Some(ClickLink::Opaque {
                url: "javascript:alert(1)".to_string(),
            }),
            annotation: None,
        }),
        ..terminal_context(PaneRole::Solo)
    };
    let labels = labels(&compose(&ctx));
    assert!(!labels.contains(&s::ctx::open_link()));
    assert!(labels.contains(&s::ctx::copy_link_address()));
}

/// A chat link that resolves to a file is the one case with two ways to
/// open it, and neither is the browser.
#[test]
fn a_file_link_offers_the_viewer_and_the_external_opener() {
    let ctx = PaneMenuContext {
        click: Some(ClickInfo {
            link: Some(ClickLink::File {
                url: "src/main.rs:42".to_string(),
                path: std::path::PathBuf::from("/repo/src/main.rs"),
                kind: LocalKind::Text,
            }),
            annotation: None,
        }),
        ..base(PaneMenuKind::AgentChat { busy: false })
    };
    let labels = labels(&compose(&ctx));
    assert!(labels.contains(&s::ctx::open_link_in_file_view()));
    assert!(labels.contains(&s::ctx::open_link_externally()));
    assert!(labels.contains(&s::ctx::copy_link_address()));
    // The browser has nothing to do with a path.
    assert!(!labels.contains(&s::ctx::open_link()));
}

/// An image is not something the code viewer can show, so the viewer
/// entry would be dead — only the OS opener is offered.
#[test]
fn an_image_link_offers_only_the_external_opener() {
    let ctx = PaneMenuContext {
        click: Some(ClickInfo {
            link: Some(ClickLink::File {
                url: "/tmp/shot.png".to_string(),
                path: std::path::PathBuf::from("/tmp/shot.png"),
                kind: LocalKind::Image,
            }),
            annotation: None,
        }),
        ..base(PaneMenuKind::AgentChat { busy: false })
    };
    let labels = labels(&compose(&ctx));
    assert!(!labels.contains(&s::ctx::open_link_in_file_view()));
    assert!(labels.contains(&s::ctx::open_link_externally()));
    assert!(labels.contains(&s::ctx::copy_link_address()));
}

/// A file that is gone can still have its address copied, and nothing else.
#[test]
fn a_missing_file_link_offers_only_copy() {
    let ctx = PaneMenuContext {
        click: Some(ClickInfo {
            link: Some(ClickLink::File {
                url: "/tmp/gone.rs".to_string(),
                path: std::path::PathBuf::from("/tmp/gone.rs"),
                kind: LocalKind::Missing,
            }),
            annotation: None,
        }),
        ..base(PaneMenuKind::AgentChat { busy: false })
    };
    let labels = labels(&compose(&ctx));
    assert!(!labels.contains(&s::ctx::open_link_in_file_view()));
    assert!(!labels.contains(&s::ctx::open_link_externally()));
    assert!(labels.contains(&s::ctx::copy_link_address()));
}

/// The mirror: a web link in the chat reads exactly like one in the
/// terminal, because the entries follow the link kind, not the pane.
#[test]
fn a_web_link_in_the_chat_offers_the_browser_and_copy() {
    let ctx = PaneMenuContext {
        click: Some(ClickInfo {
            link: Some(ClickLink::Web {
                url: "https://example.com".to_string(),
            }),
            annotation: None,
        }),
        ..base(PaneMenuKind::AgentChat { busy: false })
    };
    let labels = labels(&compose(&ctx));
    assert!(labels.contains(&s::ctx::open_link()));
    assert!(labels.contains(&s::ctx::copy_link_address()));
    assert!(!labels.contains(&s::ctx::open_link_in_file_view()));
}

/// Only a terminal whose shell exited offers a restart.
#[test]
fn restart_shell_is_offered_only_once_the_shell_exited() {
    let terminal = |exited| {
        labels(&compose(&base(PaneMenuKind::Terminal {
            annotation_range: None,
            exited,
        })))
    };
    assert!(terminal(true).contains(&s::ctx::restart_shell()));
    assert!(!terminal(false).contains(&s::ctx::restart_shell()));
}

/// No right-click on a link means no link section at all — the chat menu
/// is what it always was.
#[test]
fn a_chat_menu_without_a_link_offers_none_of_it() {
    let labels = labels(&compose(&base(PaneMenuKind::AgentChat { busy: false })));
    assert!(!labels.contains(&s::ctx::copy_link_address()));
    assert!(!labels.contains(&s::ctx::open_link()));
    assert!(!labels.contains(&s::ctx::open_link_in_file_view()));
    assert!(!labels.contains(&s::ctx::open_link_externally()));
}

#[test]
fn single_line_selection_enables_add_annotation() {
    let ctx = PaneMenuContext {
        kind: PaneMenuKind::Terminal {
            annotation_range: Some(line_range()),
            exited: false,
        },
        ..terminal_context(PaneRole::Solo)
    };
    let entries = compose(&ctx);
    assert!(
        !find(&entries, &s::terminal::annotation_action_add())
            .expect("add annotation present")
            .is_disabled()
    );
}

// -- agent chat head --------------------------------------------------

#[test]
fn stop_appears_only_while_a_turn_is_running() {
    let idle = labels(&compose(&base(PaneMenuKind::AgentChat { busy: false })));
    assert!(!idle.contains(&s::common::btn_stop()));

    let busy = labels(&compose(&base(PaneMenuKind::AgentChat { busy: true })));
    assert!(busy.contains(&s::common::btn_stop()));
}

#[test]
fn agent_chat_omits_terminal_only_editing_entries() {
    let labels = labels(&compose(&base(PaneMenuKind::AgentChat { busy: false })));
    assert!(!labels.contains(&s::menu::paste()));
    assert!(!labels.contains(&s::menu::select_all()));
    assert!(!labels.contains(&s::menu::clear_buffer()));
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
