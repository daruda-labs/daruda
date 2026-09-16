//! Remote agent control through Telegram, Slack, and Discord.

pub mod bridge;
pub(crate) mod command;
pub(crate) mod dispatch;
pub mod global;
pub mod keychain;
mod pairing;
pub(crate) mod runtime;
pub(crate) mod settings;
pub(crate) mod transport;

pub(crate) fn send_approval_card(
    id: crate::control::approval::ApprovalId,
    summary: String,
    cx: &mut gpui::App,
) -> bool {
    let telegram =
        crate::telegram::global::TelegramBridge::send_approval_card(id, summary.clone(), cx);
    global::RemoteChannels::send_approval(id, summary, cx) || telegram
}

pub(crate) fn forget_approval(id: crate::control::approval::ApprovalId, cx: &mut gpui::App) {
    crate::telegram::global::TelegramBridge::forget_approval(id, cx);
    global::RemoteChannels::forget_approval(id, cx);
}

/// Introduce a chat the phone approved opening, on every channel that can
/// hear it, and make it their target. Best effort by design: the request has
/// already succeeded, so a channel that went away between the tap and now
/// costs a trace line, not an outcome.
pub(crate) fn announce_chat_created(
    pane: bridge::PaneRef,
    header: String,
    agent: &str,
    cx: &mut gpui::App,
) {
    let tail = crate::surface::strings::control_chat_announced(agent);
    let telegram = crate::telegram::global::TelegramBridge::announce_chat(
        pane,
        header.clone(),
        tail.clone(),
        cx,
    );
    let remote = global::RemoteChannels::announce_chat(pane, header, tail, cx);
    if !telegram && !remote {
        crate::telegram::trace::delivery("announce.dropped", || {
            format!("pane={}", crate::telegram::trace::pane(pane))
        });
    }
}

/// Report one remote-channel failure. `#[track_caller]` puts the call site in
/// the log instead of this helper, and `dedup` must be unique per site: a
/// shared key merges a different failure into a live toast's repeat count.
#[track_caller]
pub(crate) fn log_error(message: &str, error: &dyn std::fmt::Display, dedup: &str) {
    use daruda_store::observability::{
        error_report::{ErrorReport, ErrorSeverity},
        log_writer::LogWriter,
    };
    let at = std::panic::Location::caller();
    LogWriter::log(
        ErrorReport::new(message.to_owned())
            .severity(ErrorSeverity::Warning)
            .message(error.to_string())
            .at(at.file(), at.line())
            .dedup(dedup)
            .build(),
    );
}
