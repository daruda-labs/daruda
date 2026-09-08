//! The control-command half of the bridge's GPUI wiring.
//!
//! Split from the transport loops next door because the two answer different
//! questions: `mod.rs` owns *when* daruda talks to Telegram (long-poll cadence,
//! send queue, token handling), and this file owns *what* a control command
//! does once one has been routed — resolve it, run it, fold the outcome back
//! into the adapter's ordinal table, and answer.
//!
//! Everything here is `pub(super)`: the poll loop is the only caller.

use gpui::App;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use daruda_store::persistence;

use super::TelegramBridge;
use crate::control::result::{ControlOutcome, ControlResult};
use crate::control::spec::ControlCommand;
use crate::telegram::bridge::PaneRef;
use crate::telegram::client;
use crate::telegram::command;

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

/// Write the routing core's high-water mark to its state file.
///
/// Deliberately not a `SettingsPatch`: mutating the `SettingsStore` global
/// notifies app-wide observers that rebuild the native menu bar and call
/// `cx.refresh_windows()`, which the root `CLAUDE.md` (pitfall 10) reserves for
/// genuinely global invalidation — driving it at message rate is exactly the
/// ban. See `daruda_store::telegram` for why `config.toml` is also the wrong
/// file to keep this in.
///
/// Guarded on a real change so an idle long-poll, which returns nothing every
/// [`super::POLL_TIMEOUT_SECS`], does not rewrite the file.
pub(super) fn persist_offset(cx: &mut gpui::AsyncApp) {
    let offset = cx.update(|cx| {
        let bridge = cx.global_mut::<TelegramBridge>();
        let offset = bridge.core.current_offset();
        (bridge.persisted_offset != offset).then(|| {
            bridge.persisted_offset = offset;
            offset
        })
    });
    let Some(offset) = offset else {
        return;
    };
    let state = daruda_store::telegram::TelegramState {
        update_offset: offset,
    };
    if let Err(e) =
        daruda_store::telegram::save_telegram_state_in(&persistence::default_data_dir(), &state)
    {
        LogWriter::log(
            ErrorReport::new("Telegram update offset failed to persist")
                .severity(ErrorSeverity::Info)
                .from_error(&e)
                .at(file!(), line!())
                .dedup("telegram.offset.persist")
                .build(),
        );
    }
}

/// Resolve, run, and render one command.
///
/// Three steps, each holding the bridge global for as long as it needs and no
/// longer: resolution needs the ordinal table, execution needs the whole `App`
/// to walk every window, and rendering needs the table again — now updated by
/// whatever the execution produced.
pub(super) fn run_command(
    command: ControlCommand,
    cx: &mut gpui::AsyncApp,
) -> command::RenderedReply {
    let step = cx.update(|cx| {
        let state = cx.global_mut::<TelegramBridge>().core.command_state_mut();
        command::resolve_command(command, state)
    });
    let (outcome, addressed) = match step {
        command::Resolution::Answer(outcome) => (outcome, None),
        command::Resolution::Run(resolved, addressed) => (
            cx.update(|cx| crate::control::exec::run(resolved, cx)),
            addressed,
        ),
    };
    cx.update(|cx| {
        let state = cx.global_mut::<TelegramBridge>().core.command_state_mut();
        let absorbed = command::absorb(&outcome, addressed, state);
        command::render(&outcome, absorbed, state)
    })
}

/// Render an outcome the executor never saw, against the live ordinal table.
pub(super) fn render_outcome(outcome: &ControlOutcome, cx: &mut App) -> command::RenderedReply {
    let state = cx.global_mut::<TelegramBridge>().core.command_state_mut();
    command::render(outcome, command::Absorbed::Nothing, state)
}

/// Point the target at `pane` and produce the toast naming it. A tap is the
/// same act as `/use <n>`, so it goes through the same render funnel.
pub(super) fn select_target(pane: PaneRef, cx: &mut App) -> String {
    let state = cx.global_mut::<TelegramBridge>().core.command_state_mut();
    state.select(Some(pane));
    let summary = state.summary_for(pane);
    command::render(
        &Ok(ControlResult::Selected { target: summary }),
        command::Absorbed::Nothing,
        state,
    )
    .text
}

/// Send a command reply. Deliberately does NOT call `record_sent`: that sets
/// `last_pinged`, so a `/list` answer would silently become the next plain
/// message's destination.
pub(super) async fn send_command_reply(
    cx: &mut gpui::AsyncApp,
    token: &str,
    reply: command::RenderedReply,
) {
    let Some(chat_id) = cx.update(|cx| cx.global::<TelegramBridge>().core.authorized_chat_id())
    else {
        return;
    };
    let send_token = token.to_string();
    let sent = cx
        .background_executor()
        .spawn(async move {
            client::send_message(&send_token, chat_id, &reply.text, None, reply.keyboard)
        })
        .await;
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
