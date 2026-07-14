//! Manual runner to verify ACP `session/load` resume **across process
//! restarts** — the risk gate for the agent-chat session-restore feature.
//!
//! The adapter (Claude Agent SDK) must persist a session so a *later* process
//! can reload it by id and replay the prior conversation. This proves it end to
//! end against the real adapter. Needs `npx` on PATH + a Claude Code login, and
//! the `CLAUDECODE` env var cleared (the adapter refuses a nested ACP session):
//!
//! ```bash
//! # 1. create a session, note the printed id
//! env -u CLAUDECODE cargo run -p daruda_acp --example acp_resume -- new "What is 2 + 2? One short sentence."
//! #    → prints  [session-id] <ID>   then the turn completes and the process exits
//!
//! # 2. in a SEPARATE run (fresh adapter process), load that id
//! env -u CLAUDECODE cargo run -p daruda_acp --example acp_resume -- load <ID>
//! #    → if resume works, the prior user+assistant turn prints as [update] … before [connected]
//! ```
//!
//! GATE: if `load` prints no replayed `[update]` lines, cross-restart resume
//! does not work with this adapter and the restore design must be reconsidered.

use agent_client_protocol::schema::v1::SessionId;
use daruda_acp::{AcpEvent, PermissionDecision, connect_session};
use futures::StreamExt;

fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_else(|| "new".to_string());
    let cwd = std::env::current_dir().expect("current dir");

    let resume = if mode == "load" {
        let id = args.next().expect("usage: acp_resume load <session_id>");
        Some(SessionId::new(id))
    } else {
        None
    };
    let is_load = resume.is_some();
    let prompt = args
        .next()
        .unwrap_or_else(|| "What is 2 + 2? Reply in one short sentence.".to_string());

    smol::block_on(async move {
        let (handle, mut events) = match connect_session(Default::default(), cwd, None, resume, "")
        {
            Ok(pair) => pair,
            Err(err) => {
                eprintln!("connect failed: {err}");
                std::process::exit(1);
            }
        };

        let mut replayed = 0usize;
        while let Some(event) = events.next().await {
            match event {
                AcpEvent::Connected { session_id, .. } => {
                    eprintln!("[session-id] {session_id}");
                    if is_load {
                        // On load, the adapter replays history as `session/update`
                        // BEFORE the load response resolves, so every replayed
                        // `[update]` above has already printed by the time
                        // `Connected` arrives. Report and exit.
                        eprintln!("[connected] resumed — {replayed} replayed update(s)");
                        if replayed == 0 {
                            eprintln!("[GATE-FAIL] no history replayed on load");
                        }
                        break;
                    }
                    eprintln!("[send] {prompt}");
                    handle.send_prompt(prompt.clone());
                }
                AcpEvent::Update(u) => {
                    if is_load {
                        replayed += 1;
                    }
                    eprintln!("[update] {u:?}");
                }
                AcpEvent::PermissionRequested { id, .. } => {
                    // Keep the diagnostic non-interactive; the default prompt uses
                    // no tools, so this only fires for a custom tool prompt.
                    handle.respond_permission(id, PermissionDecision::Cancelled);
                }
                AcpEvent::TurnEnded { .. } => {
                    eprintln!("[turn-ended]");
                    break;
                }
                AcpEvent::Error(e) => {
                    eprintln!("[error] {e}");
                    break;
                }
                other => eprintln!("[event] {other:?}"),
            }
        }
        eprintln!("done");
    });
}
