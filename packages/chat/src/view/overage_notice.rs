//! What a pane keeps after stopping a turn to avoid extra-usage billing.

use daruda_acp::{RateLimitView, RateLimitWindow};

use crate::strings as s;

/// The limit an overage stop reported, shown in the pane's banner until the
/// next prompt dispatches. Its presence also marks the turn as already
/// stopped, so a second billed `usage_update` cannot cancel again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverageNotice {
    /// The limit that ran out.
    pub window: Option<RateLimitWindow>,
    /// When that window resets, in unix seconds.
    pub resets_at: Option<i64>,
}

impl OverageNotice {
    /// Localized name of the limit, shared by the banner and the host's
    /// toast. An unknown window is shown verbatim rather than hidden.
    pub fn window_label(&self) -> String {
        match &self.window {
            Some(RateLimitWindow::FiveHour) => s::agent_chat::overage_window_five_hour(),
            Some(RateLimitWindow::SevenDay) => s::agent_chat::overage_window_seven_day(),
            Some(RateLimitWindow::Other(other)) => other.clone(),
            None => s::agent_chat::overage_window_generic(),
        }
    }
}

impl From<&RateLimitView> for OverageNotice {
    fn from(limit: &RateLimitView) -> Self {
        Self {
            window: limit.window.clone(),
            resets_at: limit.resets_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notice_keeps_the_window_and_reset_of_the_limit() {
        let limit = RateLimitView {
            window: Some(RateLimitWindow::FiveHour),
            resets_at: Some(1_800_000_000),
            is_using_overage: true,
        };
        assert_eq!(
            OverageNotice::from(&limit),
            OverageNotice {
                window: Some(RateLimitWindow::FiveHour),
                resets_at: Some(1_800_000_000),
            }
        );
    }
}
