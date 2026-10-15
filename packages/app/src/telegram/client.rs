//! Raw Telegram Bot API HTTP client — `getUpdates` / `sendMessage` /
//! `answerCallbackQuery` / `editMessageText`, plus the wire-format parsing
//! the routing layer needs.
//!
//! GPUI-free; the token is an argument, and all that persists between calls
//! is the one [`agent`] whose pool keeps the TLS connection open.

use std::io::Read;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use serde::Deserialize;
use ureq::Body;
use ureq::http::Response;

/// Failure surface for the Bot API calls, mirroring
/// `daruda_agent::http::FetchError`'s transport-vs-parse split.
#[derive(Debug)]
pub enum ClientError {
    /// Transport, TLS, non-2xx status, or a Telegram `"ok": false` reply.
    /// One variant because the caller's recourse is the same for all of them.
    Http(String),
    /// JSON that would not decode, or decoded to a shape this module does
    /// not expect.
    Parse(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Http(e) => write!(f, "telegram HTTP error: {e}"),
            ClientError::Parse(e) => write!(f, "telegram parse error: {e}"),
        }
    }
}

impl std::error::Error for ClientError {}

/// One inbound `getUpdates` item, reduced to the fields the routing
/// layer needs.
#[derive(Debug, Clone, PartialEq)]
pub struct Update {
    pub update_id: i64,
    pub kind: UpdateKind,
}

/// The two update shapes this bridge acts on, plus everything else.
///
/// INVARIANT: every update Telegram sends reaches the routing layer, which
/// advances the `getUpdates` offset from the ids it sees. A kind this bridge
/// cannot act on — a sticker, an `edited_message`, a callback whose message
/// was deleted — becomes [`Self::Unsupported`] rather than being dropped.
/// Drop one and the offset stays behind it, so Telegram re-delivers that
/// batch on every poll, forever.
#[derive(Debug, Clone, PartialEq)]
pub enum UpdateKind {
    Message {
        chat_id: i64,
        text: String,
        reply_to_message_id: Option<i64>,
    },
    Callback {
        chat_id: i64,
        callback_id: String,
        data: String,
        /// The tapped message's id, for `edit_message_text` (strip the
        /// buttons + append the outcome so the phone shows the decision took).
        message_id: i64,
        /// The tapped message's current text, so the edit can preserve the
        /// original prompt and append the outcome rather than replacing it.
        message_text: String,
    },
    /// Carries nothing but its `update_id`, which is the point: the offset has
    /// to move past it.
    Unsupported,
}

pub use crate::remote_channel::bridge::InlineKeyboard;

/// The `reply_markup` payload for a keyboard. Split out so its shape is
/// testable without an HTTP call.
pub(crate) fn inline_keyboard_json(keyboard: &InlineKeyboard) -> serde_json::Value {
    let rows: Vec<serde_json::Value> = keyboard
        .rows
        .iter()
        .map(|row| {
            row.iter()
                .map(|(label, data)| serde_json::json!({ "text": label, "callback_data": data }))
                .collect::<Vec<_>>()
                .into()
        })
        .collect();
    serde_json::json!({ "inline_keyboard": rows })
}

/// Body read cap, mirroring `daruda_agent::http::MAX_BODY_BYTES`.
/// `getUpdates` batches can be larger than a typical status response,
/// but 1 MiB is still generous for a handful of text messages.
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// Added to the caller's `timeout_s` for [`get_updates`]'s HTTP timeout.
/// Telegram holds the connection for the full long poll before answering
/// empty, so a transport timeout at exactly `timeout_s` would abort first.
const LONG_POLL_MARGIN: Duration = Duration::from_secs(5);

/// Timeout for the two non-long-poll calls (`sendMessage`,
/// `answerCallbackQuery`), which are expected to complete in well
/// under a second on a normal connection.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Idle connections the pool keeps to `api.telegram.org`: one for the long
/// poll that is almost always in flight, one for whatever is being sent
/// meanwhile. A third would only ever be dropped.
const IDLE_CONNECTIONS_PER_HOST: usize = 2;

/// The one HTTP agent every call goes through: its pool saves the TCP + TLS
/// handshake, ~0.55 s of a ~0.8 s `sendMessage`. Timeouts are per request.
fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .max_idle_connections_per_host(IDLE_CONNECTIONS_PER_HOST)
            .proxy(None)
            .build()
            .into()
    })
}

/// POST `payload` to `method`, retrying once on a fresh connection when the
/// pooled one turns out dead — after sleep or a network change the pool hands
/// back a socket the server already dropped, and ureq retries no POST itself.
fn post_json(
    token: &str,
    method: &str,
    payload: serde_json::Value,
) -> Result<Response<Body>, ClientError> {
    let url = base_url(token, method);
    let sent = match agent()
        .post(&url)
        .config()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .build()
        .send_json(payload.clone())
    {
        Err(error) if is_dead_connection(&error) => {
            let fresh: ureq::Agent = ureq::Agent::config_builder()
                .max_idle_connections(0)
                .timeout_global(Some(REQUEST_TIMEOUT))
                .proxy(None)
                .build()
                .into();
            fresh.post(&url).send_json(payload)
        }
        other => other,
    };
    sent.map_err(|e| http_error(token, e))
}

/// Whether a transport error says the socket was gone rather than slow. A
/// timeout is excluded: the request may have landed, and a retry would repeat it.
fn is_dead_connection(error: &ureq::Error) -> bool {
    matches!(error, ureq::Error::Io(io) if is_dead_connection_kind(io.kind()))
}

fn is_dead_connection_kind(kind: std::io::ErrorKind) -> bool {
    matches!(
        kind,
        std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::BrokenPipe
    )
}

/// Truncate a callback toast to Telegram's limit, counting `char`s so a
/// multi-byte label never splits mid-character.
fn clamp_callback_text(text: &str) -> String {
    text.chars().take(ANSWER_CALLBACK_MAX_CHARS).collect()
}

fn base_url(token: &str, method: &str) -> String {
    format!("https://api.telegram.org/bot{token}/{method}")
}

/// Placeholder standing in for the bot token in anything that leaves this
/// module as text.
const REDACTED_TOKEN: &str = "<redacted>";

/// Strip the bot token out of a transport error before it becomes a
/// [`ClientError`].
///
/// Telegram carries the token in the URL path and every HTTP client puts the
/// URL it failed on into its `Display`, so the raw string is a live bot
/// credential — and `crate::telegram::global` writes these to the on-disk log.
fn http_error(token: &str, e: impl std::fmt::Display) -> ClientError {
    ClientError::Http(e.to_string().replace(token, REDACTED_TOKEN))
}

/// Long-poll for updates after `offset`. `timeout_s` goes straight to
/// Telegram as its long-poll duration; the transport waits
/// [`LONG_POLL_MARGIN`] longer so it cannot give up first.
pub fn get_updates(token: &str, offset: i64, timeout_s: u64) -> Result<Vec<Update>, ClientError> {
    // Tests must never hit the network: a real long-poll blocks teardown for
    // the poll timeout and 409-conflicts a running app polling the same
    // token. Return no updates hermetically — the poll loop then just idles.
    if cfg!(test) {
        return Ok(Vec::new());
    }
    let url = format!(
        "{}?offset={offset}&timeout={timeout_s}",
        base_url(token, "getUpdates")
    );
    let response = agent()
        .get(&url)
        .config()
        .timeout_global(Some(Duration::from_secs(timeout_s) + LONG_POLL_MARGIN))
        .build()
        .call()
        .map_err(|e| http_error(token, e))?;

    let body = read_body(response)?;
    parse_updates(&body)
}

/// Send a text message, optionally with a `parse_mode` (see
/// `crate::telegram::markdown`) and an inline keyboard.
///
/// Text that is malformed for its `parse_mode` — an unclosed tag — makes
/// Telegram reject the call outright. Retrying without the `parse_mode` is
/// `global`'s send loop's job, not this function's.
pub fn send_message(
    token: &str,
    chat_id: i64,
    text: &str,
    parse_mode: Option<&str>,
    keyboard: Option<InlineKeyboard>,
) -> Result<i64, ClientError> {
    // Tests must never hit the network (see `get_updates`): answer as a
    // delivered message with a fresh id, which is all a caller keeps.
    if cfg!(test) {
        static NEXT_OFFLINE_MESSAGE_ID: AtomicI64 = AtomicI64::new(1);
        return Ok(NEXT_OFFLINE_MESSAGE_ID.fetch_add(1, Ordering::Relaxed));
    }
    let mut payload = serde_json::json!({
        "chat_id": chat_id,
        "text": text,
    });
    if let Some(parse_mode) = parse_mode {
        payload["parse_mode"] = serde_json::json!(parse_mode);
    }
    if let Some(keyboard) = keyboard {
        payload["reply_markup"] = inline_keyboard_json(&keyboard);
    }

    let response = post_json(token, "sendMessage", payload)?;

    let body = read_body(response)?;
    parse_send_message_response(&body)
}

/// Upper bound Telegram documents for `answerCallbackQuery`'s `text`. A longer
/// toast is rejected outright, and a rejected ack leaves the button spinning —
/// so the cap is enforced here, at the one call site every caller goes through,
/// rather than trusted to each of them.
const ANSWER_CALLBACK_MAX_CHARS: usize = 200;

/// Acknowledge a callback-query button tap, optionally with a toast.
///
/// Required after every tap: without it the phone spins on the button until
/// Telegram times the query out. `text` is the "your tap registered" feedback
/// and is clamped to [`ANSWER_CALLBACK_MAX_CHARS`] on a char boundary.
pub fn answer_callback(
    token: &str,
    callback_id: &str,
    text: Option<&str>,
) -> Result<(), ClientError> {
    if cfg!(test) {
        return Ok(());
    }
    let mut payload = serde_json::json!({ "callback_query_id": callback_id });
    if let Some(text) = text {
        payload["text"] = serde_json::json!(clamp_callback_text(text));
    }

    let response = post_json(token, "answerCallbackQuery", payload)?;

    let body = read_body(response)?;
    parse_answer_callback_response(&body)
}

/// Rewrite a sent message and drop its buttons — omitting `reply_markup` is
/// what removes them. Answers a tapped permission prompt with its outcome.
///
/// Deliberately no `parse_mode`: `text` is built from the message's own
/// display text, and re-parsing that as HTML would misrender it.
pub fn edit_message_text(
    token: &str,
    chat_id: i64,
    message_id: i64,
    text: &str,
) -> Result<(), ClientError> {
    if cfg!(test) {
        return Ok(());
    }
    let payload = serde_json::json!({
        "chat_id": chat_id,
        "message_id": message_id,
        "text": text,
    });

    let response = post_json(token, "editMessageText", payload)?;

    let body = read_body(response)?;
    parse_edit_message_response(&body)
}

fn read_body(response: Response<Body>) -> Result<String, ClientError> {
    let mut body = String::new();
    response
        .into_body()
        .into_reader()
        .take(MAX_BODY_BYTES as u64)
        // No redaction hop here: a body-read failure is a local IO error and
        // never carries the request URL, which is the only place the token
        // appears.
        .read_to_string(&mut body)
        .map_err(|e| ClientError::Http(e.to_string()))?;
    Ok(body)
}

/// Top-level Telegram API envelope shared by all three endpoints:
/// `{"ok": bool, "result": ..., "description": "..."}`. `description`
/// is only present when `ok` is `false`.
#[derive(Debug, Deserialize)]
struct RawEnvelope {
    ok: bool,
    description: Option<String>,
}

/// Shared `"ok": false` → [`ClientError::Http`] check. `fallback` covers the
/// rare reply that sets `ok: false` and omits `description`.
fn require_ok(ok: bool, description: Option<String>, fallback: &str) -> Result<(), ClientError> {
    if ok {
        Ok(())
    } else {
        Err(ClientError::Http(
            description.unwrap_or_else(|| fallback.to_string()),
        ))
    }
}

#[derive(Debug, Deserialize)]
struct GetUpdatesEnvelope {
    ok: bool,
    #[serde(default)]
    result: Vec<RawUpdate>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawUpdate {
    update_id: i64,
    #[serde(default)]
    message: Option<RawMessage>,
    #[serde(default)]
    callback_query: Option<RawCallbackQuery>,
}

#[derive(Debug, Deserialize)]
struct RawMessage {
    #[serde(default)]
    text: Option<String>,
    chat: RawChat,
    #[serde(default)]
    reply_to_message: Option<RawReplyToMessage>,
}

#[derive(Debug, Deserialize)]
struct RawReplyToMessage {
    message_id: i64,
}

#[derive(Debug, Deserialize)]
struct RawChat {
    id: i64,
}

/// `message` is `Option` because Telegram documents it so: absent for an
/// inline-query callback, or once the original message is deleted. Required
/// here, one such callback would fail the whole batch's decode — see
/// [`UpdateKind`].
#[derive(Debug, Deserialize)]
struct RawCallbackQuery {
    id: String,
    #[serde(default)]
    data: Option<String>,
    #[serde(default)]
    message: Option<RawCallbackMessage>,
}

#[derive(Debug, Deserialize)]
struct RawCallbackMessage {
    chat: RawChat,
    message_id: i64,
    #[serde(default)]
    text: Option<String>,
}

/// Decode a `getUpdates` response body. Separate from [`get_updates`] so
/// tests reach it with string fixtures and no network mock.
///
/// Every shape Telegram can send has to get through here, the whole batch
/// with it — see [`UpdateKind`] for what one dropped update costs.
fn parse_updates(body: &str) -> Result<Vec<Update>, ClientError> {
    let envelope: GetUpdatesEnvelope =
        serde_json::from_str(body).map_err(|e| ClientError::Parse(e.to_string()))?;
    require_ok(envelope.ok, envelope.description, "getUpdates failed")?;

    // `map`, never `filter_map`: every update Telegram sent must reach the
    // routing layer so its id can advance the offset. See [`UpdateKind`].
    let updates = envelope
        .result
        .into_iter()
        .map(|raw| Update {
            update_id: raw.update_id,
            kind: update_kind(raw),
        })
        .collect();

    Ok(updates)
}

/// Classify one raw update. Anything this bridge cannot act on becomes
/// [`UpdateKind::Unsupported`] rather than disappearing.
fn update_kind(raw: RawUpdate) -> UpdateKind {
    if let Some(message) = raw.message {
        // A message with no `text` is a photo, sticker, voice note, or a
        // service message — nothing the routing layer can read.
        return match message.text {
            Some(text) => UpdateKind::Message {
                chat_id: message.chat.id,
                text,
                reply_to_message_id: message.reply_to_message.map(|m| m.message_id),
            },
            None => UpdateKind::Unsupported,
        };
    }
    // A callback whose message Telegram no longer has (deleted, or too old)
    // cannot be answered in place, so there is nothing to route.
    let Some(cq) = raw.callback_query else {
        return UpdateKind::Unsupported;
    };
    let Some(message) = cq.message else {
        return UpdateKind::Unsupported;
    };
    UpdateKind::Callback {
        chat_id: message.chat.id,
        callback_id: cq.id,
        data: cq.data.unwrap_or_default(),
        message_id: message.message_id,
        message_text: message.text.unwrap_or_default(),
    }
}

#[derive(Debug, Deserialize)]
struct SendMessageEnvelope {
    ok: bool,
    #[serde(default)]
    result: Option<SendMessageResult>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SendMessageResult {
    message_id: i64,
}

fn parse_send_message_response(body: &str) -> Result<i64, ClientError> {
    let envelope: SendMessageEnvelope =
        serde_json::from_str(body).map_err(|e| ClientError::Parse(e.to_string()))?;
    require_ok(envelope.ok, envelope.description, "sendMessage failed")?;
    envelope
        .result
        .map(|r| r.message_id)
        .ok_or_else(|| ClientError::Parse("sendMessage response missing result".to_string()))
}

/// Decode an `answerCallbackQuery` response body.
fn parse_answer_callback_response(body: &str) -> Result<(), ClientError> {
    let envelope: RawEnvelope =
        serde_json::from_str(body).map_err(|e| ClientError::Parse(e.to_string()))?;
    require_ok(
        envelope.ok,
        envelope.description,
        "answerCallbackQuery failed",
    )
}

/// Decode an `editMessageText` response body.
fn parse_edit_message_response(body: &str) -> Result<(), ClientError> {
    let envelope: RawEnvelope =
        serde_json::from_str(body).map_err(|e| ClientError::Parse(e.to_string()))?;
    require_ok(envelope.ok, envelope.description, "editMessageText failed")
}

#[cfg(test)]
mod tests;
