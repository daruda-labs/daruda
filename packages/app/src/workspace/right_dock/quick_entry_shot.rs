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
    SkillPaste,
    SkillReview,
}

pub(crate) fn drive(
    shot: QuickEntryShot,
    workspace: &Entity<Workspace>,
    window: &mut Window,
    cx: &mut App,
) {
    let workspace = workspace.downgrade();
    match shot {
        QuickEntryShot::McpPaste | QuickEntryShot::McpReview => {
            dialog_helpers::open_form_modal(
                strings::mcp::new_title(),
                Some(px(theme::FORM_MODAL_WIDE)),
                move |window, cx| {
                    let mut modal = AddMcpServerModal::new(
                        workspace,
                        AddMcpInitial {
                            default_scope: McpScope::Project,
                            project_root: Some("/workspace/project".into()),
                            existing_names: Vec::new(),
                        },
                        window,
                        cx,
                    );
                    modal.seed_for_shot(shot == QuickEntryShot::McpReview, window, cx);
                    modal
                },
                window,
                cx,
            );
        }
        QuickEntryShot::SkillPaste | QuickEntryShot::SkillReview => {
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
                    modal.seed_for_shot(shot == QuickEntryShot::SkillReview, window, cx);
                    modal
                },
                window,
                cx,
            );
        }
    }
}
