//! `SessionHostId` serialization and defaults.

use daruda_store::project::session_host_id::*;
use uuid::Uuid;

#[test]
fn round_trips_through_json() {
    let id = SessionHostId::new();
    let json = serde_json::to_string(&id).expect("serialize");
    let back: SessionHostId = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, id);
}

#[test]
fn default_is_the_nil_sentinel() {
    assert_eq!(SessionHostId::default(), SessionHostId(Uuid::nil()));
}

#[test]
fn new_mints_a_non_nil_id() {
    assert_ne!(SessionHostId::new(), SessionHostId::default());
}
