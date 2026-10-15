//! The control-command half of the bridge's GPUI wiring.
//!
//! Separate from the transport loops next door because the two answer different
//! questions: `mod.rs` owns *when* daruda talks to Telegram (long-poll cadence,
//! send queue, token handling), and this file owns *what* a control command
//! does once one has been routed — resolve it, run it, fold the outcome back
//! into the adapter's ordinal table, and answer.
//!
//! Everything here is `pub(super)`: the poll loop is the only caller.

use crate::telegram::client;
use crate::telegram::command;
use crate::telegram::trace;
use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;

/// How long one "unauthorized inbound" line suppresses the next.
///
/// A bot's username is publicly discoverable, so anyone can send it messages —
/// and `ErrorReport::dedup` does *not* help here: `LogWriter` writes every
/// report it is given, and `dedup_key` only merges toasts (see
/// `workspace::error::toast`). Without a real window, a probing sender writes
/// one NDJSON line per message and drowns genuine diagnostics.
const UNAUTHORIZED_LOG_WINDOW: std::time::Duration = std::time::Duration::from_secs(300);

/// Note an inbound from an unpaired chat, at most once per
/// [`UNAUTHORIZED_LOG_WINDOW`]. Worth recording at all — it is the only signal
/// that someone found the bot — but not worth one line per message.
pub(super) fn log_unauthorized_inbound() {
    use std::sync::Mutex;
    static LAST: Mutex<Option<std::time::Instant>> = Mutex::new(None);

    // Ahead of the rate limit on purpose: the limit exists to keep the shared
    // NDJSON log readable, and an opt-in trace wants every occurrence.
    trace::delivery("inbound.unauthorized", || "dropped=1".to_string());

    let now = std::time::Instant::now();
    {
        let Ok(mut last) = LAST.lock() else {
            return;
        };
        if last.is_some_and(|t| now.duration_since(t) < UNAUTHORIZED_LOG_WINDOW) {
            return;
        }
        *last = Some(now);
    }
    LogWriter::log(
        ErrorReport::new("Telegram message from an unauthorized chat ignored")
            .severity(ErrorSeverity::Info)
            .at(file!(), line!())
            .dedup("telegram.unauthorized")
            .build(),
    );
}

/// Send a command reply. Deliberately does NOT call `record_sent`: that moves
/// the conversation, so a `/list` answer would silently become the next plain
/// message's destination.
pub(super) async fn send_command_reply(
    cx: &mut gpui::AsyncApp,
    token: &str,
    chat_id: i64,
    reply: command::RenderedReply,
) {
    let text = trace::is_on().then(|| trace::preview(&reply.text));
    let buttons: usize = reply
        .keyboard
        .as_ref()
        .map_or(0, |k| k.rows.iter().map(Vec::len).sum());
    let send_token = token.to_string();
    let sent = cx
        .background_executor()
        .spawn(async move {
            client::send_message(&send_token, chat_id, &reply.text, None, reply.keyboard)
        })
        .await;
    trace::delivery("send.reply", || {
        format!(
            "chat_id={chat_id} ok={} buttons={buttons} text={}",
            sent.is_ok(),
            trace::opt(text)
        )
    });
    if let Err(e) = sent {
        LogWriter::log(
            ErrorReport::new("Telegram command reply failed")
                .severity(ErrorSeverity::Info)
                .from_error(&e)
                .at(file!(), line!())
                .dedup("telegram.command_reply")
                .build(),
        );
    }
}

/// Acknowledge a tapped button with a toast and leave the message alone.
/// Unlike [`answer_and_edit`], a listing button is not consumed by the tap —
/// rewriting the message would strip the rows the user still wants.
pub(super) async fn answer_only(
    cx: &mut gpui::AsyncApp,
    token: &str,
    callback_id: String,
    label: &str,
) {
    let ack_token = token.to_string();
    let toast = label.to_string();
    let answered = cx
        .background_executor()
        .spawn(async move { client::answer_callback(&ack_token, &callback_id, Some(&toast)) })
        .await;
    trace::delivery("answer_callback", || {
        format!(
            "ok={} edits=false label={}",
            answered.is_ok(),
            trace::preview(label)
        )
    });
    if let Err(e) = answered {
        LogWriter::log(
            ErrorReport::new("Telegram answerCallbackQuery failed")
                .severity(ErrorSeverity::Info)
                .from_error(&e)
                .at(file!(), line!())
                .dedup("telegram.answer_callback")
                .build(),
        );
    }
}
