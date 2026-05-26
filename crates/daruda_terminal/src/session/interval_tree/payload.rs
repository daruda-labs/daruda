//! Payload types stored in the interval tree.
//!
//! Each variant of [`MarkPayload`] represents a distinct kind of mark
//! (annotation, prompt region, search hit, ...). Only the `Annotation`
//! variant ships in SP-1 — future variants extend this enum.
//!
//! The enum uses serde's adjacently-tagged representation
//! (`#[serde(tag = "kind", content = "data")]`) so the on-disk NDJSON form
//! is `{"kind": "annotation", "data": {...}}`, matching the schema planned
//! for Task 3 persistence.

use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// Stable `"kind"` tag identifying the `Annotation` variant in NDJSON and
/// any other discriminator context. Kept as a single source of truth so
/// future call sites (Task 3 persistence, diagnostics) cannot drift from
/// the value used by `kind_tag`. The `#[serde(rename = ...)]` attribute
/// below still uses the bare literal because Rust attributes do not
/// accept `const` expressions.
pub(crate) const KIND_ANNOTATION: &str = "annotation";

/// Payload for user-authored annotations attached to a line range.
///
/// `start_col` / `end_col` are optional because annotations may span whole
/// lines (no column resolution) or specific column spans on a single line.
/// `hidden_in_alt_screen` defaults to `true` so notes pinned to the primary
/// screen do not bleed into full-screen apps (vim, less, ...).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnnotationPayload {
    /// Full annotation body, possibly multi-line.
    pub text: String,
    /// First column (1-indexed) the annotation anchors to, or `None` for whole-row marks.
    pub start_col: Option<u16>,
    /// Last column inclusive (1-indexed), or `None` for whole-row marks.
    pub end_col: Option<u16>,
    /// Wall-clock time the annotation was created.
    pub created_at: SystemTime,
    /// Wall-clock time of the last `text` mutation.
    pub updated_at: SystemTime,
    /// When true, the annotation is filtered out by `iter_visible` while the
    /// terminal is in the alternate screen (TUI applications).
    pub hidden_in_alt_screen: bool,
}

impl AnnotationPayload {
    pub fn new(text: String) -> Self {
        let now = SystemTime::now();
        Self {
            text,
            start_col: None,
            end_col: None,
            created_at: now,
            updated_at: now,
            hidden_in_alt_screen: true,
        }
    }

    pub fn touch_updated(&mut self) {
        self.updated_at = SystemTime::now();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data")]
pub enum MarkPayload {
    #[serde(rename = "annotation")]
    Annotation(AnnotationPayload),
}

impl MarkPayload {
    /// Stable identifier used as the `"kind"` field in NDJSON records.
    /// SP-1 only knows `"annotation"`; future variants will extend this.
    pub fn kind_tag(&self) -> &'static str {
        match self {
            MarkPayload::Annotation(_) => KIND_ANNOTATION,
        }
    }

    /// Whether the mark should be visible while the alt screen is active.
    /// For `Annotation`: returns `!hidden_in_alt_screen`.
    pub fn is_visible_in_alt_screen(&self) -> bool {
        match self {
            MarkPayload::Annotation(a) => !a.hidden_in_alt_screen,
        }
    }

    /// Clamp any column metadata to `max_cols`. Returns `true` if anything
    /// changed. `None` column values stay `None` — they are not promoted to
    /// `Some(max_cols)`.
    pub fn clamp_cols(&mut self, max_cols: u16) -> bool {
        match self {
            MarkPayload::Annotation(a) => {
                let mut changed = false;
                if let Some(c) = a.start_col.as_mut()
                    && *c > max_cols
                {
                    *c = max_cols;
                    changed = true;
                }
                if let Some(c) = a.end_col.as_mut()
                    && *c > max_cols
                {
                    *c = max_cols;
                    changed = true;
                }
                changed
            }
        }
    }
}
