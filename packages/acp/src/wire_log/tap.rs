//! The writer half of the wire tap — raw JSON-RPC traffic appended to a file
//! for off-line inspection. The on-disk format (marker syntax, sidecar naming)
//! is defined by the parent module and shared with the reader.

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
use daruda_core::process_env;
use serde_json::Value;

use super::{DEFAULT_BASE_STEM, UNRECORDED_ID, payload_marker, payload_sidecar_path};

/// Spill threshold. Chosen from a real capture: keeps every protocol field
/// (ids, statuses, titles, message chunks) inline while catching the fat
/// payload fields.
const DEFAULT_MAX_FIELD: usize = 512;

/// Sibling directory holding the one retained generation of each capture.
pub const PREVIOUS_GENERATION_DIR: &str = "prev";

/// How much of a spilled string stays inline, so the slim log still shows what
/// the payload was.
const PREVIEW_BYTES: usize = 96;

/// `Spill::path` for a non-JSON line (adapter stderr) spilled whole.
const WHOLE_LINE_PATH: &str = "$line";

/// Process-global so a marker id is unambiguous across concurrent sidecars.
static PAYLOAD_IDS: AtomicU64 = AtomicU64::new(1);

/// What the tap writes and how, fixed once per process by [`configure_from_env`].
#[derive(Debug, Clone, PartialEq, Eq)]
struct TapSettings {
    base: PathBuf,
    /// Spill threshold in bytes; `0` writes raw lines with no elision.
    cap: usize,
    sidecar: bool,
}

impl TapSettings {
    /// An explicit base wins over `default_base`; neither leaves the tap off.
    fn resolve(
        base: Option<&OsStr>,
        max_field: Option<&OsStr>,
        payloads: Option<&OsStr>,
        default_base: Option<PathBuf>,
    ) -> Option<Self> {
        let base = base.map(PathBuf::from).or(default_base)?;
        let cap = max_field_cap(max_field);
        Some(Self {
            base,
            cap,
            sidecar: cap > 0 && sidecar_enabled(payloads),
        })
    }
}

static SETTINGS: std::sync::OnceLock<Option<TapSettings>> = std::sync::OnceLock::new();

/// Fix this process's tap from `DARUDA_ACP_WIRE_LOG*`, falling back to
/// `default_base` when the base is unset. Returns the base in effect.
///
/// Held in-process rather than read per session, so the tap belongs to the
/// process that asked for it: a child that inherits the variables — a
/// `cargo test` an agent runs — never touches this process's capture. Only
/// the first call decides; later ones return the same answer.
pub fn configure_from_env(default_base: Option<PathBuf>) -> Option<&'static Path> {
    SETTINGS
        .get_or_init(|| {
            TapSettings::resolve(
                process_env::ACP_WIRE_LOG.read_os().as_deref(),
                process_env::ACP_WIRE_LOG_MAX_FIELD.read_os().as_deref(),
                process_env::ACP_WIRE_LOG_PAYLOADS.read_os().as_deref(),
                default_base,
            )
        })
        .as_ref()
        .map(|settings| settings.base.as_path())
}

/// The base [`configure_from_env`] fixed; `None` when the tap is off.
pub fn configured_base() -> Option<&'static Path> {
    configured().map(|settings| settings.base.as_path())
}

fn configured() -> Option<&'static TapSettings> {
    SETTINGS.get().and_then(Option::as_ref)
}

/// Attach the wire tap to `agent`, or return it unchanged when the process
/// never configured one or the file can't be opened — so a shipping build
/// never touches the wire unless explicitly asked.
///
/// `agent_id` (the catalog id, empty for the crate's own examples) is spliced
/// into the file name so concurrent sessions from different agents land in
/// separate files instead of interleaving in one.
pub(crate) fn attach(agent: AcpAgent, agent_id: &str) -> AcpAgent {
    let Some(settings) = configured() else {
        return agent;
    };
    let path = wire_log_path_for(&settings.base, agent_id);
    let Some(slim) = open_log(&path) else {
        return agent;
    };
    let sidecar = settings
        .sidecar
        .then(|| open_log(&payload_sidecar_path(&path)))
        .flatten();

    let tap = Arc::new(WireTap {
        slim,
        sidecar,
        cap: settings.cap,
    });
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
            UNRECORDED_ID.to_string()
        };
        let preview = &text[..text.floor_char_boundary(PREVIEW_BYTES.min(self.cap))];
        format!("{preview}…{}", payload_marker(&id, bytes))
    }
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

fn open_log(path: &Path) -> Option<Mutex<File>> {
    // `attach` runs per ACP session, not per app launch, so only the first open
    // of a process starts a generation; later sessions share the file. Open once
    // and share the handle — a per-line reopen would thrash under streaming
    // turns.
    //
    // Append unless rotation left the old bytes in place: two sessions hold two
    // handles, and a non-append writer keeps its own offset, so it overwrites
    // whatever its sibling appended past it.
    let truncate = first_open(path) && !rotate(path);
    // A capture holds whole transcripts — every file an agent read and every
    // command's output — so it is the owner's alone.
    daruda_core::path::open_owner_only(
        std::fs::OpenOptions::new()
            .create(true)
            .append(!truncate)
            .truncate(truncate)
            .write(truncate),
        path,
    )
    .ok()
    .map(Mutex::new)
}

/// Move the previous run's capture aside, keeping one generation. Returns
/// whether `path` is now clear.
///
/// The file name is fixed and a capture cannot be reproduced — no rerun replays
/// a session that already happened — so one generation turns an unrelated
/// launch into an undo rather than a loss. Best-effort: a failed move leaves
/// the caller to truncate, since a tap that refuses to open is worse than one
/// that loses history.
fn rotate(path: &Path) -> bool {
    if !path.exists() {
        return true;
    }
    let target = previous_generation_path(path);
    target.parent().is_some_and(|dir| {
        daruda_core::path::create_owner_only_dir(dir).is_ok()
            && std::fs::rename(path, &target).is_ok()
    })
}

/// Where [`rotate`] parks the previous run's file: a `prev/` sibling directory,
/// under the **same file name**. The name is what the reader parses — the agent
/// id (`agent_id_from_path`) and the sidecar (`payload_sidecar_path`) are both
/// derived from it — so decorating it would make a rotated capture unreadable
/// by the replay path that exists to consume it.
fn previous_generation_path(path: &Path) -> PathBuf {
    let dir = path.parent().unwrap_or(Path::new(""));
    match path.file_name() {
        Some(name) => dir.join(PREVIOUS_GENERATION_DIR).join(name),
        None => dir.join(PREVIOUS_GENERATION_DIR),
    }
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
pub(super) fn wire_log_path_for(base: &Path, agent_id: &str) -> PathBuf {
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
        .unwrap_or(DEFAULT_BASE_STEM);
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
mod tests;
