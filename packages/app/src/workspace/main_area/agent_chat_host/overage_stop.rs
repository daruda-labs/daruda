//! What the workspace says when `[usage] stop_on_overage` stops a chat turn:
//! the pane's banner is only seen on screen, so the stop is also toasted,
//! marks its lane unread, and answers a phone that started the turn.

use daruda_chat::view::OverageNotice;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use gpui::Context;

use crate::surface::strings as s;
use crate::telegram::bridge::TelegramTail;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane_tree::PaneId;

/// The banner's sentence for a reader away from the pane: it names the chat,
/// and the reset carries its date since a toast or phone reads it later.
fn overage_stopped_text(notice: &OverageNotice) -> String {
    let window = notice.window_label();
    match notice
        .resets_at
        .and_then(|secs| chrono::DateTime::from_timestamp(secs, 0))
    {
        Some(at) => s::agent_chat::overage_stopped_resets(window, s::flow::overage_reset_at(at)),
        None => s::agent_chat::overage_stopped(window),
    }
}

impl Workspace {
    /// Signal an overage stop on `pane_id` beyond its banner. `phone_owed` is
    /// read before the stop, which retires the pane's phone ledger. The
    /// `Stopped` completion still fires once, through the settle edge.
    pub(super) fn signal_overage_stop(
        &mut self,
        pane_id: PaneId,
        phone_owed: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = self
            .agent_chat_view(pane_id)
            .and_then(|view| view.read(cx).overage_notice().map(overage_stopped_text))
        else {
            return;
        };
        // A limit is account-wide, so panes trip together: one key merges
        // them into one toast with a repeat count.
        self.report_error(
            ErrorReport::new(text.clone())
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .dedup("agent_chat.overage_stopped")
                .build(),
            cx,
        );
        self.mark_lane_unread_for_pane(pane_id, cx);
        if phone_owed {
            let header = self.telegram_header(pane_id, cx);
            self.relay_to_telegram(pane_id, header, TelegramTail::Plain(text), None, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use daruda_acp::RateLimitWindow;

    #[test]
    fn the_text_names_the_window_and_the_reset_when_known() {
        let mut notice = OverageNotice {
            window: Some(RateLimitWindow::FiveHour),
            resets_at: None,
        };
        assert_eq!(
            overage_stopped_text(&notice),
            s::agent_chat::overage_stopped(notice.window_label())
        );
        notice.resets_at = Some(1_791_340_200);
        let at = chrono::DateTime::from_timestamp(1_791_340_200, 0).expect("valid");
        assert_eq!(
            overage_stopped_text(&notice),
            s::agent_chat::overage_stopped_resets(
                notice.window_label(),
                s::flow::overage_reset_at(at)
            )
        );
    }
}
