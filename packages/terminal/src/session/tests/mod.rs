use daruda_core::path_style::PathStyle;

use super::scanners::parse_osc7_path;
use super::*;
use crate::coords::ViewportRow;
use crate::{TerminalConfig, TerminalDims};

mod capability_queries;
mod command_history;
mod line_buffer;
mod osc133_prompt_marks;
mod osc7_cwd;
mod osc_sequences;
mod prompt_mark_abs_y;
mod screen_modes;

// Capability queries ---------------------------------------------------

fn collect_responses(session: &mut TerminalSession, query: &[u8]) -> Vec<u8> {
    let mut responses = Vec::new();
    session
        .feed_with_pty_responses(query, |bytes| responses.extend_from_slice(bytes))
        .unwrap();
    responses
}

// LineBuffer capture wiring ---------------------------------------

fn session_with(cols: u16, rows: u16, max_scrollback: usize) -> TerminalSession {
    let dims = TerminalDims { cols, rows };
    let config = TerminalConfig {
        max_scrollback,
        ..TerminalConfig::default()
    };
    TerminalSession::new(dims, config).expect("failed to create session")
}
