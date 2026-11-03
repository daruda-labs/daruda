//! Bottom dock body for the built-in Terminal Input panel.
//!
//! Owns the visual layout and button-click handler; keyboard handling lives in
//! `workspace::mod.rs` on the shared `InputState`. Dropped paths are quoted via
//! [`daruda_core::shell::quote`] using the focused pane's shell flavour before
//! insertion.

use crate::ui::ScrollableElement as _;
use crate::ui::theme;
use crate::ui::{ButtonVariants as _, Disableable as _};
use gpui::{
    AnyElement, App, ClickEvent, Context, ExternalPaths, IntoElement, SharedString, WeakEntity,
    Window, div, prelude::*, px,
};

use crate::workspace::Workspace;
use crate::workspace::layout::Dock;
use crate::workspace::layout::{BottomDockSnapshot, ComposerExpand, SlashMenuView};
use crate::workspace::main_area::bottom_dock::composer_ops::HistoryDir;
use crate::workspace::path_drag::PathDrag;

/// Build the terminal input panel body.
pub(super) fn render_body(snap: &BottomDockSnapshot, cx: &mut Context<Dock>) -> AnyElement {
    if let Some(cli) = &snap.agent_cli_snapshot {
        return render_cli_snapshot(snap, cli, cx);
    }
    let state = snap.terminal_input.clone();
    let ws_for_path = snap.workspace.clone();
    let ws_for_external = snap.workspace.clone();
    let ws_for_paste = snap.workspace.clone();
    let workspace = snap.workspace.clone();
    // Input edit shortcuts are handled by gpui_component's `"Input"` context.
    // Mid-turn agent panes show Stop; otherwise the button sends input.
    // DESIGN.md: Submit button — height 28px, radius md (6px). The primary / danger
    // factory applies the accent / danger background; height and radius are pinned
    // here to match the spec's fixed heights table (Button: 28px) and radius scale
    // (md: 6px), overriding `Button::small()`'s 24px default.
    // A bound question outranks Stop: the turn is busy only because it waits
    // on this answer, so the button sends it.
    let answering = snap.question.as_ref().is_some_and(|q| q.composer.bound);
    let submit = match snap.agent_stop_pane.filter(|_| !answering) {
        Some(pane_id) => {
            crate::ui::button_danger("send", crate::surface::strings::common::btn_stop())
                .h(px(theme::BUTTON_HEIGHT))
                .rounded(px(theme::RADIUS_MD))
                .on_click(cx.listener(move |_dock, _: &ClickEvent, _window, cx| {
                    if let Some(ws) = workspace.upgrade() {
                        ws.update(cx, |ws, cx| ws.cancel_agent_turn(pane_id, cx));
                    }
                }))
        }
        None => crate::ui::button_primary(
            "send",
            if answering {
                crate::surface::strings::bottom_dock::question_send()
            } else {
                crate::surface::strings::common::btn_submit()
            },
        )
        .when(answering, |button| button.ghost())
        .h(px(theme::BUTTON_HEIGHT))
        .rounded(px(theme::RADIUS_MD))
        .on_click(cx.listener(move |_dock, _: &ClickEvent, window, cx| {
            if let Some(ws) = workspace.upgrade() {
                ws.update(cx, |ws, cx| ws.send_terminal_input(window, cx));
            }
        })),
    };
    // When the focused pane is an Agent chat pane, selector chips sit to the
    // left of the Submit button (all in the input's right-hand action column):
    // the mode chip (permission mode), then one chip per Model / ThoughtLevel
    // config option (model, effort). All agent-only: a terminal-pane focus
    // carries `None`, so only Submit shows. Selecting dispatches through
    // `Workspace::set_agent_mode` / `set_agent_config_option` (one-way data flow).
    let mut chips: Vec<AnyElement> = Vec::new();
    let expand = match snap.composer_expand {
        ComposerExpand::Hidden => None,
        ComposerExpand::Expand => Some((
            crate::ui::icons::EXPAND,
            crate::surface::strings::bottom_dock::composer_expand(),
            true,
        )),
        ComposerExpand::Collapse => Some((
            crate::ui::icons::COLLAPSE,
            crate::surface::strings::bottom_dock::composer_collapse(),
            false,
        )),
    };
    // The expand toggle rides the action row's left edge, apart from the
    // send-side controls: it acts on the draft above, not on the send.
    let expand_toggle = expand.map(|(icon, tooltip, expanded)| {
        let ws = snap.workspace.clone();
        crate::ui::button_icon("composer-expand", icon, cx)
            .tooltip(tooltip)
            .debug_selector(|| "composer-expand".into())
            // Not `cx.listener`: expanding re-lays out this dock, which
            // must not still be leased by its own click handler.
            .on_click(move |_, window, cx| {
                if let Some(ws) = ws.upgrade() {
                    ws.update(cx, |ws, cx| ws.set_composer_expanded(expanded, window, cx));
                }
            })
    });
    if snap.attachment_draft.is_some() {
        let ws = snap.workspace.clone();
        chips.push(
            // An answer is text only, so attaching waits until the composer
            // is back to prompts.
            crate::ui::button_icon("attach-files", crate::ui::icons::ATTACH_FILE, cx)
                .tooltip(crate::surface::strings::agent_chat::attachment_add())
                .disabled(answering)
                .on_click(move |_, _, cx| {
                    if let Some(ws) = ws.upgrade() {
                        ws.update(cx, |ws, cx| ws.pick_composer_attachments(cx));
                    }
                })
                .into_any_element(),
        );
    }
    if let Some((pane_id, modes)) = &snap.agent_mode {
        chips.push(
            super::mode_chip::mode_chip(*pane_id, modes, snap.workspace.clone()).into_any_element(),
        );
    }
    if let Some((pane_id, options)) = &snap.agent_config_options {
        for opt in options {
            chips.push(
                super::config_chip::config_chip(*pane_id, opt, snap.workspace.clone())
                    .into_any_element(),
            );
        }
    }
    let action: AnyElement = if chips.is_empty() {
        submit.into_any_element()
    } else {
        let mut row = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(gpui::px(theme::AGENT_CHAT_MSG_GAP));
        for chip in chips {
            row = row.child(chip);
        }
        row.child(submit).into_any_element()
    };
    let action: AnyElement = match expand_toggle {
        None => action,
        Some(toggle) => div()
            .flex_1()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .child(toggle)
            .child(action)
            .into_any_element(),
    };
    // The action column sits beside the text in its own column (see
    // `input_with_action_grow`). In auto-grow mode the editor self-sizes
    // to content (the cap is owned by `InputState` — set at construction
    // via `auto_grow(1, max_rows)` and kept in sync on live config reload
    // via `set_auto_grow`); the outer dock height is driven by
    // `adapt_dock_to_input_lines` on every `InputEvent::Change`. In fill
    // mode (fallback) the editor fills the dock's fixed height and scrolls.
    // The question keys bind under these contexts (see `bind_keys`): Tab and
    // Shift+Tab while bound, digits and Space only over an empty composer.
    let answer_context = snap
        .question
        .as_ref()
        .filter(|q| q.composer.bound)
        .map(|q| {
            let mut context = gpui::KeyContext::default();
            context.add("QuestionAnswer");
            if q.composer_empty {
                context.add("QuestionAnswerEmpty");
            }
            context
        });
    let mut cell = div()
        .relative()
        .flex_1()
        .flex()
        .when_some(answer_context, |el, context| el.key_context(context))
        .child(
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .child(crate::ui::input_with_action_grow(
                    &state,
                    action,
                    cx,
                    0_isize,
                    crate::ui::InputGrowMode::AutoGrow,
                )),
        );
    if let Some((pane, names)) = &snap.attachment_draft
        && !names.is_empty()
    {
        let pane = *pane;
        let mut attachments = div().flex().flex_row().gap(px(theme::GAP_SM));
        for (index, name) in names.iter().enumerate() {
            let ws = snap.workspace.clone();
            attachments = attachments.child(
                div()
                    .flex()
                    .items_center()
                    .flex_shrink_0()
                    .max_w(px(theme::AGENT_ATTACHMENT_CHIP_MAX_W))
                    .gap(px(theme::GAP_SM))
                    .px(px(theme::GAP_SM))
                    .border_1()
                    .border_color(theme::current(cx).border)
                    .rounded(px(theme::RADIUS_MD))
                    .text_color(theme::current(cx).text_body)
                    .text_size(px(theme::FONT_SIZE_SM))
                    .child(div().min_w_0().truncate().child(name.clone()))
                    .child(
                        crate::ui::button_delete_glyph(("attachment", index), cx)
                            .tooltip(crate::surface::strings::agent_chat::attachment_remove(name))
                            .on_click(move |_, _, cx| {
                                if let Some(ws) = ws.upgrade() {
                                    ws.update(cx, |ws, cx| {
                                        ws.remove_composer_attachment(pane, index, cx)
                                    });
                                }
                            }),
                    ),
            );
        }
        cell = cell.flex_col().min_w_0().gap(px(theme::GAP_SM)).child(
            div()
                .id("composer-attachments")
                .h(px(theme::BUTTON_HEIGHT))
                .min_w_0()
                .flex_shrink_0()
                .overflow_x_scrollbar()
                .child(attachments),
        );
    }
    if let Some(menu) = &snap.slash_menu {
        // Deferred: laid out in place above the composer, but painted after
        // the window without the dock's clip, so it can rise over the panes.
        cell = cell.child(
            gpui::deferred(render_slash_menu(menu, snap.workspace.0.clone(), cx)).with_priority(1),
        );
    }
    let ws = snap.workspace.0.clone();
    super::bottom_panel_body()
        .capture_action(move |_: &crate::ui::InputPaste, _, cx| {
            if let Some(ws) = ws_for_paste.upgrade() {
                ws.update(cx, |ws, cx| ws.composer_paste_action(cx));
            }
        })
        // The composer's own keys run before the field's (capture phase); an
        // op that handles one stops it there.
        .capture_action(composer_key(
            ws.clone(),
            |ws, action: &crate::ui::InputEnter, window, cx| ws.composer_enter(action, window, cx),
        ))
        .on_action(composer_key(
            ws.clone(),
            |ws, _: &crate::ui::InputEscape, _, cx| ws.composer_escape(cx),
        ))
        .capture_action(composer_key(
            ws.clone(),
            |ws, _: &crate::ui::InputMoveUp, window, cx| {
                ws.composer_move(HistoryDir::Up, window, cx)
            },
        ))
        .capture_action(composer_key(
            ws.clone(),
            |ws, _: &crate::ui::InputMoveDown, window, cx| {
                ws.composer_move(HistoryDir::Down, window, cx)
            },
        ))
        .capture_action(composer_key(
            ws.clone(),
            |ws, _: &crate::ui::InputIndent, window, cx| ws.composer_tab(window, cx),
        ))
        .capture_action(composer_key(
            ws,
            |ws, _: &crate::ui::InputOutdent, _, cx| ws.composer_shift_tab(cx),
        ))
        .drag_over::<PathDrag>(|style, _, _, cx| {
            style.bg(theme::current(cx).input_panel_drop_target_bg)
        })
        .drag_over::<ExternalPaths>(|style, _, _, cx| {
            style.bg(theme::current(cx).input_panel_drop_target_bg)
        })
        .on_drop::<PathDrag>(cx.listener(move |_dock, drag: &PathDrag, window, cx| {
            if let Some(ws) = ws_for_path.upgrade() {
                ws.update(cx, |ws, cx| {
                    ws.composer_drop(vec![drag.path.clone()], window, cx)
                });
            }
        }))
        .on_drop::<ExternalPaths>(
            cx.listener(move |_dock, paths: &ExternalPaths, window, cx| {
                if let Some(ws) = ws_for_external.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.composer_drop(paths.paths().to_vec(), window, cx)
                    });
                }
            }),
        )
        .child(cell)
        .into_any_element()
}

/// A capture-phase listener that runs a composer op and stops the action
/// when the op handled it.
fn composer_key<A: gpui::Action>(
    ws: WeakEntity<Workspace>,
    op: impl Fn(&mut Workspace, &A, &mut Window, &mut Context<Workspace>) -> bool + 'static,
) -> impl Fn(&A, &mut Window, &mut App) + 'static {
    move |action, window, cx| {
        let handled = ws
            .upgrade()
            .is_some_and(|ws| ws.update(cx, |ws, cx| op(ws, action, window, cx)));
        if handled {
            cx.stop_propagation();
        }
    }
}

/// The slash-command list, floating above the composer. Rows are clickable;
/// the keyboard drives it through the composer ops.
fn render_slash_menu(
    menu: &SlashMenuView,
    ws: WeakEntity<Workspace>,
    cx: &mut Context<Dock>,
) -> AnyElement {
    let t = theme::current(cx);
    let rows = menu
        .items
        .iter()
        .enumerate()
        .map(|(index, (name, description))| {
            let ws = ws.clone();
            div()
                .id(("slash-item", index))
                .flex()
                .flex_row()
                .items_baseline()
                .gap(px(theme::GAP_SM))
                .px(px(theme::PAD_SM))
                .py(px(theme::GAP_SM))
                .rounded(px(theme::RADIUS_SM))
                .when(index == menu.selected, |row| row.bg(t.lane_row_hover_bg))
                .hover(|row| row.bg(t.lane_row_hover_bg))
                .cursor_pointer()
                .child(
                    div()
                        .flex_none()
                        .text_color(t.text_primary)
                        .child(SharedString::from(format!("/{name}"))),
                )
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(t.text_muted)
                        .child(description.clone()),
                )
                .on_click(move |_, window, cx| {
                    if let Some(ws) = ws.upgrade() {
                        ws.update(cx, |ws, cx| ws.accept_slash_item(Some(index), window, cx));
                    }
                })
        });
    div()
        .id("composer-slash-menu")
        .debug_selector(|| "composer-slash-menu".into())
        .absolute()
        .bottom_full()
        .left_0()
        .right_0()
        .mb(px(theme::GAP_SM))
        .p(px(theme::GAP_SM))
        .max_h(px(theme::COMPOSER_SLASH_MENU_MAX_H))
        .overflow_y_scroll()
        .bg(t.float_panel_bg)
        .border_1()
        .border_color(t.border)
        .rounded(px(theme::RADIUS_MD))
        .text_size(px(theme::FONT_SIZE_MD))
        .occlude()
        .children(rows)
        .into_any_element()
}

/// Read-only CLI history takes the composer's place: a snapshot can only be
/// reloaded, and continued once the original process is confirmed gone.
fn render_cli_snapshot(
    snap: &BottomDockSnapshot,
    cli: &crate::workspace::layout::snap::CliSnapshot,
    cx: &mut Context<Dock>,
) -> AnyElement {
    use crate::surface::strings as s;
    use crate::ui::{self, Disableable as _};
    use daruda_store::tasks::CliProcessState;

    let label = match cli.process {
        None => s::task::cli_run_missing(),
        Some(CliProcessState::Discovering | CliProcessState::Unknown) => s::task::cli_unknown(),
        Some(CliProcessState::Running { .. }) => s::task::cli_running(),
        Some(CliProcessState::ExitConfirmed { .. }) => match cli.end_reason {
            Some(daruda_store::tasks::CliSessionEndReason::SessionEnded) => {
                s::task::cli_session_ended()
            }
            Some(daruda_store::tasks::CliSessionEndReason::ProcessExited) => {
                s::task::cli_process_lost()
            }
            Some(daruda_store::tasks::CliSessionEndReason::PaneClosed) => {
                s::task::cli_pane_closed()
            }
            Some(daruda_store::tasks::CliSessionEndReason::AppRestart) => {
                s::task::cli_app_restarted()
            }
            None => s::task::cli_exited(),
        },
    };
    let exited = matches!(cli.process, Some(CliProcessState::ExitConfirmed { .. }));
    let pane_id = cli.pane_id;
    let refresh_ws = snap.workspace.clone();
    let continue_ws = snap.workspace.clone();
    let row = div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(theme::GAP_LG))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_color(theme::current(cx).text_muted)
                .text_size(px(theme::FONT_SIZE_SM))
                .child(label),
        )
        .child(
            ui::button_icon("cli-refresh", ui::icons::REFRESH, cx)
                .tooltip(s::task::cli_refresh())
                .disabled(cli.loading)
                .on_click(move |_, _, cx| {
                    if let Some(ws) = refresh_ws.upgrade() {
                        ws.update(cx, |ws, cx| ws.refresh_cli_chat(pane_id, cx));
                    }
                }),
        )
        .when(exited, |row| {
            row.child(
                ui::button("cli-continue", s::task::cli_continue())
                    .disabled(cli.loading)
                    .on_click(move |_, _, cx| {
                        if let Some(ws) = continue_ws.upgrade() {
                            ws.update(cx, |ws, cx| ws.continue_cli_chat(pane_id, cx));
                        }
                    }),
            )
        });
    super::bottom_panel_body().child(row).into_any_element()
}
