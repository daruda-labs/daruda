//! Remote agent control through Telegram, Slack, and Discord.

pub(crate) mod backoff;
pub mod bridge;
pub(crate) mod cards;
pub(crate) mod command;
pub(crate) mod delivery;
pub(crate) mod dispatch;
pub mod global;
pub mod keychain;
pub(crate) mod lock;
mod pairing;
pub(crate) mod runtime;
pub(crate) mod settings;
pub(crate) mod transport;

pub(crate) fn send_approval_card(
    id: crate::control::approval::ApprovalId,
    summary: String,
    delivery: global::Delivery,
    cx: &mut gpui::App,
) -> bool {
    let telegram = crate::telegram::global::TelegramBridge::send_approval_card(
        id,
        summary.clone(),
        delivery,
        cx,
    );
    global::RemoteChannels::send_approval(id, summary, delivery, cx) || telegram
}

pub(crate) fn settle_card(card: cards::CardId, label: &str, cx: &mut gpui::App) {
    crate::telegram::global::TelegramBridge::settle_card(&card, label, cx);
    global::RemoteChannels::settle_card(&card, label, cx);
}

pub(crate) fn settle_permissions(pane: bridge::PaneRef, session: u64, cx: &mut gpui::App) {
    let label = crate::surface::strings::notification::telegram_permission_cancelled();
    crate::telegram::global::TelegramBridge::settle_permissions(pane, session, &label, cx);
    global::RemoteChannels::settle_permissions(pane, session, &label, cx);
}

/// Introduce a chat on eligible channels. Explicit, solicited delivery selects
/// the chat and records its ping; presence-based delivery is only a Notice,
/// preserving selection and reply routing. Best effort by design: the request has
/// already succeeded, so a channel that went away between the tap and now
/// costs a log line, not an outcome.
pub(crate) fn announce_chat_created(
    pane: bridge::PaneRef,
    header: String,
    agent: &str,
    delivery: global::Delivery,
    cx: &mut gpui::App,
) {
    use daruda_store::observability::{
        error_report::{ErrorReport, ErrorSeverity},
        log_writer::LogWriter,
    };
    let tail = match delivery {
        global::Delivery::Explicit => crate::surface::strings::control::chat_announced(agent),
        global::Delivery::Presence { .. } => {
            crate::surface::strings::control::chat_announced_only(agent)
        }
    };
    let telegram = crate::telegram::global::TelegramBridge::announce_chat(
        pane,
        header.clone(),
        tail.clone(),
        delivery,
        cx,
    );
    let remote = global::RemoteChannels::announce_chat(pane, header, tail, delivery, cx);
    if !telegram && !remote && matches!(delivery, global::Delivery::Explicit) {
        LogWriter::log(
            ErrorReport::new("Chat announcement not sent: no remote channel can deliver it")
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .with_context("pane", crate::telegram::trace::pane(pane).to_string())
                .dedup("announce.undeliverable")
                .build(),
        );
    }
}

/// Tell every channel that can hear it something owed to a command the phone
/// sent earlier. Best effort, like [`announce_chat_created`].
pub(crate) fn send_notice_everywhere(text: String, cx: &gpui::App) {
    use daruda_store::observability::{
        error_report::{ErrorReport, ErrorSeverity},
        log_writer::LogWriter,
    };
    let remote = global::RemoteChannels::send_notice_with_delivery(
        text.clone(),
        global::Delivery::Explicit,
        cx,
    );
    let telegram = crate::telegram::global::TelegramBridge::notify(text, cx);
    if !telegram && !remote {
        LogWriter::log(
            ErrorReport::new("Notice not sent: no remote channel can deliver it")
                .severity(ErrorSeverity::Warning)
                .at(file!(), line!())
                .dedup("notice.undeliverable")
                .build(),
        );
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
