//! Native prompt-file selection, resolved relative to the Flow definition.
use super::super::FlowGraphView;
use crate::surface::strings as s;
use daruda_store::observability::{error_report::ErrorReport, log_writer::LogWriter};
use gpui::{Context, Window};

impl FlowGraphView {
    pub(in crate::workspace) fn pick_prompt_file(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(edit) = self.begin_prompt_edit(super::prompt_edit::PromptField::File, cx) else {
            return;
        };
        let root = self.path.parent().unwrap_or(&self.path).to_path_buf();
        let selection = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(s::flow::form_pick_prompt().into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = selection.await;
            if let Err(error) = this.update_in(cx, |view, window, cx| match result {
                Ok(Ok(Some(paths))) => {
                    if let Some(path) = paths.first() {
                        match daruda_core::path::strip_root(path, &root) {
                            Some(relative) => {
                                if !view.apply_prompt_edit(
                                    &edit,
                                    relative.to_string_lossy().into_owned(),
                                    window,
                                    cx,
                                ) {
                                    view.set_save_error(
                                        Some(s::flow::form_file_picker_conflict()),
                                        cx,
                                    );
                                }
                            }
                            None => view.set_save_error(Some(s::flow::form_file_outside()), cx),
                        }
                    }
                }
                Ok(Err(error)) => {
                    LogWriter::log(
                        ErrorReport::new("Flow prompt file picker failed")
                            .from_error(error.root_cause())
                            .at(file!(), line!())
                            .build(),
                    );
                    view.set_save_error(Some(error.to_string()), cx);
                }
                _ => {}
            }) {
                LogWriter::log(
                    ErrorReport::new("Flow prompt picker lost its window")
                        .from_error(error.root_cause())
                        .at(file!(), line!())
                        .build(),
                );
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {}
