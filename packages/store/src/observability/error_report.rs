//! `ErrorReport` — the unit every surfaced error / panic / failed
//! operation flows through.
//!
//! A report is constructed via [`ErrorReport::new`] (builder shape)
//! or via the [`from_error`](ErrorReport::from_error) /
//! [`from_panic`](ErrorReport::from_panic) helpers. Once built it is
//! cheap to clone and is consumed by:
//!
//! - the toast queue (Layer 1) — surfaces `title` + `message` to the
//!   user with a copy / details affordance,
//! - the details modal (Layer 2) — renders the full body via
//!   [`to_human_text`](ErrorReport::to_human_text),
//! - the on-disk log writer (Layer 3) — appends one NDJSON line via
//!   [`to_ndjson_line`](ErrorReport::to_ndjson_line),
//! - the panic hook — writes [`to_plain_text`](ErrorReport::to_plain_text)
//!   to `panic-<timestamp>.log` for next-launch recovery.
//!
//! Severity drives both the toast colour and the auto-dismiss timer
//! ([`ErrorSeverity::auto_dismiss_after`]).
//!
//! GPUI-free.

use std::collections::BTreeMap;
use std::error::Error;
use std::panic::PanicHookInfo;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::system_info;
use crate::project::ProjectId;

/// Severity buckets driving toast colour, auto-dismiss timer, and log
/// filtering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ErrorSeverity {
    /// No functional impact (an optional watcher could not start, a
    /// deferred reload missed a beat, ...). 5s auto-dismiss.
    Info,
    /// Partial impact (one feature degraded, retryable failure,
    /// transient network error). 8s auto-dismiss.
    Warning,
    /// Core functionality broke (PTY thread died, config could not be
    /// loaded, panic). 30s auto-dismiss. The user can always close
    /// earlier with the toast's ✕ button.
    Error,
}

impl ErrorSeverity {
    /// Auto-dismiss timer for a toast at this severity. Manual ✕
    /// dismiss is always available regardless of severity.
    pub fn auto_dismiss_after(self) -> Duration {
        match self {
            Self::Info => Duration::from_secs(5),
            Self::Warning => Duration::from_secs(8),
            Self::Error => Duration::from_secs(30),
        }
    }

    /// Lowercase ASCII tag for plain-text headers.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

/// User-facing / clipboard-bound error capture. See module doc for the
/// full pipeline.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ErrorReport {
    /// One-line summary — surfaces verbatim in the toast title slot.
    /// Convention: short imperative-past sentence ("PTY writer thread
    /// died", "Config reload failed", ...).
    pub title: String,
    /// Full message — typically the underlying error's `Display`.
    pub message: String,
    /// `std::error::Error::source()` chain flattened from outermost to
    /// innermost. Empty when there is no transitive source.
    pub source_chain: Vec<String>,
    /// Optional call-site location captured at construction. Pair this
    /// with `at(file!(), line!())` on the builder.
    pub location: Option<String>,
    /// Backtrace string, populated for panics or when the caller opts
    /// in via `with_backtrace()`. Plain text — not parsed.
    pub backtrace: Option<String>,
    /// Free-form key/value context. `BTreeMap` so plain-text rendering
    /// is deterministic. Values must already be redacted (`$HOME` →
    /// `~`); see [`system_info::redact_home`].
    pub context: BTreeMap<String, String>,
    /// Wall-clock instant at construction (UTC).
    pub timestamp: DateTime<Utc>,
    /// Stable key used by the toast queue to merge identical errors
    /// into a single entry with a repeat count. `None` opts out of
    /// merging — every push becomes a distinct toast.
    pub dedup_key: Option<String>,
    /// Severity bucket (toast colour + auto-dismiss).
    pub severity: ErrorSeverity,
    /// Project the failure is attributable to, when the workspace has
    /// more than one project loaded. Populated by callers in the app
    /// crate once the multi-project runtime lands; emitted to the
    /// NDJSON log so post-mortems can group errors by project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<ProjectId>,
}

impl ErrorReport {
    /// Start a fresh builder with the given title and a default
    /// severity of [`ErrorSeverity::Error`]. Mutate via the builder
    /// methods, finalize with [`build`](ErrorReportBuilder::build).
    #[allow(clippy::new_ret_no_self)] // Builder pattern entry point.
    pub fn new(title: impl Into<String>) -> ErrorReportBuilder {
        ErrorReportBuilder {
            title: title.into(),
            message: String::new(),
            source_chain: Vec::new(),
            location: None,
            backtrace: None,
            context: BTreeMap::new(),
            dedup_key: None,
            severity: ErrorSeverity::Error,
            project_id: None,
        }
    }

    /// Convenience — wrap an `&dyn Error`, flattening its source chain
    /// into a report titled by the caller. Default severity is
    /// `Error`; downgrade with `.severity(...)` on the builder via
    /// [`new`](Self::new) if needed.
    pub fn from_error<E: Error + ?Sized>(title: impl Into<String>, err: &E) -> Self {
        Self::new(title).from_error(err).build()
    }

    /// Build a report from a panic-hook callback. Captures the payload
    /// (`&str`/`String`), source location, and a forced backtrace. The
    /// resulting report carries severity `Error` and a stable dedup
    /// key (`"panic"`) so repeated panics in a tight loop do not
    /// flood the queue.
    pub fn from_panic(info: &PanicHookInfo<'_>) -> Self {
        let payload = panic_payload_string(info);
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()));
        let backtrace = std::backtrace::Backtrace::force_capture().to_string();

        let mut builder = Self::new("daruda panicked")
            .severity(ErrorSeverity::Error)
            .message(payload)
            .with_backtrace(backtrace)
            .dedup("panic");

        if let Some(loc) = location {
            builder = builder.location(loc);
        }
        builder.build()
    }

    /// Plain-text rendering used for clipboard copy and the modal
    /// body. Stable, deterministic — safe to compare in tests.
    pub fn to_plain_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("[daruda] {}\n", self.title));
        out.push_str(&format!("severity: {}\n", self.severity.as_str()));
        out.push_str(&format!(
            "time:     {}\n",
            self.timestamp.format("%Y-%m-%d %H:%M:%S UTC")
        ));
        if let Some(loc) = &self.location {
            out.push_str(&format!("location: {loc}\n"));
        }

        if !self.message.is_empty() {
            out.push_str("\nmessage:\n");
            for line in self.message.lines() {
                out.push_str(&format!("  {line}\n"));
            }
        }

        if !self.source_chain.is_empty() {
            out.push_str("\nsource chain:\n");
            for cause in &self.source_chain {
                out.push_str(&format!("  └─ {cause}\n"));
            }
        }

        if !self.context.is_empty() {
            out.push_str("\ncontext:\n");
            let label_width = self
                .context
                .keys()
                .map(|k| k.len())
                .max()
                .unwrap_or(0)
                .min(24);
            for (k, v) in &self.context {
                out.push_str(&format!("  {k:<label_width$}  {v}\n"));
            }
        }

        if let Some(bt) = &self.backtrace {
            out.push_str("\nbacktrace:\n");
            for line in bt.lines() {
                out.push_str(&format!("  {line}\n"));
            }
        }

        out.push('\n');
        out.push_str(&format!("[{}]\n", system_info::summary()));
        out
    }

    /// Modal-body rendering. Currently identical to
    /// [`to_plain_text`](Self::to_plain_text) — kept as a separate
    /// entry point so the modal can diverge later (e.g. soft-wrap
    /// hints, link parsing) without touching clipboard output.
    pub fn to_human_text(&self) -> String {
        self.to_plain_text()
    }

    /// One-line NDJSON record for the on-disk log. Surfaced
    /// `Warning` / `Error` entries embed
    /// [`to_plain_text`](Self::to_plain_text) in a `text` field so the
    /// log is greppable without `jq`.
    ///
    /// `Info` entries omit that block — they are high-volume diagnostic
    /// traces (e.g. Claude status transitions) whose `message` +
    /// `context` already carry everything, and the duplicated multi-line
    /// plain-text would otherwise dominate the file.
    pub fn to_ndjson_line(&self) -> String {
        let mut record = serde_json::to_value(self).unwrap_or(serde_json::Value::Null);
        if self.severity != ErrorSeverity::Info
            && let serde_json::Value::Object(map) = &mut record
        {
            map.insert(
                "text".to_string(),
                serde_json::Value::String(self.to_plain_text()),
            );
        }
        let mut out = serde_json::to_string(&record).unwrap_or_else(|_| "{}".to_string());
        out.push('\n');
        out
    }
}

/// Builder for [`ErrorReport`]. Default severity is
/// [`ErrorSeverity::Error`]; call [`severity`](Self::severity) to
/// downgrade.
#[must_use]
pub struct ErrorReportBuilder {
    title: String,
    message: String,
    source_chain: Vec<String>,
    location: Option<String>,
    backtrace: Option<String>,
    context: BTreeMap<String, String>,
    dedup_key: Option<String>,
    severity: ErrorSeverity,
    project_id: Option<ProjectId>,
}

impl ErrorReportBuilder {
    /// Override the message text. Otherwise inherited from
    /// [`from_error`](Self::from_error) when one is supplied.
    pub fn message(mut self, message: impl Into<String>) -> Self {
        self.message = message.into();
        self
    }

    /// Severity bucket. Default `Error`.
    pub fn severity(mut self, severity: ErrorSeverity) -> Self {
        self.severity = severity;
        self
    }

    /// Capture call-site location. Idiomatic call: `.at(file!(), line!())`.
    pub fn at(self, file: &str, line: u32) -> Self {
        self.location(format!("{file}:{line}"))
    }

    /// Set a pre-formatted location string. Prefer
    /// [`at`](Self::at) for `file!()/line!()` capture.
    pub fn location(mut self, location: impl Into<String>) -> Self {
        self.location = Some(location.into());
        self
    }

    /// Attach a backtrace string. Panic hook fills this automatically.
    pub fn with_backtrace(mut self, backtrace: impl Into<String>) -> Self {
        self.backtrace = Some(backtrace.into());
        self
    }

    /// Append a key/value context pair. Multiple calls accumulate;
    /// a repeated key overwrites.
    pub fn with_context(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.context.insert(key.into(), value.into());
        self
    }

    /// Stable dedup key. Toasts sharing this key are merged into one
    /// entry with a repeat count and refreshed expiry.
    pub fn dedup(mut self, key: impl Into<String>) -> Self {
        self.dedup_key = Some(key.into());
        self
    }

    /// Tag the report with the [`ProjectId`] the failure is attributable
    /// to. Only meaningful once the workspace has more than one project
    /// loaded; single-project callers leave this unset.
    pub fn project_id(mut self, id: ProjectId) -> Self {
        self.project_id = Some(id);
        self
    }

    /// Inherit message + source chain from an existing error. The
    /// builder's message slot is overwritten only if currently empty.
    pub fn from_error<E: Error + ?Sized>(mut self, err: &E) -> Self {
        if self.message.is_empty() {
            self.message = err.to_string();
        }
        let mut cause: Option<&dyn Error> = err.source();
        while let Some(c) = cause {
            self.source_chain.push(c.to_string());
            cause = c.source();
        }
        self
    }

    /// Finalize. Stamps the timestamp at this call.
    pub fn build(self) -> ErrorReport {
        ErrorReport {
            title: self.title,
            message: self.message,
            source_chain: self.source_chain,
            location: self.location,
            backtrace: self.backtrace,
            context: self.context,
            timestamp: Utc::now(),
            dedup_key: self.dedup_key,
            severity: self.severity,
            project_id: self.project_id,
        }
    }
}

fn panic_payload_string(info: &PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}
