use super::{in_zone, local_month_day_time};
use chrono::{DateTime, FixedOffset};

fn kst() -> FixedOffset {
    FixedOffset::east_opt(9 * 3600).expect("valid offset")
}

fn at(iso: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(iso).expect("valid rfc3339")
}

/// The default locale is en, which reads on a 12-hour clock.
#[test]
fn en_renders_a_us_wall_clock() {
    assert_eq!(
        in_zone(at("2026-07-01T14:32:05Z"), &kst()),
        "Jul 1, 11:32 PM"
    );
}

/// Every shipped locale must render the shape. A typo in a pattern would
/// otherwise panic inside `DelayedFormat`'s `Display` at paint time, and only
/// for the locale that carries it.
#[test]
fn every_locale_renders_the_shape() {
    let when = at("2026-07-01T14:32:05Z").with_timezone(&kst());
    for (locale, expected) in [("en", "Jul 1, 11:32 PM"), ("ko", "7월 1일 23:32")] {
        let pattern = rust_i18n::t!("timestamp.month_day_and_time", locale = locale);
        assert_eq!(
            when.format(&pattern).to_string(),
            expected,
            "locale {locale}"
        );
    }
}

/// A timestamp whose year the UI implies must not repeat it. Asserted through
/// the year rather than an exact string, so the machine's zone cannot decide
/// the outcome.
#[test]
fn the_year_is_left_out() {
    assert!(!local_month_day_time(at("2026-07-01T14:32:05Z")).contains("2026"));
}
