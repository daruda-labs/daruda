//! Pane-owned attachment drafts and their explicit composer submit path.

use super::{AgentChatView, AgentSessionStatus, PromptDispatch, PromptOrigin};
use crate::surface::strings as s;
use daruda_acp::PromptAttachment;
use gpui::Context;

impl AgentChatView {
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn seed_attachment_draft_for_shot(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        self.session_capabilities.embedded_context = true;
        let attachments = [
            "implementation-notes.txt",
            "configuration-for-the-windows-release.txt",
            "terminal-output.txt",
            "release-manifest.json",
        ]
        .into_iter()
        .map(|name| PromptAttachment {
            name: name.into(),
            content: daruda_acp::AttachmentContent::TextFile {
                text: "Capture fixture".into(),
                uri: format!("urn:screenshot:{name}"),
            },
        })
        .collect();
        self.add_attachments(attachments, cx)
    }
    pub(in crate::workspace) fn draft_attachments(&self) -> &[PromptAttachment] {
        &self.draft_attachments
    }

    pub(in crate::workspace) fn add_attachments(
        &mut self,
        attachments: Vec<PromptAttachment>,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if self.is_read_only() || !matches!(self.status, AgentSessionStatus::Connected) {
            return Err(s::agent_chat::attachment_connect_first().to_string());
        }
        if attachments
            .iter()
            .any(|a| !a.supported_by(self.session_capabilities))
        {
            return Err(s::agent_chat::attachment_unsupported().to_string());
        }
        self.check_attachment_budget(&attachments)?;
        self.draft_attachments.extend(attachments);
        cx.notify();
        Ok(())
    }

    fn check_attachment_budget(&self, attachments: &[PromptAttachment]) -> Result<(), String> {
        let total = self
            .draft_attachments
            .iter()
            .chain(attachments.iter())
            .map(PromptAttachment::encoded_size)
            .sum::<usize>();
        let queued = self
            .queue
            .pending_prompts
            .iter()
            .chain(&self.queue.paused_prompts)
            .flat_map(|q| &q.attachments)
            .map(PromptAttachment::encoded_size)
            .sum::<usize>();
        let editing = self.queue.editing_prompt.and_then(|id| {
            self.queue
                .pending_prompts
                .iter()
                .chain(&self.queue.paused_prompts)
                .find(|q| q.id == id)
        });
        let retained_count = editing.map_or(0, |q| q.attachments.len());
        let retained_size = editing.map_or(0, |q| {
            q.attachments
                .iter()
                .map(PromptAttachment::encoded_size)
                .sum::<usize>()
        });
        if self.draft_attachments.len() + attachments.len() + retained_count > 8
            || total + retained_size > 12 * 1024 * 1024
            || total + queued > 48 * 1024 * 1024
        {
            return Err(s::agent_chat::attachment_limit().to_string());
        }
        Ok(())
    }

    pub(in crate::workspace) fn remove_attachment(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.draft_attachments.len() {
            self.draft_attachments.remove(index);
            cx.notify();
        }
    }

    pub(in crate::workspace) fn send_composer_prompt(
        &mut self,
        text: String,
        cx: &mut Context<Self>,
    ) -> Result<PromptDispatch, String> {
        // Editing can start after the draft was built; validate the merged prompt now.
        self.check_attachment_budget(&[])?;
        let attachments = std::mem::take(&mut self.draft_attachments);
        let dispatch = self.send_prompt_content(text, attachments.clone(), PromptOrigin::InApp, cx);
        if matches!(
            dispatch,
            PromptDispatch::QueueFull | PromptDispatch::ReadOnly
        ) {
            self.draft_attachments = attachments;
        }
        Ok(dispatch)
    }

    pub(super) fn echo_attached_prompt(
        &mut self,
        text: String,
        attachments: &[PromptAttachment],
        cx: &mut Context<Self>,
    ) {
        self.echo_prompt(display_prompt(&text, attachments), cx);
    }
}

pub(in crate::workspace) fn display_prompt(text: &str, attachments: &[PromptAttachment]) -> String {
    if attachments.is_empty() {
        return text.to_owned();
    }
    let names = attachments
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    format!("{text}\n\n{}", s::agent_chat::attachment_names(&names))
}
