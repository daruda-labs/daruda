//! Acquire bounded attachment snapshots off the UI thread, bound to their pane.

use super::super::{
    agent_chat_pane::view::{AgentChatView, PromptDispatch},
    pane_tree::PaneId,
};
use crate::{surface::strings as s, workspace::Workspace};
use base64::Engine as _;
use daruda_acp::{AttachmentContent, PromptAttachment};
use daruda_store::observability::{error_report::ErrorReport, log_writer::LogWriter};
use gpui::{ClipboardEntry, Context, WeakEntity, Window};
use std::{io::Read as _, path::PathBuf};

const FILE_LIMIT: usize = 8 * 1024 * 1024;

enum Source {
    File(PathBuf),
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
                view.downgrade(),
                paths.into_iter().map(Source::File).collect(),
                cx,
            );
        } else {
            let shell = self
                .mirrors
                .shell_program
                .as_deref()
                .map(daruda_core::shell::quote::Shell::detect_from_program)
                .unwrap_or_default();
            let quoted = daruda_core::shell::quote::format_paths_for_drop(&paths, shell);
            self.input_dock
                .input
                .update(cx, |state, cx| state.insert(quoted, window, cx));
        }
    }

    pub(in crate::workspace) fn pick_composer_attachments(&mut self, cx: &mut Context<Self>) {
        let pane = self.active_runtime().focused_pane_id;
        let Some(view) = self.agent_chat_view(pane).cloned() else {
            return;
        };
        let target = view.downgrade();
        let selection = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(s::agent_chat::attachment_add().into()),
        });
        cx.spawn(async move |ws, cx| {
            let result = selection.await;
            if let Err(error) = ws.update(cx, |ws, cx| match result {
                Ok(Ok(Some(paths))) => {
                    ws.load_attachments(target, paths.into_iter().map(Source::File).collect(), cx)
                }
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
        let mut sources = Vec::new();
        for entry in item.into_entries() {
            match entry {
                ClipboardEntry::Image(image) => {
                    if image.bytes().len() > FILE_LIMIT {
                        self.attachment_error(s::agent_chat::attachment_limit().to_string(), cx);
                        return true;
                    }
                    sources.push(Source::Image(image.bytes().to_vec()));
                }
                ClipboardEntry::ExternalPaths(paths) => {
                    sources.extend(paths.paths().iter().cloned().map(Source::File))
                }
                _ => {}
            }
        }
        if sources.is_empty() {
            return false;
        }
        self.load_attachments(view.downgrade(), sources, cx);
        true
    }

    fn load_attachments(
        &mut self,
        target: WeakEntity<AgentChatView>,
        sources: Vec<Source>,
        cx: &mut Context<Self>,
    ) {
        if sources.len() > 8 {
            self.attachment_error(s::agent_chat::attachment_limit().to_string(), cx);
            return;
        }
        cx.spawn(async move |ws, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    sources
                        .into_iter()
                        .map(load)
                        .collect::<anyhow::Result<Vec<_>>>()
                })
                .await;
            if let Err(error) = ws.update(cx, |ws, cx| {
                let Some(view) = target.upgrade() else {
                    return;
                };
                let outcome = result
                    .map_err(|e| e.to_string())
                    .and_then(|a| view.update(cx, |v, cx| v.add_attachments(a, cx)));
                if let Err(error) = outcome {
                    ws.attachment_error(error, cx);
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

fn load(source: Source) -> anyhow::Result<PromptAttachment> {
    match source {
        Source::Image(bytes) => image_attachment(
            s::agent_chat::attachment_clipboard_name().to_string(),
            &bytes,
        ),
        Source::File(path) => {
            let file = std::fs::File::open(&path)?;
            anyhow::ensure!(
                file.metadata()?.is_file(),
                "Only regular files can be attached"
            );
            let mut bytes = Vec::new();
            file.take((FILE_LIMIT + 1) as u64).read_to_end(&mut bytes)?;
            anyhow::ensure!(bytes.len() <= FILE_LIMIT, "Attachment exceeds 8 MiB");
            let name = path
                .file_name()
                .ok_or_else(|| anyhow::anyhow!("File has no name"))?
                .to_string_lossy()
                .into_owned();
            if image::guess_format(&bytes).is_ok() {
                return image_attachment(name, &bytes);
            }
            let text = String::from_utf8(bytes)?;
            anyhow::ensure!(
                !text.contains('\0'),
                "Binary attachments are not supported by the agent protocol"
            );
            let uri = daruda_core::file_url::from_local_path(&path)
                .ok_or_else(|| anyhow::anyhow!("Attachment path must be absolute"))?;
            Ok(PromptAttachment {
                name,
                content: AttachmentContent::TextFile { text, uri },
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
    fn queued_file_contents_are_snapshots_and_oversized_files_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.txt");
        std::fs::write(&path, "original").unwrap();
        let attachment = load(Source::File(path.clone())).unwrap();
        std::fs::write(&path, "changed").unwrap();
        assert!(
            matches!(attachment.content, AttachmentContent::TextFile { text, .. } if text == "original")
        );
        std::fs::File::create(&path)
            .unwrap()
            .set_len((FILE_LIMIT + 1) as u64)
            .unwrap();
        assert!(load(Source::File(path)).is_err());
    }
}
