//! Manual multi-turn session runner — proves the long-lived [`connect_session`]
//! handle keeps one ACP session alive across several prompts on the smol
//! executor (gpui's executor).
//!
//! Like the one-shot spike, this needs `npx` on PATH and a Claude Code login
//! (the adapter handles auth). It is a manual tool, not a CI test.
//!
//! The adapter refuses to start a *nested* ACP session, so the `CLAUDECODE`
//! environment variable (set inside an active Claude Code session) must be
//! cleared before running:
//!
//! ```bash
//! env -u CLAUDECODE cargo run -p daruda_acp --example acp_session
//! ```
//!
//! It sends two prompts back to back over the *same* session and waits for each
//! turn to end, demonstrating that the connection is not torn down between
//! prompts.

use daruda_acp::{AcpEvent, PermissionDecision, connect_session};
use futures::StreamExt;

fn main() {
    let cwd = std::env::current_dir().expect("current dir");

    smol::block_on(async move {
        let (handle, mut events) = match connect_session(Default::default(), cwd, None) {
            Ok(pair) => pair,
            Err(err) => {
                eprintln!("connect failed: {err}");
                std::process::exit(1);
            }
        };

        let prompts = [
            "What is 2 + 2? Reply in one short sentence.",
            "Now multiply that result by 10. One short sentence.",
        ];
        let mut next_prompt = 0usize;
        let mut turns_completed = 0usize;

        while let Some(event) = events.next().await {
            match event {
                AcpEvent::Connected {
                    modes,
                    config_options,
                } => {
                    eprintln!(
                        "[connected] modes={modes:?} config_options={config_options:?} \
                         sending prompt 1"
                    );
                    handle.send_prompt(prompts[next_prompt].to_string());
                    next_prompt += 1;
                }
                AcpEvent::ConfigOptionsChanged(options) => {
                    eprintln!("[config-options] {options:?}");
                }
                AcpEvent::Update(update) => {
                    eprintln!("[update] {update:?}");
                }
                AcpEvent::PermissionRequested { id, request } => {
                    // Auto-approve the first option, mirroring the spike.
                    eprintln!("[permission] {request:?}");
                    let decision = match request.options.first() {
                        Some(opt) => PermissionDecision::Allow {
                            option_id: opt.option_id.0.to_string(),
                        },
                        None => PermissionDecision::Cancelled,
                    };
                    handle.respond_permission(id, decision);
                }
                AcpEvent::TurnEnded { stop_reason } => {
                    turns_completed += 1;
                    eprintln!("[turn {turns_completed} ended] stop_reason={stop_reason}");
                    if next_prompt < prompts.len() {
                        eprintln!("[sending prompt {}]", next_prompt + 1);
                        handle.send_prompt(prompts[next_prompt].to_string());
                        next_prompt += 1;
                    } else {
                        // Both turns done: drop the handle to close the session,
                        // which ends the connection task and the event stream.
                        eprintln!("[done] dropping handle");
                        drop(handle);
                        break;
                    }
                }
                AcpEvent::ModeChanged { mode_id } => {
                    eprintln!("[mode-changed] mode_id={mode_id}");
                }
                AcpEvent::AvailableCommandsChanged(cmds) => {
                    eprintln!("[commands] {} available", cmds.len());
                }
                AcpEvent::PlanChanged(entries) => {
                    eprintln!("[plan] {} entries", entries.len());
                }
                AcpEvent::SessionInfoChanged { title, updated_at } => {
                    eprintln!("[session-info] title={title:?} updated_at={updated_at:?}");
                }
                AcpEvent::Notice(msg) => {
                    eprintln!("[notice] {msg}");
                }
                AcpEvent::Error(err) => {
                    eprintln!("[error] {err}");
                    std::process::exit(1);
                }
            }
        }

        // Drain any trailing events after the handle drop until end-of-stream.
        while let Some(event) = events.next().await {
            eprintln!("[trailing] {event:?}");
        }

        eprintln!("session ok ({turns_completed} turns)");
    });
}
