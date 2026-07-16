//! Telegram bot bridge — lets a phone receive agent-chat notifications
//! and reply/approve permissions remotely. Holds the keychain wrapper
//! for the bot token (non-secret settings live in
//! `daruda_config::TelegramConfig`), the raw Bot API HTTP client, the
//! pure routing state machine, and the GPUI poll/send wiring.

pub mod bridge;
pub mod client;
pub mod global;
pub mod keychain;
mod markdown;
