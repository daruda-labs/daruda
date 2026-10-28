//! One immutable file revision, parsed once for all inspector operations.
use daruda_flow::{
    NodeId,
    parse::{FlowFile, NodeFile},
};
use std::sync::Arc;

#[derive(Clone)]
pub(super) struct Baseline {
    text: Arc<str>,
    file: Option<Arc<FlowFile>>,
}

impl Baseline {
    pub(super) fn new(text: Arc<str>) -> Self {
        let file = daruda_flow::parse::parse_flow_file(&text)
            .ok()
            .map(Arc::new);
        Self { text, file }
    }

    pub(super) fn text(&self) -> &Arc<str> {
        &self.text
    }
    pub(super) fn file(&self) -> Option<&FlowFile> {
        self.file.as_deref()
    }
    pub(super) fn node(&self, node: &NodeId) -> Option<&NodeFile> {
        self.file()?.nodes.iter().find(|entry| &entry.id == node)
    }
    pub(super) fn matches(&self, other: &Self, node: &NodeId) -> bool {
        if Arc::ptr_eq(&self.text, &other.text) || self.text == other.text {
            return true;
        }
        match (self.file(), other.file()) {
            (Some(a), Some(b)) => a.defaults == b.defaults && self.node(node) == other.node(node),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {}
