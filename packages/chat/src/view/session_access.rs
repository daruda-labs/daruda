//! The pane's session access: interactive or a read-only mirror, and the
//! handle each may reach.

use daruda_acp::AcpSessionHandle;

use super::{AgentChatView, Turn};

pub(super) fn new_permission_session() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// What the pane holds of its ACP session and what it may do with it. The
/// handle is `None` until connect resolves or after a terminal error; dropping
/// it closes the command channel. A `Mirror` shows a CLI task run read-only:
/// its handle carries the `session/load` replay and nothing the user does may
/// reach it, which [`AgentChatView::live_handle`] enforces by construction.
pub(crate) enum Session {
    Interactive(Option<AcpSessionHandle>),
    Mirror {
        run: daruda_store::tasks::ExecutionRef,
        intent: LoadIntent,
        handle: Option<AcpSessionHandle>,
    },
}

/// Why a mirror is loading its session: to show the history again, or to
/// take the conversation over once the CLI is confirmed gone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LoadIntent {
    #[default]
    Snapshot,
    Continue,
}

impl AgentChatView {
    /// The persisted form of the pane's session — interactive, or a mirror.
    pub fn access(&self) -> daruda_store::tasks::AgentChatAccess {
        match &self.session {
            Session::Interactive(_) => daruda_store::tasks::AgentChatAccess::Interactive,
            Session::Mirror { run, .. } => {
                daruda_store::tasks::AgentChatAccess::CliSnapshot(run.clone())
            }
        }
    }

    pub fn is_read_only(&self) -> bool {
        matches!(self.session, Session::Mirror { .. })
    }

    /// The run a mirror shows; `None` for an interactive pane.
    pub fn mirrored_run(&self) -> Option<&daruda_store::tasks::ExecutionRef> {
        match &self.session {
            Session::Interactive(_) => None,
            Session::Mirror { run, .. } => Some(run),
        }
    }

    /// The handle behind the pane, whatever its access — only for the replay
    /// bookkeeping a mirror still owes the adapter (cancelling permissions).
    /// User input goes through `live_handle`.
    pub fn any_handle(&self) -> Option<&AcpSessionHandle> {
        match &self.session {
            Session::Interactive(handle) | Session::Mirror { handle, .. } => handle.as_ref(),
        }
    }

    /// The handle user input may reach: an interactive pane's, never a
    /// mirror's. Every send in this module goes through here.
    pub(crate) fn live_handle(&self) -> Option<&AcpSessionHandle> {
        match &self.session {
            Session::Interactive(handle) => handle.as_ref(),
            Session::Mirror { .. } => None,
        }
    }

    /// Store a resolved connection; the access mode is untouched.
    pub fn attach_handle(&mut self, next: AcpSessionHandle) {
        match &mut self.session {
            Session::Interactive(handle) | Session::Mirror { handle, .. } => *handle = Some(next),
        }
    }

    /// Let go of the connection; the access mode is untouched.
    pub fn detach_handle(&mut self) {
        match &mut self.session {
            Session::Interactive(handle) | Session::Mirror { handle, .. } => *handle = None,
        }
    }

    /// Restore the persisted access mode. A mirror keeps whatever handle is
    /// held, so this is safe before and after connect — but only on a pane
    /// with nothing in flight: the read-only guarantee rests on a mirror
    /// never holding a live turn, a pending permission or a queued prompt.
    pub fn set_access(&mut self, access: daruda_store::tasks::AgentChatAccess) {
        debug_assert!(
            !access.is_read_only()
                || (matches!(self.queue.turn, Turn::Idle)
                    && self.pending_permissions.is_empty()
                    && self.queued_prompt_count() == 0),
            "a mirror is set on a quiescent pane"
        );
        let handle = self.detach_handle_value();
        self.session = match access {
            daruda_store::tasks::AgentChatAccess::Interactive => Session::Interactive(handle),
            daruda_store::tasks::AgentChatAccess::CliSnapshot(run) => Session::Mirror {
                run,
                intent: LoadIntent::Snapshot,
                handle,
            },
        };
    }

    /// A continue that completed: the same session, now the user's.
    pub fn make_interactive(&mut self) {
        let handle = self.detach_handle_value();
        self.session = Session::Interactive(handle);
    }

    pub fn load_intent(&self) -> Option<LoadIntent> {
        match &self.session {
            Session::Interactive(_) => None,
            Session::Mirror { intent, .. } => Some(*intent),
        }
    }

    pub fn set_load_intent(&mut self, next: LoadIntent) {
        if let Session::Mirror { intent, .. } = &mut self.session {
            *intent = next;
        }
    }

    /// A load has settled, one way or the other; the next one starts as a
    /// plain snapshot.
    pub fn settle_mirror_intent(&mut self) {
        self.set_load_intent(LoadIntent::Snapshot);
    }

    fn detach_handle_value(&mut self) -> Option<AcpSessionHandle> {
        match &mut self.session {
            Session::Interactive(handle) | Session::Mirror { handle, .. } => handle.take(),
        }
    }
}
