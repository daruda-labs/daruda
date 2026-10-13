//! In-memory forms for native visual verification; no config is written.
use super::skills::create_modal::CreateSkillModal;
use super::tools::add_modal::{AddMcpInitial, AddMcpServerModal};
use crate::agent::{mcp::McpScope, skills::SkillsSnapshot};
use crate::surface::strings;
use crate::ui::theme;
use crate::workspace::{Workspace, dialog_helpers};
use gpui::{App, Entity, Window, px};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QuickEntryShot {
    McpPaste,
    McpReview,
    McpStdio,
    McpSaved,
    McpError,
    McpCodexReview,
    McpCodexTail,
    LibraryClaude,
    LibraryCodex,
    SkillPaste,
    SkillReview,
    SkillCodexReview,
    DeleteConfirm,
}

pub(crate) fn drive(
    shot: QuickEntryShot,
    workspace: &Entity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) {
    if matches!(
        shot,
        QuickEntryShot::LibraryClaude | QuickEntryShot::LibraryCodex
    ) {
        workspace.update(cx, |ws, cx| {
            ws.right_views.library.manual = Some(if shot == QuickEntryShot::LibraryCodex {
                crate::agent::configuration::Provider::Codex
            } else {
                crate::agent::configuration::Provider::Claude
            });
            ws.reveal_right_dock_view(daruda_store::project::RightDockView::Tools, cx);
        });
        return;
    }
    if shot == QuickEntryShot::McpCodexTail {
        window
            .spawn(cx, async move |cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(350))
                    .await;
                // SILENT-OK: a capture window may be closed before its scroll probe.
                let _ = cx.update(|window, cx| {
                    let size = window.viewport_size();
                    window.dispatch_event(
                        gpui::PlatformInput::ScrollWheel(gpui::ScrollWheelEvent {
                            position: gpui::point(
                                size.width / 2.,
                                px(theme::TITLE_BAR_HEIGHT + theme::PAD_XL * 6.),
                            ),
                            delta: gpui::ScrollDelta::Pixels(gpui::point(
                                px(0.),
                                -size.height * 2.,
                            )),
                            modifiers: Default::default(),
                            touch_phase: gpui::TouchPhase::Moved,
                        }),
                        cx,
                    );
                });
            })
            .detach();
    }
    let workspace = workspace.downgrade();
    match shot {
        QuickEntryShot::McpSaved => super::tools::saved::open(
            "project-docs".into(),
            Some(crate::agent::configuration::Provider::Codex),
            false,
            window,
            cx,
        ),
        QuickEntryShot::DeleteConfirm => super::delete_modal::open(
            strings::skills::delete_title(),
            format!("{} sample", strings::skills::delete_body_prefix()),
            |_, _| Ok(()),
            window,
            cx,
        ),
        QuickEntryShot::LibraryClaude | QuickEntryShot::LibraryCodex => unreachable!(),
        QuickEntryShot::McpPaste
        | QuickEntryShot::McpReview
        | QuickEntryShot::McpStdio
        | QuickEntryShot::McpError
        | QuickEntryShot::McpCodexReview
        | QuickEntryShot::McpCodexTail => {
            dialog_helpers::open_form_modal(
                strings::mcp::new_title(),
                Some(px(theme::FORM_MODAL_WIDE)),
                move |window, cx| {
                    let mut modal = AddMcpServerModal::new(
                        workspace,
                        AddMcpInitial {
                            targets: Vec::new(),
                            config: (matches!(
                                shot,
                                QuickEntryShot::McpCodexReview
                                    | QuickEntryShot::McpCodexTail
                                    | QuickEntryShot::McpStdio
                            ))
                            .then(|| {
                                std::sync::Arc::new(
                                    crate::agent::configuration::Configuration::resolve(
                                        if shot == QuickEntryShot::McpStdio {
                                            crate::agent::configuration::Provider::Claude
                                        } else {
                                            crate::agent::configuration::Provider::Codex
                                        },
                                        Some(std::env::temp_dir()),
                                        Some(std::env::temp_dir().join(
                                            if shot == QuickEntryShot::McpStdio {
                                                "example-claude-account"
                                            } else {
                                                "example-codex-account"
                                            },
                                        )),
                                        Some(std::env::temp_dir().join("example-project")),
                                    )
                                    .unwrap(),
                                )
                            }),
                            default_scope: McpScope::Project,
                            project_root: Some("/workspace/project".into()),
                            existing_names: Vec::new(),
                        },
                        window,
                        cx,
                    );
                    if matches!(
                        shot,
                        QuickEntryShot::McpCodexReview | QuickEntryShot::McpCodexTail
                    ) {
                        modal.seed_codex_for_shot(window, cx);
                    } else if shot == QuickEntryShot::McpStdio {
                        modal.seed_stdio_for_shot(window, cx);
                    } else if shot == QuickEntryShot::McpError {
                        modal.seed_error_for_shot(window, cx);
                    } else {
                        modal.seed_for_shot(shot == QuickEntryShot::McpReview, window, cx);
                    }
                    modal
                },
                window,
                cx,
            );
        }
        QuickEntryShot::SkillPaste
        | QuickEntryShot::SkillReview
        | QuickEntryShot::SkillCodexReview => {
            dialog_helpers::open_form_modal(
                strings::skills::new_title(),
                Some(px(theme::FORM_MODAL_WIDE)),
                move |window, cx| {
                    let mut modal = CreateSkillModal::new(
                        workspace,
                        None,
                        Some("/workspace/project".into()),
                        SkillsSnapshot::default(),
                        window,
                        cx,
                    );
                    modal.seed_for_shot(
                        shot != QuickEntryShot::SkillPaste,
                        shot == QuickEntryShot::SkillCodexReview,
                        window,
                        cx,
                    );
                    modal
                },
                window,
                cx,
            );
        }
    }
}
