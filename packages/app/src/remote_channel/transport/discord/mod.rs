//! Discord Gateway sessions and REST messages; interactions are deferred at receipt.

use super::{
    Credentials, Incoming, IncomingKind, Message, Result, TransportError, chunks, http, socket,
};
use daruda_config::remote::RemoteRecipient;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const API: &str = "https://discord.com/api/v10";
const GATEWAY: &str = "wss://gateway.discord.gg/?v=10&encoding=json";
const INTENTS: u64 = (1 << 0) | (1 << 9) | (1 << 12) | (1 << 15);
const CONTENT_LIMIT: usize = 2000;
const BUTTON_LABEL_LIMIT: usize = 80;
const BUTTONS_PER_ROW: usize = 5;
const MAX_BUTTONS: usize = 25;
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Default)]
pub struct Gateway {
    resume: Option<Resume>,
    sequence: Option<u64>,
}

struct Resume {
    id: String,
    url: String,
}

struct Heartbeat {
    interval: Duration,
    next: Instant,
    awaiting_ack: bool,
}

impl Heartbeat {
    fn tick(&mut self, now: Instant) -> Result<bool> {
        if now < self.next {
            return Ok(false);
        }
        if self.awaiting_ack {
            return Err(TransportError::Message(
                "Discord heartbeat acknowledgement timed out".into(),
            ));
        }
        self.awaiting_ack = true;
        self.next = now + self.interval;
        Ok(true)
    }
}

impl Gateway {
    pub fn reset(&mut self) {
        self.resume = None;
        self.sequence = None;
    }
    pub fn session(
        &mut self,
        credentials: &Credentials,
        stop: &AtomicBool,
        mut publish: impl FnMut(Incoming) -> bool,
        mut ready: impl FnMut(),
    ) -> Result<()> {
        let url = self
            .resume
            .as_ref()
            .map(|r| format!("{}/?v=10&encoding=json", r.url.trim_end_matches('/')))
            .unwrap_or_else(|| GATEWAY.to_owned());
        let mut socket = socket::connect(&url)?;
        let deadline = Instant::now() + HELLO_TIMEOUT;
        let interval = loop {
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(TransportError::Message(
                    "Discord gateway hello timed out".into(),
                ));
            }
            if let Some(hello) = socket::read(&mut socket)?
                && hello["op"] == 10
            {
                let millis = hello["d"]["heartbeat_interval"]
                    .as_u64()
                    .filter(|n| *n > 0 && *n <= 120_000)
                    .ok_or_else(|| {
                        TransportError::Message("Invalid Discord heartbeat interval".into())
                    })?;
                break Duration::from_millis(millis);
            }
        };
        let mut heartbeat = Heartbeat {
            interval,
            next: Instant::now() + interval.mul_f64(jitter()),
            awaiting_ack: false,
        };
        let identify = match (&self.resume, self.sequence) {
            (Some(resume), Some(sequence)) => {
                json!({"op":6, "d":{"token":credentials.bot, "session_id":resume.id, "seq":sequence}})
            }
            _ => json!({"op":2, "d":{"token":credentials.bot, "intents":INTENTS,
                "properties":{"os":std::env::consts::OS, "browser":"daruda", "device":"daruda"}}}),
        };
        socket::send(&mut socket, &identify)?;
        while !stop.load(Ordering::Relaxed) {
            if heartbeat.tick(Instant::now())? {
                socket::send(&mut socket, &json!({"op":1,"d":self.sequence}))?;
            }
            let Some(event) = socket::read(&mut socket)? else {
                continue;
            };
            match event["op"].as_u64() {
                Some(0) => {
                    // A failed interaction ACK must not trap Resume on the same expired receipt.
                    self.sequence = event["s"].as_u64().or(self.sequence);
                    match event["t"].as_str() {
                        Some("READY") => {
                            self.resume = Some(Resume {
                                id: http::required(&event["d"], "session_id")?.to_owned(),
                                url: http::required(&event["d"], "resume_gateway_url")?.to_owned(),
                            });
                            ready();
                        }
                        Some("RESUMED") => ready(),
                        Some("INTERACTION_CREATE") if event["d"]["type"] == 3 => {
                            let id = http::required(&event["d"], "id")?;
                            let token = http::required(&event["d"], "token")?;
                            // This deadline precedes all GPUI work, permission decisions, and sends.
                            http::acknowledge(
                                &format!("{API}/interactions/{id}/{token}/callback"),
                                &json!({"type":6}),
                            )?;
                        }
                        _ => {}
                    }
                    if let Some(incoming) = parse(&event)
                        && !publish(incoming)
                    {
                        return Ok(());
                    }
                }
                Some(1) => {
                    socket::send(&mut socket, &json!({"op":1,"d":self.sequence}))?;
                    heartbeat.awaiting_ack = true;
                }
                Some(7) => {
                    return Err(TransportError::Message(
                        "Discord requested reconnection".into(),
                    ));
                }
                Some(9) => {
                    if event["d"] != true {
                        self.resume = None;
                        self.sequence = None;
                    }
                    return Err(TransportError::Message(
                        "Discord session invalidated".into(),
                    ));
                }
                Some(11) => heartbeat.awaiting_ack = false,
                _ => {}
            }
        }
        Ok(())
    }
}

fn jitter() -> f64 {
    f64::from(uuid::Uuid::new_v4().as_bytes()[0]) / 256.0
}

fn parse(event: &Value) -> Option<Incoming> {
    let data = &event["d"];
    let (user, kind) = match event["t"].as_str()? {
        "MESSAGE_CREATE" if data["author"]["bot"] != true && data.get("webhook_id").is_none() => (
            &data["author"],
            IncomingKind::Message {
                text: data["content"].as_str()?.to_owned(),
                reply_to: data["message_reference"]["message_id"]
                    .as_str()
                    .map(str::to_owned),
            },
        ),
        "INTERACTION_CREATE" if data["type"] == 3 && data["data"]["component_type"] == 2 => (
            data.get("user").unwrap_or(&data["member"]["user"]),
            IncomingKind::Callback {
                data: data["data"]["custom_id"].as_str()?.to_owned(),
                message_id: data["message"]["id"].as_str()?.to_owned(),
                original: data["message"]["content"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
            },
        ),
        _ => return None,
    };
    Some(Incoming {
        event_id: data["id"].as_str()?.to_owned(),
        sender: RemoteRecipient {
            user_id: user["id"].as_str()?.to_owned(),
            conversation_id: data["channel_id"].as_str()?.to_owned(),
            scope_id: data["guild_id"].as_str().unwrap_or_default().to_owned(),
        },
        kind,
    })
}

fn components(keyboard: Option<&crate::remote_channel::bridge::InlineKeyboard>) -> Vec<Value> {
    let Some(keyboard) = keyboard else {
        return Vec::new();
    };
    let buttons: Vec<_> = keyboard
        .rows
        .iter()
        .flatten()
        .map(|(label, token)| {
            let label = chunks(label, BUTTON_LABEL_LIMIT)
                .into_iter()
                .next()
                .unwrap_or_else(|| "?".into());
            json!({"type":2, "style":2, "label":label, "custom_id":token})
        })
        .collect();
    buttons
        .chunks(BUTTONS_PER_ROW)
        .map(|row| json!({"type":1, "components":row}))
        .collect()
}

fn payloads(message: &Message) -> Vec<Value> {
    let mut payloads = chunks(&super::format::discord(message), CONTENT_LIMIT)
        .into_iter()
        .map(|text| json!({"content":text,"allowed_mentions":{"parse":[]},"components":[]}))
        .collect::<Vec<_>>();
    let rows = components(message.keyboard.as_ref());
    for (index, group) in rows.chunks(MAX_BUTTONS / BUTTONS_PER_ROW).enumerate() {
        if index == 0
            && let Some(last) = payloads.last_mut()
        {
            last["components"] = json!(group);
        } else {
            payloads.push(json!({"components":group,"allowed_mentions":{"parse":[]}}));
        }
    }
    payloads
}

pub fn send(
    credentials: &Credentials,
    recipient: &RemoteRecipient,
    message: Message,
    mut on_sent: impl FnMut(&str),
) -> Result<Vec<String>> {
    let mut ids = Vec::new();
    for payload in payloads(&message) {
        let value = http::request(
            "POST",
            &format!("{API}/channels/{}/messages", recipient.conversation_id),
            Some(&format!("Bot {}", credentials.bot)),
            &payload,
        )?;
        let id = http::required(&value, "id")?;
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
    let text = super::feedback(original, label, CONTENT_LIMIT);
    http::request(
        "PATCH",
        &format!(
            "{API}/channels/{}/messages/{message_id}",
            recipient.conversation_id
        ),
        Some(&format!("Bot {}", credentials.bot)),
        &json!({"content":text,"components":[],"allowed_mentions":{"parse":[]}}),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
