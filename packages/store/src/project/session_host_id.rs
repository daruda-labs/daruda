//! `SessionHostId` — stable identifier for one entry in the session host
//! registry (`daruda_config::SessionHostEntry`).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Identifies a registry entry a [`super::LaneSessionHost::Ssh`] /
/// [`super::LaneSessionHost::Docker`] can reference via its `registry_id`
/// field, so renaming or re-pointing the catalog entry doesn't require
/// touching every lane that picked it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionHostId(pub Uuid);

impl SessionHostId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
    pub fn as_inner(&self) -> &Uuid {
        &self.0
    }
}

impl Default for SessionHostId {
    /// Returns the nil UUID sentinel (`00000000-...`). Use this when
    /// you need a placeholder; call `SessionHostId::new()` to mint a
    /// real one. Returning a fresh UUID here would be a footgun for
    /// `#[serde(default)]` paths.
    fn default() -> Self {
        Self(Uuid::nil())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
