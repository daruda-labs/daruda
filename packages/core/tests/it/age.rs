//! Public API tests for age.

use daruda_core::age::*;

#[test]
fn bucket_boundaries_round_down_to_whole_units() {
    assert_eq!(bucket(0), AgeBucket::JustNow);
    assert_eq!(bucket(59), AgeBucket::JustNow);
    assert_eq!(bucket(60), AgeBucket::Minutes(1));
    assert_eq!(bucket(119), AgeBucket::Minutes(1));
    assert_eq!(bucket(3_599), AgeBucket::Minutes(59));
    assert_eq!(bucket(3_600), AgeBucket::Hours(1));
    assert_eq!(bucket(23 * 3_600), AgeBucket::Hours(23));
    assert_eq!(bucket(86_399), AgeBucket::Hours(23));
    assert_eq!(bucket(86_400), AgeBucket::Days(1));
    assert_eq!(bucket(3 * 86_400), AgeBucket::Days(3));
    assert_eq!(bucket(u64::MAX), AgeBucket::Days(u64::MAX / 86_400));
}
