//! A node's identifier as written in the flow file.
//!
//! A type rather than a `String` because a flow id is not interchangeable
//! with an arbitrary string, and because the host draws the same graph with a
//! second kind of node id — the canvas's — that this must not be mistaken
//! for. The direction of a dependency is the one thing a graph editor cannot
//! get wrong quietly, and both ends of it are named by this.
//!
//! **The wire form is the bare string.** Ids are already on disk in two
//! formats, and both have to keep reading: the progress journal lists what
//! passed (NDJSON), and a flow file names every node and its `deps` (YAML) —
//! the run's own `run.yaml` included, which is what a resume loads.
//! `record.rs` is deliberately not in that list: it carries ids too, but only
//! in memory, and reaches disk as rendered markdown.
//!
//! A one-field newtype already serialises as its inner value, so the form is
//! what it was without asking. `#[serde(transparent)]` is here because that
//! default runs through `serialize_newtype_struct`, which is a hook a format
//! is free to answer differently; transparent skips it and writes the string.
//! The tests below are what actually hold the format, either way.
//!
//! Construction is infallible. Whether an id is *usable* — unique, non-empty,
//! safe as a filename — is a question about a whole flow rather than one
//! string, and `validate` answers it where the rest of the flow is in view.

use std::borrow::Borrow;
use std::fmt::{self, Display};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(String);

impl NodeId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

impl From<String> for NodeId {
    fn from(id: String) -> Self {
        Self(id)
    }
}

impl From<&str> for NodeId {
    fn from(id: &str) -> Self {
        Self(id.to_string())
    }
}

impl AsRef<str> for NodeId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Lets a map keyed by id still be asked with a `&str`. Sound because the
/// wrapper's `Hash` and `Eq` are the string's own.
impl Borrow<str> for NodeId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Comparison only — an id cannot be *assigned* from a string without saying
/// so, which is what the type is for. This is what lets an assertion name the
/// id it expects as a literal, including `Vec<NodeId> == [&str; N]`.
///
/// The cost is that a second `PartialEq` impl makes `"design".into()`
/// ambiguous in a comparison, so an expected id is spelled `NodeId::from`
/// there. Measured both ways: that is a handful of sites against a couple of
/// dozen `as_str()` calls, several of them a whole mapped collect.
impl PartialEq<str> for NodeId {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for NodeId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// The file format, pinned. Written as a literal rather than a round-trip
    /// through the type so that it keeps saying what the bytes are even if the
    /// type changes shape again.
    #[test]
    fn an_id_is_the_bare_string_it_was() {
        assert_eq!(
            serde_json::to_string(&NodeId::from("design")).unwrap(),
            "\"design\""
        );
        assert_eq!(
            serde_json::from_str::<NodeId>("\"design\"").unwrap(),
            NodeId::from("design")
        );
    }

    /// The other format an id is written in, and the one a person reads:
    /// `run.yaml` is the run's own spec, and a node named `{0: design}` there
    /// would be unreadable long before it stopped loading. Asserted directly
    /// rather than left to `resolve`'s round-trip, which would also pass on a
    /// shape that was wrong symmetrically.
    #[test]
    fn an_id_is_a_bare_scalar_in_yaml_too() {
        assert_eq!(
            yaml_serde::to_string(&NodeId::from("design"))
                .unwrap()
                .trim(),
            "design"
        );
        assert_eq!(
            yaml_serde::from_str::<NodeId>("design").unwrap(),
            NodeId::from("design")
        );
    }

    /// A record or journal line holds ids inside a list, which is where a
    /// non-transparent wrapper would show up as `[{"0":"design"}]`.
    #[test]
    fn a_list_of_ids_is_a_list_of_strings() {
        let ids = vec![NodeId::from("design"), NodeId::from("build")];
        assert_eq!(
            serde_json::to_string(&ids).unwrap(),
            "[\"design\",\"build\"]"
        );
    }

    #[test]
    fn a_map_of_ids_answers_a_str() {
        let m = HashMap::from([(NodeId::from("design"), 1)]);
        assert_eq!(m.get("design"), Some(&1));
        assert_eq!(m.get("build"), None);
    }

    #[test]
    fn an_id_says_itself() {
        let id = NodeId::from("design");
        assert_eq!(id.to_string(), "design");
        assert_eq!(id.as_str(), "design");
        assert!(id == "design", "an assertion can name the id it expects");
        assert!(id != "build");
    }
}
