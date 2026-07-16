//! Render pipeline benchmarks.
//!
//! These benches intentionally stay at the `TerminalSession` layer — the GPUI
//! `TerminalView::prepaint`/`paint` path needs a live window/text_system and
//! cannot run headless. What we can cover here:
//!
//!   * `feed_and_reconcile_*` — VT parsing + dirty tracking hot path.
//!   * `style_runs_*` — the per-row style-run extraction invoked once per
//!     visible row every prepaint.
//!   * `dump_viewport_*` — raw viewport serialization used by the line cache
//!     and selection/copy paths.
//!
//! Pure helpers inside `view::element` (e.g. `byte_index_for_column_in_line`)
//! are `pub(crate)`, so they are benched indirectly through the session layer.

use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use daruda_terminal::coords::ViewportRow;
use daruda_terminal::{TerminalConfig, TerminalDims, TerminalSession};

const ROWS: u16 = 24;
const COLS: u16 = 80;

fn fresh_session() -> TerminalSession {
    TerminalSession::new(
        TerminalDims {
            cols: COLS,
            rows: ROWS,
        },
        TerminalConfig::default(),
    )
    .expect("session")
}

/// Build a byte sequence that exercises the terminal: 1,000 full viewport
/// writes interleaved with style changes. Roughly 300 KB, similar in shape to
/// `cargo test` output or a `cat` of a colorized log.
fn synthetic_output() -> Vec<u8> {
    let mut buf = Vec::with_capacity(300_000);
    let lorem = b"The quick brown fox jumps over the lazy dog 0123456789 ";
    for i in 0..1_000u32 {
        // Alternate SGR attributes to force style-run churn.
        let sgr: &[u8] = match i % 6 {
            0 => b"\x1b[0m",
            1 => b"\x1b[1;31m",
            2 => b"\x1b[32m",
            3 => b"\x1b[1;4;33m",
            4 => b"\x1b[34;44m",
            _ => b"\x1b[0;36m",
        };
        buf.extend_from_slice(sgr);
        buf.extend_from_slice(lorem);
        if i % (COLS as u32 / lorem.len() as u32) == 0 {
            buf.extend_from_slice(b"\r\n");
        }
    }
    buf
}

fn bench_feed_and_reconcile(c: &mut Criterion) {
    let payload = synthetic_output();
    c.bench_function("feed_and_reconcile_300kb", |b| {
        b.iter(|| {
            let mut session = fresh_session();
            session.feed(black_box(&payload)).unwrap();
            // Drain dirty/scroll state — this is what reconcile does every
            // frame in the real path.
            let _ = session.take_viewport_scroll_delta();
            let _ = session.take_dirty_viewport_rows();
        });
    });
}

fn bench_style_run_extraction_full_viewport(c: &mut Criterion) {
    let mut session = fresh_session();
    session.feed(&synthetic_output()).unwrap();
    let _ = session.take_viewport_scroll_delta();
    let _ = session.take_dirty_viewport_rows();

    c.bench_function("style_runs_full_viewport", |b| {
        b.iter(|| {
            for row in 0..ROWS {
                let runs = session
                    .dump_viewport_row_style_runs(ViewportRow::new(row))
                    .unwrap();
                black_box(runs);
            }
        });
    });
}

fn bench_dump_viewport_text_full(c: &mut Criterion) {
    let mut session = fresh_session();
    session.feed(&synthetic_output()).unwrap();

    c.bench_function("dump_viewport_full", |b| {
        b.iter(|| {
            let text = session.dump_viewport().unwrap();
            black_box(text);
        });
    });
}

fn bench_dump_viewport_row_loop(c: &mut Criterion) {
    let mut session = fresh_session();
    session.feed(&synthetic_output()).unwrap();

    c.bench_function("dump_viewport_per_row", |b| {
        b.iter(|| {
            for row in 0..ROWS {
                let line = session.dump_viewport_row(ViewportRow::new(row)).unwrap();
                black_box(line);
            }
        });
    });
}

criterion_group!(
    render_benches,
    bench_feed_and_reconcile,
    bench_style_run_extraction_full_viewport,
    bench_dump_viewport_text_full,
    bench_dump_viewport_row_loop,
);
criterion_main!(render_benches);
