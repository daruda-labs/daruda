//! Skill-related `Workspace` operations.
//!
//! Thin shim layer that forwards to `super::right_panel::skills::*`
//! — create / edit / delete / rename modals, invocation picker,
//! file viewer / Finder navigation, plus a couple of UI-state
//! setters for the right-panel Skills tab.

use gpui::{Context, Window};

use super::Workspace;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::system_info::redact_home;

impl Workspace {
    pub fn open_create_skill_modal(
        &mut self,
        prefill_scope: Option<crate::agent::skills::SkillScope>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        super::right_panel::skills::open_create_skill_modal(self, prefill_scope, window, cx);
    }

    /// Open the Edit Skill modal for the skill at `dir`. The skill
    /// must already exist in `self.skills`; the call is a no-op
    /// otherwise (the row that fired this is stale).
    pub fn open_edit_skill_modal(
        &mut self,
        dir: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        super::right_panel::skills::open_edit_skill_modal(self, dir, window, cx);
    }

    /// Open the Delete confirmation modal for `dir`.
    pub fn open_delete_skill_confirm(
        &mut self,
        scope: crate::agent::skills::SkillScope,
        dir: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        super::right_panel::skills::open_delete_skill_confirm(self, scope, dir, window, cx);
    }

    /// Open the Rename Skill prompt — a single-field modal that asks
    /// for the new directory name, validates it, and renames on disk.
    pub fn open_rename_skill_modal(
        &mut self,
        scope: crate::agent::skills::SkillScope,
        dir: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        super::right_panel::skills::open_rename_skill_modal(self, scope, dir, window, cx);
    }

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
        }
    }

    /// Open the [`SkillPickerModal`](super::right_panel::skills::SkillPickerModal)
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
        use super::right_panel::skills::SkillPickerModal;

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

    /// Open the [`SkillInvocationModal`](super::right_panel::skills::SkillInvocationModal)
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
        use super::right_panel::skills::{SkillInvocationLabel, SkillInvocationModal};

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
            target_pane_id: self.focused_pane_id,
        };

        let workspace = cx.weak_entity();
        crate::workspace::dialog_helpers::open_form_modal(
            "Run skill",
            Some(gpui::px(crate::ui::theme::FORM_MODAL_WIDE)),
            move |window, cx| {
                SkillInvocationModal::new(workspace.clone(), label.clone(), window, cx)
            },
            window,
            cx,
        );
    }

    /// Open the SKILL.md inside `dir` in the daruda file viewer
    /// (the same one the sidebar Files / Git Changes views use). Used
    /// for plugin-scope skills (read-only on disk) so the user can
    /// still inspect contents without an Edit button. The active
    /// worktree id is borrowed only to satisfy the file-pane API —
    /// the file is read by absolute path so it can live anywhere on
    /// disk.
    pub fn open_skill_in_file_viewer(
        &mut self,
        dir: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = dir.join("SKILL.md");
        let worktree_id = self.active_worktree_id;
        self.open_pane_file_view(
            worktree_id,
            path,
            false,
            None,
            crate::workspace::pane_file_view::FileViewMode::Raw,
            window,
            cx,
        );
    }

    /// Open the skill directory in macOS Finder via `/usr/bin/open`.
    /// Lives on `Workspace` so the modal render path stays free of
    /// `std::process` (G2 / `render.rs` responsibility fence).
    pub fn open_skill_dir_in_finder(&mut self, dir: &std::path::Path, cx: &mut Context<Self>) {
        if let Err(e) = std::process::Command::new("open").arg(dir).spawn() {
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
