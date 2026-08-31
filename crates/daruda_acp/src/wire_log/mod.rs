//! Dev-build ACP wire tap — the two sides of one on-disk format.
//!
//! [`tap`] writes the format from a live session; [`replay`] reads it back into
//! the item list a chat pane would hold. The marker syntax and the sidecar
//! naming below are what the two sides agree on, so they live here rather than
//! in either one — a reader that re-declared them would be a second definition
//! of the same format.
//!
//! Payload text (tool output, diff bodies, terminal dumps) is ~93% of a raw
//! capture and drowns the protocol skeleton, so a string over the spill
//! threshold becomes a preview plus a [`payload_marker`] and its full text
//! moves to a sidecar NDJSON file keyed by that id. The slim log stays valid
//! JSON per line.
//!
//! | Variable | Default | Effect |
//! |---|---|---|
//! | `DARUDA_ACP_WIRE_LOG` | unset (tap off) | slim log path; the app sets it in debug builds |
//! | `DARUDA_ACP_WIRE_LOG_MAX_FIELD` | `512` | spill threshold in bytes; `0` disables elision |
//! | `DARUDA_ACP_WIRE_LOG_PAYLOADS` | on | `0`/`off`/`false` drops payloads instead of writing the sidecar |

use std::path::{Path, PathBuf};

mod replay;
mod tap;

pub use replay::{Replay, ReplayError, replay_log};
pub(crate) use tap::attach;

/// Opening of a spilled-payload marker. Ends with `:`, so the id follows it directly.
const MARKER_PREFIX: &str = "@@acp-payload:";

/// Closing of a spilled-payload marker, and the tail of any elided string.
const MARKER_SUFFIX: &str = "@@";

/// Id written when no sidecar is open, so the marker records that a payload was
/// dropped rather than pointing at a record that does not exist.
const UNRECORDED_ID: &str = "-";

const SIDECAR_EXTENSION: &str = "payload.jsonl";

/// Base file stem the tap falls back to, and the one debug builds configure.
const DEFAULT_BASE_STEM: &str = "acp-wire";

/// The sidecar holding full text for the markers in `slim`.
fn payload_sidecar_path(slim: &Path) -> PathBuf {
    slim.with_extension(SIDECAR_EXTENSION)
}

/// The marker that replaces a spilled string's tail, e.g.
/// `@@acp-payload:12:4096@@`. `id` is [`UNRECORDED_ID`] when no sidecar is open.
fn payload_marker(id: &str, bytes: usize) -> String {
    format!("{MARKER_PREFIX}{id}:{bytes}{MARKER_SUFFIX}")
}

/// The agent id the tap spliced into `path`'s file name, e.g. `codex-acp` for
/// `acp-wire-codex-acp.log`.
///
/// The partial inverse of the tap's naming. It assumes the [`DEFAULT_BASE_STEM`]
/// that debug builds configure, so a capture written under a custom
/// `DARUDA_ACP_WIRE_LOG` base returns `None` rather than a wrong guess — the
/// caller then falls back to whatever it would have used anyway.
pub fn agent_id_from_path(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let id = stem.strip_prefix(DEFAULT_BASE_STEM)?.strip_prefix('-')?;
    (!id.is_empty()).then(|| id.to_owned())
}

/// What an elided string's tail points at. A spilled field always carries one
/// of these two, so "elided" and "resolvable" stay separate questions instead
/// of collapsing into an ambiguous `Option<u64>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Marker {
    /// The payload was written to the sidecar under this id.
    Recorded(u64),
    /// The capture ran with payloads disabled, so the full text is gone.
    Dropped,
}

/// The marker `text` ends with, or `None` when it is not an elided string.
///
/// Matches on the tail rather than the whole string because the marker is
/// appended to a preview of the original text, which may itself contain
/// anything — including something that looks like a marker.
fn marker_of(text: &str) -> Option<Marker> {
    let body = text.strip_suffix(MARKER_SUFFIX)?;
    let (_, marker) = body.rsplit_once(MARKER_PREFIX)?;
    let (id, bytes) = marker.split_once(':')?;
    // A trailing field that is not a byte count means the tail only looked like
    // a marker; refusing here keeps a payload-shaped preview from resolving.
    bytes.parse::<usize>().ok()?;
    if id == UNRECORDED_ID {
        return Some(Marker::Dropped);
    }
    id.parse::<u64>().ok().map(Marker::Recorded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_written_marker_reads_back_as_its_id() {
        let text = format!("preview…{}", payload_marker("12", 4096));
        assert_eq!(marker_of(&text), Some(Marker::Recorded(12)));
    }

    #[test]
    fn an_unrecorded_marker_is_elided_but_unresolvable() {
        // Distinct from "not a marker": the field *was* spilled, so a reader
        // must leave it alone rather than treat the preview as the full text.
        let text = format!("preview…{}", payload_marker(UNRECORDED_ID, 600));
        assert_eq!(marker_of(&text), Some(Marker::Dropped));
    }

    #[test]
    fn plain_text_is_not_a_marker() {
        assert_eq!(marker_of("just a tool title"), None);
        assert_eq!(marker_of("ends in at signs @@"), None);
    }

    #[test]
    fn a_preview_that_looks_like_a_marker_resolves_to_the_real_tail() {
        // The preview is arbitrary captured text, so it can contain the marker
        // syntax itself. The tail is what counts.
        let text = format!(
            "see @@acp-payload:99:1@@ in the docs…{}",
            payload_marker("7", 512)
        );
        assert_eq!(marker_of(&text), Some(Marker::Recorded(7)));
    }

    #[test]
    fn a_marker_with_a_non_numeric_byte_count_is_not_a_marker() {
        assert_eq!(marker_of("x…@@acp-payload:7:many@@"), None);
    }

    #[test]
    fn the_agent_id_round_trips_through_the_tap_naming() {
        // Whatever the tap writes, the reader must be able to name again.
        for id in ["claude", "codex-acp"] {
            let written = super::tap::wire_log_path_for(Path::new("/logs/acp-wire.log"), id);
            assert_eq!(agent_id_from_path(&written).as_deref(), Some(id));
        }
    }

    #[test]
    fn an_unconventional_base_stem_yields_no_guess() {
        assert_eq!(
            agent_id_from_path(Path::new("/logs/my-capture-claude.log")),
            None
        );
        assert_eq!(agent_id_from_path(Path::new("/logs/acp-wire.log")), None);
    }

    #[test]
    fn the_sidecar_sits_beside_the_slim_log() {
        assert_eq!(
            payload_sidecar_path(Path::new("/logs/acp-wire-claude.log")),
            PathBuf::from("/logs/acp-wire-claude.payload.jsonl")
        );
    }
}
