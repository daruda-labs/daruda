use super::*;
use daruda_runlog::testing::{assert_machine_laws, enumerate_crashes};

const FIRST: &str = "0123456789abcdef0123456789abcdef";
const SECOND: &str = "fedcba9876543210fedcba9876543210";

#[test]
fn source_capture_obeys_machine_laws_and_survives_all_crash_points() {
    let states = [
        Attribution::default(),
        Attribution {
            captured: Some(Captured {
                bot: FIRST.into(),
                next_offset: 42,
            }),
        },
    ];
    let events = vec![
        SourceEvent::Captured {
            bot: FIRST.into(),
            next_offset: 42,
        },
        SourceEvent::Captured {
            bot: SECOND.into(),
            next_offset: 0,
        },
        SourceEvent::Captured {
            bot: FIRST.into(),
            next_offset: -1,
        },
        SourceEvent::Captured {
            bot: "not-a-digest".into(),
            next_offset: 0,
        },
    ];
    assert_machine_laws::<SourceMachine>(&states, &events);
    assert!(enumerate_crashes::<SourceMachine>(events).cases > 0);
}

#[test]
fn capture_before_inbox_initialization_never_moves_to_a_later_bot() {
    let data = tempfile::tempdir().unwrap();
    let file = crate::telegram::telegram_state_path_in(data.path());
    std::fs::write(&file, b"{\"update_offset\":42}").unwrap();
    let first = telegram_for_bot(data.path(), FIRST, "test").unwrap();
    assert_eq!(first.monotonic[&ChannelKey::new("telegram")], 41);
    std::fs::write(&file, b"{\"update_offset\":999}").unwrap();
    assert!(
        telegram_for_bot(data.path(), SECOND, "test")
            .unwrap()
            .monotonic
            .is_empty()
    );
    let restarted = telegram_for_bot(data.path(), FIRST, "test").unwrap();
    assert_eq!(restarted, first);
}

#[test]
fn consuming_a_missing_or_zero_source_prevents_later_reimport() {
    for present in [false, true] {
        let data = tempfile::tempdir().unwrap();
        let file = crate::telegram::telegram_state_path_in(data.path());
        if present {
            std::fs::write(&file, b"{\"update_offset\":0}").unwrap();
        }
        let first = telegram_for_bot(data.path(), FIRST, "test").unwrap();
        assert_eq!(first.monotonic[&ChannelKey::new("telegram")], -1);
        std::fs::write(&file, b"{\"update_offset\":999}").unwrap();
        assert!(
            telegram_for_bot(data.path(), SECOND, "test")
                .unwrap()
                .monotonic
                .is_empty()
        );
        assert_eq!(telegram_for_bot(data.path(), FIRST, "test").unwrap(), first);
    }
}
