//! Read-only legacy inputs. The caller establishes which bot owns the source;
//! migration itself never infers identity from a profile-local config id.

use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::path::Path;

use daruda_runlog::{FileBackend, Machine, RunLog};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::{ChannelKey, INBOX_RING_CAPACITY, InboxSeed};

fn load<T: DeserializeOwned + Default>(path: &Path) -> io::Result<T> {
    match crate::persistence::read_json_bytes(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(T::default()),
        Err(error) => Err(error),
    }
}

pub(crate) fn read_legacy<T: DeserializeOwned + Default>(path: &Path) -> T {
    match load(path) {
        Ok(value) => value,
        Err(error) => {
            use crate::observability::error_report::{ErrorReport, ErrorSeverity};
            use crate::observability::{log_writer::LogWriter, system_info::redact_home};
            LogWriter::log(
                ErrorReport::new("Legacy remote history could not be read")
                    .severity(ErrorSeverity::Error)
                    .from_error(&error)
                    .at(file!(), line!())
                    .with_context("path", redact_home(path))
                    .dedup("store.inbox.legacy.read")
                    .build(),
            );
            T::default()
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Attribution {
    #[serde(default)]
    captured: Option<Captured>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Captured {
    bot: String,
    next_offset: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum SourceEvent {
    Captured { bot: String, next_offset: i64 },
}

#[derive(Debug, thiserror::Error)]
enum SourceRejected {
    #[error("legacy Telegram source has already been captured")]
    AlreadyCaptured,
    #[error("legacy Telegram source has an invalid cursor or bot identity")]
    InvalidSource,
}

struct SourceMachine;

impl Machine for SourceMachine {
    type State = Attribution;
    type Event = SourceEvent;
    type Rejected = SourceRejected;

    fn check(state: &Attribution, event: &SourceEvent) -> Result<(), SourceRejected> {
        if state.captured.is_some() {
            return Err(SourceRejected::AlreadyCaptured);
        }
        match event {
            SourceEvent::Captured { bot, next_offset } => {
                if *next_offset < 0 || crate::storage::StorageLayout::validate_bot(bot).is_err() {
                    Err(SourceRejected::InvalidSource)
                } else {
                    Ok(())
                }
            }
        }
    }

    fn apply(state: &mut Attribution, event: &SourceEvent) {
        match event {
            SourceEvent::Captured { bot, next_offset } => {
                state.captured = Some(Captured {
                    bot: bot.clone(),
                    next_offset: *next_offset,
                });
            }
        }
    }
}

/// Attribute the legacy source to the first current bot, including an empty
/// source, before importing it. The cursor is frozen here, so later tokens
/// or an older app rewriting the source cannot attribute it again.
/// The caller holds the profile's primary desktop ownership and serializes
/// Telegram startup; this journal has one writer. Run off the UI thread.
pub fn telegram_for_bot(data: &Path, bot: &str, build: &str) -> io::Result<InboxSeed> {
    crate::storage::StorageLayout::validate_bot(bot)?;
    let directory = crate::storage::StorageLayout::telegram_inbox_migration_at(data);
    daruda_core::path::create_owner_only_dir(&directory)?;
    let mut source = RunLog::<SourceMachine, _>::open(FileBackend::new(directory), build)
        .map_err(io::Error::other)?;
    if source.state().captured.is_none() {
        let legacy: crate::telegram::TelegramState =
            load(&crate::telegram::telegram_state_path_in(data))?;
        source
            .commit(SourceEvent::Captured {
                bot: bot.into(),
                next_offset: legacy.update_offset,
            })
            .map_err(io::Error::other)?;
    }
    let captured = source
        .state()
        .captured
        .as_ref()
        .expect("source was durably captured");
    if captured.bot != bot {
        return Ok(InboxSeed::default());
    }
    Ok(InboxSeed {
        monotonic: BTreeMap::from([(ChannelKey::new("telegram"), captured.next_offset - 1)]),
        ..InboxSeed::default()
    })
}

/// Map one legacy profile-local connection's ring onto its stable bot stream.
/// Normalize old inputs with the same oldest-eviction policy as live claims.
pub fn remote(data: &Path, legacy_channel: &str, channel: ChannelKey) -> io::Result<InboxSeed> {
    let source: crate::remote_channels::RemoteChannelState =
        load(&data.join(crate::remote_channels::FILE_NAME))?;
    let mut ring = VecDeque::new();
    for id in source.events(legacy_channel) {
        if !ring.iter().any(|seen| seen == id) {
            ring.push_back(id.to_owned());
        }
        while ring.len() > INBOX_RING_CAPACITY {
            ring.pop_front();
        }
    }
    Ok(InboxSeed {
        rings: BTreeMap::from([(channel, ring)]),
        ..InboxSeed::default()
    })
}

#[cfg(test)]
mod tests;
