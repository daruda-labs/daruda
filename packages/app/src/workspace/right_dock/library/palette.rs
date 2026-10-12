//! The command palette is a read consumer even while the right dock is closed.
use super::*;

impl Workspace {
    pub(in crate::workspace) fn open_library_skill_palette(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut Context<Self>,
    ) {
        let Some(config) = self.library_context().config else {
            self.reveal_right_dock_view(daruda_store::project::RightDockView::Skills, cx);
            return;
        };
        let pane = self.active_runtime().focused_pane_id;
        cx.spawn_in(window, async move |this, cx| {
            let input = config.clone();
            let result = cx.background_executor().spawn(async move { discover(&input) }).await;
            // SILENT-OK: the window can close while discovering palette entries.
            let _ = this.update_in(cx, |ws, window, cx| {
                if ws.active_runtime().focused_pane_id != pane || ws.library_context().config.as_ref() != Some(&config) { return; }
                match result {
                    Ok(inventory) => {
                        let mut skills = inventory.skills.project;
                        skills.extend(inventory.skills.personal);
                        skills.extend(inventory.skills.plugin.into_iter().filter(|skill| skill.plugin_availability.is_none() || matches!(skill.plugin_availability, Some(crate::agent::skills::plugins::PluginAvailability::Installed))));
                        ws.open_skill_picker_modal(&skills, crate::surface::strings::skills::invoke_title().into(), window, cx);
                    }
                    Err(_) => ws.report_error(daruda_store::observability::error_report::ErrorReport::new(crate::surface::strings::library::read_error()).severity(daruda_store::observability::error_report::ErrorSeverity::Warning).at(file!(), line!()).dedup("library.palette").build(), cx),
                }
            });
        }).detach();
    }
}
