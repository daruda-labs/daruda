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

/// A socket the server dropped is retried; a slow one is not, since its
/// request may already have been delivered.
#[test]
fn only_a_dead_socket_is_worth_a_retry() {
    use std::io::ErrorKind;
    for kind in [
        ErrorKind::ConnectionReset,
        ErrorKind::ConnectionAborted,
        ErrorKind::BrokenPipe,
    ] {
        assert!(is_dead_connection_kind(kind), "{kind:?}");
    }
    for kind in [ErrorKind::TimedOut, ErrorKind::WouldBlock, ErrorKind::Other] {
        assert!(!is_dead_connection_kind(kind), "{kind:?}");
    }
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
fn outbound_calls_are_stubbed_under_test_no_network() {
    let first = send_message("dummy-token", 1, "hi", None, None).expect("stub sends");
    let second = send_message("dummy-token", 1, "hi", None, None).expect("stub sends");
    assert_ne!(first, second, "each stubbed send gets its own message id");
    answer_callback("dummy-token", "callback", Some("ok")).expect("stub acks");
    edit_message_text("dummy-token", 1, first, "edited").expect("stub edits");
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
