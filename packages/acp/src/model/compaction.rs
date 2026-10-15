//! Session-scoped context compaction retained in the conversation timeline.

use super::ToolOutputBlock;

#[derive(Debug, Clone, PartialEq)]
pub struct CompactionItem {
    pub session_id: String,
    pub id: String,
    pub status: CompactionStatusView,
    pub summary: Vec<ToolOutputBlock>,
    pub error: Option<String>,
    pub meta: Option<serde_json::Map<String, serde_json::Value>>,
}

impl CompactionItem {
    pub fn new(session_id: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            id: id.into(),
            status: CompactionStatusView::InProgress,
            summary: Vec::new(),
            error: None,
            meta: None,
        }
    }

    /// Separate from tool IDs even when the agent reuses the same strings.
    pub fn output_key(&self) -> String {
        format!(
            "compaction:{}",
            serde_json::json!([self.session_id, self.id])
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactionStatusView {
    InProgress,
    Completed,
    Failed,
    Cancelled,
    Other(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_identity_preserves_session_and_id_boundaries() {
        assert_ne!(
            CompactionItem::new("a:b", "c").output_key(),
            CompactionItem::new("a", "b:c").output_key()
        );
    }
}
