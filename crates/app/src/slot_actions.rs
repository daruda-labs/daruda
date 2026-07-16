//! Slot-table macros for `Cmd+1..9` tab and `Cmd+Ctrl+1..9` lane switching.
//!
//! One table feeds static bindings, config overrides, listener registration,
//! and public config names. The action idents still live in
//! `workspace/mod.rs` because the `actions!` procedural macro must see them.

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
macro_rules! lane_slot_table {
    (@bindings) => {
        [
            ::gpui::KeyBinding::new(
                $crate::surface::keybindings::SHORTCUT_ACTIVATE_LANE_1,
                $crate::workspace::ActivateLane1,
                None,
            ),
            ::gpui::KeyBinding::new(
                $crate::surface::keybindings::SHORTCUT_ACTIVATE_LANE_2,
                $crate::workspace::ActivateLane2,
                None,
            ),
            ::gpui::KeyBinding::new(
                $crate::surface::keybindings::SHORTCUT_ACTIVATE_LANE_3,
                $crate::workspace::ActivateLane3,
                None,
            ),
            ::gpui::KeyBinding::new(
                $crate::surface::keybindings::SHORTCUT_ACTIVATE_LANE_4,
                $crate::workspace::ActivateLane4,
                None,
            ),
            ::gpui::KeyBinding::new(
                $crate::surface::keybindings::SHORTCUT_ACTIVATE_LANE_5,
                $crate::workspace::ActivateLane5,
                None,
            ),
            ::gpui::KeyBinding::new(
                $crate::surface::keybindings::SHORTCUT_ACTIVATE_LANE_6,
                $crate::workspace::ActivateLane6,
                None,
            ),
            ::gpui::KeyBinding::new(
                $crate::surface::keybindings::SHORTCUT_ACTIVATE_LANE_7,
                $crate::workspace::ActivateLane7,
                None,
            ),
            ::gpui::KeyBinding::new(
                $crate::surface::keybindings::SHORTCUT_ACTIVATE_LANE_8,
                $crate::workspace::ActivateLane8,
                None,
            ),
            ::gpui::KeyBinding::new(
                $crate::surface::keybindings::SHORTCUT_ACTIVATE_LANE_9,
                $crate::workspace::ActivateLane9,
                None,
            ),
        ]
    };

    (@try_bind_override $key:expr, $name:expr, $cx:expr) => {{
        match $name {
            "activate_lane_1" => {
                $cx.bind_keys([::gpui::KeyBinding::new(
                    $key,
                    $crate::workspace::ActivateLane1,
                    None,
                )]);
                true
            }
            "activate_lane_2" => {
                $cx.bind_keys([::gpui::KeyBinding::new(
                    $key,
                    $crate::workspace::ActivateLane2,
                    None,
                )]);
                true
            }
            "activate_lane_3" => {
                $cx.bind_keys([::gpui::KeyBinding::new(
                    $key,
                    $crate::workspace::ActivateLane3,
                    None,
                )]);
                true
            }
            "activate_lane_4" => {
                $cx.bind_keys([::gpui::KeyBinding::new(
                    $key,
                    $crate::workspace::ActivateLane4,
                    None,
                )]);
                true
            }
            "activate_lane_5" => {
                $cx.bind_keys([::gpui::KeyBinding::new(
                    $key,
                    $crate::workspace::ActivateLane5,
                    None,
                )]);
                true
            }
            "activate_lane_6" => {
                $cx.bind_keys([::gpui::KeyBinding::new(
                    $key,
                    $crate::workspace::ActivateLane6,
                    None,
                )]);
                true
            }
            "activate_lane_7" => {
                $cx.bind_keys([::gpui::KeyBinding::new(
                    $key,
                    $crate::workspace::ActivateLane7,
                    None,
                )]);
                true
            }
            "activate_lane_8" => {
                $cx.bind_keys([::gpui::KeyBinding::new(
                    $key,
                    $crate::workspace::ActivateLane8,
                    None,
                )]);
                true
            }
            "activate_lane_9" => {
                $cx.bind_keys([::gpui::KeyBinding::new(
                    $key,
                    $crate::workspace::ActivateLane9,
                    None,
                )]);
                true
            }
            _ => false,
        }
    }};

    (@register_listeners $cx:expr, $div:expr) => {
        $div.on_action($cx.listener(
            |this: &mut Self, _: &$crate::workspace::ActivateLane1, window, cx| {
                this.activate_lane_by_index(0, window, cx);
            },
        ))
        .on_action($cx.listener(
            |this: &mut Self, _: &$crate::workspace::ActivateLane2, window, cx| {
                this.activate_lane_by_index(1, window, cx);
            },
        ))
        .on_action($cx.listener(
            |this: &mut Self, _: &$crate::workspace::ActivateLane3, window, cx| {
                this.activate_lane_by_index(2, window, cx);
            },
        ))
        .on_action($cx.listener(
            |this: &mut Self, _: &$crate::workspace::ActivateLane4, window, cx| {
                this.activate_lane_by_index(3, window, cx);
            },
        ))
        .on_action($cx.listener(
            |this: &mut Self, _: &$crate::workspace::ActivateLane5, window, cx| {
                this.activate_lane_by_index(4, window, cx);
            },
        ))
        .on_action($cx.listener(
            |this: &mut Self, _: &$crate::workspace::ActivateLane6, window, cx| {
                this.activate_lane_by_index(5, window, cx);
            },
        ))
        .on_action($cx.listener(
            |this: &mut Self, _: &$crate::workspace::ActivateLane7, window, cx| {
                this.activate_lane_by_index(6, window, cx);
            },
        ))
        .on_action($cx.listener(
            |this: &mut Self, _: &$crate::workspace::ActivateLane8, window, cx| {
                this.activate_lane_by_index(7, window, cx);
            },
        ))
        .on_action($cx.listener(
            |this: &mut Self, _: &$crate::workspace::ActivateLane9, window, cx| {
                this.activate_lane_by_index(8, window, cx);
            },
        ))
    };

    (@names) => {
        [
            "activate_lane_1",
            "activate_lane_2",
            "activate_lane_3",
            "activate_lane_4",
            "activate_lane_5",
            "activate_lane_6",
            "activate_lane_7",
            "activate_lane_8",
            "activate_lane_9",
        ]
    };

    (@menu_items) => {
        ::std::vec![
            ::gpui::MenuItem::action(
                $crate::surface::strings::menu_activate_lane_1(),
                $crate::workspace::ActivateLane1,
            ),
            ::gpui::MenuItem::action(
                $crate::surface::strings::menu_activate_lane_2(),
                $crate::workspace::ActivateLane2,
            ),
            ::gpui::MenuItem::action(
                $crate::surface::strings::menu_activate_lane_3(),
                $crate::workspace::ActivateLane3,
            ),
            ::gpui::MenuItem::action(
                $crate::surface::strings::menu_activate_lane_4(),
                $crate::workspace::ActivateLane4,
            ),
            ::gpui::MenuItem::action(
                $crate::surface::strings::menu_activate_lane_5(),
                $crate::workspace::ActivateLane5,
            ),
            ::gpui::MenuItem::action(
                $crate::surface::strings::menu_activate_lane_6(),
                $crate::workspace::ActivateLane6,
            ),
            ::gpui::MenuItem::action(
                $crate::surface::strings::menu_activate_lane_7(),
                $crate::workspace::ActivateLane7,
            ),
            ::gpui::MenuItem::action(
                $crate::surface::strings::menu_activate_lane_8(),
                $crate::workspace::ActivateLane8,
            ),
            ::gpui::MenuItem::action(
                $crate::surface::strings::menu_activate_lane_9(),
                $crate::workspace::ActivateLane9,
            ),
        ]
    };
}
