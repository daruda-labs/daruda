//! What a chat asks of the window that hosts it. The view never reaches for
//! the `Workspace`; it emits one of these and the subscription made where the
//! view is built acts on it. Data only — wording is the host's to choose.

use std::path::PathBuf;

use daruda_content::link_target::LinkTarget;
use daruda_store::observability::error_report::ErrorReport;
use daruda_store::tasks::ExecutionRef;
use gpui::{Pixels, Point};

pub enum AgentChatEvent {
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
    /// A task's prompt was just handed to the live session. Once per tagged
    /// prompt, whether it went out at once or drained from the queue; an
    /// untagged prompt reports nothing.
    PromptSent { task: ExecutionRef },
    /// Task prompts left the queue without ever being sent. Only tagged
    /// prompts are listed, and the event is skipped when none were dropped.
    PromptsDropped {
        tasks: Vec<ExecutionRef>,
        reason: PromptDropReason,
    },
}

/// Why queued prompts were discarded unsent. Parking on Stop is not a drop:
/// a parked prompt can still be resumed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptDropReason {
    /// The session failed terminally, so nothing queued can be delivered.
    ConnectFailed,
    /// `/clear` or a reconnect replaced the session the prompts waited on.
    SessionTornDown,
    /// The user removed the prompt from the strip or cleared the queue.
    RemovedByUser,
}

impl gpui::EventEmitter<AgentChatEvent> for super::AgentChatView {}
