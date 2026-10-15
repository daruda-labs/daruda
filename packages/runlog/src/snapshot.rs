//! The snapshot file that starts every generation after 0: the machine state
//! at `seq`, with a checksum over the exact state bytes on disk.

use std::io;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use sha2::{Digest as _, Sha256};

use crate::record::{FORMAT_VERSION, VersionProbe};

/// A snapshot that read back whole.
#[derive(Debug, PartialEq)]
pub(crate) struct Base<S> {
    pub state: S,
    /// The last record seq the state includes.
    pub seq: u64,
    /// The highest `opened` epoch at compaction, so epochs keep rising after
    /// the records that carried them are gone.
    pub epoch: u64,
}

#[derive(Debug, PartialEq)]
pub(crate) enum Decoded<S> {
    Whole(Base<S>),
    TooNew(u32),
    /// Unparseable, checksum mismatch, or a state the machine cannot read.
    Broken,
}

#[derive(Serialize, Deserialize)]
struct SnapshotFile<'a> {
    v: u32,
    seq: u64,
    epoch: u64,
    checksum: String,
    #[serde(borrow)]
    state: &'a RawValue,
}

fn checksum(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) fn encode<S: Serialize>(state: &S, seq: u64, epoch: u64) -> io::Result<Vec<u8>> {
    let raw = serde_json::value::to_raw_value(state).map_err(io::Error::other)?;
    let file = SnapshotFile {
        v: FORMAT_VERSION,
        seq,
        epoch,
        checksum: checksum(raw.get().as_bytes()),
        state: &raw,
    };
    serde_json::to_vec(&file).map_err(io::Error::other)
}

pub(crate) fn decode<S: DeserializeOwned>(bytes: &[u8]) -> Decoded<S> {
    match serde_json::from_slice::<VersionProbe>(bytes) {
        Ok(probe) if probe.v > FORMAT_VERSION => return Decoded::TooNew(probe.v),
        Ok(_) => {}
        Err(_) => return Decoded::Broken,
    }
    let Ok(file) = serde_json::from_slice::<SnapshotFile<'_>>(bytes) else {
        return Decoded::Broken;
    };
    if checksum(file.state.get().as_bytes()) != file.checksum {
        return Decoded::Broken;
    }
    match serde_json::from_str(file.state.get()) {
        Ok(state) => Decoded::Whole(Base {
            state,
            seq: file.seq,
            epoch: file.epoch,
        }),
        Err(_) => Decoded::Broken,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn state() -> BTreeMap<String, u32> {
        BTreeMap::from([("b".into(), 2), ("a".into(), 1)])
    }

    #[test]
    fn a_snapshot_round_trips() {
        let bytes = encode(&state(), 7, 3).unwrap();
        assert_eq!(
            decode(&bytes),
            Decoded::Whole(Base {
                state: state(),
                seq: 7,
                epoch: 3
            })
        );
    }

    #[test]
    fn an_edited_state_fails_its_checksum() {
        let bytes = encode(&state(), 7, 3).unwrap();
        let edited = String::from_utf8(bytes)
            .unwrap()
            .replace("\"a\":1", "\"a\":9");
        assert_eq!(
            decode::<BTreeMap<String, u32>>(edited.as_bytes()),
            Decoded::Broken
        );
    }

    #[test]
    fn a_newer_version_is_reported_before_its_shape_is_read() {
        let bytes = format!("{{\"v\":{},\"anything\":true}}", FORMAT_VERSION + 1);
        assert_eq!(
            decode::<u32>(bytes.as_bytes()),
            Decoded::TooNew(FORMAT_VERSION + 1)
        );
    }

    #[test]
    fn a_truncated_file_is_broken() {
        let bytes = encode(&state(), 7, 3).unwrap();
        assert_eq!(
            decode::<BTreeMap<String, u32>>(&bytes[..bytes.len() / 2]),
            Decoded::Broken
        );
    }
}
