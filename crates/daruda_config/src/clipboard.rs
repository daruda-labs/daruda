use serde::{Deserialize, Serialize};

/// Clipboard write limits. Applies to OSC 52 and OSC 1337
/// `Copy=` / `CopyToClipboard=…EndCopy` streaming variants.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct ClipboardConfig {
    /// Maximum decoded bytes accepted from a single clipboard-write
    /// sequence. Streaming chunks are accumulated until either
    /// `EndCopy` arrives or the buffer reaches this cap, at which
    /// point the partial payload is discarded.
    pub streaming_max_bytes: usize,
}

const DEFAULT_STREAMING_MAX_BYTES: usize = 10 * 1024 * 1024;

impl Default for ClipboardConfig {
    fn default() -> Self {
        Self {
            streaming_max_bytes: DEFAULT_STREAMING_MAX_BYTES,
        }
    }
}
