//! Slot-table macros — single source of truth for the `Cmd+1..9`
//! tab quick-switch and `Cmd+Ctrl+1..9` worktree quick-switch
//! action wiring. The 1..=9 enumeration would otherwise need to be
//! kept in lockstep across four files (`actions!` declaration, key
//! bindings, config-override `bind!` arms, render-time
//! `cx.listener` registrations); a per-section macro keeps the list
//! in one place so adding the next slot is one line, not nine.
//!
//! Section keywords (passed as the first token of each invocation):
//!
//! | keyword | yields |
//! |---|---|
//! | `@bindings` | array literal of [`gpui::KeyBinding`] for `cx.bind_keys([...])` |
//! | `@try_bind_override $key, $name, $cx` | block returning `bool` — `true` if the name matches a slot and the binding was applied; `false` otherwise (so callers fall through to other handlers) |
//! | `@register_listeners $cx, $div` | chained `.on_action(...)` clauses on `$div` |
//! | `@names` | array literal of `&'static str` config name strings |
//!
//! The action types themselves still live inside the `actions!`
//! invocation in `workspace/mod.rs` — that is a procedural macro and
//! must see literal idents.

#[macro_export]
macro_rules! tab_slot_table {
    (@bindings) => {
        [
            ::gpui::KeyBinding::new(
                $crate::surface::keybindings::SHORTCUT_ACTIVATE_TAB_1,
                $crate::workspace::ActivateTab1,
                None,
            ),
            ::gpui::KeyBinding::new(
                $crate::surface::keybindings::SHORTCUT_ACTIVATE_TAB_2,
                $crate::workspace::ActivateTab2,
                None,
            ),
            ::gpui::KeyBinding::new(
                $crate::surface::keybindings::SHORTCUT_ACTIVATE_TAB_3,
                $crate::workspace::ActivateTab3,
                None,
            ),
            ::gpui::KeyBinding::new(
                $crate::surface::keybindings::SHORTCUT_ACTIVATE_TAB_4,
                $crate::workspace::ActivateTab4,
                None,
            ),
            ::gpui::KeyBinding::new(
                $crate::surface::keybindings::SHORTCUT_ACTIVATE_TAB_5,
                $crate::workspace::ActivateTab5,
                None,
            ),
            ::gpui::KeyBinding::new(
                $crate::surface::keybindings::SHORTCUT_ACTIVATE_TAB_6,
                $crate::workspace::ActivateTab6,
                None,
            ),
            ::gpui::KeyBinding::new(
                $crate::surface::keybindings::SHORTCUT_ACTIVATE_TAB_7,
                $crate::workspace::ActivateTab7,
                None,
            ),
            ::gpui::KeyBinding::new(
                $crate::surface::keybindings::SHORTCUT_ACTIVATE_TAB_8,
                $crate::workspace::ActivateTab8,
                None,
            ),
            ::gpui::KeyBinding::new(
                $crate::surface::keybindings::SHORTCUT_ACTIVATE_TAB_9,
                $crate::workspace::ActivateTab9,
                None,
            ),
        ]
    };

    (@try_bind_override $key:expr, $name:expr, $cx:expr) => {{
        match $name {
            "activate_tab_1" => {
                $cx.bind_keys([::gpui::KeyBinding::new(
                    $key,
                    $crate::workspace::ActivateTab1,
                    None,
                )]);
                true
            }
            "activate_tab_2" => {
                $cx.bind_keys([::gpui::KeyBinding::new(
                    $key,
                    $crate::workspace::ActivateTab2,
                    None,
                )]);
                true
            }
            "activate_tab_3" => {
                $cx.bind_keys([::gpui::KeyBinding::new(
                    $key,
                    $crate::workspace::ActivateTab3,
                    None,
                )]);
                true
            }
            "activate_tab_4" => {
                $cx.bind_keys([::gpui::KeyBinding::new(
                    $key,
                    $crate::workspace::ActivateTab4,
                    None,
                )]);
                true
            }
            "activate_tab_5" => {
                $cx.bind_keys([::gpui::KeyBinding::new(
                    $key,
                    $crate::workspace::ActivateTab5,
                    None,
                )]);
                true
            }
            "activate_tab_6" => {
                $cx.bind_keys([::gpui::KeyBinding::new(
                    $key,
                    $crate::workspace::ActivateTab6,
                    None,
                )]);
                true
            }
            "activate_tab_7" => {
                $cx.bind_keys([::gpui::KeyBinding::new(
                    $key,
                    $crate::workspace::ActivateTab7,
                    None,
                )]);
                true
            }
            "activate_tab_8" => {
                $cx.bind_keys([::gpui::KeyBinding::new(
                    $key,
                    $crate::workspace::ActivateTab8,
                    None,
                )]);
                true
            }
            "activate_tab_9" => {
                $cx.bind_keys([::gpui::KeyBinding::new(
                    $key,
                    $crate::workspace::ActivateTab9,
                    None,
                )]);
                true
            }
            _ => false,
        }
    }};

    (@register_listeners $cx:expr, $div:expr) => {
        $div.on_action($cx.listener(
            |this: &mut Self, _: &$crate::workspace::ActivateTab1, window, cx| {
                this.on_activate_tab_n(0, window, cx);
            },
        ))
        .on_action($cx.listener(
            |this: &mut Self, _: &$crate::workspace::ActivateTab2, window, cx| {
                this.on_activate_tab_n(1, window, cx);
            },
        ))
        .on_action($cx.listener(
            |this: &mut Self, _: &$crate::workspace::ActivateTab3, window, cx| {
                this.on_activate_tab_n(2, window, cx);
            },
        ))
        .on_action($cx.listener(
            |this: &mut Self, _: &$crate::workspace::ActivateTab4, window, cx| {
                this.on_activate_tab_n(3, window, cx);
            },
        ))
        .on_action($cx.listener(
            |this: &mut Self, _: &$crate::workspace::ActivateTab5, window, cx| {
                this.on_activate_tab_n(4, window, cx);
            },
        ))
        .on_action($cx.listener(
            |this: &mut Self, _: &$crate::workspace::ActivateTab6, window, cx| {
                this.on_activate_tab_n(5, window, cx);
            },
        ))
        .on_action($cx.listener(
            |this: &mut Self, _: &$crate::workspace::ActivateTab7, window, cx| {
                this.on_activate_tab_n(6, window, cx);
            },
        ))
        .on_action($cx.listener(
            |this: &mut Self, _: &$crate::workspace::ActivateTab8, window, cx| {
                this.on_activate_tab_n(7, window, cx);
            },
        ))
        .on_action($cx.listener(
            |this: &mut Self, _: &$crate::workspace::ActivateTab9, window, cx| {
                this.on_activate_tab_n(8, window, cx);
            },
        ))
    };

    (@names) => {
        [
            "activate_tab_1",
            "activate_tab_2",
            "activate_tab_3",
            "activate_tab_4",
            "activate_tab_5",
            "activate_tab_6",
            "activate_tab_7",
            "activate_tab_8",
            "activate_tab_9",
        ]
    };
}

#[macro_export]
macro_rules! worktree_slot_table {
    (@bindings) => {
        [
            ::gpui::KeyBinding::new(
                $crate::surface::keybindings::SHORTCUT_ACTIVATE_WORKTREE_1,
                $crate::workspace::ActivateWorktree1,
                None,
            ),
            ::gpui::KeyBinding::new(
                $crate::surface::keybindings::SHORTCUT_ACTIVATE_WORKTREE_2,
                $crate::workspace::ActivateWorktree2,
                None,
            ),
            ::gpui::KeyBinding::new(
                $crate::surface::keybindings::SHORTCUT_ACTIVATE_WORKTREE_3,
                $crate::workspace::ActivateWorktree3,
                None,
            ),
            ::gpui::KeyBinding::new(
                $crate::surface::keybindings::SHORTCUT_ACTIVATE_WORKTREE_4,
                $crate::workspace::ActivateWorktree4,
                None,
            ),
            ::gpui::KeyBinding::new(
                $crate::surface::keybindings::SHORTCUT_ACTIVATE_WORKTREE_5,
                $crate::workspace::ActivateWorktree5,
                None,
            ),
            ::gpui::KeyBinding::new(
                $crate::surface::keybindings::SHORTCUT_ACTIVATE_WORKTREE_6,
                $crate::workspace::ActivateWorktree6,
                None,
            ),
            ::gpui::KeyBinding::new(
                $crate::surface::keybindings::SHORTCUT_ACTIVATE_WORKTREE_7,
                $crate::workspace::ActivateWorktree7,
                None,
            ),
            ::gpui::KeyBinding::new(
                $crate::surface::keybindings::SHORTCUT_ACTIVATE_WORKTREE_8,
                $crate::workspace::ActivateWorktree8,
                None,
            ),
            ::gpui::KeyBinding::new(
                $crate::surface::keybindings::SHORTCUT_ACTIVATE_WORKTREE_9,
                $crate::workspace::ActivateWorktree9,
                None,
            ),
        ]
    };

    (@try_bind_override $key:expr, $name:expr, $cx:expr) => {{
        match $name {
            "activate_worktree_1" => {
                $cx.bind_keys([::gpui::KeyBinding::new(
                    $key,
                    $crate::workspace::ActivateWorktree1,
                    None,
                )]);
                true
            }
            "activate_worktree_2" => {
                $cx.bind_keys([::gpui::KeyBinding::new(
                    $key,
                    $crate::workspace::ActivateWorktree2,
                    None,
                )]);
                true
            }
            "activate_worktree_3" => {
                $cx.bind_keys([::gpui::KeyBinding::new(
                    $key,
                    $crate::workspace::ActivateWorktree3,
                    None,
                )]);
                true
            }
            "activate_worktree_4" => {
                $cx.bind_keys([::gpui::KeyBinding::new(
                    $key,
                    $crate::workspace::ActivateWorktree4,
                    None,
                )]);
                true
            }
            "activate_worktree_5" => {
                $cx.bind_keys([::gpui::KeyBinding::new(
                    $key,
                    $crate::workspace::ActivateWorktree5,
                    None,
                )]);
                true
            }
            "activate_worktree_6" => {
                $cx.bind_keys([::gpui::KeyBinding::new(
                    $key,
                    $crate::workspace::ActivateWorktree6,
                    None,
                )]);
                true
            }
            "activate_worktree_7" => {
                $cx.bind_keys([::gpui::KeyBinding::new(
                    $key,
                    $crate::workspace::ActivateWorktree7,
                    None,
                )]);
                true
            }
            "activate_worktree_8" => {
                $cx.bind_keys([::gpui::KeyBinding::new(
                    $key,
                    $crate::workspace::ActivateWorktree8,
                    None,
                )]);
                true
            }
            "activate_worktree_9" => {
                $cx.bind_keys([::gpui::KeyBinding::new(
                    $key,
                    $crate::workspace::ActivateWorktree9,
                    None,
                )]);
                true
            }
            _ => false,
        }
    }};

    (@register_listeners $cx:expr, $div:expr) => {
        $div.on_action($cx.listener(
            |this: &mut Self, _: &$crate::workspace::ActivateWorktree1, window, cx| {
                this.activate_worktree_by_index(0, window, cx);
            },
        ))
        .on_action($cx.listener(
            |this: &mut Self, _: &$crate::workspace::ActivateWorktree2, window, cx| {
                this.activate_worktree_by_index(1, window, cx);
            },
        ))
        .on_action($cx.listener(
            |this: &mut Self, _: &$crate::workspace::ActivateWorktree3, window, cx| {
                this.activate_worktree_by_index(2, window, cx);
            },
        ))
        .on_action($cx.listener(
            |this: &mut Self, _: &$crate::workspace::ActivateWorktree4, window, cx| {
                this.activate_worktree_by_index(3, window, cx);
            },
        ))
        .on_action($cx.listener(
            |this: &mut Self, _: &$crate::workspace::ActivateWorktree5, window, cx| {
                this.activate_worktree_by_index(4, window, cx);
            },
        ))
        .on_action($cx.listener(
            |this: &mut Self, _: &$crate::workspace::ActivateWorktree6, window, cx| {
                this.activate_worktree_by_index(5, window, cx);
            },
        ))
        .on_action($cx.listener(
            |this: &mut Self, _: &$crate::workspace::ActivateWorktree7, window, cx| {
                this.activate_worktree_by_index(6, window, cx);
            },
        ))
        .on_action($cx.listener(
            |this: &mut Self, _: &$crate::workspace::ActivateWorktree8, window, cx| {
                this.activate_worktree_by_index(7, window, cx);
            },
        ))
        .on_action($cx.listener(
            |this: &mut Self, _: &$crate::workspace::ActivateWorktree9, window, cx| {
                this.activate_worktree_by_index(8, window, cx);
            },
        ))
    };

    (@names) => {
        [
            "activate_worktree_1",
            "activate_worktree_2",
            "activate_worktree_3",
            "activate_worktree_4",
            "activate_worktree_5",
            "activate_worktree_6",
            "activate_worktree_7",
            "activate_worktree_8",
            "activate_worktree_9",
        ]
    };

    (@menu_items) => {
        ::std::vec![
            ::gpui::MenuItem::action(
                $crate::surface::strings::menu_activate_worktree_1(),
                $crate::workspace::ActivateWorktree1,
            ),
            ::gpui::MenuItem::action(
                $crate::surface::strings::menu_activate_worktree_2(),
                $crate::workspace::ActivateWorktree2,
            ),
            ::gpui::MenuItem::action(
                $crate::surface::strings::menu_activate_worktree_3(),
                $crate::workspace::ActivateWorktree3,
            ),
            ::gpui::MenuItem::action(
                $crate::surface::strings::menu_activate_worktree_4(),
                $crate::workspace::ActivateWorktree4,
            ),
            ::gpui::MenuItem::action(
                $crate::surface::strings::menu_activate_worktree_5(),
                $crate::workspace::ActivateWorktree5,
            ),
            ::gpui::MenuItem::action(
                $crate::surface::strings::menu_activate_worktree_6(),
                $crate::workspace::ActivateWorktree6,
            ),
            ::gpui::MenuItem::action(
                $crate::surface::strings::menu_activate_worktree_7(),
                $crate::workspace::ActivateWorktree7,
            ),
            ::gpui::MenuItem::action(
                $crate::surface::strings::menu_activate_worktree_8(),
                $crate::workspace::ActivateWorktree8,
            ),
            ::gpui::MenuItem::action(
                $crate::surface::strings::menu_activate_worktree_9(),
                $crate::workspace::ActivateWorktree9,
            ),
        ]
    };
}
