//! Dev-build ACP wire tap — raw JSON-RPC traffic written to a file for
//! off-line inspection.
//!
//! Payload text (tool output, diff bodies, terminal dumps) is ~93% of a raw
//! capture and drowns the protocol skeleton, so a string over
//! [`DEFAULT_MAX_FIELD`] bytes becomes a preview plus an
//! `@@acp-payload:<id>:<bytes>@@` marker and its full text moves to a sidecar
//! NDJSON file keyed by that id. The slim log stays valid JSON per line.
//!
//! | Variable | Default | Effect |
//! |---|---|---|
//! | `DARUDA_ACP_WIRE_LOG` | unset (tap off) | slim log path; the app sets it in debug builds |
//! | `DARUDA_ACP_WIRE_LOG_MAX_FIELD` | `512` | spill threshold in bytes; `0` disables elision |
//! | `DARUDA_ACP_WIRE_LOG_PAYLOADS` | on | `0`/`off`/`false` drops payloads instead of writing the sidecar |

use std::ffi::OsStr;
use std::fmt::Write as _;
use std::fs::File;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use agent_client_protocol::{AcpAgent, LineDirection};
use serde_json::Value;

/// Spill threshold. Chosen from a real capture: keeps every protocol field
/// (ids, statuses, titles, message chunks) inline while catching the fat
/// payload fields.
const DEFAULT_MAX_FIELD: usize = 512;

/// How much of a spilled string stays inline, so the slim log still shows what
/// the payload was.
const PREVIEW_BYTES: usize = 96;

const SIDECAR_EXTENSION: &str = "payload.jsonl";

/// `Spill::path` for a non-JSON line (adapter stderr) spilled whole.
const WHOLE_LINE_PATH: &str = "$line";

/// Process-global so a marker id is unambiguous across concurrent sidecars.
static PAYLOAD_IDS: AtomicU64 = AtomicU64::new(1);

/// Attach the wire tap to `agent`, or return it unchanged when
/// `DARUDA_ACP_WIRE_LOG` is unset or the file can't be opened — so a shipping
/// build never touches the wire unless explicitly asked.
///
/// `agent_id` (the catalog id, empty for the crate's own examples) is spliced
/// into the file name so concurrent sessions from different agents land in
/// separate files instead of interleaving in one.
pub(crate) fn attach(agent: AcpAgent, agent_id: &str) -> AcpAgent {
    let Some(base) = std::env::var_os("DARUDA_ACP_WIRE_LOG") else {
        return agent;
    };
    let path = wire_log_path_for(Path::new(&base), agent_id);
    let Some(slim) = open_log(&path) else {
        return agent;
    };
    let cap = max_field_cap(std::env::var_os("DARUDA_ACP_WIRE_LOG_MAX_FIELD").as_deref());
    let sidecar = (cap > 0
        && sidecar_enabled(std::env::var_os("DARUDA_ACP_WIRE_LOG_PAYLOADS").as_deref()))
    .then(|| open_log(&payload_sidecar_path(&path)))
    .flatten();

    let tap = Arc::new(WireTap { slim, sidecar, cap });
    agent.with_debug(move |line, direction| tap.write(line, direction))
}

struct WireTap {
    slim: Mutex<File>,
    sidecar: Option<Mutex<File>>,
    /// Spill threshold in bytes; `0` writes raw lines with no elision.
    cap: usize,
}

impl WireTap {
    fn write(&self, line: &str, direction: LineDirection) {
        let marker = match direction {
            LineDirection::Stdin => "-> stdin ",
            LineDirection::Stdout => "<- stdout",
            LineDirection::Stderr => "!! stderr",
        };
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);

        let mut elider = Elider {
            cap: self.cap,
            record: self.sidecar.is_some(),
            spills: Vec::new(),
        };
        let slim_line = elider.slim(line);
        // Payloads first, so a marker in the slim log always resolves.
        if let Some(sidecar) = &self.sidecar
            && !elider.spills.is_empty()
            && let Ok(mut f) = sidecar.lock()
        {
            for spill in &elider.spills {
                let record = serde_json::json!({
                    "id": spill.id,
                    "ts": ts,
                    "dir": marker.trim(),
                    "path": spill.path,
                    "bytes": spill.text.len(),
                    "text": spill.text,
                });
                let _ = writeln!(f, "{record}");
            }
        }
        if let Ok(mut f) = self.slim.lock() {
            let _ = writeln!(f, "{ts} {marker} {slim_line}");
        }
    }
}

struct Spill {
    id: u64,
    path: String,
    text: String,
}

/// Rewrites one wire line into its slim form, collecting what it lifted out.
struct Elider {
    cap: usize,
    /// Whether a sidecar is open. When false, payloads are dropped and markers
    /// carry `-` instead of an id.
    record: bool,
    spills: Vec<Spill>,
}

impl Elider {
    fn slim(&mut self, line: &str) -> String {
        if self.cap == 0 {
            return line.to_string();
        }
        match serde_json::from_str::<Value>(line) {
            Ok(mut value) => {
                let mut path = String::new();
                self.walk(&mut value, &mut path);
                value.to_string()
            }
            // Not JSON — adapter stderr or protocol noise. Verbatim unless it's
            // a dump, in which case spill it whole.
            Err(_) if line.len() <= self.cap => line.to_string(),
            Err(_) => self.elide(WHOLE_LINE_PATH, line),
        }
    }

    fn walk(&mut self, value: &mut Value, path: &mut String) {
        match value {
            Value::String(text) if text.len() > self.cap => {
                *text = self.elide(path, text);
            }
            Value::Array(items) => {
                for (index, item) in items.iter_mut().enumerate() {
                    let parent = path.len();
                    let _ = write!(path, "[{index}]");
                    self.walk(item, path);
                    path.truncate(parent);
                }
            }
            Value::Object(fields) => {
                for (key, field) in fields.iter_mut() {
                    let parent = path.len();
                    if !path.is_empty() {
                        path.push('.');
                    }
                    path.push_str(key);
                    self.walk(field, path);
                    path.truncate(parent);
                }
            }
            _ => {}
        }
    }

    /// Replace `text` with `<preview>…@@acp-payload:<id>:<bytes>@@`, recording
    /// the full text for the sidecar when one is open.
    fn elide(&mut self, path: &str, text: &str) -> String {
        let bytes = text.len();
        let id = if self.record {
            let id = PAYLOAD_IDS.fetch_add(1, Ordering::Relaxed);
            self.spills.push(Spill {
                id,
                path: path.to_string(),
                text: text.to_string(),
            });
            id.to_string()
        } else {
            "-".to_string()
        };
        let preview = &text[..floor_char_boundary(text, PREVIEW_BYTES.min(self.cap))];
        format!("{preview}…@@acp-payload:{id}:{bytes}@@")
    }
}

/// Largest index `<= max` that starts a UTF-8 character, so a preview never
/// splits a multi-byte character (which would panic the slice).
fn floor_char_boundary(text: &str, max: usize) -> usize {
    if max >= text.len() {
        return text.len();
    }
    (0..=max)
        .rev()
        .find(|&i| text.is_char_boundary(i))
        .unwrap_or(0)
}

/// An unset or unparseable value falls back to [`DEFAULT_MAX_FIELD`]; an
/// explicit `0` disables elision so the tap writes raw lines as before.
fn max_field_cap(var: Option<&OsStr>) -> usize {
    var.and_then(OsStr::to_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(DEFAULT_MAX_FIELD)
}

/// On unless explicitly opted out — the point of eliding is to keep the payload
/// available elsewhere, not to lose it.
fn sidecar_enabled(var: Option<&OsStr>) -> bool {
    match var.and_then(OsStr::to_str).map(str::trim) {
        Some(v) => !matches!(
            v.to_ascii_lowercase().as_str(),
            "0" | "off" | "false" | "no"
        ),
        None => true,
    }
}

fn payload_sidecar_path(slim: &Path) -> PathBuf {
    slim.with_extension(SIDECAR_EXTENSION)
}

fn open_log(path: &Path) -> Option<Mutex<File>> {
    // `attach` runs per ACP session, not per app launch, so a plain
    // `.truncate(true)` would wipe earlier sessions of the same run. Open once
    // (append) and share the handle; a per-line reopen would thrash under
    // streaming turns.
    let truncate = first_open(path);
    std::fs::OpenOptions::new()
        .create(true)
        .append(!truncate)
        .truncate(truncate)
        .write(truncate)
        .open(path)
        .ok()
        .map(Mutex::new)
}

/// Whether `path` is being opened for the first time in this process — tracked
/// so restart-vs-same-run is decided by process lifetime, not session count.
fn first_open(path: &Path) -> bool {
    static SEEN: std::sync::OnceLock<Mutex<std::collections::HashSet<PathBuf>>> =
        std::sync::OnceLock::new();
    let mut seen = match SEEN
        .get_or_init(|| Mutex::new(std::collections::HashSet::new()))
        .lock()
    {
        Ok(guard) => guard,
        // A poisoned lock only means a prior holder panicked mid-insert; append
        // (don't truncate) so we never wipe an existing run's log on recovery.
        Err(_) => return false,
    };
    seen.insert(path.to_path_buf())
}

/// Splice `-<agent_id>` into `base`'s file name before the extension (e.g.
/// `acp-wire.log` + `"claude"` → `acp-wire-claude.log`). Empty leaves `base`
/// untouched. Non-alphanumeric bytes in `agent_id` (a user-editable catalog
/// field) map to `_` so it can never escape `base`'s directory.
fn wire_log_path_for(base: &Path, agent_id: &str) -> PathBuf {
    if agent_id.is_empty() {
        return base.to_path_buf();
    }
    let safe_id: String = agent_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let stem = base
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("acp-wire");
    let file_name = match base.extension().and_then(|s| s.to_str()) {
        Some(ext) => format!("{stem}-{safe_id}.{ext}"),
        None => format!("{stem}-{safe_id}"),
    };
    match base.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(file_name),
        _ => PathBuf::from(file_name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn elider() -> Elider {
        Elider {
            cap: DEFAULT_MAX_FIELD,
            record: true,
            spills: Vec::new(),
        }
    }

    #[test]
    fn wire_log_path_splices_agent_id_before_the_extension() {
        assert_eq!(
            wire_log_path_for(Path::new("/logs/acp-wire.log"), "claude"),
            PathBuf::from("/logs/acp-wire-claude.log")
        );
    }

    #[test]
    fn wire_log_path_handles_no_extension() {
        assert_eq!(
            wire_log_path_for(Path::new("/logs/acp-wire"), "codex"),
            PathBuf::from("/logs/acp-wire-codex")
        );
    }

    #[test]
    fn wire_log_path_leaves_base_untouched_when_agent_id_is_empty() {
        assert_eq!(
            wire_log_path_for(Path::new("/logs/acp-wire.log"), ""),
            PathBuf::from("/logs/acp-wire.log")
        );
    }

    #[test]
    fn first_open_truncates_once_then_appends() {
        let path_a = PathBuf::from("/logs/acp-wire-first-open-test-a.log");
        let path_b = PathBuf::from("/logs/acp-wire-first-open-test-b.log");
        assert!(first_open(&path_a), "first open truncates");
        assert!(!first_open(&path_a), "second open appends");
        assert!(!first_open(&path_a), "third open still appends");
        assert!(first_open(&path_b), "a different path truncates once");
    }

    #[test]
    fn wire_log_path_sanitizes_unsafe_agent_id_characters() {
        // A user-editable catalog id must never let a path separator escape the
        // configured log directory.
        assert_eq!(
            wire_log_path_for(Path::new("/logs/acp-wire.log"), "../evil"),
            PathBuf::from("/logs/acp-wire-___evil.log")
        );
    }

    #[test]
    fn wire_log_path_handles_a_bare_file_name_with_no_directory() {
        assert_eq!(
            wire_log_path_for(Path::new("acp-wire.log"), "claude"),
            PathBuf::from("acp-wire-claude.log")
        );
    }

    #[test]
    fn a_fat_field_is_replaced_by_a_resolvable_marker_and_short_ones_survive() {
        let line = serde_json::json!({
            "params": {"update": {
                "status": "completed",
                "content": [{"oldText": "a".repeat(600), "newText": "short"}],
            }},
        })
        .to_string();
        let mut elider = elider();
        let slim: Value = serde_json::from_str(&elider.slim(&line)).expect("slim line is JSON");

        let update = &slim["params"]["update"];
        assert_eq!(update["status"], "completed");
        assert_eq!(update["content"][0]["newText"], "short");

        let [spill] = &elider.spills[..] else {
            panic!("exactly one spill, got {}", elider.spills.len());
        };
        assert_eq!(spill.path, "params.update.content[0].oldText");
        assert_eq!(spill.text.len(), 600);
        let elided = update["content"][0]["oldText"]
            .as_str()
            .expect("still a string");
        assert!(
            elided.ends_with(&format!("@@acp-payload:{}:600@@", spill.id)),
            "marker resolves to the record: {elided}"
        );
    }

    #[test]
    fn a_multibyte_payload_previews_on_a_char_boundary() {
        // Every char is 3 bytes, so PREVIEW_BYTES lands mid-character unless the
        // boundary is honored — a naive slice would panic here.
        let line = serde_json::json!({"text": "한".repeat(400)}).to_string();
        let slim: Value = serde_json::from_str(&elider().slim(&line)).expect("slim line is JSON");
        let elided = slim["text"].as_str().expect("still a string");
        assert!(elided.starts_with("한"), "preview kept: {elided}");
        assert!(elided.contains("@@acp-payload:"));
    }
}
