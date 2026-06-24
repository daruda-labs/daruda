//! Manual spike runner — proves the ACP round-trip against a real adapter on
//! the smol executor (gpui's executor).
//!
//! Requires `npx` on PATH and a Claude Code login (the adapter handles auth;
//! do NOT set `ANTHROPIC_API_KEY` if you want subscription billing). Network +
//! credentials are needed, so this is a manual tool, not a CI test.
//!
//! ```bash
//! cargo run -p daruda_acp --example acp_spike -- "list the files in this directory"
//! ```
//! A prompt that makes the agent run a tool also exercises the permission card
//! round-trip (the spike auto-approves).

use futures::StreamExt;

fn main() {
    let prompt = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "What is 2 + 2? Reply in one short sentence.".to_string());
    let cwd = std::env::current_dir().expect("current dir");

    let (tx, mut rx) = futures::channel::mpsc::unbounded();

    smol::block_on(async move {
        let result = daruda_acp::run_one_shot(Default::default(), cwd, prompt, tx).await;

        // All senders are dropped once `run_one_shot` returns, so the buffered
        // events drain to end-of-stream here.
        while let Some(event) = rx.next().await {
            println!("[event] {event:?}");
        }

        if let Err(err) = result {
            eprintln!("spike failed: {err}");
            std::process::exit(1);
        }
        eprintln!("spike ok");
    });
}
