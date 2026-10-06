//! Shared transport vocabulary and bounded HTTP/WebSocket I/O.

use super::bridge::{InlineKeyboard, MessageTail, PreparedPing};
use daruda_config::remote::{ChannelKind, RemoteRecipient};

pub(crate) mod discord;
mod format;
pub(crate) mod http;
pub(crate) mod slack;
pub(crate) mod socket;

/// Deliberately not Debug: interaction receipts and credentials are secrets.
#[derive(Clone)]
pub struct Credentials {
    pub bot: String,
    pub app: Option<String>,
}

pub struct Incoming {
    pub event_id: String,
    pub sender: RemoteRecipient,
    pub kind: IncomingKind,
}

pub enum IncomingKind {
    Message {
        text: String,
        reply_to: Option<String>,
    },
    Callback {
        data: String,
        message_id: String,
        original: String,
    },
}

pub type Message = PreparedPing;

impl PreparedPing {
    pub fn plain(text: String, keyboard: Option<InlineKeyboard>) -> Self {
        Self {
            header: String::new(),
            tail: MessageTail::Plain(text),
            keyboard,
        }
    }
}

#[derive(Debug)]
pub enum TransportError {
    Message(String),
    Closed(u16),
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(message) => f.write_str(message),
            Self::Closed(code) => write!(f, "Gateway closed with code {code}"),
        }
    }
}

impl std::error::Error for TransportError {}

pub type Result<T> = std::result::Result<T, TransportError>;

pub fn send(
    kind: ChannelKind,
    credentials: &Credentials,
    recipient: &RemoteRecipient,
    message: Message,
) -> Result<Vec<String>> {
    send_recording(kind, credentials, recipient, message, |_| {})
}

pub fn send_recording(
    kind: ChannelKind,
    credentials: &Credentials,
    recipient: &RemoteRecipient,
    message: Message,
    on_sent: impl FnMut(&str),
) -> Result<Vec<String>> {
    match kind {
        ChannelKind::Slack => slack::send(credentials, recipient, message, on_sent),
        ChannelKind::Discord => discord::send(credentials, recipient, message, on_sent),
    }
}

pub fn edit(
    kind: ChannelKind,
    credentials: &Credentials,
    recipient: &RemoteRecipient,
    message_id: &str,
    original: &str,
    label: &str,
) -> Result<()> {
    match kind {
        ChannelKind::Slack => slack::edit(credentials, recipient, message_id, original, label),
        ChannelKind::Discord => discord::edit(credentials, recipient, message_id, original, label),
    }
}

/// Split on scalar boundaries while measuring UTF-16, the stricter platform unit.
pub(crate) fn feedback(original: &str, label: &str, limit: usize) -> String {
    let suffix = format!("\n\n{label}");
    let reserve = suffix.encode_utf16().count();
    let prefix = chunks(original, limit.saturating_sub(reserve).max(2))
        .into_iter()
        .next()
        .unwrap_or_default();
    format!("{prefix}{suffix}")
}

pub(crate) fn chunks(text: &str, limit: usize) -> Vec<String> {
    let mut result = Vec::new();
    let mut part = String::new();
    let mut size = 0;
    for ch in text.chars() {
        if size + ch.len_utf16() > limit && !part.is_empty() {
            result.push(std::mem::take(&mut part));
            size = 0;
        }
        part.push(ch);
        size += ch.len_utf16();
    }
    if !part.is_empty() {
        result.push(part);
    }
    result
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_full_length_prompt_keeps_its_decision_after_editing() {
        let body = super::feedback(&"x".repeat(2000), "Allowed", 2000);
        assert!(body.ends_with("Allowed"));
        assert!(body.encode_utf16().count() <= 2000);
    }
    #[test]
    fn unicode_messages_are_lossless_and_bounded() {
        let text = "A\u{1f600}\u{d55c}".repeat(1000);
        let chunks = super::chunks(&text, 2000);
        assert_eq!(chunks.concat(), text);
        assert!(
            chunks
                .iter()
                .all(|part| part.encode_utf16().count() <= 2000)
        );
    }
}
