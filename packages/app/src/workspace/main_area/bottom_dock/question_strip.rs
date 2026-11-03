//! Bottom-dock question strip: a compact band between the queued-prompt strip and
//! the composer while the focused agent pane has a question open. Bound, it
//! names the question the composer's Enter answers, or says why Enter took
//! no text; released, it offers to
//! answer; after a held-back prompt, it warns that the prompt would wait
//! behind the question and offers its two ways out. A pure builder
//! over [`BottomDockSnapshot::question`]; buttons dispatch named operations.

use gpui::{
    AnyElement, ClickEvent, Context, IntoElement, SharedString, canvas, div, prelude::*, px,
};

use crate::surface::strings as s;
use crate::ui::ButtonVariants as _;
use crate::ui::theme;
use crate::workspace::layout::{BottomDockSnapshot, Dock, QuestionStripSnapshot};

/// Marks the line that names what the composer answers.
const BOUND_GLYPH: &str = "↳";

pub(super) fn render(snap: &BottomDockSnapshot, cx: &mut Context<Dock>) -> Option<AnyElement> {
    let strip = snap.question.as_ref()?;
    let t = theme::current(cx);
    let (body, subtle, border, warn_text, warn_bg) = (
        t.text_body,
        t.text_subtle,
        t.border,
        t.banner_warning_text,
        t.banner_warning_bg,
    );
    let tint = theme::agent_chat_tint(cx);
    let pane = strip.pane_id;
    let mut actions: Vec<AnyElement> = Vec::new();
    let mut action = |id: &'static str, label: String, cx: &mut Context<Dock>, run: Action| {
        let ws = snap.workspace.clone();
        actions.push(
            crate::ui::button_compact(id, label)
                .ghost()
                .debug_selector(move || id.into())
                .on_click(cx.listener(move |_dock, _: &ClickEvent, window, cx| {
                    if let Some(ws) = ws.upgrade() {
                        ws.update(cx, |ws, cx| run.dispatch(ws, pane, window, cx));
                    }
                }))
                .into_any_element(),
        );
    };
    let (message, warn) = if strip.pending_prompt.is_some() {
        action(
            "question-skip-and-send",
            if strip.narrow {
                s::bottom_dock::question_skip_and_send_short()
            } else {
                s::bottom_dock::question_skip_and_send()
            },
            cx,
            Action::SkipAndSend,
        );
        action(
            "question-keep-queued",
            s::bottom_dock::question_keep_queued(),
            cx,
            Action::KeepQueued,
        );
        (s::bottom_dock::question_prompt_blocked(), true)
    } else if strip.composer.bound {
        action(
            "question-release",
            if strip.narrow {
                s::bottom_dock::question_release_short()
            } else {
                s::bottom_dock::question_release()
            },
            cx,
            Action::Release,
        );
        action(
            "question-show",
            s::bottom_dock::question_show(),
            cx,
            Action::Show,
        );
        // A refused Enter would otherwise do nothing visible; the reason
        // replaces the bound line until the condition clears.
        if strip.text_not_taken {
            (s::bottom_dock::question_text_not_taken(), true)
        } else {
            (bound_message(strip, strip.narrow), false)
        }
    } else {
        action(
            "question-answer",
            s::bottom_dock::question_answer(),
            cx,
            Action::Answer,
        );
        action(
            "question-show",
            s::bottom_dock::question_show(),
            cx,
            Action::Show,
        );
        (s::bottom_dock::question_waiting(), false)
    };
    let message_color = if warn { warn_text } else { body };
    let esc_hint = (strip.esc_again && strip.pending_prompt.is_none()).then(|| {
        div()
            .flex_shrink_0()
            .text_size(px(theme::FONT_SIZE_SM))
            .text_color(subtle)
            .child(SharedString::from(s::bottom_dock::question_esc_again()))
    });
    // Other questions behind this one. Its own segment, so the narrow
    // strip keeps it after the message truncates.
    let waiting_after = waiting_after_label(strip, warn).map(|label| {
        div()
            .flex_shrink_0()
            .text_color(subtle)
            .child(SharedString::from(format!("· {label}")))
    });
    // The bound line ends with the time the answer timeout leaves; its last
    // half minute turns brighter and heavier, never colour alone.
    let time_left = strip
        .time_left
        .filter(|_| strip.composer.bound && !warn)
        .map(|left| {
            let final_stretch = left.is_final_stretch();
            div()
                .flex_shrink_0()
                .text_color(if final_stretch { body } else { subtle })
                .when(final_stretch, |d| d.font_weight(gpui::FontWeight::SEMIBOLD))
                .child(SharedString::from(format!("· {}", left.label())))
        });
    let glyph = (strip.composer.bound && !warn).then(|| {
        div()
            .flex_shrink_0()
            .text_color(subtle)
            .child(SharedString::from(BOUND_GLYPH))
    });
    // The width decides the narrow layout; recording it is the layout
    // geometry caching `canvas` exists for.
    let width_capture = {
        let ws = snap.workspace.clone();
        canvas(
            move |bounds, _window, cx| {
                if let Some(ws) = ws.upgrade() {
                    ws.update(cx, |ws, cx| {
                        ws.set_question_strip_width(f32::from(bounds.size.width), cx)
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
    };
    Some(
        div()
            .id("question-strip")
            .relative()
            .debug_selector(|| "question-strip".into())
            .flex_shrink_0()
            .h(px(if strip.narrow {
                theme::AGENT_QUESTION_STRIP_NARROW_H
            } else {
                theme::AGENT_QUESTION_STRIP_H
            }))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(theme::AGENT_QUEUE_STRIP_GAP))
            .when(strip.narrow, |row| row.flex_col().items_stretch().gap_0())
            .px(px(theme::PANEL_BODY_PAD_X))
            .border_b_1()
            .border_color(if warn { warn_text } else { border })
            .bg(if warn { warn_bg } else { tint })
            .text_size(px(theme::AGENT_QUEUE_STRIP_FONT_SIZE))
            .child(
                div()
                    .flex()
                    .items_center()
                    .flex_1()
                    .min_w_0()
                    .h(px(theme::AGENT_QUESTION_STRIP_H))
                    .gap(px(theme::AGENT_QUEUE_STRIP_GAP))
                    .when(strip.narrow, |row| row.flex_none().w_full())
                    .children(glyph)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(message_color)
                            .child(SharedString::from(message)),
                    )
                    .children(waiting_after)
                    .children(time_left)
                    .children(esc_hint),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_end()
                    .h(px(theme::AGENT_QUESTION_STRIP_H))
                    .gap(px(theme::AGENT_QUEUE_STRIP_GAP))
                    .children(actions),
            )
            .child(width_capture)
            .into_any_element(),
    )
}

/// "Answering question 1/2 “Scope”", plus the note or draft hint. A narrow
/// strip drops the header and keeps the position.
fn bound_message(strip: &QuestionStripSnapshot, narrow: bool) -> String {
    let q = &strip.composer;
    let (n, total) = q.position;
    let mut message = if narrow {
        s::bottom_dock::question_answering_narrow(n, total)
    } else {
        s::bottom_dock::question_answering(n, total, header_line(&q.header))
    };
    for (on, hint) in [
        (q.note_mode, s::bottom_dock::question_note_hint()),
        (q.draft_became_answer, s::bottom_dock::question_draft_hint()),
    ] {
        if on {
            message.push_str(" · ");
            message.push_str(&hint);
        }
    }
    message
}

/// "+N waiting" while other questions on the pane wait behind this one;
/// a warning line says only the warning.
fn waiting_after_label(strip: &QuestionStripSnapshot, warn: bool) -> Option<String> {
    let count = strip.composer.waiting_after;
    (count > 0 && !warn).then(|| s::bottom_dock::question_waiting_after(count))
}

/// A header is the agent's words verbatim; only its first line fits a strip.
fn header_line(header: &str) -> &str {
    header.lines().next().unwrap_or_default().trim()
}

/// What a strip button does, dispatched as one workspace operation.
#[derive(Clone, Copy)]
enum Action {
    Release,
    Show,
    Answer,
    SkipAndSend,
    KeepQueued,
}

impl Action {
    fn dispatch(
        self,
        ws: &mut crate::workspace::Workspace,
        pane: crate::workspace::main_area::pane_tree::PaneId,
        window: &mut gpui::Window,
        cx: &mut Context<crate::workspace::Workspace>,
    ) {
        match self {
            Self::Release => ws.release_question_for_prompt(pane, cx),
            Self::Show => ws.reveal_pending_question(pane, cx),
            Self::Answer => ws.answer_question_from_strip(pane, window, cx),
            Self::SkipAndSend => ws.skip_question_and_send_prompt(pane, window, cx),
            Self::KeepQueued => ws.keep_prompt_queued_behind_question(pane, window, cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{bound_message, header_line, waiting_after_label};
    use crate::workspace::layout::QuestionStripSnapshot;

    fn strip(waiting_after: usize) -> QuestionStripSnapshot {
        QuestionStripSnapshot {
            pane_id: 1,
            composer: daruda_chat::view::QuestionComposer {
                question_id: 1,
                field: 0,
                position: (1, 2),
                waiting_after,
                header: "Which checks?".into(),
                note_mode: true,
                bound: true,
                draft_became_answer: false,
                send_highlighted: false,
                deadline: None,
            },
            pending_prompt: None,
            esc_again: false,
            text_not_taken: false,
            narrow: false,
            composer_empty: true,
            time_left: None,
        }
    }

    #[test]
    fn a_header_shows_its_first_line_only() {
        assert_eq!(header_line("Scope"), "Scope");
        assert_eq!(header_line("  Which one?\nMore detail"), "Which one?");
        assert_eq!(header_line(""), "");
    }

    /// A narrow strip keeps "question i/N" and drops the header.
    #[test]
    fn a_narrow_strip_drops_the_header_but_keeps_the_position() {
        let strip = strip(0);
        let wide = bound_message(&strip, false);
        let narrow = bound_message(&strip, true);
        assert!(wide.contains("Which checks?") && wide.contains("1/2"));
        assert!(!narrow.contains("Which checks?"));
        assert!(narrow.contains("1/2"));
        assert!(narrow.contains(&crate::surface::strings::bottom_dock::question_note_hint()));
    }

    /// The strip counts the questions waiting behind this one, and says
    /// nothing when it is the only one or a warning takes the line.
    #[test]
    fn the_strip_counts_questions_waiting_behind() {
        assert_eq!(waiting_after_label(&strip(0), false), None);
        assert_eq!(
            waiting_after_label(&strip(2), false),
            Some(crate::surface::strings::bottom_dock::question_waiting_after(2))
        );
        assert_eq!(waiting_after_label(&strip(2), true), None);
    }
}
