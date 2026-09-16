use super::*;

#[test]
fn message_fixture_preserves_thread_and_authorization_coordinates() {
    let value = json!({"type":"events_api", "payload":{"event_id":"Ev1", "team_id":"T1",
        "event":{"type":"message", "user":"U1", "channel":"D1", "thread_ts":"1234.000001", "text":"!say a &lt; b"}}});
    let event = parse(&value).unwrap();
    assert_eq!(event.sender.scope_id, "T1");
    assert_eq!(event.sender.user_id, "U1");
    assert!(
        matches!(event.kind, IncomingKind::Message { reply_to: Some(id), text } if id == "1234.000001" && text == "!say a < b")
    );
    let mut bot = value.clone();
    bot["payload"]["event"]["bot_id"] = json!("B1");
    assert!(parse(&bot).is_none());
    bot = value;
    bot["payload"]["event"]["subtype"] = json!("message_changed");
    assert!(parse(&bot).is_none());
}

#[test]
fn callback_uses_the_clicking_user_and_opaque_button_value() {
    let event = parse(
        &json!({"type":"interactive", "envelope_id":"e1", "payload":{
            "type":"block_actions", "user":{"id":"U2"}, "channel":{"id":"D1"}, "team":{"id":"T1"},
            "message":{"ts":"12.34", "text":"permission"}, "actions":[{"value":"opaque-token"}]
        }}),
    )
    .unwrap();
    assert_eq!(event.sender.user_id, "U2");
    assert!(matches!(event.kind, IncomingKind::Callback { data, .. } if data == "opaque-token"));
}

#[test]
fn rendering_keeps_text_literal_and_callback_values_intact() {
    let keyboard = crate::remote_channel::bridge::InlineKeyboard::single_row(vec![(
        "Allow".into(),
        "token".into(),
    )]);
    let blocks = blocks("file_name.txt <@U1>", Some(&keyboard), false);
    assert_eq!(
        blocks[0]["elements"][0]["elements"][0]["text"],
        "file_name.txt <@U1>"
    );
    assert_eq!(blocks[1]["elements"][0]["value"], "token");
}

/// The defect this guards: `chunks` returns no parts for an empty body, so the
/// send loop never ran and the header — the only thing naming the pane — was
/// dropped along with it, without an error.
#[test]
fn a_body_that_renders_to_nothing_still_delivers_its_header() {
    assert_eq!(body_parts("", "my_project"), vec![String::new()]);
    assert!(body_parts("", "").is_empty());
    assert_eq!(body_parts("done", "my_project"), vec!["done".to_string()]);

    // Slack rejects an empty text object, so the empty body contributes no
    // block at all and the header block is what ships.
    assert!(blocks("", None, false).is_empty());
    assert_eq!(blocks("done", None, false).len(), 1);
}

/// The defect this guards: a `plain_text` section renders every `\n` as a
/// space, so a listing arrived as one run-on line. Literal text ships as
/// rich_text, which keeps the breaks and still parses no markup.
#[test]
fn literal_text_keeps_its_line_breaks() {
    let body = "Open agent chats\n1. daruda/main\n2. under_score *star*";
    let blocks = blocks(body, None, false);
    assert_eq!(blocks[0]["type"], "rich_text");
    let element = &blocks[0]["elements"][0]["elements"][0];
    assert_eq!(element["type"], "text");
    assert_eq!(element["text"], body);
}
