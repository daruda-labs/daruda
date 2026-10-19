//! One line of a run log: a versioned envelope around an `opened` marker or
//! a machine event, serialized as a single JSON object per line.

use serde::{Deserialize, Serialize};

use crate::error::{OpenError, Versioned};

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
    /// `schema` is the writer's [`Machine::SCHEMA`](crate::Machine::SCHEMA):
    /// every event after it, up to the next `opened`, is in that schema.
    Opened {
        epoch: u64,
        build: String,
        #[serde(default = "first_schema")]
        schema: u32,
    },
    Event {
        event: E,
    },
}

/// A record or snapshot written before machines declared a schema carries
/// none; those writers all had the first one.
fn first_schema() -> u32 {
    1
}

/// Just enough of a line or snapshot to read its versions before trusting
/// its shape. Only `opened` records and snapshots carry a schema.
#[derive(Deserialize)]
pub(crate) struct VersionProbe {
    v: u32,
    #[serde(default)]
    schema: Option<u32>,
}

impl VersionProbe {
    /// Refuse what a newer build wrote, envelope first: a newer envelope may
    /// not even carry the schema where this build looks for it.
    pub(crate) fn admit(&self, schema: u32) -> Result<(), OpenError> {
        if self.v > FORMAT_VERSION {
            return Err(OpenError::TooNew {
                what: Versioned::Format,
                found: self.v,
                supported: FORMAT_VERSION,
            });
        }
        match self.schema {
            Some(found) if found > schema => Err(OpenError::TooNew {
                what: Versioned::Schema,
                found,
                supported: schema,
            }),
            _ => Ok(()),
        }
    }
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
                build: "b".into(),
                schema: 1,
            }
        );
    }

    #[test]
    fn a_newer_envelope_is_refused_before_its_schema_is_read() {
        let probe: VersionProbe = serde_json::from_str(r#"{"v":9,"schema":9}"#).unwrap();
        assert!(matches!(
            probe.admit(9),
            Err(OpenError::TooNew {
                what: Versioned::Format,
                found: 9,
                ..
            })
        ));
        let probe: VersionProbe = serde_json::from_str(r#"{"v":1,"schema":3}"#).unwrap();
        assert!(matches!(
            probe.admit(2),
            Err(OpenError::TooNew {
                what: Versioned::Schema,
                found: 3,
                supported: 2
            })
        ));
        assert!(probe.admit(3).is_ok());
        let event: VersionProbe = serde_json::from_str(r#"{"v":1,"event":7}"#).unwrap();
        assert!(event.admit(1).is_ok());
    }
}
