use super::*;

#[test]
fn solicited_delivery_bypasses_presence_and_other_delivery_preserves_it() {
    for (solicited, away, expected) in [
        (true, false, Delivery::Explicit),
        (true, true, Delivery::Explicit),
        (false, false, Delivery::Presence { away: false }),
        (false, true, Delivery::Presence { away: true }),
    ] {
        assert_eq!(remote_delivery(solicited, away), expected);
    }
}
