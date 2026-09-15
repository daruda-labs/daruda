//! Telegram compatibility exports for the shared remote-control vocabulary.

use crate::remote_channel::bridge::{InlineKeyboard, MessageTail};

mod core;
pub use crate::remote_channel::bridge::*;
pub use core::BridgeCore;

/// Telegram's name for the shared [`MessageTail`], kept so the relay call
/// sites that predate the other channels still read as they did.
pub type TelegramTail = MessageTail;

/// What the caller should actually send to Telegram. Telegram-only: it names
/// its recipient by `chat_id`, where the other channels carry a
/// `RemoteRecipient` alongside a `PreparedPing`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundMsg {
    pub chat_id: i64,
    pub header: String,
    pub tail: MessageTail,
    pub keyboard: Option<InlineKeyboard>,
}
