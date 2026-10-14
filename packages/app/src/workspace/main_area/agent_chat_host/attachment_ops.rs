//! Acquire path references and bounded clipboard images, bound to their pane.

use super::super::pane_tree::PaneId;
use crate::{surface::strings as s, workspace::Workspace};
use base64::Engine as _;
use daruda_acp::{AttachmentContent, PromptAttachment};
use daruda_chat::view::{AgentChatView, PromptDispatch};
use daruda_store::observability::{error_report::ErrorReport, log_writer::LogWriter};
use gpui::{ClipboardEntry, Context, WeakEntity, Window};
use std::path::PathBuf;

const FILE_LIMIT: usize = 8 * 1024 * 1024;

enum Source {
    Path(PathBuf),
    Image(Vec<u8>),
}

impl Workspace {
    fn composer_attachments_changed(&mut self, cx: &mut Context<Self>) {
        self.main_area.pending_resize = true;
        cx.notify();
    }
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn open_attachment_draft_for_shot(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_agent_chat_transcript_for_shot(window, cx);
        // Leave spare editor space so attachment-induced collapse is visible.
        self.set_bottom_dock_row_preset(
            crate::workspace::layout::ops::bottom_dock_height_for_rows(usize::from(
                self.mirrors.agent.input_max_rows,
            )),
            window,
            cx,
        );
        let pane = self.active_runtime().focused_pane_id;
        if let Some(view) = self.agent_chat_view(pane).cloned() {
            let result = view.update(cx, |view, cx| view.seed_attachment_draft_for_shot(cx));
            if let Err(error) = result {
                self.attachment_error(error, cx);
            }
            self.composer_attachments_changed(cx);
        }
    }
    pub(in crate::workspace) fn composer_paste_action(&mut self, cx: &mut Context<Self>) {
        if self.paste_composer_attachments(cx) {
            cx.stop_propagation();
        } else {
            cx.propagate();
        }
    }
    pub(in crate::workspace) fn composer_drop(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pane = self.active_runtime().focused_pane_id;
        if let Some(view) = self.agent_chat_view(pane).cloned() {
            self.load_attachments(
                pane,
                view.downgrade(),
                paths.into_iter().map(Source::Path).collect(),
                cx,
            );
        } else {
            self.insert_composer_paths(&paths, window, cx);
        }
    }

    fn insert_composer_paths(
        &mut self,
        paths: &[PathBuf],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let quoted = self.composer_paths_text(paths);
        self.input_dock
            .input
            .update(cx, |state, cx| state.insert(quoted, window, cx));
    }

    fn composer_paths_text(&self, paths: &[PathBuf]) -> String {
        let shell = self
            .mirrors
            .shell_program
            .as_deref()
            .map(daruda_core::shell::quote::Shell::detect_from_program)
            .unwrap_or_default();
        daruda_core::shell::quote::format_paths_for_drop(paths, shell)
    }

    pub(in crate::workspace) fn pick_composer_attachments(&mut self, cx: &mut Context<Self>) {
        let pane = self.active_runtime().focused_pane_id;
        let Some(view) = self.agent_chat_view(pane).cloned() else {
            return;
        };
        let target = view.downgrade();
        let selection = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: true,
            multiple: true,
            prompt: Some(s::agent_chat::attachment_add().into()),
        });
        cx.spawn(async move |ws, cx| {
            let result = selection.await;
            if let Err(error) = ws.update(cx, |ws, cx| match result {
                Ok(Ok(Some(paths))) => ws.load_attachments(
                    pane,
                    target,
                    paths.into_iter().map(Source::Path).collect(),
                    cx,
                ),
                Ok(Err(error)) => ws.attachment_error(error.to_string(), cx),
                _ => {}
            }) {
                LogWriter::log(
                    ErrorReport::new("Attachment picker closed with its workspace")
                        .from_error(error.root_cause())
                        .at(file!(), line!())
                        .build(),
                );
            }
        })
        .detach();
    }

    /// Return whether clipboard attachment content consumed the paste action.
    pub(in crate::workspace) fn paste_composer_attachments(
        &mut self,
        cx: &mut Context<Self>,
    ) -> bool {
        let pane = self.active_runtime().focused_pane_id;
        let Some(view) = self.agent_chat_view(pane).cloned() else {
            return false;
        };
        let Some(item) = cx.read_from_clipboard() else {
            return false;
        };
        let entries: Vec<_> = item.into_entries().collect();
        let paths: Vec<_> = entries
            .iter()
            .filter_map(|entry| match entry {
                ClipboardEntry::ExternalPaths(paths) => Some(paths.paths()),
                _ => None,
            })
            .flatten()
            .cloned()
            .map(Source::Path)
            .collect();
        // File copies can include an image preview; the original paths take precedence.
        if !paths.is_empty() {
            self.load_attachments(pane, view.downgrade(), paths, cx);
            return true;
        }
        let mut sources = Vec::new();
        for entry in entries {
            if let ClipboardEntry::Image(image) = entry {
                if image.bytes().len() > FILE_LIMIT {
                    self.attachment_error(daruda_chat::labels::attachment_limit().to_string(), cx);
                    return true;
                }
                sources.push(Source::Image(image.bytes().to_vec()));
            }
        }
        if sources.is_empty() {
            return false;
        }
        self.load_attachments(pane, view.downgrade(), sources, cx);
        true
    }

    fn load_attachments(
        &mut self,
        pane: PaneId,
        target: WeakEntity<AgentChatView>,
        sources: Vec<Source>,
        cx: &mut Context<Self>,
    ) {
        let fallback_paths: Vec<_> = sources
            .iter()
            .filter_map(|source| match source {
                Source::Path(path) => Some(path.clone()),
                Source::Image(_) => None,
            })
            .collect();
        cx.spawn(async move |ws, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { load_sources(sources) })
                .await;
            if let Err(error) = ws.update_in(cx, |ws, window, cx| {
                let Some(view) = target.upgrade() else {
                    return;
                };
                let outcome = result.map_err(|e| e.to_string()).and_then(|attachments| {
                    view.update(cx, |v, cx| v.add_attachments(attachments, cx))
                });
                if let Err(error) = outcome {
                    if !fallback_paths.is_empty() {
                        if ws.input_dock.owner == Some(pane) {
                            ws.insert_composer_paths(&fallback_paths, window, cx);
                        } else {
                            // Preserve the original pane's draft if focus changed during loading.
                            let text = ws.composer_paths_text(&fallback_paths);
                            let draft = ws.input_dock.drafts.entry(pane).or_default();
                            if !draft.is_empty() {
                                draft.push(' ');
                            }
                            draft.push_str(&text);
                        }
                        LogWriter::log(
                            ErrorReport::new("Attachment paths inserted as text")
                                .message(error)
                                .severity(
                                    daruda_store::observability::error_report::ErrorSeverity::Info,
                                )
                                .at(file!(), line!())
                                .build(),
                        );
                    } else {
                        ws.attachment_error(error, cx);
                    }
                }
                ws.composer_attachments_changed(cx);
            }) {
                LogWriter::log(
                    ErrorReport::new("Attachment load closed with its workspace")
                        .from_error(error.root_cause())
                        .at(file!(), line!())
                        .build(),
                );
            }
        })
        .detach();
    }

    pub(in crate::workspace) fn remove_composer_attachment(
        &mut self,
        pane: PaneId,
        index: usize,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.agent_chat_view(pane).cloned() {
            view.update(cx, |v, cx| v.remove_attachment(index, cx));
            self.composer_attachments_changed(cx);
        }
    }

    /// Only an explicit composer submission consumes its pane's attachment draft.
    pub(in crate::workspace) fn send_attached_composer_if_any(
        &mut self,
        text: String,
        cx: &mut Context<Self>,
    ) -> Option<bool> {
        let pane = self.active_runtime().focused_pane_id;
        let view = self.agent_chat_view(pane)?.clone();
        if view.read(cx).draft_attachments().is_empty() {
            return None;
        }
        let dispatch = match view.update(cx, |v, cx| v.send_composer_prompt(text, cx)) {
            Ok(dispatch) => dispatch,
            Err(error) => {
                self.attachment_error(error, cx);
                return Some(false);
            }
        };
        self.composer_attachments_changed(cx);
        let accepted = matches!(dispatch, PromptDispatch::Queued | PromptDispatch::SentNow);
        if !accepted {
            self.attachment_error(s::agent_chat::queue_full().to_string(), cx);
        }
        let edge = view.update(cx, |v, cx| v.tick_activity(std::time::Instant::now(), cx));
        if let Some(outcome) = edge {
            self.fire_activity_completion(pane, outcome, cx);
        }
        cx.notify();
        Some(accepted)
    }

    fn attachment_error(&mut self, detail: String, cx: &mut Context<Self>) {
        self.report_error(
            ErrorReport::new(s::agent_chat::attachment_failed())
                .message(detail)
                .at(file!(), line!())
                .build(),
            cx,
        );
    }
}

fn load_sources(sources: Vec<Source>) -> anyhow::Result<Vec<PromptAttachment>> {
    anyhow::ensure!(
        sources.len() <= 8,
        "{}",
        daruda_chat::labels::attachment_limit()
    );
    sources.into_iter().map(load).collect()
}

fn load(source: Source) -> anyhow::Result<PromptAttachment> {
    match source {
        Source::Image(bytes) => image_attachment(
            s::agent_chat::attachment_clipboard_name().to_string(),
            &bytes,
        ),
        Source::Path(path) => {
            let uri = daruda_core::file_url::from_local_path(&path)
                .ok_or_else(|| anyhow::anyhow!("Attachment path must be absolute"))?;
            let name = path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .into_owned();
            Ok(PromptAttachment {
                name,
                content: AttachmentContent::ResourceLink { uri },
            })
        }
    }
}

fn image_attachment(name: String, bytes: &[u8]) -> anyhow::Result<PromptAttachment> {
    use image::ImageEncoder as _;
    anyhow::ensure!(bytes.len() <= FILE_LIMIT, "Image exceeds 8 MiB");
    let mut raster = daruda_content::visual::decode_image_bounded(bytes, 4096, 64 * 1024 * 1024)?;
    for pixel in raster.bgra.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let mut png = Vec::new();
    image::codecs::png::PngEncoder::new(&mut png).write_image(
        &raster.bgra,
        raster.width,
        raster.height,
        image::ExtendedColorType::Rgba8,
    )?;
    anyhow::ensure!(png.len() <= FILE_LIMIT, "Encoded image exceeds 8 MiB");
    Ok(PromptAttachment {
        name,
        content: AttachmentContent::Image {
            data: base64::engine::general_purpose::STANDARD.encode(png),
            mime_type: "image/png".into(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_paths_are_references_without_reading_file_contents() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("folder with spaces");
        std::fs::create_dir(&folder).unwrap();
        let file = dir.path().join("binary.dat");
        std::fs::File::create(&file)
            .unwrap()
            .set_len((FILE_LIMIT + 1) as u64)
            .unwrap();
        let missing = dir.path().join("missing.txt");
        for path in [folder, file, missing, dir.path().to_path_buf()] {
            let attachment = load(Source::Path(path.clone())).unwrap();
            assert_eq!(
                attachment.content,
                AttachmentContent::ResourceLink {
                    uri: daruda_core::file_url::from_local_path(&path).unwrap(),
                }
            );
        }
    }
}
