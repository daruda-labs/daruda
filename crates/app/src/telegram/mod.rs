//! Telegram bot bridge — lets a phone receive agent-chat notifications
//! and reply/approve permissions remotely.
//!
//! This module holds the storage substrate (the OS keychain wrapper
//! for the bot token — non-secret settings live in
//! `daruda_config::TelegramConfig`), the raw Bot API HTTP client, and
//! the pure routing state machine. GPUI wiring is added by later work
//! on top of this.

pub mod bridge;
pub mod client;
pub mod global;
pub mod keychain;
mod markdown;
