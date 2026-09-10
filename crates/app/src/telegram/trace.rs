//! Env-gated Telegram debug trace.
//!
//! A Telegram message that never arrived raises two questions — *did it
//! move?* and *what did the bridge think its state was?* — and this file
//! answers both into its own append-only log, separate from the NDJSON
//! error log. Separate because these lines are not failures: folding
//! per-message traffic into the error log would drown the reports that
//! are, and `LogWriter`'s retention/rolling policy is sized for those.
//!
//! Off unless `DARUDA_TELEGRAM_LOG` names a writable path. Debug builds
//! point it at `<log dir>/telegram.log` from `bootstrap.rs`, matching the
//! ACP wire tap. Every entry point takes its detail as a closure, so when
//! the trace is off a call site costs one `OnceLock` read and builds no
//! string at all.
//!
//! Volume: the poll loop traces one `poll` line per `getUpdates` return —
//! roughly one per `global::POLL_TIMEOUT_SECS` while idle, well under a
//! megabyte a day. Deliberate: an investigation into a message
//! that never showed up has to separate "the loop is alive and Telegram
//! sent nothing" from "the loop is gone", and only a line on the empty
//! case says which.
//!
//! Never trace the bot token or a live pairing code — this file is plain
//! text and outlives the session. Token state is traced as a boolean.

use daruda_store::project::LaneRef;

use super::bridge::{InboundAction, Outbound, PaneRef, TelegramTail};
use super::client::UpdateKind;

use std::fs::{File, OpenOptions};
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::io::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// Env var naming the trace file. Unset (or unopenable) → trace off.
pub(crate) const TRACE_ENV: &str = "DARUDA_TELEGRAM_LOG";

/// Default file name a debug build's `bootstrap` points [`TRACE_ENV`] at,
/// beside the NDJSON logs.
#[cfg(debug_assertions)]
pub(crate) const TRACE_FILE_NAME: &str = "telegram.log";

/// How much of a message body a preview keeps: long enough to tell which
/// message a line is about, short enough that one happening stays one line.
const PREVIEW_CHARS: usize = 120;

/// Channel tag — a message moved across the bridge, or failed to.
const DELIVERY: &str = "delivery";

/// Channel tag — the bridge's own state changed.
const STATE: &str = "state";

/// How an absent `Option` renders in a trace line.
const NONE: &str = "none";

/// Timestamp format: RFC-3339 UTC with milliseconds, so lines from this
/// file can be lined up against the NDJSON log and the ACP wire tap.
const TS_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%.3fZ";

/// Successful enqueues onto / drains off the outbound channel. The channel
/// is FIFO with a single consumer, so the nth enqueue is the nth drain: the
/// two counters pair a `queue.*` line with its `queue.drained` line by
/// sequence alone, with no id threaded through [`Outbound`] (which lives in
/// the pure routing layer and should not learn about tracing).
static ENQUEUED: AtomicU64 = AtomicU64::new(0);
static DRAINED: AtomicU64 = AtomicU64::new(0);

static SINK: OnceLock<Option<Mutex<File>>> = OnceLock::new();

/// The trace file, opened once per process on first use.
fn sink() -> Option<&'static Mutex<File>> {
    SINK.get_or_init(|| {
        let path = std::env::var_os(TRACE_ENV)?;
        open(Path::new(&path))
    })
    .as_ref()
}

/// Open `path` for appending, creating its directory if needed. `None` on any
/// failure — a trace that cannot be written must never take the bridge down.
fn open(path: &Path) -> Option<Mutex<File>> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).ok()?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .ok()
        .map(Mutex::new)
}

/// Whether the trace is on. For a caller whose detail is expensive enough to
/// want skipping before it even builds the closure's captures.
pub(crate) fn is_on() -> bool {
    sink().is_some()
}

/// Trace a message crossing the bridge — fetched, routed, acted on, sent.
pub(crate) fn delivery(event: &str, detail: impl FnOnce() -> String) {
    write_line(DELIVERY, event, detail);
}

/// Trace a transition in the bridge's own state — the delivery gate, the
/// update offset, the remembered target, the callback-token tables.
pub(crate) fn state(event: &str, detail: impl FnOnce() -> String) {
    write_line(STATE, event, detail);
}

/// Trace the delivery gate — feature flag, paired chat, token presence — and
/// only when it actually changed.
///
/// Both transport loops resync the gate from live config on every iteration,
/// so an unguarded line would repeat forever while nothing had moved. The
/// first call always writes: that one records the gate the process started
/// with, which is the baseline every later transition is read against.
pub(crate) fn gate_change(enabled: bool, chat_id: Option<i64>, has_token: bool) {
    static LAST: Mutex<Option<(bool, Option<i64>, bool)>> = Mutex::new(None);

    if !is_on() {
        return;
    }
    let now = (enabled, chat_id, has_token);
    {
        let Ok(mut last) = LAST.lock() else {
            return;
        };
        if *last == Some(now) {
            return;
        }
        *last = Some(now);
    }
    state("gate", || {
        format!(
            "enabled={enabled} chat_id={} token={has_token}",
            opt(chat_id)
        )
    });
}

/// A single-line, length-bounded excerpt of a message body. Newlines and tabs
/// are escaped so one happening stays one line, and the cut lands on a char
/// boundary rather than mid-codepoint.
pub(crate) fn preview(text: &str) -> String {
    let mut out = String::with_capacity(PREVIEW_CHARS);
    for c in text.chars().take(PREVIEW_CHARS) {
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    if text.chars().nth(PREVIEW_CHARS).is_some() {
        out.push('…');
    }
    out
}

/// Outbound message text, recorded only as much as it takes to tell two
/// messages apart: `<chars>c/<hash>`.
///
/// Not a [`preview`] like an inbound payload gets. What goes out is an
/// agent's own response — the last thing that should sit in a plain-text file
/// outliving the session. The digest still answers both questions a delivery
/// trace asks of it: which of several queued messages a line is about, and
/// whether the text that reached `sendMessage` is the one that was queued
/// (same digest on both lines).
pub(crate) fn digest(text: &str) -> String {
    // `DefaultHasher`'s output is fixed for a given build rather than across
    // releases, which is all a within-run correlation id needs — and beats
    // hand-rolling a second copy of the hash in `daruda_config::project`.
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{}c/{:016x}", text.chars().count(), hasher.finish())
}

/// [`digest`] of a ping's body, whichever tail variant carries it.
pub(crate) fn tail_digest(tail: &TelegramTail) -> String {
    match tail {
        TelegramTail::Plain(t) | TelegramTail::Markdown(t) => digest(t),
    }
}

/// A stable short name per outbound queue item, for the `queue.drained` line.
pub(crate) fn outbound_kind(outbound: &Outbound) -> &'static str {
    match outbound {
        Outbound::Ping(_) => "ping",
        Outbound::Approval(_) => "approval",
        Outbound::Notice(_) => "notice",
    }
}

/// Count one successful enqueue and render where it landed: its position in
/// the enqueue order, and how many items are unsent behind it.
///
/// Only ever called from inside a trace closure, so the counters stay paired
/// with [`drain_slot`] and cost nothing when the trace is off.
pub(crate) fn enqueue_slot() -> String {
    let seq = ENQUEUED.fetch_add(1, Ordering::Relaxed) + 1;
    format!(
        "seq={seq} depth={}",
        seq.saturating_sub(DRAINED.load(Ordering::Relaxed))
    )
}

/// Count one drain and render the sequence it pairs with, plus what is left.
pub(crate) fn drain_slot() -> String {
    let seq = DRAINED.fetch_add(1, Ordering::Relaxed) + 1;
    format!(
        "seq={seq} depth={}",
        ENQUEUED.load(Ordering::Relaxed).saturating_sub(seq)
    )
}

/// A [`LaneRef`] in one whitespace-free field, `<project>/<lane>`.
pub(crate) fn lane_ref(lane: LaneRef) -> String {
    format!("{}/{}", lane.project, lane.lane)
}

/// The presence facts an outbound relay's line carries: was anyone looking at
/// daruda, and were they looking at *this* message's lane.
///
/// All three fields, not just whether they match, because "the message came
/// from a lane nobody had in front of them" and "nobody was in daruda at all"
/// are the two different answers this exists to separate. `lane` is `None` for
/// a pane that owns no lane (the orchestrator).
pub(crate) fn presence(app_active: bool, lane: Option<LaneRef>, active: LaneRef) -> String {
    format!(
        "app_active={app_active} lane={} active_lane={} lane_active={}",
        lane.map_or_else(|| NONE.to_string(), lane_ref),
        lane_ref(active),
        opt(lane.map(|l| l == active))
    )
}

/// An `Option` rendered for a trace line: the value, or [`NONE`].
pub(crate) fn opt(value: Option<impl std::fmt::Display>) -> String {
    value.map_or_else(|| NONE.to_string(), |v| v.to_string())
}

/// A [`PaneRef`] in one whitespace-free field, `<workspace uuid>/<pane id>`,
/// so a line stays parseable by field position and the uuid still greps
/// against the workspace state file that names it.
pub(crate) fn pane(pane: PaneRef) -> String {
    format!("{}/{}", pane.workspace.as_inner(), pane.pane)
}

/// A stable short name per routed action, for the `routed` line.
///
/// Lives here rather than on [`InboundAction`] because it is trace
/// vocabulary, not routing policy: `bridge.rs` is a pure state machine and
/// gains nothing from knowing how its decisions are spelled in a log.
pub(crate) fn action_name(action: &InboundAction) -> &'static str {
    match action {
        InboundAction::Ignore => "ignore",
        InboundAction::Paired { .. } => "paired",
        InboundAction::InjectPrompt { .. } => "inject_prompt",
        InboundAction::RespondPermission { .. } => "respond_permission",
        InboundAction::RunCommand { .. } => "run_command",
        InboundAction::ReportParseError { .. } => "report_parse_error",
        InboundAction::UnknownSlash { .. } => "unknown_slash",
        InboundAction::UnownedSlashNoTarget { .. } => "unowned_slash_no_target",
        InboundAction::ResolveApproval { .. } => "resolve_approval",
        InboundAction::SelectTarget { .. } => "select_target",
        InboundAction::StaleListing => "stale_listing",
        InboundAction::NoTarget { .. } => "no_target",
        InboundAction::Unsupported => "unsupported",
    }
}

/// A stable short name per inbound update shape, for the `inbound` line.
pub(crate) fn update_kind_name(kind: &UpdateKind) -> &'static str {
    match kind {
        UpdateKind::Message { .. } => "message",
        UpdateKind::Callback { .. } => "callback",
        UpdateKind::Unsupported => "unsupported",
    }
}

/// The chat an inbound update came from, when its shape names one.
pub(crate) fn update_chat_id(kind: &UpdateKind) -> Option<i64> {
    match kind {
        UpdateKind::Message { chat_id, .. } | UpdateKind::Callback { chat_id, .. } => {
            Some(*chat_id)
        }
        UpdateKind::Unsupported => None,
    }
}

/// What an inbound update carries: a message's body, or a callback's payload
/// token. Previewed, so a long prompt does not take the line over.
pub(crate) fn update_payload(kind: &UpdateKind) -> String {
    match kind {
        UpdateKind::Message { text, .. } => preview(text),
        UpdateKind::Callback { data, .. } => preview(data),
        UpdateKind::Unsupported => String::new(),
    }
}

fn write_line(channel: &str, event: &str, detail: impl FnOnce() -> String) {
    let Some(sink) = sink() else {
        return;
    };
    let ts = chrono::Utc::now().format(TS_FORMAT).to_string();
    let line = compose(&ts, channel, event, &detail());
    if let Ok(mut file) = sink.lock() {
        let _ = writeln!(file, "{line}");
    }
}

/// The on-disk line shape, split out so it is testable without a clock or a
/// file: `<ts> <channel> <event> <detail>`, whitespace-separated so `grep`
/// and `awk` both work on it.
fn compose(ts: &str, channel: &str, event: &str, detail: &str) -> String {
    format!("{ts} {channel} {event} {detail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_puts_the_channel_and_event_ahead_of_the_detail() {
        assert_eq!(
            compose("2026-01-01T00:00:00.000Z", DELIVERY, "poll", "count=0"),
            "2026-01-01T00:00:00.000Z delivery poll count=0"
        );
    }

    #[test]
    fn short_text_previews_verbatim() {
        assert_eq!(preview("deploy the thing"), "deploy the thing");
    }

    #[test]
    fn a_preview_keeps_one_happening_on_one_line() {
        assert_eq!(
            preview("first\nsecond\tthird\r"),
            "first\\nsecond\\tthird\\r"
        );
    }

    #[test]
    fn a_long_preview_is_cut_on_a_char_boundary_and_marked() {
        // Multibyte on purpose: a byte-indexed cut would panic or produce
        // invalid UTF-8 here.
        let text = "가".repeat(PREVIEW_CHARS + 10);
        let cut = preview(&text);
        assert_eq!(cut.chars().count(), PREVIEW_CHARS + 1);
        assert!(cut.ends_with('…'));
        assert!(cut.starts_with('가'));
    }

    #[test]
    fn a_preview_exactly_at_the_budget_is_not_marked() {
        let text = "x".repeat(PREVIEW_CHARS);
        assert_eq!(preview(&text), text);
    }

    #[test]
    fn an_absent_option_reads_as_none_rather_than_an_empty_field() {
        assert_eq!(opt(None::<i64>), NONE);
        assert_eq!(opt(Some(-1001)), "-1001");
    }

    #[test]
    fn a_pane_ref_renders_as_one_whitespace_free_field() {
        let pane_ref = PaneRef {
            workspace: daruda_store::project::WorkspaceUuid(uuid::Uuid::nil()),
            pane: 7,
        };
        assert_eq!(pane(pane_ref), "00000000-0000-0000-0000-000000000000/7");
    }

    #[test]
    fn every_routed_action_gets_its_own_name() {
        // The `routed` line is only useful if two different decisions never
        // read the same; the match is exhaustive, so this is the other half.
        let names = [
            action_name(&InboundAction::Ignore),
            action_name(&InboundAction::Paired { chat_id: 1 }),
            action_name(&InboundAction::StaleListing),
            action_name(&InboundAction::NoTarget {
                text: String::new(),
            }),
            action_name(&InboundAction::Unsupported),
        ];
        let mut unique = names.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), names.len());
    }

    #[test]
    fn an_inbound_message_traces_its_chat_and_body() {
        let kind = UpdateKind::Message {
            chat_id: -1001,
            text: "ship it".to_string(),
            reply_to_message_id: None,
        };
        assert_eq!(update_kind_name(&kind), "message");
        assert_eq!(update_chat_id(&kind), Some(-1001));
        assert_eq!(update_payload(&kind), "ship it");
    }

    #[test]
    fn an_inbound_callback_traces_its_token_rather_than_the_message_text() {
        let kind = UpdateKind::Callback {
            chat_id: 42,
            callback_id: "cb".to_string(),
            data: "perm:9:allow".to_string(),
            message_id: 5,
            message_text: "Allow this?".to_string(),
        };
        assert_eq!(update_kind_name(&kind), "callback");
        assert_eq!(update_chat_id(&kind), Some(42));
        assert_eq!(update_payload(&kind), "perm:9:allow");
    }

    #[test]
    fn an_unsupported_update_names_no_chat_and_carries_no_payload() {
        let kind = UpdateKind::Unsupported;
        assert_eq!(update_kind_name(&kind), "unsupported");
        assert_eq!(update_chat_id(&kind), None);
        assert_eq!(update_payload(&kind), "");
    }

    #[test]
    fn a_digest_distinguishes_two_messages_of_the_same_length() {
        let a = digest("deploy the thing");
        let b = digest("deploy the thang");
        assert_ne!(a, b);
        assert!(a.starts_with("16c/"), "{a}");
    }

    #[test]
    fn the_same_text_digests_the_same_at_queue_time_and_at_send_time() {
        // The whole point: one line says what was queued, another says what
        // went out, and matching digests are how they are tied together.
        assert_eq!(digest("hello"), digest("hello"));
    }

    #[test]
    fn a_digest_counts_chars_not_bytes() {
        assert!(digest("가나다").starts_with("3c/"));
    }

    #[test]
    fn either_tail_variant_digests_its_own_body() {
        assert_eq!(
            tail_digest(&TelegramTail::Plain("x".to_string())),
            tail_digest(&TelegramTail::Markdown("x".to_string()))
        );
    }

    #[test]
    fn presence_separates_an_absent_user_from_an_unwatched_lane() {
        let watched = LaneRef {
            project: 1,
            lane: 2,
        };
        let other = LaneRef {
            project: 1,
            lane: 9,
        };
        assert_eq!(
            presence(true, Some(watched), watched),
            "app_active=true lane=1/2 active_lane=1/2 lane_active=true"
        );
        assert_eq!(
            presence(true, Some(other), watched),
            "app_active=true lane=1/9 active_lane=1/2 lane_active=false"
        );
        assert_eq!(
            presence(false, None, watched),
            "app_active=false lane=none active_lane=1/2 lane_active=none"
        );
    }

    #[test]
    fn a_queue_slot_pairs_an_enqueue_with_its_drain_and_reports_the_backlog() {
        // Process-global counters, so this asserts on the deltas a single
        // enqueue/drain pair produces rather than on absolute sequences.
        let start_in = ENQUEUED.load(Ordering::Relaxed);
        let start_out = DRAINED.load(Ordering::Relaxed);

        let first = enqueue_slot();
        let second = enqueue_slot();
        assert_eq!(
            first,
            format!("seq={} depth={}", start_in + 1, start_in + 1 - start_out)
        );
        assert_eq!(
            second,
            format!("seq={} depth={}", start_in + 2, start_in + 2 - start_out)
        );

        // The nth drain names the nth enqueue's sequence — that pairing is
        // what makes a stuck queue readable.
        let drained = drain_slot();
        assert_eq!(
            drained,
            format!(
                "seq={} depth={}",
                start_out + 1,
                start_in + 2 - start_out - 1
            )
        );
    }

    #[test]
    fn opening_creates_the_parent_directory_and_appends() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nested").join("telegram.log");
        {
            let sink = open(&path).expect("first open");
            writeln!(sink.lock().expect("lock"), "one").expect("write");
        }
        {
            let sink = open(&path).expect("second open");
            writeln!(sink.lock().expect("lock"), "two").expect("write");
        }
        assert_eq!(
            std::fs::read_to_string(&path).expect("read back"),
            "one\ntwo\n"
        );
    }

    #[test]
    fn an_unopenable_path_yields_no_sink_instead_of_failing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let blocker = dir.path().join("not-a-dir");
        std::fs::write(&blocker, b"").expect("write blocker");
        // The parent component is a regular file, so `create_dir_all` fails.
        assert!(open(&blocker.join("telegram.log")).is_none());
    }
}
