use super::{Shape, in_zone, local_datetime, local_month_day_time, now_rfc3339, rfc3339_in_zone};
use chrono::{DateTime, FixedOffset};

fn kst() -> FixedOffset {
    FixedOffset::east_opt(9 * 3600).expect("valid offset")
}

fn utc() -> FixedOffset {
    FixedOffset::east_opt(0).expect("valid offset")
}

fn at(iso: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(iso).expect("valid rfc3339")
}

/// The default locale is en, which reads on a 12-hour clock.
#[test]
fn en_renders_a_us_wall_clock() {
    let when = at("2026-07-01T14:32:05Z");
    assert_eq!(
        in_zone(when, &kst(), Shape::DateAndTime),
        "Jul 1, 2026, 11:32 PM"
    );
    assert_eq!(
        in_zone(when, &kst(), Shape::MonthDayAndTime),
        "Jul 1, 11:32 PM"
    );
}

/// Midnight and afternoon are where a 12-hour clock goes wrong: hour 0 reads
/// as 12 AM, and neither hour carries a leading zero.
#[test]
fn en_reads_midnight_and_afternoon_without_padding() {
    assert_eq!(
        in_zone(at("2026-12-05T00:07:00Z"), &utc(), Shape::DateAndTime),
        "Dec 5, 2026, 12:07 AM"
    );
    assert_eq!(
        in_zone(at("2026-12-05T13:05:00Z"), &utc(), Shape::DateAndTime),
        "Dec 5, 2026, 1:05 PM"
    );
}

/// Every shipped locale must render every shape. A typo in a pattern would
/// otherwise panic inside `DelayedFormat`'s `Display` at paint time, and only
/// for the locale that carries it.
#[test]
fn every_locale_renders_every_shape() {
    let when = at("2026-07-01T14:32:05Z").with_timezone(&kst());
    for (locale, date_and_time, month_day_and_time) in [
        ("en", "Jul 1, 2026, 11:32 PM", "Jul 1, 11:32 PM"),
        ("ko", "2026년 7월 1일 23:32", "7월 1일 23:32"),
    ] {
        for (key, expected) in [
            ("timestamp.date_and_time", date_and_time),
            ("timestamp.month_day_and_time", month_day_and_time),
        ] {
            let pattern = rust_i18n::t!(key, locale = locale);
            assert_eq!(
                when.format(&pattern).to_string(),
                expected,
                "locale {locale} key {key} pattern {pattern:?}"
            );
        }
    }
}

/// Two spellings of one instant must render identically — the offset in the
/// text is information about the input, not about what the user should read.
#[test]
fn the_spelling_of_an_instant_does_not_survive_into_the_output() {
    assert_eq!(
        rfc3339_in_zone("2026-12-25T09:00:00+09:00", &utc(), Shape::DateAndTime),
        rfc3339_in_zone("2026-12-25T00:00:00Z", &utc(), Shape::DateAndTime),
    );
}

#[test]
fn an_unparseable_timestamp_yields_nothing() {
    for input in ["not-a-timestamp", "2026-07-01", ""] {
        assert_eq!(rfc3339_in_zone(input, &utc(), Shape::DateAndTime), None);
    }
}

#[test]
fn a_parseable_timestamp_is_converted_into_the_target_zone() {
    assert_eq!(
        rfc3339_in_zone("2026-07-01T14:32:05.123Z", &kst(), Shape::DateAndTime),
        Some("Jul 1, 2026, 11:32 PM".to_owned())
    );
}

/// The producer and the consumer of a stored timestamp have to agree on the
/// format. Nothing else in the codebase reads this field, so a drift between
/// them would only surface as a tooltip that quietly stopped appearing.
#[test]
fn a_stamped_timestamp_reads_back() {
    let stamped = now_rfc3339();
    assert!(
        local_datetime(&stamped).is_some(),
        "stamped {stamped:?} did not parse back"
    );
}

/// The two public shapes differ by exactly one field, so the wrappers are only
/// correct if each reaches for the right one. Asserted through the year rather
/// than an exact string, so the machine's zone cannot decide the outcome.
#[test]
fn each_wrapper_reaches_for_its_own_shape() {
    let iso = "2026-07-01T14:32:05Z";
    let at = at(iso);
    assert!(
        local_datetime(iso).expect("parses").contains("2026"),
        "a standalone timestamp needs its year"
    );
    assert!(
        !local_month_day_time(at).contains("2026"),
        "a timestamp whose year the UI implies must not repeat it"
    );
}
