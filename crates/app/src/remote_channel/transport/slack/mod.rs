//! Slack Web API and Socket Mode adapter.

use super::{
    Credentials, Incoming, IncomingKind, Message, Result, TransportError, chunks, http, socket,
};
use daruda_config::remote::RemoteRecipient;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};

const API: &str = "https://slack.com/api";
const SECTION_LIMIT: usize = 3000;
const BUTTON_LABEL_LIMIT: usize = 75;
const BUTTONS_PER_ROW: usize = 25;

fn api(credentials: &str, method: &str, payload: &Value) -> Result<Value> {
    let value = http::request(
        "POST",
        &format!("{API}/{method}"),
        Some(&format!("Bearer {credentials}")),
        payload,
    )?;
    if value["ok"] != true {
        // Only Slack's symbolic error code leaves the adapter, never payload text.
        let code = value["error"].as_str().unwrap_or("unknown_error");
        let safe: String = code
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
            .take(80)
            .collect();
        return Err(TransportError::Message(format!(
            "Slack {method} failed: {safe}"
        )));
    }
    Ok(value)
}

pub fn session(
    credentials: &Credentials,
    stop: &AtomicBool,
    mut publish: impl FnMut(Incoming) -> bool,
    ready: impl FnOnce(),
) -> Result<()> {
    let app = credentials
        .app
        .as_deref()
        .ok_or_else(|| TransportError::Message("Slack app token is missing".into()))?;
    let opened = api(app, "apps.connections.open", &json!({}))?;
    let mut socket = socket::connect(http::required(&opened, "url")?)?;
    ready();
    while !stop.load(Ordering::Relaxed) {
        let Some(envelope) = socket::read(&mut socket)? else {
            continue;
        };
        if envelope["type"] == "disconnect" {
            return Err(TransportError::Message(
                "Slack requested a new connection".into(),
            ));
        }
        if let Some(id) = envelope["envelope_id"].as_str() {
            socket::send(&mut socket, &json!({"envelope_id": id}))?;
        }
        if let Some(incoming) = parse(&envelope)
            && !publish(incoming)
        {
            break;
        }
    }
    Ok(())
}

fn parse(envelope: &Value) -> Option<Incoming> {
    let payload = &envelope["payload"];
    match envelope["type"].as_str()? {
        "events_api" => {
            let event = &payload["event"];
            if event["type"] != "message"
                || event.get("subtype").is_some()
                || event.get("bot_id").is_some()
            {
                return None;
            }
            Some(Incoming {
                event_id: payload["event_id"].as_str()?.to_owned(),
                sender: RemoteRecipient {
                    user_id: event["user"].as_str()?.to_owned(),
                    conversation_id: event["channel"].as_str()?.to_owned(),
                    scope_id: payload["team_id"].as_str()?.to_owned(),
                },
                kind: IncomingKind::Message {
                    text: unescape(event["text"].as_str()?),
                    reply_to: event["thread_ts"].as_str().map(str::to_owned),
                },
            })
        }
        "interactive" if payload["type"] == "block_actions" => Some(Incoming {
            event_id: envelope["envelope_id"].as_str()?.to_owned(),
            sender: RemoteRecipient {
                user_id: payload["user"]["id"].as_str()?.to_owned(),
                conversation_id: payload["channel"]["id"].as_str()?.to_owned(),
                scope_id: payload["team"]["id"].as_str()?.to_owned(),
            },
            kind: IncomingKind::Callback {
                data: payload["actions"][0]["value"].as_str()?.to_owned(),
                message_id: payload["message"]["ts"].as_str()?.to_owned(),
                original: payload["message"]["text"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            },
        }),
        _ => None,
    }
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn blocks(
    text: &str,
    keyboard: Option<&crate::remote_channel::bridge::InlineKeyboard>,
    markdown: bool,
) -> Vec<Value> {
    let text_is_empty = text.is_empty();
    let text = if markdown {
        json!({"type": "mrkdwn", "text": text, "verbatim": true})
    } else {
        json!({"type": "plain_text", "text": text, "emoji": false})
    };
    // Slack rejects an empty `plain_text`, so a body that rendered to nothing
    // contributes no section at all — the header block still ships.
    let mut blocks = Vec::new();
    if !text_is_empty {
        blocks.push(json!({"type": "section", "text": text}));
    }
    if let Some(keyboard) = keyboard {
        for (row_index, row) in keyboard.rows.iter().enumerate() {
            for buttons in row.chunks(BUTTONS_PER_ROW) {
                let elements: Vec<_> = buttons
                    .iter()
                    .enumerate()
                    .map(|(index, (label, token))| {
                        let label = chunks(label, BUTTON_LABEL_LIMIT)
                            .into_iter()
                            .next()
                            .unwrap_or_else(|| "?".into());
                        json!({"type": "button", "action_id": format!("remote-{row_index}-{index}"),
                        "text": {"type": "plain_text", "text": label}, "value": token})
                    })
                    .collect();
                blocks.push(json!({"type": "actions", "elements": elements}));
            }
        }
    }
    blocks
}

/// One slot per outgoing message. A body that rendered to nothing keeps one
/// empty slot when there is a header to carry, so `chunks`' empty result
/// cannot silently drop the whole message.
fn body_parts(text: &str, header: &str) -> Vec<String> {
    let mut parts = chunks(text, SECTION_LIMIT);
    if parts.is_empty() && !header.is_empty() {
        parts.push(String::new());
    }
    parts
}

pub fn send(
    credentials: &Credentials,
    recipient: &RemoteRecipient,
    message: Message,
    mut on_sent: impl FnMut(&str),
) -> Result<Vec<String>> {
    let (text, markdown) = super::format::slack(&message);
    let parts = body_parts(&text, &message.header);
    let mut ids = Vec::new();
    for (index, text) in parts.iter().enumerate() {
        let keyboard = (index + 1 == parts.len())
            .then_some(message.keyboard.as_ref())
            .flatten();
        let mut message_blocks = blocks(text, keyboard, markdown);
        let fallback = if index == 0 && !message.header.is_empty() {
            let header = chunks(&message.header, SECTION_LIMIT)
                .into_iter()
                .next()
                .unwrap_or_default();
            message_blocks.insert(0, json!({"type":"section", "text":{"type":"plain_text", "text":header, "emoji":false}}));
            if text.is_empty() {
                header
            } else {
                format!("{header}\n{text}")
            }
        } else {
            text.clone()
        };
        let response = api(
            &credentials.bot,
            "chat.postMessage",
            &json!({
                "channel": recipient.conversation_id, "text": fallback, "mrkdwn": false,
                "parse": "none", "unfurl_links": false, "unfurl_media": false,
                "blocks": message_blocks,
            }),
        )?;
        let id = http::required(&response, "ts")?;
        on_sent(id);
        ids.push(id.to_owned());
    }
    Ok(ids)
}

pub fn edit(
    credentials: &Credentials,
    recipient: &RemoteRecipient,
    message_id: &str,
    original: &str,
    label: &str,
) -> Result<()> {
    let text = super::feedback(original, label, SECTION_LIMIT);
    api(
        &credentials.bot,
        "chat.update",
        &json!({"channel": recipient.conversation_id, "ts": message_id,
        "text": text, "parse": "none", "blocks": blocks(&text, None, false)}),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
