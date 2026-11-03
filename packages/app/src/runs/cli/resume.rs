//! Exclusive ownership while a CLI conversation is loaded and continued.

use std::collections::HashMap;
use std::rc::{Rc, Weak};

use crate::runs::RunPane;

#[derive(Default)]
pub(in crate::runs) struct ResumeClaims {
    sessions: HashMap<String, Weak<()>>,
}

/// Dropping the pane or abandoning its load makes the session claim reusable.
pub(crate) struct CliResumeLease {
    session: String,
    owner: RunPane,
    _token: Rc<()>,
}

impl CliResumeLease {
    pub(crate) fn matches(&self, session: &str, owner: RunPane) -> bool {
        self.session == session && self.owner == owner
    }
}

impl ResumeClaims {
    pub(in crate::runs) fn claim(
        &mut self,
        session: String,
        owner: RunPane,
    ) -> Option<CliResumeLease> {
        self.sessions.retain(|_, token| token.strong_count() > 0);
        if self.sessions.contains_key(&session) {
            return None;
        }
        let token = Rc::new(());
        self.sessions.insert(session.clone(), Rc::downgrade(&token));
        Some(CliResumeLease {
            session,
            owner,
            _token: token,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_store::project::WorkspaceUuid;

    #[test]
    fn two_windows_cannot_continue_the_same_session_until_its_owner_releases_it() {
        let mut claims = ResumeClaims::default();
        let a = RunPane {
            workspace: WorkspaceUuid::new(),
            pane: 1,
        };
        let b = RunPane {
            workspace: WorkspaceUuid::new(),
            pane: 1,
        };
        let lease = claims.claim("session".into(), a).unwrap();
        assert!(claims.claim("session".into(), b).is_none());
        assert!(claims.claim("different".into(), b).is_some());
        assert!(lease.matches("session", a));
        drop(lease);
        assert!(claims.claim("session".into(), b).is_some());
    }
}
