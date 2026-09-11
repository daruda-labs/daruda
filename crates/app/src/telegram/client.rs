//! Raw Telegram Bot API HTTP client — `getUpdates` / `sendMessage` /
//! `answerCallbackQuery` / `editMessageText`, plus the wire-format parsing
//! the routing layer needs.
//!
//! GPUI-free and stateless: the token is an argument, nothing is kept between
//! calls. `bridge` owns the offset; `global`'s poll loop owns the timer.

use std::io::Read;
use std::time::Duration;

use serde::Deserialize;

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

/// Inline keyboard rows attached to a `sendMessage` call, outermost first.
/// Telegram lays each inner vector out horizontally, so a long list must be
/// split into rows or it renders as one unreadable strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineKeyboard {
    pub rows: Vec<Vec<(String, String)>>,
}

impl InlineKeyboard {
    /// One horizontal row — what a permission prompt wants.
    pub fn single_row(buttons: Vec<(String, String)>) -> Self {
        Self {
            rows: vec![buttons],
        }
    }
}

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
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(timeout_s) + LONG_POLL_MARGIN)
        .build();

    let response = agent.get(&url).call().map_err(|e| http_error(token, e))?;

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

    let agent = ureq::AgentBuilder::new().timeout(REQUEST_TIMEOUT).build();
    let response = agent
        .post(&base_url(token, "sendMessage"))
        .send_json(payload)
        .map_err(|e| http_error(token, e))?;

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
    let mut payload = serde_json::json!({ "callback_query_id": callback_id });
    if let Some(text) = text {
        payload["text"] = serde_json::json!(clamp_callback_text(text));
    }

    let agent = ureq::AgentBuilder::new().timeout(REQUEST_TIMEOUT).build();
    let response = agent
        .post(&base_url(token, "answerCallbackQuery"))
        .send_json(payload)
        .map_err(|e| http_error(token, e))?;

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
    let payload = serde_json::json!({
        "chat_id": chat_id,
        "message_id": message_id,
        "text": text,
    });

    let agent = ureq::AgentBuilder::new().timeout(REQUEST_TIMEOUT).build();
    let response = agent
        .post(&base_url(token, "editMessageText"))
        .send_json(payload)
        .map_err(|e| http_error(token, e))?;

    let body = read_body(response)?;
    parse_edit_message_response(&body)
}

fn read_body(response: ureq::Response) -> Result<String, ClientError> {
    let mut body = String::new();
    response
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
mod tests {
    use super::*;

    /// The regression this guards is a hang, not a wrong answer: one dropped
    /// sticker wedged the poll loop and took every later command with it.
    /// [`UpdateKind`] has the why.
    #[test]
    fn an_update_this_bridge_cannot_act_on_still_carries_its_id() {
        let body = r#"{"ok":true,"result":[
            {"update_id":41,"message":{"chat":{"id":7},"sticker":{"emoji":"x"}}},
            {"update_id":42,"edited_message":{"chat":{"id":7},"text":"redone"}},
            {"update_id":43,"callback_query":{"id":"cb","data":"tok"}},
            {"update_id":44,"message":{"chat":{"id":7},"text":"hello"}}
        ]}"#;
        let updates = parse_updates(body).expect("parses");
        let ids: Vec<i64> = updates.iter().map(|u| u.update_id).collect();
        assert_eq!(ids, vec![41, 42, 43, 44], "every id must survive parsing");
        assert_eq!(
            updates
                .iter()
                .filter(|u| u.kind == UpdateKind::Unsupported)
                .count(),
            3,
            "a sticker, an edit, and a message-less callback are all unsupported"
        );
        assert!(matches!(updates[3].kind, UpdateKind::Message { .. }));
    }

    #[test]
    fn a_long_callback_toast_is_clamped_on_a_char_boundary() {
        // The regression: a listing label carries a project/lane/session title,
        // so it can exceed the limit — and a rejected ack leaves the phone's
        // button spinning until Telegram times it out.
        let long = "한".repeat(500);
        let clamped = clamp_callback_text(&long);
        assert_eq!(clamped.chars().count(), ANSWER_CALLBACK_MAX_CHARS);
        assert!(long.starts_with(&clamped), "a prefix, not a re-encoding");
    }

    #[test]
    fn a_short_callback_toast_is_untouched() {
        assert_eq!(clamp_callback_text("Allowed"), "Allowed");
    }

    #[test]
    fn a_single_row_keyboard_serializes_as_one_row() {
        let kb = InlineKeyboard::single_row(vec![("Allow".into(), "a".into())]);
        assert_eq!(
            inline_keyboard_json(&kb),
            serde_json::json!({ "inline_keyboard": [[{ "text": "Allow", "callback_data": "a" }]] })
        );
    }

    #[test]
    fn multiple_rows_serialize_as_nested_arrays() {
        let kb = InlineKeyboard {
            rows: vec![
                vec![("1".into(), "u1".into()), ("2".into(), "u2".into())],
                vec![("3".into(), "u3".into())],
            ],
        };
        let json = inline_keyboard_json(&kb);
        let rows = json["inline_keyboard"].as_array().expect("rows");
        assert_eq!(rows.len(), 2, "two rows, not one flattened row");
        assert_eq!(rows[0].as_array().expect("row").len(), 2);
        assert_eq!(rows[1].as_array().expect("row").len(), 1);
    }

    #[test]
    fn a_transport_error_never_carries_the_bot_token() {
        // Fake value in Telegram's `<bot_id>:<35 chars>` shape. Never paste a
        // real token here: this file is committed to a public repository.
        let token = "123456789:AA-this-is-not-a-real-bot-token-000";
        let raw = format!("{}: connection timed out", base_url(token, "getUpdates"));

        let ClientError::Http(message) = http_error(token, raw) else {
            panic!("http_error builds an Http error");
        };

        assert!(!message.contains(token), "the token leaked: {message}");
        assert!(
            message.contains(REDACTED_TOKEN),
            "the placeholder marks where it was: {message}"
        );
        assert!(
            message.contains("getUpdates"),
            "the failing call is still identifiable: {message}"
        );
    }

    #[test]
    fn get_updates_is_stubbed_under_test_no_network() {
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
    fn callback_query_without_message_does_not_fail_the_whole_batch() {
        // A required `message` field would fail the whole envelope's decode
        // and take the normal update beside it — see `RawCallbackQuery`.
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
        // Two, not one: the orphan callback's id still has to move the offset.
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].update_id, 1);
        assert_eq!(updates[0].kind, UpdateKind::Unsupported);
        assert_eq!(updates[1].update_id, 2);
        assert_eq!(
            updates[1].kind,
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
    fn update_with_neither_message_nor_callback_is_unsupported() {
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
        // Two, not one: an `edited_message` is nothing this bridge acts on,
        // but its id still has to move the offset.
        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].kind, UpdateKind::Unsupported);
        assert_eq!(updates[1].update_id, 2);
    }

    #[test]
    fn message_with_missing_text_is_unsupported_not_dropped() {
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
        // Not empty: a photo the bridge ignores still has to move the offset.
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].update_id, 1);
        assert_eq!(updates[0].kind, UpdateKind::Unsupported);
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
