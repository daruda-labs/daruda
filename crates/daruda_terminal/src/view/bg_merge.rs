//! Background run coalescing (GPUI-free, pure logic).
//!
//! ghostty_vt already returns `StyleRun`s that are RLE-compressed on
//! (fg, bg, flags). Adjacent runs sharing the **same background** but
//! differing in fg/flags still produce separate `PaintQuad`s in the old
//! prepaint path, which inflates draw calls on heavily-styled output
//! (e.g. diff hunks, selection overlay, search highlights).
//!
//! This module exposes `merge_bg_runs`, which flattens a row's style runs
//! into a minimal list of `BgSpan { start_col, end_col, bg }`:
//!
//!   * runs whose `bg == default_bg` are dropped (default fill is painted
//!     once for the whole viewport);
//!   * runs that share a bg and are column-adjacent (`prev.end + 1 ==
//!     next.start`) are coalesced into one span.
//!
//! The Alacritty `rects.rs` batch builder and iTerm2's
//! `BackgroundColorRenderer` both rely on the same "same-color adjacency"
//! observation, so keeping the helper pure lets us mirror those code paths
//! without dragging GPUI types into the logic.

use ghostty_vt::{Rgb, StyleRun};

/// One contiguous horizontal run of cells sharing a single background color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct BgSpan {
    pub start_col: u16,
    pub end_col: u16,
    pub bg: Rgb,
}

/// Merge adjacent runs with the same background color, skipping
/// default-background cells.
///
/// `runs` is expected to be the output of
/// `TerminalSession::dump_viewport_row_style_runs`, i.e. already sorted by
/// ascending `start_col` with no overlaps and no gaps between runs.
pub(super) fn merge_bg_runs(runs: &[StyleRun], default_bg: Rgb) -> Vec<BgSpan> {
    let mut out: Vec<BgSpan> = Vec::new();
    for run in runs {
        if run.bg == default_bg {
            continue;
        }
        if let Some(last) = out.last_mut()
            && last.bg == run.bg
            && last.end_col.saturating_add(1) == run.start_col
        {
            last.end_col = run.end_col;
            continue;
        }
        out.push(BgSpan {
            start_col: run.start_col,
            end_col: run.end_col,
            bg: run.bg,
        });
    }
    out
}

/// Build a gap-free span list covering every column from 1 to `total_cols`.
///
/// Used in transparent mode so every cell has exactly one background quad and
/// Metal alpha does not accumulate past `background_alpha` (the blend formula
/// is `result_alpha = src_alpha + dst_alpha`; two overlapping quads at 0.6
/// would yield 1.2 → fully opaque).
///
/// Algorithm:
/// 1. Merge adjacent same-colour runs (including default_bg ones).
/// 2. Fill leading, inter-span, and trailing gaps with `default_bg`.
///
/// An empty `runs` slice produces a single full-row `default_bg` span.
//
// Slated for transparent-mode background painting; not wired yet.
// Kept here so the algorithm + invariants survive between then and now.
#[allow(dead_code)]
pub(super) fn merge_bg_runs_transparent(
    runs: &[StyleRun],
    total_cols: u16,
    default_bg: Rgb,
) -> Vec<BgSpan> {
    // Step 1 — merge same-colour adjacent runs.
    let mut merged: Vec<BgSpan> = Vec::new();
    for run in runs {
        if let Some(last) = merged.last_mut()
            && last.bg == run.bg
            && last.end_col.saturating_add(1) == run.start_col
        {
            last.end_col = run.end_col;
            continue;
        }
        merged.push(BgSpan {
            start_col: run.start_col,
            end_col: run.end_col,
            bg: run.bg,
        });
    }

    // Step 2 — fill any gaps so coverage is contiguous 1..=total_cols.
    let mut out: Vec<BgSpan> = Vec::new();
    let mut cursor: u16 = 1;
    for span in &merged {
        if cursor < span.start_col {
            out.push(BgSpan {
                start_col: cursor,
                end_col: span.start_col - 1,
                bg: default_bg,
            });
        }
        out.push(*span);
        cursor = span.end_col.saturating_add(1);
    }
    if cursor <= total_cols {
        out.push(BgSpan {
            start_col: cursor,
            end_col: total_cols,
            bg: default_bg,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT: Rgb = Rgb { r: 0, g: 0, b: 0 };
    const RED: Rgb = Rgb {
        r: 0xFF,
        g: 0,
        b: 0,
    };
    const BLUE: Rgb = Rgb {
        r: 0,
        g: 0,
        b: 0xFF,
    };

    fn run(start: u16, end: u16, bg: Rgb) -> StyleRun {
        StyleRun {
            start_col: start,
            end_col: end,
            fg: DEFAULT,
            bg,
            flags: 0,
        }
    }

    #[test]
    fn drops_default_background_runs() {
        let input = [run(1, 5, DEFAULT), run(6, 10, DEFAULT)];
        assert!(merge_bg_runs(&input, DEFAULT).is_empty());
    }

    #[test]
    fn keeps_single_non_default_run() {
        let input = [run(1, 5, RED)];
        let out = merge_bg_runs(&input, DEFAULT);
        assert_eq!(
            out,
            vec![BgSpan {
                start_col: 1,
                end_col: 5,
                bg: RED
            }]
        );
    }

    #[test]
    fn merges_adjacent_runs_with_same_bg_but_different_fg() {
        // Simulates "bold red on blue" followed by "italic yellow on blue".
        let a = StyleRun {
            start_col: 1,
            end_col: 3,
            fg: RED,
            bg: BLUE,
            flags: 0x02,
        };
        let b = StyleRun {
            start_col: 4,
            end_col: 7,
            fg: DEFAULT,
            bg: BLUE,
            flags: 0x04,
        };
        let out = merge_bg_runs(&[a, b], DEFAULT);
        assert_eq!(
            out,
            vec![BgSpan {
                start_col: 1,
                end_col: 7,
                bg: BLUE
            }]
        );
    }

    #[test]
    fn does_not_merge_across_default_gap() {
        // RED, DEFAULT, RED — the default slot breaks the run even though
        // the surrounding bg colors match.
        let input = [run(1, 3, RED), run(4, 6, DEFAULT), run(7, 10, RED)];
        let out = merge_bg_runs(&input, DEFAULT);
        assert_eq!(
            out,
            vec![
                BgSpan {
                    start_col: 1,
                    end_col: 3,
                    bg: RED,
                },
                BgSpan {
                    start_col: 7,
                    end_col: 10,
                    bg: RED,
                },
            ]
        );
    }

    #[test]
    fn does_not_merge_different_bg_colors() {
        let input = [run(1, 3, RED), run(4, 7, BLUE)];
        let out = merge_bg_runs(&input, DEFAULT);
        assert_eq!(
            out,
            vec![
                BgSpan {
                    start_col: 1,
                    end_col: 3,
                    bg: RED,
                },
                BgSpan {
                    start_col: 4,
                    end_col: 7,
                    bg: BLUE,
                },
            ]
        );
    }

    #[test]
    fn does_not_merge_when_columns_are_not_adjacent() {
        // Gaps in `start_col`/`end_col` should never happen in ghostty_vt
        // output, but we defend against it anyway.
        let a = run(1, 3, RED);
        let b = run(5, 7, RED); // col 4 missing
        let out = merge_bg_runs(&[a, b], DEFAULT);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn alternating_colors_produce_no_merges() {
        let input = [
            run(1, 2, RED),
            run(3, 4, BLUE),
            run(5, 6, RED),
            run(7, 8, BLUE),
        ];
        let out = merge_bg_runs(&input, DEFAULT);
        assert_eq!(out.len(), 4);
    }

    #[test]
    fn merges_realistic_selection_overlay_shape() {
        // Simulates a highlighted line where fg changes char-by-char
        // (syntax-highlighted code) but bg is a single selection color.
        let bg = BLUE;
        let fgs = [
            Rgb {
                r: 0xC0,
                g: 0xC0,
                b: 0xC0,
            },
            Rgb {
                r: 0xFF,
                g: 0xAA,
                b: 0x00,
            },
            Rgb {
                r: 0x44,
                g: 0xCC,
                b: 0x44,
            },
            Rgb {
                r: 0xFF,
                g: 0xFF,
                b: 0xFF,
            },
        ];
        let runs: Vec<StyleRun> = (0..40u16)
            .map(|i| StyleRun {
                start_col: i * 2 + 1,
                end_col: i * 2 + 2,
                fg: fgs[(i as usize) % fgs.len()],
                bg,
                flags: 0,
            })
            .collect();
        let out = merge_bg_runs(&runs, DEFAULT);
        assert_eq!(
            out,
            vec![BgSpan {
                start_col: 1,
                end_col: 80,
                bg,
            }],
            "40 adjacent same-bg runs must collapse to 1 quad"
        );
    }

    #[test]
    fn three_adjacent_runs_collapse_to_one() {
        let input = [run(1, 3, RED), run(4, 6, RED), run(7, 10, RED)];
        let out = merge_bg_runs(&input, DEFAULT);
        assert_eq!(
            out,
            vec![BgSpan {
                start_col: 1,
                end_col: 10,
                bg: RED,
            }]
        );
    }

    // ── merge_bg_runs_transparent ────────────────────────────────────────────

    #[test]
    fn transparent_empty_runs_produces_full_row_default_span() {
        // Simulates a completely empty line below shell output: ghostty_vt
        // emits no style runs, so we must cover 1..=total_cols with
        // default_bg or those cells become transparent.
        let out = merge_bg_runs_transparent(&[], 80, DEFAULT);
        assert_eq!(
            out,
            vec![BgSpan {
                start_col: 1,
                end_col: 80,
                bg: DEFAULT,
            }]
        );
    }

    #[test]
    fn transparent_trailing_gap_filled_with_default_bg() {
        // "ls" (cols 1-2) — cols 3..=80 have no style run from ghostty_vt.
        let input = [run(1, 2, RED)];
        let out = merge_bg_runs_transparent(&input, 80, DEFAULT);
        assert_eq!(
            out,
            vec![
                BgSpan {
                    start_col: 1,
                    end_col: 2,
                    bg: RED,
                },
                BgSpan {
                    start_col: 3,
                    end_col: 80,
                    bg: DEFAULT,
                },
            ]
        );
    }

    #[test]
    fn transparent_leading_gap_filled_with_default_bg() {
        // Run starts after col 1; cols 1..start-1 must be filled.
        let input = [run(5, 10, BLUE)];
        let out = merge_bg_runs_transparent(&input, 10, DEFAULT);
        assert_eq!(
            out,
            vec![
                BgSpan {
                    start_col: 1,
                    end_col: 4,
                    bg: DEFAULT,
                },
                BgSpan {
                    start_col: 5,
                    end_col: 10,
                    bg: BLUE,
                },
            ]
        );
    }

    #[test]
    fn transparent_inter_span_gap_filled_with_default_bg() {
        // RED col 1-3, DEFAULT gap col 4-6, BLUE col 7-10.
        let input = [run(1, 3, RED), run(4, 6, DEFAULT), run(7, 10, BLUE)];
        let out = merge_bg_runs_transparent(&input, 10, DEFAULT);
        // DEFAULT run at 4-6 merges with no neighbours so stays as-is;
        // no extra gap spans needed here.
        assert_eq!(
            out,
            vec![
                BgSpan {
                    start_col: 1,
                    end_col: 3,
                    bg: RED,
                },
                BgSpan {
                    start_col: 4,
                    end_col: 6,
                    bg: DEFAULT,
                },
                BgSpan {
                    start_col: 7,
                    end_col: 10,
                    bg: BLUE,
                },
            ]
        );
    }

    #[test]
    fn transparent_full_coverage_no_runs_missing() {
        // Ensure the sum of all span widths equals total_cols.
        let input = [run(3, 5, RED), run(8, 10, BLUE)];
        let total_cols: u16 = 12;
        let out = merge_bg_runs_transparent(&input, total_cols, DEFAULT);
        let covered: u16 = out.iter().map(|s| s.end_col - s.start_col + 1).sum();
        assert_eq!(covered, total_cols, "all columns must be covered");
        assert_eq!(out.first().unwrap().start_col, 1);
        assert_eq!(out.last().unwrap().end_col, total_cols);
    }

    #[test]
    fn transparent_full_row_same_color_stays_one_span() {
        // 80-col row entirely RED (syntax highlight) collapses to one span.
        let input: Vec<StyleRun> = (0..80u16).map(|i| run(i + 1, i + 1, RED)).collect();
        let out = merge_bg_runs_transparent(&input, 80, DEFAULT);
        assert_eq!(
            out,
            vec![BgSpan {
                start_col: 1,
                end_col: 80,
                bg: RED,
            }]
        );
    }

    #[test]
    fn transparent_default_bg_runs_kept_not_dropped() {
        // Unlike merge_bg_runs, the transparent variant must NOT drop
        // default_bg runs — they become explicit quads to prevent alpha
        // accumulation.
        let input = [run(1, 5, DEFAULT), run(6, 10, DEFAULT)];
        let out = merge_bg_runs_transparent(&input, 10, DEFAULT);
        assert_eq!(
            out,
            vec![BgSpan {
                start_col: 1,
                end_col: 10,
                bg: DEFAULT,
            }]
        );
    }
}
