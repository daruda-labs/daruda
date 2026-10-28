//! Native prompt-file selection, resolved relative to the Flow definition.
use super::super::FlowGraphView;
use crate::surface::strings as s;
use daruda_store::observability::{error_report::ErrorReport, log_writer::LogWriter};
use gpui::{Context, Window};
use std::path::Path;

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
                                if !view.apply_prompt_edit(&edit, portable(&relative), window, cx) {
                                    view.inspector.set_notice(Some(
                                        super::super::inspector::Notice::PromptStale,
                                    ));
                                    cx.notify();
                                }
                            }
                            None => {
                                view.inspector.set_notice(Some(
                                    super::super::inspector::Notice::PromptOutside,
                                ));
                                cx.notify();
                            }
                        }
                    }
                }
                Ok(Err(error)) => cx.emit(super::super::FlowGraphEvent::PromptPickerFailed {
                    detail: format!("{error:#}"),
                }),
                _ => {}
            }) {
                // The view is gone, so there is no workspace path to report on.
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

/// The path as the flow file stores it: `/`-separated on every OS, so a flow
/// authored on Windows resolves the same file on macOS and Linux.
fn portable(relative: &Path) -> String {
    relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_paths_use_forward_slashes() {
        let relative: std::path::PathBuf = ["prompts", "nested", "plan.md"].iter().collect();
        assert_eq!(portable(&relative), "prompts/nested/plan.md");
    }
}
