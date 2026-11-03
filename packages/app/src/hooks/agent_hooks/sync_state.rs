//! State transitions for the single hook-configuration worker.

use super::Request;

#[derive(Default)]
pub(super) enum SyncState {
    #[default]
    Idle,
    Applied(Request),
    Running(Request),
}

impl SyncState {
    /// Return work only when a worker must be started. An active worker
    /// consumes the latest desired request when its current work finishes.
    pub(super) fn request(&mut self, request: Request) -> Option<Request> {
        match self {
            Self::Running(desired) => {
                *desired = request;
                None
            }
            Self::Applied(applied) if *applied == request => None,
            _ => {
                *self = Self::Running(request.clone());
                Some(request)
            }
        }
    }

    pub(super) fn complete(&mut self, completed: &Request, succeeded: bool) -> Option<Request> {
        if let Self::Running(desired) = self {
            if desired != completed {
                return Some(desired.clone());
            }
            *self = if succeeded {
                Self::Applied(completed.clone())
            } else {
                Self::Idle
            };
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(enabled: bool) -> Request {
        Request {
            enabled,
            targets: Vec::new(),
        }
    }

    #[test]
    fn an_inflight_install_is_followed_by_the_latest_disable() {
        let mut state = SyncState::default();
        let installing = state.request(request(true)).unwrap();
        assert!(state.request(request(false)).is_none());
        let removing = state.complete(&installing, true).unwrap();
        assert!(!removing.enabled);
        assert!(state.complete(&removing, true).is_none());
        assert!(state.request(request(false)).is_none());
        assert!(matches!(state, SyncState::Applied(ref applied) if !applied.enabled));
    }

    #[test]
    fn a_failed_request_can_be_retried_without_an_automatic_retry_loop() {
        let mut state = SyncState::default();
        let work = state.request(request(true)).unwrap();
        assert!(state.complete(&work, false).is_none());
        assert!(matches!(state, SyncState::Idle));
        assert!(state.request(request(true)).is_some());
    }

    #[test]
    fn changes_back_to_the_inflight_value_need_no_second_job() {
        let mut state = SyncState::default();
        let work = state.request(request(true)).unwrap();
        state.request(request(false));
        state.request(request(true));
        assert!(state.complete(&work, true).is_none());
        assert!(matches!(state, SyncState::Applied(ref applied) if applied.enabled));
    }
}
