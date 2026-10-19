//! Strict reading of one generation's log on open: versions first, then
//! every record in seq order. Only a torn last line is tolerated.

use crate::error::OpenError;
use crate::machine::Machine;
use crate::record::{Body, Record, VersionProbe};
use crate::snapshot::Base;

/// The log replayed onto its generation's base.
pub(crate) struct Replayed<S> {
    pub state: S,
    pub last_seq: u64,
    pub epoch: u64,
    pub records: u64,
    /// Bytes of whole records; anything after is a torn tail to cut.
    pub whole_len: u64,
    pub torn: bool,
}

/// Lines with their start offsets, and whether the final one lacks its
/// newline.
fn split_lines(log: &[u8]) -> (Vec<(usize, &[u8])>, Option<usize>) {
    let mut lines = Vec::new();
    let mut start = 0;
    while start < log.len() {
        match log[start..].iter().position(|&byte| byte == b'\n') {
            Some(end) => {
                lines.push((start, &log[start..start + end]));
                start += end + 1;
            }
            None => return (lines, Some(start)),
        }
    }
    (lines, None)
}

/// Refuse a log any of whose lines a newer build wrote: its envelope, or
/// an `opened` record naming a newer machine schema. Read before any line
/// is trusted or anything is cut, so a newer log is never called corrupt.
pub(crate) fn admit<M: Machine>(log: &[u8]) -> Result<(), OpenError> {
    let (lines, fragment) = split_lines(log);
    let fragment_line = fragment.map(|start| &log[start..]);
    for line in lines.iter().map(|(_, line)| *line).chain(fragment_line) {
        if let Ok(probe) = serde_json::from_slice::<VersionProbe>(line) {
            probe.admit(M::SCHEMA)?;
        }
    }
    Ok(())
}

/// Replay `log` onto `base`, handing each whole record to `seen` in order.
pub(crate) fn replay<M: Machine>(
    log: &[u8],
    base: Base<M::State>,
    mut seen: impl FnMut(Record<M::Event>),
) -> Result<Replayed<M::State>, OpenError> {
    admit::<M>(log)?;
    let (lines, fragment) = split_lines(log);

    let mut replayed = Replayed {
        state: base.state,
        last_seq: base.seq,
        epoch: base.epoch,
        records: 0,
        whole_len: log.len() as u64,
        torn: false,
    };
    if let Some(start) = fragment {
        replayed.whole_len = start as u64;
        replayed.torn = true;
    }

    for (index, &(start, line)) in lines.iter().enumerate() {
        let number = index as u64 + 1;
        let is_last = index + 1 == lines.len() && fragment.is_none();
        let record = match serde_json::from_slice::<Record<serde_json::Value>>(line) {
            Ok(record) => record,
            Err(_) if is_last => {
                replayed.whole_len = start as u64;
                replayed.torn = true;
                break;
            }
            Err(_) => return Err(OpenError::Corrupt { line: number }),
        };
        if record.seq != replayed.last_seq + 1 {
            return Err(OpenError::Corrupt { line: number });
        }
        let body = match record.body {
            Body::Opened {
                epoch,
                build,
                schema,
            } => {
                replayed.epoch = replayed.epoch.max(epoch);
                Body::Opened {
                    epoch,
                    build,
                    schema,
                }
            }
            Body::Event { event } => {
                let event: M::Event = serde_json::from_value(event)
                    .map_err(|_| OpenError::Corrupt { line: number })?;
                check_replayed::<M>(&replayed.state, &event, number);
                M::apply(&mut replayed.state, &event);
                Body::Event { event }
            }
        };
        replayed.last_seq = record.seq;
        replayed.records += 1;
        seen(Record {
            v: record.v,
            seq: record.seq,
            at_ms: record.at_ms,
            body,
        });
    }
    Ok(replayed)
}

/// Records were checked when written; a test build re-checks them so a
/// machine whose `check` and `apply` disagree fails loudly.
#[cfg(any(test, feature = "test-support"))]
fn check_replayed<M: Machine>(state: &M::State, event: &M::Event, line: u64) {
    if let Err(rejected) = M::check(state, event) {
        panic!("replayed record at line {line} fails its own precondition: {rejected}");
    }
}

#[cfg(not(any(test, feature = "test-support")))]
fn check_replayed<M: Machine>(_: &M::State, _: &M::Event, _: u64) {}

#[cfg(test)]
mod tests {
    use super::*;

    struct Sum;

    impl Machine for Sum {
        type State = u64;
        type Event = u64;
        type Rejected = std::fmt::Error;

        fn check(_: &u64, event: &u64) -> Result<(), std::fmt::Error> {
            if *event == 0 {
                Err(std::fmt::Error)
            } else {
                Ok(())
            }
        }

        fn apply(state: &mut u64, event: &u64) {
            *state += event;
        }
    }

    fn line(seq: u64, event: u64) -> String {
        format!("{{\"v\":1,\"seq\":{seq},\"at_ms\":0,\"kind\":\"event\",\"event\":{event}}}\n")
    }

    fn base() -> Base<u64> {
        Base {
            state: 0,
            seq: 0,
            epoch: 0,
        }
    }

    #[test]
    fn lines_split_at_newlines_and_report_a_fragment() {
        let (lines, fragment) = split_lines(b"a\nbc\nd");
        assert_eq!(lines, vec![(0, &b"a"[..]), (2, &b"bc"[..])]);
        assert_eq!(fragment, Some(5));
        assert_eq!(split_lines(b"a\n").1, None);
    }

    #[test]
    fn whole_records_replay_in_order() {
        let log = line(1, 2) + &line(2, 3);
        let replayed = replay::<Sum>(log.as_bytes(), base(), drop).unwrap();
        assert_eq!(
            (replayed.state, replayed.last_seq, replayed.torn),
            (5, 2, false)
        );
        assert_eq!(replayed.whole_len, log.len() as u64);
    }

    #[test]
    fn a_seq_gap_is_corrupt() {
        let log = line(1, 2) + &line(3, 3);
        assert!(matches!(
            replay::<Sum>(log.as_bytes(), base(), drop),
            Err(OpenError::Corrupt { line: 2 })
        ));
    }

    #[test]
    fn an_unparseable_last_line_is_torn() {
        let log = line(1, 2) + "garbage\n";
        let replayed = replay::<Sum>(log.as_bytes(), base(), drop).unwrap();
        assert!(replayed.torn);
        assert_eq!(replayed.whole_len, line(1, 2).len() as u64);
    }

    /// An event this build cannot parse, after an `opened` naming a newer
    /// schema, is a newer log, not a corrupt one.
    #[test]
    fn a_newer_schema_is_too_new_not_corrupt() {
        let opened = "{\"v\":1,\"seq\":1,\"at_ms\":0,\"kind\":\"opened\",\"epoch\":1,\"build\":\"b\",\"schema\":2}\n";
        let log = opened.to_owned()
            + "{\"v\":1,\"seq\":2,\"at_ms\":0,\"kind\":\"event\",\"event\":\"new\"}\n";
        assert!(matches!(
            replay::<Sum>(log.as_bytes(), base(), drop),
            Err(OpenError::TooNew {
                what: crate::error::Versioned::Schema,
                found: 2,
                supported: 1
            })
        ));
    }

    #[test]
    fn whole_records_are_handed_over_in_order() {
        let log = line(1, 2) + &line(2, 3) + "{\"v\":1,\"seq\":3";
        let mut seqs = Vec::new();
        let replayed =
            replay::<Sum>(log.as_bytes(), base(), |record| seqs.push(record.seq)).unwrap();
        assert_eq!(seqs, vec![1, 2]);
        assert!(replayed.torn);
    }

    #[test]
    #[should_panic(expected = "fails its own precondition")]
    fn a_replayed_record_that_fails_check_panics_in_tests() {
        let _ = replay::<Sum>(line(1, 0).as_bytes(), base(), drop);
    }
}
