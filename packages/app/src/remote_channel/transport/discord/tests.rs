use super::*;

#[test]
fn message_and_interaction_fixtures_preserve_snowflakes_and_clicker() {
    let message = json!({"t":"MESSAGE_CREATE","d":{"id":"18446744073709551615","channel_id":"2","guild_id":"3",
        "author":{"id":"4"},"content":"!list","message_reference":{"message_id":"5"}}});
    let parsed = parse(&message).unwrap();
    assert_eq!(parsed.event_id, "18446744073709551615");
    assert_eq!(parsed.sender.user_id, "4");
    assert!(matches!(parsed.kind, IncomingKind::Message { reply_to: Some(id), .. } if id == "5"));
    let callback = json!({"t":"INTERACTION_CREATE","d":{"id":"6","channel_id":"2","guild_id":"3","type":3,
        "member":{"user":{"id":"7"}},"data":{"component_type":2,"custom_id":"opaque"},"message":{"id":"8","content":"Allow?"}}});
    let parsed = parse(&callback).unwrap();
    assert_eq!(parsed.sender.user_id, "7");
    assert!(matches!(parsed.kind, IncomingKind::Callback { data, .. } if data == "opaque"));
}

#[test]
fn bot_messages_are_not_reflected_back_into_the_agent() {
    assert!(
        parse(&json!({"t":"MESSAGE_CREATE","d":{"author":{"bot":true},"content":"hello"}}))
            .is_none()
    );
}

#[test]
fn heartbeat_requires_ack_before_the_next_scheduled_beat() {
    let now = Instant::now();
    let mut beat = Heartbeat {
        interval: Duration::from_secs(1),
        next: now,
        awaiting_ack: false,
    };
    assert!(beat.tick(now).unwrap());
    assert!(!beat.tick(now).unwrap());
    assert!(beat.tick(now + Duration::from_secs(1)).is_err());
    beat.awaiting_ack = false;
    assert!(beat.tick(now + Duration::from_secs(1)).unwrap());
}

#[test]
fn keyboard_respects_discord_row_limits_and_keeps_tokens() {
    let keyboard = crate::remote_channel::bridge::InlineKeyboard::single_row(
        (0..24)
            .map(|n| (format!("Option {n}"), format!("token-{n}")))
            .collect(),
    );
    let rows = components(Some(&keyboard));
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[4]["components"][3]["custom_id"], "token-23");
}

#[test]
fn oversized_permission_lists_keep_every_button_across_messages() {
    let keyboard = crate::remote_channel::bridge::InlineKeyboard::single_row(
        (0..51)
            .map(|n| (format!("Option {n}"), format!("token-{n}")))
            .collect(),
    );
    let payloads = payloads(&Message::plain("Choose".into(), Some(keyboard)));
    assert_eq!(payloads.len(), 3);
    assert_eq!(payloads[0]["content"], "Choose");
    let tokens = payloads
        .iter()
        .flat_map(|payload| {
            let rows = payload["components"].as_array().unwrap();
            assert!(rows.len() <= 5);
            rows.iter().flat_map(|row| {
                let buttons = row["components"].as_array().unwrap();
                assert!(buttons.len() <= 5);
                buttons
                    .iter()
                    .map(|button| button["custom_id"].as_str().unwrap())
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(tokens.len(), 51);
    assert_eq!(tokens[50], "token-50");
}
