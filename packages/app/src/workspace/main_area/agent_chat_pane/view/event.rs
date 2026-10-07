//! What a chat asks of the window that hosts it. The view never reaches for
//! the `Workspace`; it emits one of these and the subscription made where the
//! view is built acts on it. Data only — wording is the host's to choose.

use std::path::PathBuf;

use daruda_content::link_target::LinkTarget;
use daruda_store::observability::error_report::ErrorReport;
use gpui::{Pixels, Point};

pub(in crate::workspace) enum AgentChatEvent {
    /// A failure to show and log the way every other one is.
    ReportError(ErrorReport),
    /// A pane preference the saved layout carries changed.
    PrefsChanged,
    /// The status banner's retry after a failed connect.
    RetryConnect,
    /// "Sign in again": the session's account needs a fresh login.
    Reauthenticate,
    /// A link or tool resource was clicked, already classified against this
    /// pane's session — the host only has to open it.
    OpenLink(LinkTarget),
    /// A diff header's path: show that file in the pane-area file viewer.
    OpenDiffInFileView(PathBuf),
    /// A diff header's "open externally": hand the file to the user's editor.
    OpenFileExternally(PathBuf),
    /// A right press on a tool resource, recorded so the pane menu it opens
    /// resolves the resource as the click would.
    ResourceRightClicked {
        position: Point<Pixels>,
        uri: String,
        mime: Option<String>,
    },
}

impl gpui::EventEmitter<AgentChatEvent> for super::AgentChatView {}
