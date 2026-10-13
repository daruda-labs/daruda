//! Per-feed scan of PTY output that updates mode flags and OSC-driven
//! session state.

use super::*;

impl TerminalSession {
    pub(super) fn update_state_from_output(&mut self, bytes: &[u8]) {
        self.parse_tail.extend_from_slice(bytes);
        let buf = self.parse_tail.as_slice();

        // Track the furthest byte the CSI-mode scanner has fully
        // consumed. Used together with the OSC scanner's marker below
        // to drain already-processed prefix from `parse_tail` without
        // losing any cross-chunk partial sequence.
        let mut csi_consumed_upto = 0usize;
        let mut i = 0usize;
        while i + 2 < buf.len() {
            if buf[i] != 0x1b || buf[i + 1] != b'[' || buf[i + 2] != b'?' {
                i += 1;
                csi_consumed_upto = i;
                continue;
            }
            let csi_start = i;

            let mut k = i + 3;
            let mut nums: Vec<u32> = Vec::new();
            let mut num: u32 = 0;
            let mut saw_digit = false;
            let mut consumed = false;

            while k < buf.len() {
                let b = buf[k];
                if b.is_ascii_digit() {
                    saw_digit = true;
                    num = num.saturating_mul(10).saturating_add((b - b'0') as u32);
                    k += 1;
                    continue;
                }

                if b == b';' {
                    if saw_digit {
                        nums.push(num);
                        num = 0;
                        saw_digit = false;
                    }
                    k += 1;
                    continue;
                }

                if b == b'h' || b == b'l' {
                    if saw_digit {
                        nums.push(num);
                    }

                    let enabled = b == b'h';
                    for ps in nums {
                        match CsiMode::from_raw(ps) {
                            Some(CsiMode::DecCkm) => self.decckm_enabled = enabled,
                            Some(CsiMode::DecNkm) => self.decnkm_enabled = enabled,
                            Some(CsiMode::BracketedPaste) => self.bracketed_paste_enabled = enabled,
                            Some(CsiMode::FocusEvent) => self.focus_event_enabled = enabled,
                            Some(CsiMode::MouseX10) => self.mouse_x10_enabled = enabled,
                            Some(CsiMode::MouseButtonEvent) => {
                                self.mouse_button_event_enabled = enabled
                            }
                            Some(CsiMode::MouseAnyEvent) => self.mouse_any_event_enabled = enabled,
                            Some(CsiMode::MouseUtf8) => self.mouse_utf8_enabled = enabled,
                            Some(CsiMode::MouseSgr) => self.mouse_sgr_enabled = enabled,
                            Some(CsiMode::AlternateScroll) => {
                                self.alternate_scroll_enabled = enabled
                            }
                            Some(CsiMode::SynchronizedOutput) => self.synchronized_output = enabled,
                            None => {}
                        }
                        if matches!(
                            ps,
                            CSI_ALT_SCREEN_LEGACY | CSI_ALT_SCREEN | CSI_ALT_SCREEN_SAVE_CURSOR
                        ) && enabled != self.alt_screen
                        {
                            self.alt_screen = enabled;
                            self.screen_changed = true;
                            self.interval_tree.set_alt_screen_active(self.alt_screen);
                        }
                    }

                    i = k + 1;
                    consumed = true;
                    break;
                }

                // Unknown intermediate/terminator byte — skip the
                // whole CSI up to and including this byte so neither
                // the scanner nor the drain re-parses it.
                i = k + 1;
                consumed = true;
                break;
            }

            if k >= buf.len() && !consumed {
                // Leave the incomplete CSI in `parse_tail` — next feed
                // will finish it. `csi_consumed_upto` already reflects
                // the position just before `csi_start` from the last
                // `i += 1` skip.
                i = csi_start;
                break;
            }

            if consumed {
                csi_consumed_upto = i;
                continue;
            }

            i += 1;
            csi_consumed_upto = i;
        }
        let _ = i;

        let mut osc_out = OscDispatch::default();
        // OSC 133 prompt marks are captured by `Osc133Scanner` during
        // incremental feed — not here — so the cursor position at
        // capture reflects the shell's intended row. This scanner still
        // *recognizes* OSC 133 payloads so `consumed_upto` advances past
        // them, but does not emit events.
        // Track the furthest byte that belongs to a fully-completed
        // sequence. Whatever is beyond `consumed_upto` is either a
        // partial OSC still waiting for its terminator (next chunk
        // will complete it) or plain text we haven't decided to
        // discard yet. Draining `[0..consumed_upto]` after the scan
        // keeps cross-chunk buffering while preventing re-parse
        // duplication of `prompt_marks` entries.
        let mut consumed_upto = 0usize;
        let mut j = 0usize;
        while j + 1 < buf.len() {
            if buf[j] != 0x1b || buf[j + 1] != b']' {
                j += 1;
                consumed_upto = j;
                continue;
            }

            let osc_start = j;
            let mut k = j + 2;
            let mut ps: u32 = 0;
            let mut saw_digit = false;
            while k < buf.len() {
                let b = buf[k];
                if b.is_ascii_digit() {
                    saw_digit = true;
                    ps = ps.saturating_mul(10).saturating_add((b - b'0') as u32);
                    k += 1;
                    continue;
                }
                if b == b';' {
                    k += 1;
                    break;
                }
                break;
            }
            if !saw_digit || k >= buf.len() {
                // Incomplete — preserve this OSC head for the next
                // chunk by stopping advancement at `osc_start`.
                break;
            }
            // Bind so the `let osc_start = j;` above is read.
            let _ = osc_start;

            let payload_start = k;
            let mut terminated = false;
            while k < buf.len() {
                match buf[k] {
                    0x07 => {
                        commit_osc_payload(
                            ps,
                            &buf[payload_start..k],
                            &mut osc_out,
                            self.config.track_cwd,
                        );
                        k += 1;
                        terminated = true;
                        break;
                    }
                    0x1b if k + 1 < buf.len() && buf[k + 1] == b'\\' => {
                        commit_osc_payload(
                            ps,
                            &buf[payload_start..k],
                            &mut osc_out,
                            self.config.track_cwd,
                        );
                        k += 2;
                        terminated = true;
                        break;
                    }
                    _ => k += 1,
                }
            }

            if !terminated {
                // No terminator yet — leave `consumed_upto` before
                // `osc_start` so the next feed can pick up where we
                // stopped.
                break;
            }
            j = k;
            consumed_upto = k;
        }

        // Drain only what BOTH scanners have fully processed, so a
        // CSI mode sequence that spans a feed boundary survives into
        // the next call (same for OSC payloads).
        let drain_to = csi_consumed_upto.min(consumed_upto);
        if drain_to > 0 {
            self.parse_tail.drain(0..drain_to);
        }
        // Runaway protection: if an unterminated sequence keeps the
        // scanner stuck past the cap, abandon it rather than truncate
        // from the front (which would chop the escape header). OSC
        // 1337 `Copy=…:<base64>` payloads can legitimately exceed the
        // generic 64 KiB tail limit, so the effective cap is
        // `max(PARSE_TAIL_LIMIT, osc1337_max_bytes)`.
        let tail_limit = PARSE_TAIL_LIMIT.max(self.config.osc1337_max_bytes);
        if self.parse_tail.len() > tail_limit {
            self.parse_tail.clear();
        }

        if let Some(title) = osc_out.title {
            self.title = Some(title);
        }
        if let Some(clipboard) = osc_out.clipboard {
            self.clipboard_write = Some(clipboard);
        }
        if let Some(cwd) = osc_out.cwd {
            self.cwd = Some(cwd);
        }
        if let Some(kind) = osc_out.attention {
            // Last sequence wins. The drain side (`platform::attention::apply`)
            // cancels any in-flight request before issuing the new one,
            // so a Cancel inside the same feed naturally clears it.
            self.pending_attention = Some(kind);
        }
        if let Some(req) = osc_out.notification {
            self.pending_notification = Some(req);
        }
        if osc_out.clear_scrollback {
            self.pending_clear_scrollback = true;
        }
    }
}
