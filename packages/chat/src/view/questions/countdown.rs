//! The opt-in answer timeout (S6-b): each open question's deadline, the
//! touch that stops it for good, and the decline sent once time is up.

use std::time::{Duration, Instant};

use daruda_acp::QuestionAnswer;
use gpui::Context;

use super::{OpenQuestion, QuestionOutcome, Questions};
use crate::strings as s;
use crate::view::AgentChatView;

/// The last stretch of a countdown, drawn heavier so a glance catches it.
const FINAL_STRETCH_SECS: u64 = 30;

/// What is left of an open question's answer time, in whole seconds
/// rounded up, so "0:00" never shows while the question still waits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuestionCountdown {
    secs: u64,
}

impl QuestionCountdown {
    pub fn until(deadline: Instant, now: Instant) -> Self {
        let left = deadline.saturating_duration_since(now);
        Self {
            secs: left.as_secs() + u64::from(left.subsec_nanos() > 0),
        }
    }

    /// "4:12 left", as the card header and the composer strip say it.
    pub fn label(self) -> String {
        s::agent_chat::question_time_left(self.clock())
    }

    /// The last half minute, which is drawn heavier as well as brighter so
    /// the warning never rests on colour alone.
    pub fn is_final_stretch(self) -> bool {
        self.secs <= FINAL_STRETCH_SECS
    }

    /// `m:ss`, or `h:mm:ss` from an hour up.
    pub fn clock(self) -> String {
        let (hours, minutes, seconds) = (self.secs / 3600, self.secs / 60 % 60, self.secs % 60);
        if hours > 0 {
            format!("{hours}:{minutes:02}:{seconds:02}")
        } else {
            format!("{minutes}:{seconds:02}")
        }
    }
}

impl OpenQuestion {
    /// The deadline this question counts down to, `None` when the timeout
    /// is off or the person has touched the question.
    pub(crate) fn countdown(&self) -> Option<Instant> {
        self.deadline.filter(|_| !self.draft.touched())
    }

    /// The person typed this question's answer in the bound composer: a
    /// touch like a pick, so the countdown stops for good.
    pub(in crate::view) fn stop_countdown(&mut self) {
        self.draft.touch();
        self.deadline = None;
    }
}

impl Questions {
    /// Follow a new timeout. An untouched question keeps the instant it
    /// counts from, so a changed timeout retimes it; one that had none
    /// counts from `now`, so turning the timeout on never declines a
    /// question on the spot. A touched question stays stopped.
    pub(in crate::view) fn set_timeout(&mut self, timeout: Option<Duration>, now: Instant) {
        let previous = self.timeout;
        self.timeout = timeout;
        for question in &mut self.open {
            question.deadline = match (question.countdown(), previous, timeout) {
                (_, _, None) => None,
                _ if question.draft.touched() => None,
                (Some(deadline), Some(previous), Some(timeout)) => {
                    Some(deadline - previous + timeout)
                }
                (_, _, Some(timeout)) => Some(now + timeout),
            };
        }
    }
}

impl AgentChatView {
    /// Decline every untouched question whose time is up at `now`, each
    /// through the one settle path, so an answer that settled it first
    /// wins and a later tick finds nothing left to decline.
    pub(in crate::view) fn expire_questions(&mut self, now: Instant, cx: &mut Context<Self>) {
        let expired: Vec<(u64, u32)> = self
            .questions
            .open()
            .filter_map(|question| {
                let deadline = question.countdown().filter(|deadline| *deadline <= now)?;
                let waited = deadline.saturating_duration_since(question.asked_at);
                Some((
                    question.id,
                    u32::try_from(waited.as_secs()).unwrap_or(u32::MAX),
                ))
            })
            .collect();
        for (id, after_secs) in expired {
            self.resolve_question(
                id,
                QuestionAnswer::Decline,
                QuestionOutcome::Expired { after_secs },
                cx,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn left(secs: u64, nanos: u32) -> QuestionCountdown {
        let now = Instant::now();
        QuestionCountdown::until(now + Duration::new(secs, nanos), now)
    }

    #[test]
    fn the_clock_rounds_up_and_grows_an_hour_field() {
        assert_eq!(left(252, 0).clock(), "4:12");
        assert_eq!(
            left(0, 1).clock(),
            "0:01",
            "a waiting question never reads 0:00"
        );
        assert_eq!(left(59, 500).clock(), "1:00");
        assert_eq!(left(3725, 0).clock(), "1:02:05");
        let now = Instant::now();
        assert_eq!(
            QuestionCountdown::until(now, now + Duration::from_secs(5)).clock(),
            "0:00"
        );
    }

    #[test]
    fn the_final_stretch_is_the_last_half_minute() {
        assert!(!left(31, 0).is_final_stretch());
        assert!(left(30, 0).is_final_stretch());
        assert!(left(0, 1).is_final_stretch());
    }
}
