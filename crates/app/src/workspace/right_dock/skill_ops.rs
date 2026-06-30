//! Skill-related `Workspace` operations with real logic.
//!
//! Modal openers (create / edit / delete / rename) are free
//! functions in `super::skills::*` and renderers / action
//! handlers call them directly. This file holds the ops that
//! genuinely need `&mut Workspace`: dialog construction over
//! `dialog_helpers`, file-viewer dispatch, Finder spawn, and
//! plugin-accordion UI state.

use gpui::{Context, Window};

use crate::surface::strings;
use crate::workspace::Workspace;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::system_info::redact_home;

impl Workspace {
    /// Replace the right-bar plugin accordion's expanded set in full.
    /// `gpui_component::Accordion::on_toggle_click` fires with the
    /// current vector of open indices on every change, so daruda
    /// computes the corresponding `plugin_id` set in the renderer and
    /// hands it back here verbatim.
    pub(in crate::workspace) fn set_skill_plugin_expanded(
        &mut self,
        expanded: std::collections::HashSet<String>,
        cx: &mut Context<Self>,
    ) {
        if self.skill_plugin_expanded != expanded {
            self.skill_plugin_expanded = expanded;
            cx.notify();
            self.notify_right_dock(cx);
        }
    }

    /// Open the [`SkillPickerModal`](super::skills::SkillPickerModal)
    /// for a set of skills (typically every skill bundled with one
    /// plugin). On confirm the picker closes and immediately opens
    /// the invocation modal for the selected skill.
    pub fn open_skill_picker_modal(
        &mut self,
        skills: &[crate::agent::skills::Skill],
        title: gpui::SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use super::skills::SkillPickerModal;

        // Build the items now while we hold `&mut self` — the modal
        // must not re-enter the workspace from inside its constructor
        // (G2 / pitfall §4).
        let items = SkillPickerModal::build_items(skills);
        let workspace = cx.weak_entity();

        crate::workspace::dialog_helpers::open_form_modal(
            title,
            Some(gpui::px(crate::ui::theme::FORM_MODAL_WIDE)),
            move |window, cx| SkillPickerModal::new(workspace.clone(), items.clone(), window, cx),
            window,
            cx,
        );
    }

    /// Open the [`SkillInvocationModal`](super::skills::SkillInvocationModal)
    /// for `skill`. Translates the on-disk model into the modal's
    /// scope-agnostic `SkillInvocationLabel` carrier and captures the
    /// active terminal pane id so submit lands in the pane the user
    /// was looking at when they clicked.
    pub fn open_skill_invocation_modal(
        &mut self,
        skill: &crate::agent::skills::Skill,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use super::skills::{SkillInvocationLabel, SkillInvocationModal};

        // `scan::scan_plugins` already stores plugin-scope skills as
        // `<plugin_local>:<skill_name>` in `Skill::name` (see
        // `agent/skills/scan.rs`), so use the field verbatim. An
        // earlier revision re-applied the prefix and produced
        // `<plugin_local>:<plugin_local>:<skill>` for any skill the
        // user clicked from the plugin section.
        let display_name = skill.name.clone();

        let label = SkillInvocationLabel {
            display_name,
            description: skill.frontmatter.description.clone(),
            argument_hint: skill.frontmatter.argument_hint.clone(),
            scope: skill.scope,
            target_pane_id: self.active_runtime().focused_pane_id,
        };

        let workspace = cx.weak_entity();
        crate::workspace::dialog_helpers::open_form_modal(
            strings::skills_invoke_title(),
            Some(gpui::px(crate::ui::theme::FORM_MODAL_WIDE)),
            move |window, cx| {
                SkillInvocationModal::new(workspace.clone(), label.clone(), window, cx)
            },
            window,
            cx,
        );
    }

    /// Open the SKILL.md inside `dir` in the daruda file viewer
    /// (the same one the left dock (Files / Git Changes) views use). Used
    /// for plugin-scope skills (read-only on disk) so the user can
    /// still inspect contents without an Edit button. The active
    /// lane id is borrowed only to satisfy the file-pane API —
    /// the file is read by absolute path so it can live anywhere on
    /// disk.
    pub fn open_skill_in_file_viewer(
        &mut self,
        dir: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = dir.join("SKILL.md");
        let lane_id = self.active.lane;
        self.open_pane_file_view(
            lane_id,
            path,
            false,
            None,
            crate::workspace::main_area::file_view_pane::FileViewMode::Raw,
            window,
            cx,
        );
    }

    /// Open the skill directory in macOS Finder via the `open` crate.
    /// Lives on `Workspace` so the modal render path stays free of
    /// process-launch concerns (G2 / `render.rs` responsibility fence).
    pub fn open_skill_dir_in_finder(&mut self, dir: &std::path::Path, cx: &mut Context<Self>) {
        if let Err(e) = open::that_detached(dir) {
            let report = ErrorReport::new("Open in Finder failed")
                .severity(ErrorSeverity::Warning)
                .from_error(&e)
                .at(file!(), line!())
                .with_context("path", redact_home(dir))
                .dedup("files.finder")
                .build();
            self.report_error(report, cx);
        }
    }
}
