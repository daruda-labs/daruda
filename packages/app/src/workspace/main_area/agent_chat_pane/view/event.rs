//! What a chat asks of the window that hosts it. The view never reaches for
//! the `Workspace`; it emits one of these and the subscription made where the
//! view is built acts on it. Data only — wording is the host's to choose.

use daruda_store::observability::error_report::ErrorReport;

pub(in crate::workspace) enum AgentChatEvent {
    /// A failure to show and log the way every other one is.
    ReportError(ErrorReport),
    /// A pane preference the saved layout carries changed.
    PrefsChanged,
    /// The status banner's retry after a failed connect.
    RetryConnect,
    /// "Sign in again": the session's account needs a fresh login.
    Reauthenticate,
}

impl gpui::EventEmitter<AgentChatEvent> for super::AgentChatView {}
