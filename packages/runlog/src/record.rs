//! One line of a run log: a versioned envelope around an `opened` marker or
//! a machine event, serialized as a single JSON object per line.

use serde::{Deserialize, Serialize};

/// The record and snapshot format this build reads and writes.
pub const FORMAT_VERSION: u32 = 1;

/// A record envelope. `seq` is monotonic within one log and continues across
/// generations and reopens; `at_ms` is wall-clock time for display only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record<E> {
    pub v: u32,
    pub seq: u64,
    pub at_ms: u64,
    #[serde(flatten)]
    pub body: Body<E>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Body<E> {
    /// Written first by every successful open. `epoch` is one more than the
    /// highest epoch the log has seen, so each process lifetime is told apart.
    Opened {
        epoch: u64,
        build: String,
    },
    Event {
        event: E,
    },
}

/// Just enough of a line to read its version before trusting its shape.
#[derive(Deserialize)]
pub(crate) struct VersionProbe {
    pub v: u32,
}

/// Milliseconds since the Unix epoch; a clock before 1970 reads as 0, since
/// the value is never used for ordering.
pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_is_one_flat_json_object() {
        let record = Record {
            v: FORMAT_VERSION,
            seq: 3,
            at_ms: 9,
            body: Body::Event { event: 5u8 },
        };
        let line = serde_json::to_string(&record).unwrap();
        assert_eq!(
            line,
            r#"{"v":1,"seq":3,"at_ms":9,"kind":"event","event":5}"#
        );
        assert_eq!(serde_json::from_str::<Record<u8>>(&line).unwrap(), record);
    }

    #[test]
    fn an_opened_record_round_trips() {
        let line = r#"{"v":1,"seq":1,"at_ms":0,"kind":"opened","epoch":2,"build":"b"}"#;
        let record: Record<u8> = serde_json::from_str(line).unwrap();
        assert_eq!(
            record.body,
            Body::Opened {
                epoch: 2,
                build: "b".into()
            }
        );
    }
}
