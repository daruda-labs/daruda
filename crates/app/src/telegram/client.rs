//! Raw Telegram Bot API HTTP client — `getUpdates` / `sendMessage` /
//! `answerCallbackQuery` plus the minimal wire-format parsing the
//! routing layer needs. GPUI-free and stateless: every function takes
//! the bot token as an argument and does not remember anything
//! between calls (no offset tracking, no session state). The routing
//! state machine (`bridge.rs`) owns the offset and decides what to
//! send; the GPUI poll loop (`global.rs`) owns the timer that calls
//! these functions repeatedly.
//!
//! `get_updates` / `send_message` / `answer_callback` are called from
//! `global.rs`'s poll loop, which owns the offset bookkeeping and
//! feeds parsed `Update`s into `bridge::BridgeCore`.

use std::io::Read;
use std::time::Duration;

use serde::Deserialize;

/// Failure surface for the three Bot API calls. Mirrors
/// `daruda_agent::http::FetchError`'s transport-vs-parse split:
/// `Http` for transport/status failures (including Telegram's own
/// `"ok": false` API-level errors), `Parse` for JSON decode or
/// unexpected-shape failures.
#[derive(Debug)]
pub enum ClientError {
    /// Network error, DNS failure, TLS handshake failure, non-2xx
    /// status, response body read error, or a Telegram `"ok": false`
    /// API-level error (bad token, chat not found, etc). The wrapped
    /// string is for logging.
    Http(String),
    /// JSON could not be decoded, or the decoded shape didn't match
    /// the expected schema (missing required fields, wrong types).
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

/// The two update shapes this bridge understands. Telegram sends many
/// other update types (`edited_message`, `channel_post`, ...); those
/// are skipped during parsing rather than represented here — there is
/// nothing the routing layer could do with them.
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
}

/// A single row of inline buttons attached to a `sendMessage` call.
/// Kept to one row of `(label, callback_data)` pairs — the only shape
/// the bridge ever sends (a permission prompt with one button per
/// option the agent offered, typically Allow/Reject but sometimes more).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineKeyboard {
    pub buttons: Vec<(String, String)>,
}

/// Body read cap, mirroring `daruda_agent::http::MAX_BODY_BYTES`.
/// `getUpdates` batches can be larger than a typical status response,
/// but 1 MiB is still generous for a handful of text messages.
const MAX_BODY_BYTES: usize = 1024 * 1024;

/// Extra wall-clock margin added on top of the caller's long-poll
/// `timeout_s` when building the HTTP client timeout for
/// `get_updates`. Telegram holds the connection open for up to
/// `timeout_s` seconds before responding with an empty result; the
/// HTTP timeout must exceed that or the request aborts before
/// Telegram would have replied.
const LONG_POLL_MARGIN: Duration = Duration::from_secs(5);

/// Timeout for the two non-long-poll calls (`sendMessage`,
/// `answerCallbackQuery`), which are expected to complete in well
/// under a second on a normal connection.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

fn base_url(token: &str, method: &str) -> String {
    format!("https://api.telegram.org/bot{token}/{method}")
}

/// Long-poll for new updates starting after `offset`. `timeout_s` is
/// passed straight through to Telegram as the long-poll duration; the
/// underlying HTTP client waits `timeout_s + 5s` to give Telegram
/// margin to respond before the transport gives up. Returns whatever
/// Telegram hands back — this function does not track or advance an
/// offset itself, that's the routing layer's job.
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
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(timeout_s) + LONG_POLL_MARGIN)
        .build();

    let response = agent
        .get(&url)
        .call()
        .map_err(|e| ClientError::Http(e.to_string()))?;

    let body = read_body(response)?;
    parse_updates(&body)
}

/// Send a text message, optionally with a `parse_mode` (e.g. `"HTML"` — see
/// `crate::telegram::markdown`) and/or an inline keyboard attached. Returns
/// the new message's `message_id`. A malformed `text` for the given
/// `parse_mode` (e.g. an unclosed tag) makes Telegram reject the whole call
/// with an `Http` error — the caller (`global.rs`'s send loop) is
/// responsible for retrying with `parse_mode: None` if that happens, this
/// function does not.
pub fn send_message(
    token: &str,
    chat_id: i64,
    text: &str,
    parse_mode: Option<&str>,
    keyboard: Option<InlineKeyboard>,
) -> Result<i64, ClientError> {
    let mut payload = serde_json::json!({
        "chat_id": chat_id,
        "text": text,
    });
    if let Some(parse_mode) = parse_mode {
        payload["parse_mode"] = serde_json::json!(parse_mode);
    }
    if let Some(keyboard) = keyboard {
        let row: Vec<serde_json::Value> = keyboard
            .buttons
            .into_iter()
            .map(|(label, data)| serde_json::json!({ "text": label, "callback_data": data }))
            .collect();
        payload["reply_markup"] = serde_json::json!({ "inline_keyboard": [row] });
    }

    let agent = ureq::AgentBuilder::new().timeout(REQUEST_TIMEOUT).build();
    let response = agent
        .post(&base_url(token, "sendMessage"))
        .send_json(payload)
        .map_err(|e| ClientError::Http(e.to_string()))?;

    let body = read_body(response)?;
    parse_send_message_response(&body)
}

/// Acknowledge a callback-query button tap. Telegram requires this call after
/// handling a tap or the client shows a loading spinner on the button
/// indefinitely. When `text` is `Some`, Telegram also shows it as a brief toast
/// notification to the user — this is the immediate "your tap registered"
/// feedback; without it the spinner just clears silently.
pub fn answer_callback(
    token: &str,
    callback_id: &str,
    text: Option<&str>,
) -> Result<(), ClientError> {
    let mut payload = serde_json::json!({ "callback_query_id": callback_id });
    if let Some(text) = text {
        payload["text"] = serde_json::json!(text);
    }

    let agent = ureq::AgentBuilder::new().timeout(REQUEST_TIMEOUT).build();
    let response = agent
        .post(&base_url(token, "answerCallbackQuery"))
        .send_json(payload)
        .map_err(|e| ClientError::Http(e.to_string()))?;

    let body = read_body(response)?;
    parse_answer_callback_response(&body)
}

/// Replace a previously-sent message's text and drop its inline keyboard
/// (omitting `reply_markup` removes the buttons). Used after a permission button
/// is tapped: the prompt message is rewritten to show the resolved outcome so
/// the phone reflects the decision and the now-consumed buttons disappear. Sent
/// as plain text (no `parse_mode`) — `text` is composed from the message's own
/// display text plus an outcome line, and re-parsing display text as HTML could
/// misrender.
pub fn edit_message_text(
    token: &str,
    chat_id: i64,
    message_id: i64,
    text: &str,
) -> Result<(), ClientError> {
    let payload = serde_json::json!({
        "chat_id": chat_id,
        "message_id": message_id,
        "text": text,
    });

    let agent = ureq::AgentBuilder::new().timeout(REQUEST_TIMEOUT).build();
    let response = agent
        .post(&base_url(token, "editMessageText"))
        .send_json(payload)
        .map_err(|e| ClientError::Http(e.to_string()))?;

    let body = read_body(response)?;
    parse_edit_message_response(&body)
}

fn read_body(response: ureq::Response) -> Result<String, ClientError> {
    let mut body = String::new();
    response
        .into_reader()
        .take(MAX_BODY_BYTES as u64)
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

/// Shared `"ok": false` → `ClientError::Http` check, used by all three
/// endpoints' parsers. `fallback` supplies the error text for the rare
/// case where Telegram sets `ok: false` but omits `description`.
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

/// `message` is `Option` because Telegram documents `CallbackQuery.message`
/// as optional — it is absent for callback queries originating from an
/// inline query, or when the original message has since been deleted. A
/// `getUpdates` batch containing such a callback alongside otherwise-normal
/// updates must still parse; `parse_updates` skips this callback rather
/// than failing the whole batch (see `parse_updates`).
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

/// Decode a `getUpdates` response body into the recognized subset of
/// updates. Split out from `get_updates` so tests can exercise it
/// directly against string fixtures without a network mock (mirrors
/// `daruda_agent::limits::parse_plan_limits`).
///
/// A callback query whose `message` is absent (deleted original
/// message, or an inline-query-originated callback) is skipped rather
/// than treated as a parse failure — it's a normal, documented
/// Telegram API shape. Failing the whole batch over one such callback
/// would permanently wedge the poll loop: `get_updates` would return
/// `Err`, `update_offset` would never advance past the batch, and the
/// identical batch would be re-fetched and fail again on every poll.
fn parse_updates(body: &str) -> Result<Vec<Update>, ClientError> {
    let envelope: GetUpdatesEnvelope =
        serde_json::from_str(body).map_err(|e| ClientError::Parse(e.to_string()))?;
    require_ok(envelope.ok, envelope.description, "getUpdates failed")?;

    let updates = envelope
        .result
        .into_iter()
        .filter_map(|raw| {
            let kind = if let Some(message) = raw.message {
                let text = message.text?;
                Some(UpdateKind::Message {
                    chat_id: message.chat.id,
                    text,
                    reply_to_message_id: message.reply_to_message.map(|m| m.message_id),
                })
            } else {
                let cq = raw.callback_query?;
                let message = cq.message?;
                Some(UpdateKind::Callback {
                    chat_id: message.chat.id,
                    callback_id: cq.id,
                    data: cq.data.unwrap_or_default(),
                    message_id: message.message_id,
                    message_text: message.text.unwrap_or_default(),
                })
            }?;
            Some(Update {
                update_id: raw.update_id,
                kind,
            })
        })
        .collect();

    Ok(updates)
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

/// Decode an `answerCallbackQuery` response body. Split out from
/// `answer_callback` for the same reason `parse_updates` and
/// `parse_send_message_response` are split out — so tests can
/// exercise the error path directly against string fixtures without a
/// network mock.
fn parse_answer_callback_response(body: &str) -> Result<(), ClientError> {
    let envelope: RawEnvelope =
        serde_json::from_str(body).map_err(|e| ClientError::Parse(e.to_string()))?;
    require_ok(
        envelope.ok,
        envelope.description,
        "answerCallbackQuery failed",
    )
}

/// Decode an `editMessageText` response body. Same `"ok"`-envelope shape and
/// split-for-testability reasoning as [`parse_answer_callback_response`].
fn parse_edit_message_response(body: &str) -> Result<(), ClientError> {
    let envelope: RawEnvelope =
        serde_json::from_str(body).map_err(|e| ClientError::Parse(e.to_string()))?;
    require_ok(envelope.ok, envelope.description, "editMessageText failed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_updates_is_stubbed_under_test_no_network() {
        // The long-poll must never touch the network in tests: a real
        // `getUpdates` would block teardown for the poll timeout (and
        // 409-conflict a running app polling the same token). The stub
        // returns no updates without any HTTP call.
        let updates = get_updates("dummy-token", 0, 1).expect("stub returns Ok");
        assert!(
            updates.is_empty(),
            "test-stubbed get_updates yields nothing"
        );
    }

    #[test]
    fn parses_message_with_reply_to() {
        let body = r#"{
            "ok": true,
            "result": [
                {
                    "update_id": 123456,
                    "message": {
                        "message_id": 42,
                        "chat": { "id": 999 },
                        "text": "hello",
                        "reply_to_message": { "message_id": 41 }
                    }
                }
            ]
        }"#;

        let updates = parse_updates(body).expect("parse ok");
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].update_id, 123456);
        assert_eq!(
            updates[0].kind,
            UpdateKind::Message {
                chat_id: 999,
                text: "hello".to_string(),
                reply_to_message_id: Some(41),
            }
        );
    }

    #[test]
    fn parses_message_without_reply_to() {
        let body = r#"{
            "ok": true,
            "result": [
                {
                    "update_id": 1,
                    "message": {
                        "message_id": 1,
                        "chat": { "id": 5 },
                        "text": "no reply here"
                    }
                }
            ]
        }"#;

        let updates = parse_updates(body).expect("parse ok");
        assert_eq!(
            updates[0].kind,
            UpdateKind::Message {
                chat_id: 5,
                text: "no reply here".to_string(),
                reply_to_message_id: None,
            }
        );
    }

    #[test]
    fn parses_callback_query() {
        let body = r#"{
            "ok": true,
            "result": [
                {
                    "update_id": 123457,
                    "callback_query": {
                        "id": "abc123-callback-id",
                        "message": { "chat": { "id": 999 }, "message_id": 43, "text": "perm prompt" },
                        "data": "token_a"
                    }
                }
            ]
        }"#;

        let updates = parse_updates(body).expect("parse ok");
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].update_id, 123457);
        assert_eq!(
            updates[0].kind,
            UpdateKind::Callback {
                chat_id: 999,
                callback_id: "abc123-callback-id".to_string(),
                data: "token_a".to_string(),
                message_id: 43,
                message_text: "perm prompt".to_string(),
            }
        );
    }

    #[test]
    fn callback_query_without_message_is_skipped_without_failing_whole_batch() {
        // Telegram documents `CallbackQuery.message` as optional (absent for
        // inline-query-originated callbacks, or when the original message
        // was deleted). A naive `message` field that's required would fail
        // `serde_json::from_str` for the WHOLE envelope, dropping every
        // update in the batch — including the unrelated, perfectly normal
        // message update alongside it. This must not happen: the malformed
        // (from this bridge's perspective) callback is skipped, the normal
        // update survives.
        let body = r#"{
            "ok": true,
            "result": [
                {
                    "update_id": 1,
                    "callback_query": {
                        "id": "cbq-orphan",
                        "data": "tok-a"
                    }
                },
                {
                    "update_id": 2,
                    "message": {
                        "message_id": 2,
                        "chat": { "id": 5 },
                        "text": "kept"
                    }
                }
            ]
        }"#;

        let updates = parse_updates(body).expect("parse ok, not Err(Parse)");
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].update_id, 2);
        assert_eq!(
            updates[0].kind,
            UpdateKind::Message {
                chat_id: 5,
                text: "kept".to_string(),
                reply_to_message_id: None,
            }
        );
    }

    #[test]
    fn empty_result_yields_empty_vec() {
        let body = r#"{ "ok": true, "result": [] }"#;
        let updates = parse_updates(body).expect("parse ok");
        assert!(updates.is_empty());
    }

    #[test]
    fn update_with_neither_message_nor_callback_is_skipped() {
        let body = r#"{
            "ok": true,
            "result": [
                {
                    "update_id": 1,
                    "edited_message": {
                        "message_id": 1,
                        "chat": { "id": 5 },
                        "text": "edited"
                    }
                },
                {
                    "update_id": 2,
                    "message": {
                        "message_id": 2,
                        "chat": { "id": 5 },
                        "text": "kept"
                    }
                }
            ]
        }"#;

        let updates = parse_updates(body).expect("parse ok");
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].update_id, 2);
    }

    #[test]
    fn message_with_missing_text_is_skipped() {
        let body = r#"{
            "ok": true,
            "result": [
                {
                    "update_id": 1,
                    "message": {
                        "message_id": 1,
                        "chat": { "id": 5 },
                        "photo": [{"file_id": "abc"}]
                    }
                }
            ]
        }"#;

        let updates = parse_updates(body).expect("parse ok");
        assert!(updates.is_empty());
    }

    #[test]
    fn multiple_updates_preserve_update_id_order() {
        let body = r#"{
            "ok": true,
            "result": [
                {
                    "update_id": 100,
                    "message": { "message_id": 1, "chat": { "id": 5 }, "text": "first" }
                },
                {
                    "update_id": 101,
                    "message": { "message_id": 2, "chat": { "id": 5 }, "text": "second" }
                }
            ]
        }"#;

        let updates = parse_updates(body).expect("parse ok");
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].update_id, 100);
        assert_eq!(updates[1].update_id, 101);
    }

    #[test]
    fn endpoint_envelope_parsers_handle_ok_and_http_errors() {
        fn assert_http_contains<T: std::fmt::Debug>(result: Result<T, ClientError>, needle: &str) {
            match result.expect_err("should error") {
                ClientError::Http(msg) => {
                    assert!(msg.contains(needle), "{msg:?} should contain {needle:?}")
                }
                other => panic!("expected Http error, got {other:?}"),
            }
        }

        assert_http_contains(
            parse_updates(r#"{ "ok": false, "description": "Unauthorized" }"#),
            "Unauthorized",
        );

        assert_eq!(
            parse_send_message_response(r#"{ "ok": true, "result": { "message_id": 44 } }"#)
                .expect("parse ok"),
            44
        );
        assert_http_contains(
            parse_send_message_response(r#"{ "ok": false, "description": "chat not found" }"#),
            "chat not found",
        );

        assert!(parse_answer_callback_response(r#"{ "ok": true, "result": true }"#).is_ok());
        assert_http_contains(
            parse_answer_callback_response(r#"{ "ok": false, "description": "query is too old" }"#),
            "query is too old",
        );
        assert_http_contains(
            parse_answer_callback_response(r#"{ "ok": false }"#),
            "answerCallbackQuery failed",
        );

        assert!(parse_edit_message_response(r#"{ "ok": true, "result": {} }"#).is_ok());
        assert_http_contains(
            parse_edit_message_response(
                r#"{ "ok": false, "description": "message to edit not found" }"#,
            ),
            "message to edit not found",
        );
    }

    #[test]
    fn client_error_display_includes_inner() {
        let e = ClientError::Http("connection refused".to_string());
        assert!(e.to_string().contains("connection refused"));
        let e = ClientError::Parse("missing field".to_string());
        assert!(e.to_string().contains("missing field"));
    }
}
