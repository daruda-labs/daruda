//! Prompt attachments retain path references or content snapshots with their queued turn.

use crate::SessionCapabilitiesView;
use agent_client_protocol::schema::v1::{
    ContentBlock, EmbeddedResource, EmbeddedResourceResource, ImageContent, ResourceLink,
    TextContent, TextResourceContents,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptAttachment {
    pub name: String,
    pub content: AttachmentContent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttachmentContent {
    Image { data: String, mime_type: String },
    TextFile { text: String, uri: String },
    ResourceLink { uri: String },
}

impl PromptAttachment {
    pub fn supported_by(&self, capabilities: SessionCapabilitiesView) -> bool {
        match self.content {
            AttachmentContent::ResourceLink { .. } => true,
            AttachmentContent::Image { .. } => capabilities.images,
            AttachmentContent::TextFile { .. } => capabilities.embedded_context,
        }
    }

    pub fn encoded_size(&self) -> usize {
        match &self.content {
            AttachmentContent::ResourceLink { uri } => uri.len(),
            AttachmentContent::Image { data, .. } => data.len(),
            AttachmentContent::TextFile { text, .. } => text.len(),
        }
    }

    fn into_block(self) -> ContentBlock {
        match self.content {
            AttachmentContent::ResourceLink { uri } => {
                ContentBlock::ResourceLink(ResourceLink::new(self.name, uri))
            }
            AttachmentContent::Image { data, mime_type } => {
                ContentBlock::Image(ImageContent::new(data, mime_type))
            }
            AttachmentContent::TextFile { text, uri } => ContentBlock::Resource(
                EmbeddedResource::new(EmbeddedResourceResource::TextResourceContents(
                    TextResourceContents::new(text, uri).mime_type("text/plain"),
                )),
            ),
        }
    }
}

pub(crate) struct PromptInput {
    pub text: String,
    pub attachments: Vec<PromptAttachment>,
}

impl From<String> for PromptInput {
    fn from(text: String) -> Self {
        Self {
            text,
            attachments: Vec::new(),
        }
    }
}

impl From<&str> for PromptInput {
    fn from(text: &str) -> Self {
        text.to_owned().into()
    }
}

impl PromptInput {
    pub fn into_blocks(self) -> Vec<ContentBlock> {
        let mut blocks = Vec::new();
        if !self.text.is_empty() {
            blocks.push(ContentBlock::Text(TextContent::new(self.text)));
        }
        blocks.extend(
            self.attachments
                .into_iter()
                .map(PromptAttachment::into_block),
        );
        blocks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_references_send_only_the_uri_without_optional_capabilities() {
        let attachment = PromptAttachment {
            name: "note.txt".into(),
            content: AttachmentContent::ResourceLink {
                uri: "file:///tmp/folder%20with%20spaces/note.txt".into(),
            },
        };
        assert!(attachment.supported_by(SessionCapabilitiesView::default()));
        let blocks = PromptInput {
            text: "inspect".into(),
            attachments: vec![attachment],
        }
        .into_blocks();
        let ContentBlock::ResourceLink(link) = &blocks[1] else {
            panic!("local files must be references, never embedded contents");
        };
        assert_eq!(link.name, "note.txt");
        assert_eq!(link.uri, "file:///tmp/folder%20with%20spaces/note.txt");
    }

    #[test]
    fn attachments_keep_protocol_types_and_advertised_capability_gates() {
        let attachment = PromptAttachment {
            name: "note.txt".into(),
            content: AttachmentContent::TextFile {
                text: "snapshot".into(),
                uri: "urn:test:note".into(),
            },
        };
        assert!(!attachment.supported_by(SessionCapabilitiesView::default()));
        let caps = SessionCapabilitiesView {
            embedded_context: true,
            ..Default::default()
        };
        assert!(attachment.supported_by(caps));
        let blocks = PromptInput {
            text: "inspect".into(),
            attachments: vec![attachment],
        }
        .into_blocks();
        assert!(matches!(blocks[0], ContentBlock::Text(_)));
        assert!(matches!(blocks[1], ContentBlock::Resource(_)));
    }
}
