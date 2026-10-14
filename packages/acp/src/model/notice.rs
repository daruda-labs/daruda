//! Live advisories, separate from transcript content and turn outcomes.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionNoticeView {
    pub severity: NoticeSeverityView,
    pub title: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoticeSeverityView {
    Info,
    Warning,
    Error,
    Other(String),
}

impl NoticeSeverityView {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Other(value) => value,
        }
    }
}

impl From<agent_client_protocol::schema::v1::Notice> for SessionNoticeView {
    fn from(notice: agent_client_protocol::schema::v1::Notice) -> Self {
        use agent_client_protocol::schema::v1::NoticeSeverity;
        let severity = match notice.severity {
            NoticeSeverity::Info => NoticeSeverityView::Info,
            NoticeSeverity::Warning => NoticeSeverityView::Warning,
            NoticeSeverity::Error => NoticeSeverityView::Error,
            NoticeSeverity::Other(value) => NoticeSeverityView::Other(value),
            other => NoticeSeverityView::Other(format!("{other:?}")),
        };
        Self {
            severity,
            title: notice.title,
            description: notice.description,
        }
    }
}

#[cfg(test)]
mod tests;
