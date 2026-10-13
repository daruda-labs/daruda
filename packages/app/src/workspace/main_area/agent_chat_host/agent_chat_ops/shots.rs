//! Screenshot-scenario seeds: agent-chat panes opened with a fixed transcript
//! or state so a capture needs no ACP session.

use crate::workspace::Workspace;
#[cfg(feature = "screenshot")]
use daruda_chat::shot_transcript::{SHOT_GROUP_TAIL_WINDOW, SHOT_THOUGHT_CALL_WINDOW};
use gpui::{Context, Window};

/// Steps the narrowed screenshot scenario keeps. One of the offered choices,
/// and small enough against the seed that the tail-more row has a real count.
#[cfg(feature = "screenshot")]
const SHOT_TAIL_WINDOW: usize = 3;

/// Which state of the thought-run seed a capture shows.
#[cfg(feature = "screenshot")]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ThoughtsShot {
    /// Settled, every kind shown.
    Settled,
    /// Mid-turn, ending on a thought with no call after it yet.
    Working,
    /// Settled, with the tool facets filtered out — the group reads as what it
    /// still shows.
    ThoughtsOnly,
}

/// Age the running-tool capture backdates its call to, so the badge shows a
/// number rather than the `0s` a just-started call would.
#[cfg(feature = "screenshot")]
const SHOT_RUNNING_TOOL_AGE: std::time::Duration = std::time::Duration::from_secs(12);

impl Workspace {
    /// Seed failure copy and its remedy before focus can start a real adapter.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn open_agent_chat_failure_for_shot(
        &mut self,
        failure: daruda_acp::AcpFailure,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_agent_chat_pane_seeded(
            None,
            move |v, window, cx| {
                v.seed_transcript(
                    vec![
                        daruda_acp::ChatItem::UserText("why is the build failing?".to_string()),
                        daruda_acp::ChatItem::Failure(failure.clone()),
                    ],
                    window,
                    cx,
                );
                v.set_error(
                    daruda_chat::agent_chat_helpers::failure_message(&failure),
                    failure.remedy(),
                    cx,
                );
            },
            window,
            cx,
        );
    }

    /// Open an empty agent-chat pane with view options open.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn open_agent_chat_empty_for_shot(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_agent_chat_pane(window, cx);
        let pane_id = self.active_runtime().focused_pane_id;
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return;
        };
        view.update(cx, |v, cx| {
            v.pin_options_for_shot(daruda_chat::view::ActivityOptionsTab::Fold, cx)
        });
    }

    /// Open the compact bar's combined options popover on `tab`, with every
    /// transcript axis off its default so the gear's own selected state is
    /// visible alongside the panel.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn open_agent_chat_options_for_shot(
        &mut self,
        tab: daruda_chat::view::ActivityOptionsTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use daruda_chat::rows::tail::{TailLevel, TailWindow};
        use daruda_config::TAIL_WINDOW_CHOICES;
        use daruda_transcript::display_filter::FilterFacet;
        use daruda_transcript::fold_mode::FoldPreset;

        self.open_agent_chat_transcript_for_shot(window, cx);
        let pane_id = self.active_runtime().focused_pane_id;
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return;
        };
        view.update(cx, |v, cx| {
            v.set_fold_mode(
                FoldPreset::Summary.mode().with_collapse_history(true),
                window,
                cx,
            );
            v.toggle_display_facet(FilterFacet::ToolEdit, cx);
            for level in TailLevel::ALL {
                v.set_tail_window(level, TailWindow::last(TAIL_WINDOW_CHOICES[0]), cx);
            }
            v.seed_usage_for_shot(cx);
            v.pin_options_for_shot(tab, cx);
        });
    }

    /// Open a seeded agent-chat transcript for screenshots.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn open_agent_chat_transcript_for_shot(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_agent_chat_pane_seeded(
            None,
            |v, window, cx| {
                v.seed_transcript(
                    daruda_chat::shot_transcript::sample_transcript(),
                    window,
                    cx,
                )
            },
            window,
            cx,
        );
    }

    /// Open the prose-only seed with the first reply's own fold shut and the
    /// second left open, so one capture carries both states of the one control
    /// such a turn has. The response bar cannot stand in for it: the conclusion
    /// escape keeps a sole reply on screen through that fold.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn open_agent_chat_sole_reply_for_shot(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use daruda_chat::fold::FoldKey;

        self.open_agent_chat_pane_seeded(
            None,
            |v, window, cx| {
                v.seed_transcript(
                    daruda_chat::shot_transcript::sole_reply_transcript(),
                    window,
                    cx,
                )
            },
            window,
            cx,
        );
        let pane_id = self.active_runtime().focused_pane_id;
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return;
        };
        view.update(cx, |v, cx| {
            if let Some(ix) = v.first_conclusion_for_shot() {
                v.set_fold_for_shot(FoldKey::Assistant(ix), false, window, cx);
            }
        });
    }

    /// Open the seeded transcript with the plan region expanded beside it.
    /// `stopped` picks the post-Stop plan, whose running step settled to
    /// `Cancelled` — the pair is what shows whether the four status icons read
    /// apart by shape.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn open_agent_chat_plan_for_shot(
        &mut self,
        stopped: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_agent_chat_pane_seeded(
            None,
            move |v, window, cx| {
                v.seed_transcript(
                    daruda_chat::shot_transcript::sample_transcript(),
                    window,
                    cx,
                );
                v.seed_plan(daruda_chat::shot_transcript::shot_plan(stopped), cx);
            },
            window,
            cx,
        );
    }

    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn open_agent_chat_diff_for_shot(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_agent_chat_pane_seeded(
            None,
            |v, window, cx| {
                let mut items = vec![daruda_acp::ChatItem::UserText("Show the proposed change".into())];
                items.extend(daruda_chat::shot_transcript::sample_transcript().into_iter().filter(
                    |item| matches!(item, daruda_acp::ChatItem::ToolCall(call) if !call.diffs.is_empty()),
                ));
                v.seed_transcript(items, window, cx);
                v.set_all_folds(true, window, cx);
            },
            window,
            cx,
        );
    }

    /// Open the seeded transcript with a Stop marker closing its run — the one
    /// shape no settled seed reaches, since the marker is pushed by the Stop
    /// path rather than by anything the agent sends.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn open_agent_chat_interrupted_transcript_for_shot(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_agent_chat_pane_seeded(
            None,
            |v, window, cx| {
                v.seed_transcript(
                    daruda_chat::shot_transcript::interrupted_transcript(),
                    window,
                    cx,
                )
            },
            window,
            cx,
        );
    }

    /// Open the interrupted transcript with the queue that Stop parked, either
    /// waiting or with the resume gesture armed. The strip's key hint and its
    /// confirmation sentence share a line with the Resume button and the count,
    /// so whether they read as one control is a question only a capture answers.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn open_agent_chat_parked_queue_for_shot(
        &mut self,
        armed: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_agent_chat_pane_seeded(
            None,
            move |v, window, cx| {
                v.seed_transcript(
                    daruda_chat::shot_transcript::interrupted_transcript(),
                    window,
                    cx,
                );
                v.seed_parked_queue_for_shot(armed, cx);
            },
            window,
            cx,
        );
    }

    /// Open the seeded transcript mid-turn — the agent's answer not yet
    /// written, so its last prose is a preamble rather than a conclusion.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn open_agent_chat_working_transcript_for_shot(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_agent_chat_pane_seeded(
            None,
            |v, window, cx| {
                v.seed_working_transcript(
                    daruda_chat::shot_transcript::working_transcript(),
                    window,
                    cx,
                )
            },
            window,
            cx,
        );
    }

    /// Open the seeded transcript with filtering, tail, and filter panel active.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn open_agent_chat_narrowed_transcript_for_shot(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use daruda_chat::rows::tail::{TailLevel, TailWindow};
        use daruda_transcript::display_filter::FilterFacet;

        self.open_agent_chat_transcript_for_shot(window, cx);
        let pane_id = self.active_runtime().focused_pane_id;
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return;
        };
        view.update(cx, |v, cx| {
            // Expanded so the filter's cut is reachable: rows a fold still holds
            // are not part of what the chip offers, so a folded transcript
            // leaves it with nothing to show and no chip at all.
            v.set_fold_mode(
                daruda_transcript::fold_mode::FoldPreset::Expanded.mode(),
                window,
                cx,
            );
            // One category unchecked leaves the Tool parent mixed.
            v.toggle_display_facet(FilterFacet::ToolEdit, cx);
            // No tail: the window's own boundary has `agent-chat-tail`, and with
            // it engaged this transcript's remaining steps hold no filtered row,
            // so the chip would have nothing to offer and would not render.
            for level in TailLevel::ALL {
                v.set_tail_window(level, TailWindow::All, cx);
            }
            // The chip rides the response bar, which the filter popover would
            // cover — the popover has its own scenario (`agent-chat-options`).
            v.select_filter_tab_for_shot(cx);
        });
    }

    /// Open a stretch of work narrated thought by thought, its one tool group
    /// open under a call window narrower than it, so the capture shows the
    /// group's bar, its boundary row above the kept calls, and the thoughts
    /// among them.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn open_agent_chat_thoughts_for_shot(
        &mut self,
        shot: ThoughtsShot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use daruda_chat::fold::FoldKey;
        use daruda_chat::rows::tail::{TailLevel, TailWindow};
        use daruda_chat::shot_transcript::{THOUGHT_RUN_GID, thought_run_transcript};
        use daruda_transcript::display_filter::{DisplayFilter, FilterFacet};

        self.open_agent_chat_pane_seeded(
            None,
            move |v, window, cx| match shot {
                ThoughtsShot::Working => {
                    v.seed_working_transcript(thought_run_transcript(true), window, cx)
                }
                ThoughtsShot::Settled | ThoughtsShot::ThoughtsOnly => {
                    v.seed_transcript(thought_run_transcript(false), window, cx)
                }
            },
            window,
            cx,
        );
        let pane_id = self.active_runtime().focused_pane_id;
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return;
        };
        view.update(cx, |v, cx| {
            let every_kind = DisplayFilter::default();
            let filter = match shot {
                ThoughtsShot::Settled | ThoughtsShot::Working => every_kind,
                ThoughtsShot::ThoughtsOnly => every_kind.with_section(FilterFacet::Tools, false),
            };
            v.set_display_filter_for_shot(filter, cx);
            v.set_tail_window(
                TailLevel::Calls,
                TailWindow::Last(SHOT_THOUGHT_CALL_WINDOW),
                cx,
            );
            v.set_fold_for_shot(FoldKey::ToolGroup(THOUGHT_RUN_GID.into()), true, window, cx);
        });
    }

    /// Open the seeded transcript with the tail window engaged and nothing
    /// floating over the transcript, so the boundary row itself is what the
    /// capture shows. `reveal` opens it.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn open_agent_chat_tail_boundary_for_shot(
        &mut self,
        reveal: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use daruda_chat::fold::FoldKey;
        use daruda_chat::rows::tail::{TailLevel, TailWindow};

        self.open_agent_chat_transcript_for_shot(window, cx);
        let pane_id = self.active_runtime().focused_pane_id;
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return;
        };
        view.update(cx, |v, cx| {
            v.set_tail_window(TailLevel::Steps, TailWindow::Last(SHOT_TAIL_WINDOW), cx);
            if let Some(run_start) = v.tail_boundary_for_shot() {
                v.set_fold_for_shot(FoldKey::Tail(run_start), reveal, window, cx);
            }
        });
    }

    /// Open the seeded transcript with a window narrower than its longest tool
    /// group and that group expanded, so the boundary the group's own window
    /// puts among its calls is what the capture shows. `reveal` opens it.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn open_agent_chat_group_tail_boundary_for_shot(
        &mut self,
        reveal: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use daruda_chat::fold::FoldKey;
        use daruda_chat::rows::tail::{TailLevel, TailWindow};

        self.open_agent_chat_transcript_for_shot(window, cx);
        let pane_id = self.active_runtime().focused_pane_id;
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return;
        };
        view.update(cx, |v, cx| {
            v.set_tail_window(
                TailLevel::Calls,
                TailWindow::Last(SHOT_GROUP_TAIL_WINDOW),
                cx,
            );
            let Some(gid) = v.last_windowed_group_for_shot() else {
                return;
            };
            v.set_fold_for_shot(FoldKey::ToolGroup(gid.clone()), true, window, cx);
            v.set_fold_for_shot(FoldKey::ToolGroupTail(gid), reveal, window, cx);
        });
    }

    /// Open the seed with its last tool call still running and its group open,
    /// so a capture shows the one badge state no settled seed reaches: the live
    /// mark with its age beside it.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn open_agent_chat_running_tool_for_shot(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use daruda_transcript::fold_mode::FoldMode;

        self.open_agent_chat_transcript_for_shot(window, cx);
        let pane_id = self.active_runtime().focused_pane_id;
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return;
        };
        view.update(cx, |v, cx| {
            // Expand first: the preset reprojects, and the started call has to
            // be in a projection that shows cards for its badge to render.
            v.set_fold_mode(FoldMode::from_tokens(["expanded"]), window, cx);
            v.start_last_tool_for_shot(SHOT_RUNNING_TOOL_AGE);
            v.reproject_for_shot(cx);
        });
    }

    /// Open the subagent seed with a window narrower than the card's child
    /// count and the card expanded, so the boundary the card's own window puts
    /// among its children is what the capture shows. `reveal` opens it.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn open_agent_chat_subagent_tail_boundary_for_shot(
        &mut self,
        reveal: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use daruda_chat::fold::FoldKey;
        use daruda_chat::rows::tail::{TailLevel, TailWindow};
        use daruda_chat::shot_transcript::SUBAGENT_PARENT_ID;

        self.open_agent_chat_pane_seeded(
            None,
            |v, window, cx| {
                v.seed_transcript(
                    daruda_chat::shot_transcript::subagent_transcript(),
                    window,
                    cx,
                )
            },
            window,
            cx,
        );
        let pane_id = self.active_runtime().focused_pane_id;
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return;
        };
        view.update(cx, |v, cx| {
            v.set_tail_window(
                TailLevel::Calls,
                TailWindow::Last(SHOT_GROUP_TAIL_WINDOW),
                cx,
            );
            // The launch shares a group with the call after it, and a settled
            // group of two defaults collapsed — which takes the card's own row
            // with it.
            v.set_fold_for_shot(
                FoldKey::ToolGroup(SUBAGENT_PARENT_ID.to_string()),
                true,
                window,
                cx,
            );
            // A launch card defaults collapsed, and its children are only on
            // screen once it is open — the boundary lives among them.
            v.set_fold_for_shot(
                FoldKey::Subagent(SUBAGENT_PARENT_ID.to_string()),
                true,
                window,
                cx,
            );
            v.set_fold_for_shot(
                FoldKey::SubagentTail(SUBAGENT_PARENT_ID.to_string()),
                reveal,
                window,
                cx,
            );
        });
    }

    /// Open the seeded transcript with a custom fold matrix and editor.
    #[cfg(feature = "screenshot")]
    pub(in crate::workspace) fn open_agent_chat_fold_editor_for_shot(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use daruda_transcript::fold_mode::FoldMode;

        self.open_agent_chat_transcript_for_shot(window, cx);
        let pane_id = self.active_runtime().focused_pane_id;
        let Some(view) = self.agent_chat_view(pane_id).cloned() else {
            return;
        };
        view.update(cx, |v, cx| {
            v.set_fold_mode(
                FoldMode::from_tokens([
                    "auto",
                    "last.tool.edit=expanded",
                    "past.thinking=collapsed",
                ]),
                window,
                cx,
            );
            v.pin_fold_editor_for_shot(cx);
        });
    }
}
