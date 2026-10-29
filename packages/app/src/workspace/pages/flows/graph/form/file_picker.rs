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
            // A cancelled channel means the picker went away unanswered —
            // the same as choosing nothing.
            let Ok(result) = selection.await else {
                return;
            };
            if let Err(error) = this.update_in(cx, |view, window, cx| {
                view.finish_prompt_pick(&edit, &root, result, window, cx)
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

    /// Put the picked file into the prompt, or say why it was not: it lies
    /// outside the flow's directory, the node changed meanwhile, or the
    /// native picker failed (the workspace's toast reports that).
    fn finish_prompt_pick(
        &mut self,
        edit: &super::prompt_edit::PromptEdit,
        root: &Path,
        result: anyhow::Result<Option<Vec<std::path::PathBuf>>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use super::super::inspector::Notice;
        let path = match result {
            Ok(Some(paths)) => match paths.into_iter().next() {
                Some(path) => path,
                None => return,
            },
            Ok(None) => return,
            Err(error) => {
                cx.emit(super::super::FlowGraphEvent::PromptPickerFailed {
                    detail: format!("{error:#}"),
                });
                return;
            }
        };
        let notice = match daruda_core::path::strip_root(&path, root) {
            Some(relative) if self.apply_prompt_edit(edit, portable(&relative), window, cx) => {
                return;
            }
            Some(_) => Notice::PromptStale,
            None => Notice::PromptOutside,
        };
        self.update_inspector(|model| model.set_notice(Some(notice)), cx);
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
