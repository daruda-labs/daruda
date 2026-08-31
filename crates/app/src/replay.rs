//! `daruda --replay-acp-log <path>` — load a captured ACP wire log into a live
//! agent-chat pane and leave it there to be poked at.
//!
//! This is the hands-on counterpart to `--screenshot`: that flag freezes one
//! frame of a synthetic transcript, this one fills a real pane with a real
//! conversation and hands it to you. Scrolling it, folding it, narrowing it and
//! shrinking the pane is the point — a captured session is far rougher than any
//! fixture (hundreds of tool calls, turns over a hundred items long, titles that
//! run past the pane edge), and that roughness is exactly what the row
//! projection, fold headers and tail window have to survive.
//!
//! No ACP session is involved. The pane is seeded through the same entry point
//! the screenshot scenarios use, which parks it out of `Idle` so nothing tries
//! to connect an agent behind it.
//!
//! ```text
//! cargo build -p daruda --features replay
//! DARUDA_DATA_DIR=/tmp/daruda-replay target/debug/daruda \
//!   --replay-acp-log ~/.daruda/logs/debug/acp-wire-claude.log
//! ```
//!
//! The capture holds real prompts, real file paths and real agent prose. It is
//! read from disk and shown locally; nothing about it is written back.

use std::path::{Path, PathBuf};
use std::time::Duration;

use daruda_store::observability::error_report::{ErrorReport, ErrorSeverity};
use daruda_store::observability::log_writer::LogWriter;
use gpui::App;

/// CLI flag naming the capture to replay.
const REPLAY_FLAG: &str = "--replay-acp-log";

/// CLI flag overriding which dialect reads the capture. Only consulted when the
/// log's own `initialize` did not name a program daruda recognises.
const AGENT_FLAG: &str = "--replay-agent";

/// The wire tap's own output path — set automatically in debug builds.
const WIRE_LOG_ENV: &str = "DARUDA_ACP_WIRE_LOG";

/// Env var overriding the post-launch settle delay (milliseconds).
const SETTLE_ENV: &str = "DARUDA_REPLAY_SETTLE_MS";

/// How long to let the workspace settle (async project/git/lane restore) before
/// opening a pane — a pane cannot open until there is an accessible lane.
const SETTLE_DELAY: Duration = Duration::from_millis(2000);

/// Parse `--replay-acp-log <path>` / `--replay-acp-log=<path>`.
pub(crate) fn parse_replay_arg() -> Option<PathBuf> {
    parse_path_from(std::env::args())
}

fn parse_path_from(mut args: impl Iterator<Item = String>) -> Option<PathBuf> {
    while let Some(arg) = args.next() {
        if let Some(path) = arg.strip_prefix(concat!("--replay-acp-log", "=")) {
            return Some(PathBuf::from(path));
        }
        if arg == REPLAY_FLAG {
            return args.next().map(PathBuf::from);
        }
    }
    None
}

/// Parse `--replay-agent <id>` / `--replay-agent=<id>`. Absent → `None`, which
/// leaves the choice to the capture's own file name.
pub(crate) fn parse_replay_agent_arg() -> Option<String> {
    parse_agent_from(std::env::args())
}

fn parse_agent_from(mut args: impl Iterator<Item = String>) -> Option<String> {
    while let Some(arg) = args.next() {
        if let Some(id) = arg.strip_prefix(concat!("--replay-agent", "=")) {
            return Some(id.to_owned()).filter(|id| !id.is_empty());
        }
        if arg == AGENT_FLAG {
            return args.next().filter(|id| !id.is_empty());
        }
    }
    None
}

fn settle_delay_from(var: Option<&str>) -> Duration {
    var.and_then(|v| v.trim().parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(SETTLE_DELAY)
}

/// Replay `path` now, synchronously.
///
/// Must run **before the first window opens**. A debug build points the wire tap
/// at this same directory, and a restored agent-chat pane connects lazily on
/// focus — at which point the tap truncates the capture. Reading first is what
/// makes that unlosable, so this is deliberately not deferred to the settle
/// timer alongside the seeding.
pub(crate) fn load(path: &Path, explicit_agent: Option<String>) -> Option<Loaded> {
    warn_if_the_tap_writes_here(path);
    // The tap splices the agent id into the file name, so a capture usually
    // names its own agent. An explicit flag wins; neither leaves it to the
    // catalog default.
    let agent_id = explicit_agent.or_else(|| daruda_acp::agent_id_from_path(path));
    match daruda_acp::replay_log(path, agent_id.as_deref().unwrap_or_default()) {
        Ok(replay) => {
            describe(&replay, path);
            if let Some(id) = &agent_id {
                println!("replay: opening the pane as agent {id:?}");
            }
            Some(Loaded {
                items: replay.items,
                agent_id,
            })
        }
        Err(err) => {
            println!("replay: {err}");
            LogWriter::log(
                ErrorReport::new("ACP log replay failed")
                    .message(err.to_string())
                    .severity(ErrorSeverity::Error)
                    .at(file!(), line!())
                    .dedup("replay.load_failed")
                    .build(),
            );
            None
        }
    }
}

/// A replayed capture, ready to seed.
pub(crate) struct Loaded {
    items: Vec<daruda_acp::ChatItem>,
    /// Which catalog agent the pane opens under, so its chrome names the agent
    /// the capture came from. `None` leaves it to the catalog default.
    agent_id: Option<String>,
}

/// Seed an already-loaded transcript once the workspace has settled enough to
/// have a lane to open a pane in.
pub(crate) fn schedule_seed(loaded: Loaded, cx: &mut App) {
    cx.spawn(async move |cx| {
        let settle = settle_delay_from(std::env::var(SETTLE_ENV).ok().as_deref());
        cx.background_executor().timer(settle).await;
        cx.update(|cx| seed_pane(loaded, cx));
    })
    .detach();
}

/// Warn when the capture being replayed sits where this process's own wire tap
/// writes. The tap truncates on its first open, so a session starting in this
/// run would destroy the capture — and in a debug build the tap is on by
/// default. Losing a capture that way is silent and unrecoverable, so it is
/// worth saying loudly even though the load above has already read the file.
fn warn_if_the_tap_writes_here(path: &Path) {
    let Some(tap) = std::env::var_os(WIRE_LOG_ENV) else {
        return;
    };
    let tap = PathBuf::from(tap);
    if tap.parent() != path.parent() {
        return;
    }
    println!(
        "replay: WARNING — the wire tap ({WIRE_LOG_ENV}) writes to {}, the same \
         directory as this capture. Any session started in this run truncates \
         its log there. Point {WIRE_LOG_ENV} somewhere else to keep the capture.",
        tap.display()
    );
}

/// Report what was loaded, including the parts a caller would otherwise have to
/// guess at — unresolved payloads mean truncated text is on screen, and a
/// multi-session capture is a concatenation no live pane would ever hold.
fn describe(replay: &daruda_acp::Replay, path: &Path) {
    let turns = replay
        .items
        .iter()
        .filter(|it| matches!(it, daruda_acp::ChatItem::UserText(_)))
        .count();
    println!(
        "replay: {} items, {turns} turns from {}",
        replay.items.len(),
        path.display()
    );
    println!(
        "replay: payloads restored {}, unresolved {}",
        replay.rehydrated, replay.unresolved
    );
    if replay.unresolved > 0 {
        println!(
            "replay: {} field(s) stayed truncated — the payload sidecar is missing \
             or the capture ran with payloads disabled",
            replay.unresolved
        );
    }
    if replay.sessions > 1 {
        println!(
            "replay: capture spans {} sessions; they are concatenated, which no \
             single live pane would hold",
            replay.sessions
        );
    }
}

fn seed_pane(loaded: Loaded, cx: &mut App) {
    let Some((handle, weak)) = crate::window_registry::WindowRegistry::first_workspace(cx) else {
        println!("replay skipped: no workspace window");
        return;
    };
    crate::windows::try_update_workspace_window(handle, cx, "replay_seed", move |window, cx| {
        let Some(workspace) = weak.upgrade() else {
            return;
        };
        let Loaded { items, agent_id } = loaded;
        let opened = workspace.update(cx, |ws, cx| {
            ws.open_agent_chat_pane_with_transcript(agent_id.as_deref(), items, window, cx)
        });
        if !opened {
            println!("replay skipped: no accessible lane to open a pane in");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> impl Iterator<Item = String> + use<> {
        list.iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn a_separate_argument_carries_the_path() {
        assert_eq!(
            parse_path_from(args(&["daruda", REPLAY_FLAG, "/logs/a.log"])),
            Some(PathBuf::from("/logs/a.log"))
        );
    }

    #[test]
    fn an_inline_argument_carries_the_path() {
        assert_eq!(
            parse_path_from(args(&["daruda", "--replay-acp-log=/logs/b.log"])),
            Some(PathBuf::from("/logs/b.log"))
        );
    }

    #[test]
    fn no_flag_means_no_replay() {
        assert_eq!(
            parse_path_from(args(&["daruda", "--screenshot", "/x.png"])),
            None
        );
    }

    #[test]
    fn a_dangling_flag_is_not_a_path() {
        assert_eq!(parse_path_from(args(&["daruda", REPLAY_FLAG])), None);
    }

    #[test]
    fn the_agent_override_reads_both_spellings() {
        assert_eq!(
            parse_agent_from(args(&["daruda", AGENT_FLAG, "codex-acp"])).as_deref(),
            Some("codex-acp")
        );
        assert_eq!(
            parse_agent_from(args(&["daruda", "--replay-agent=claude"])).as_deref(),
            Some("claude")
        );
    }

    #[test]
    fn no_agent_override_leaves_the_choice_to_the_capture() {
        // `None`, not an empty id: the capture's own file name gets to decide.
        assert_eq!(
            parse_agent_from(args(&["daruda", REPLAY_FLAG, "/x.log"])),
            None
        );
        assert_eq!(parse_agent_from(args(&["daruda", AGENT_FLAG])), None);
        assert_eq!(parse_agent_from(args(&["daruda", "--replay-agent="])), None);
    }

    #[test]
    fn the_settle_delay_falls_back_on_junk() {
        assert_eq!(settle_delay_from(Some("500")), Duration::from_millis(500));
        assert_eq!(
            settle_delay_from(Some("  750 ")),
            Duration::from_millis(750)
        );
        assert_eq!(settle_delay_from(Some("soon")), SETTLE_DELAY);
        assert_eq!(settle_delay_from(None), SETTLE_DELAY);
    }
}
