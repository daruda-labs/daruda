//! Pure interpretation of hook completion for task execution policy.

use daruda_store::tasks::SessionEndReason;

/// Interpret hook completion once; only the runs layer maps it to task policy.
pub(crate) fn hook_end_reason(event: &str) -> Option<SessionEndReason> {
    use daruda_agent::hooks::turn_end::HookEnd;
    HookEnd::from_event(event).map(|end| match end {
        HookEnd::Completed => SessionEndReason::Stop,
        HookEnd::Failed => SessionEndReason::Error,
        HookEnd::SessionEnded => SessionEndReason::Other,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_hooks_and_exit_recovery_share_completion_semantics() {
        assert_eq!(hook_end_reason("Stop"), Some(SessionEndReason::Stop));
        assert_eq!(
            hook_end_reason("StopFailure"),
            Some(SessionEndReason::Error)
        );
        assert_eq!(hook_end_reason("SessionEnd"), Some(SessionEndReason::Other));
        assert_eq!(hook_end_reason("Interrupt"), None);
        assert_eq!(hook_end_reason("PostToolUseFailure"), None);
    }
}
