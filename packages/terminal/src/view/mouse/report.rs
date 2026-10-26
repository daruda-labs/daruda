//! Mouse-report encoding shared by button, motion and wheel handlers.

use super::*;

#[derive(Clone, Copy)]
pub(super) enum MouseReport {
    /// Press, motion or wheel report (SGR final M).
    Press(u8),
    Release(u8),
}

fn encode_report(report: MouseReport, col: u16, row: u16, sgr: bool, utf8: bool) -> Vec<u8> {
    let (button_value, pressed) = match report {
        MouseReport::Press(button) => (button, true),
        MouseReport::Release(button) => (button, false),
    };
    if sgr {
        return sgr_mouse_sequence(button_value, col, row, pressed).into_bytes();
    }
    // Normal reports use button 3 for every release, retaining modifier bits.
    // SGR instead retains the pressed button and changes the final byte to m.
    let normal_button = if pressed {
        button_value
    } else {
        (button_value & !3) | 3
    };
    normal_mouse_sequence(normal_button, col, row, utf8)
}

impl TerminalView {
    pub(in crate::view::mouse) fn send_mouse_report(
        &self,
        report: MouseReport,
        col: u16,
        row: u16,
        repetitions: u32,
    ) {
        let Some(input) = self.input.as_ref() else {
            return;
        };
        let bytes = encode_report(
            report,
            col,
            row,
            self.session.mouse_sgr_enabled(),
            self.session.mouse_utf8_enabled(),
        );
        if !bytes.is_empty() {
            for _ in 0..repetitions {
                input.send(&bytes);
            }
        }
    }
}

#[cfg(test)]
mod tests;
