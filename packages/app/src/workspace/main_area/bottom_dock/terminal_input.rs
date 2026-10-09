//! Bottom dock body for the built-in Terminal Input panel.
//!
//! Owns the visual layout and button-click handler; keyboard handling lives in
//! `workspace::mod.rs` on the shared `InputState`. Dropped paths are quoted via
//! [`daruda_core::shell::quote`] using the focused pane's shell flavour before
//! insertion.

use crate::ui::ScrollableElement as _;
use crate::ui::theme;
use gpui::{AnyElement, ClickEvent, Context, ExternalPaths, IntoElement, div, prelude::*, px};

use crate::workspace::layout::BottomDockSnapshot;
use crate::workspace::layout::Dock;
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
    let submit = match snap.agent_stop_pane {
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
        None => crate::ui::button_primary("send", crate::surface::strings::common::btn_submit())
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
    if snap.attachment_draft.is_some() {
        let ws = snap.workspace.clone();
        chips.push(
            crate::ui::button(
                "attach-files",
                crate::surface::strings::agent_chat::attachment_add(),
            )
            .on_click(move |_, _, cx| {
                if let Some(ws) = ws.upgrade() {
                    ws.update(cx, |ws, cx| ws.pick_composer_attachments(cx));
                }
            })
            .into_any_element(),
        );
        let ws = snap.workspace.clone();
        chips.push(
            crate::ui::button_icon("paste-attachment", crate::ui::icons::ADD, cx)
                .tooltip(crate::surface::strings::agent_chat::attachment_paste())
                .on_click(move |_, _, cx| {
                    if let Some(ws) = ws.upgrade() {
                        ws.update(cx, |ws, cx| {
                            ws.paste_composer_attachments(cx);
                        });
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
            .items_end()
            .gap(gpui::px(theme::AGENT_CHAT_MSG_GAP));
        for chip in chips {
            row = row.child(chip);
        }
        row.child(submit).into_any_element()
    };
    // The action column sits beside the text in its own column (see
    // `input_with_action_grow`). In auto-grow mode the editor self-sizes
    // to content (the cap is owned by `InputState` — set at construction
    // via `auto_grow(1, max_rows)` and kept in sync on live config reload
    // via `set_auto_grow`); the outer dock height is driven by
    // `adapt_dock_to_input_lines` on every `InputEvent::Change`. In fill
    // mode (fallback) the editor fills the dock's fixed height and scrolls.
    let mut cell = div()
        .flex_1()
        .flex()
        .child(crate::ui::input_with_action_grow(
            &state,
            action,
            cx,
            0_isize,
            crate::ui::InputGrowMode::AutoGrow,
        ));
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
    super::bottom_panel_body()
        .capture_action(move |_: &crate::ui::InputPaste, _, cx| {
            if let Some(ws) = ws_for_paste.upgrade() {
                ws.update(cx, |ws, cx| ws.composer_paste_action(cx));
            }
        })
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
        Some(CliProcessState::ExitConfirmed { .. }) => s::task::cli_exited(),
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
