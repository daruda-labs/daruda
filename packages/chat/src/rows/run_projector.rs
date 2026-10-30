//! One response run's row walk: the tail windows over its units and tool
//! groups, and the filter tally written back onto its bar.

use super::*;

struct RunRows<'a> {
    rows: &'a mut Vec<RenderRow>,
    /// Index of the response bar to back-patch the tally into. `None` for an
    /// unanchored run, which has no bar to carry it.
    bar_ix: Option<usize>,
    /// Accumulated while the walk runs; written to the bar by `finish`.
    filtered: FilteredAway,
    revealed: bool,
}

impl<'a> RunRows<'a> {
    /// A group header's children, one indent deeper. Identical for every group
    /// kind, so extracting it is what keeps the tool and thinking branches from
    /// drifting on the fold and filter terms.
    fn push_group_children(
        &mut self,
        context: ProjectionContext<'_>,
        // The caller names each child's kind: a thought reads one way inside
        // a tool group and another inside a thinking group of its own.
        children: impl Iterator<Item = (usize, RowKind)>,
        folds: GroupFolds,
        indent: u8,
        group: GroupFilter,
        window: GroupWindow,
    ) {
        let items = context.items;
        let filter = context.filter;
        for (j, kind) in children {
            let filtered = !filter.matches(&items[j]);
            // A running call stays on screen through its group's shut bar and
            // shut boundary alike — a launch settles while the work inside its
            // card goes on, so collapsing the group would hide live work.
            let live = matches!(
                &items[j],
                ChatItem::ToolCall(tc) if tool_or_subtree_live(tc, context.live_units)
            );
            let structural = folds.enclosing || (!live && (folds.collapsed || window.withholds(j)));
            match group {
                GroupFilter::Kept => self.push(kind, structural, filtered, indent + 1),
                // The header already stands for the whole cut; tallying the
                // calls under it would count the same thing twice.
                GroupFilter::Emptied => self.emit(kind, structural, filtered, indent + 1),
            }
        }
    }

    fn new(
        rows: &'a mut Vec<RenderRow>,
        bar_ix: Option<usize>,
        item_count: usize,
        filter_revealed: bool,
    ) -> Self {
        rows.reserve(item_count + 2);
        Self {
            rows,
            bar_ix,
            filtered: FilteredAway::default(),
            revealed: filter_revealed,
        }
    }

    /// Push a row and, when the filter rejected it, tally it as one block the
    /// reveal brings back.
    fn push(&mut self, kind: RowKind, structural: bool, filtered: bool, indent: u8) {
        if filtered {
            self.filtered.revealable += 1;
        }
        self.emit(kind, structural, filtered, indent);
    }

    /// Push a row the tally already covers through the group header above it.
    fn emit(&mut self, kind: RowKind, structural: bool, filtered: bool, indent: u8) {
        self.rows.push(RenderRow {
            kind,
            hidden: structural || (filtered && !self.revealed),
            indent,
            filter_revealed: self.revealed,
        });
    }

    /// Write the run's tally onto its bar. The numbers describe the filter's
    /// cut alone; whether the bar is collapsed is the bar's own state, so the
    /// chip reads both rather than having one folded into the other here.
    fn finish(self) {
        let Some(bar_ix) = self.bar_ix else { return };
        if let RowKind::ResponseHeader { filtered, .. } = &mut self.rows[bar_ix].kind {
            *filtered = self.filtered;
        }
    }
}

/// What the tail window makes of one sequence of units — a response's top-level
/// tool runs, or the calls of a single tool group. Both row levels ask the same
/// question of the same axis, so they share the arithmetic rather than each
/// deciding what "the last N" means.
///
/// The third level — a subagent card's children ([`subagent`]) —
/// deliberately does not come through here: its children own no row, so there
/// is no `window_start` item index to hand back and no filter cut to tally, and
/// it reads the call level's [`TailWindow`] directly instead.
///
/// `kept` is the window's population: counting every unit let one the filter
/// emptied spend a slot, so `Recent steps: 3` put however many of the last three
/// happened to survive on screen. `total` decides only whether the boundary row
/// exists at all, which must not move with the filter or the row would change
/// list slots as the filter changes.
#[derive(Clone, Copy)]
struct UnitWindow {
    total: usize,
    kept: usize,
    /// Units the boundary row holds back — `kept` minus what the window shows.
    hidden: usize,
    /// First item the window keeps: the end of the last covered unit. The window
    /// is a range, not a per-row test, so the prose a covered run was introduced
    /// by goes behind the boundary with it while the conclusion, which follows
    /// every run, never does.
    window_start: usize,
}

impl UnitWindow {
    /// `units` yields one entry per unit in transcript order: where it ends,
    /// and whether the filter leaves it anything to show. Taken as a `Clone`
    /// iterator rather than a slice so a level whose units are already a range
    /// — a group's calls — needs no allocation to describe them.
    fn of_units(
        units: impl Iterator<Item = (usize, bool)> + Clone,
        start: usize,
        tail: TailWindow,
    ) -> Self {
        let kept = units.clone().filter(|(_, kept)| *kept).count();
        // Position within the kept population. An emptied unit takes the
        // position of the next rendering one rather than a slot of its own, so
        // it cannot shift the window. Positions only rise, so the covered units
        // are a prefix and the last one's end is where the kept range begins.
        let mut total = 0;
        let mut position = 0;
        let mut window_start = start;
        for (end, unit_kept) in units {
            total += 1;
            if tail.hides(position, kept) {
                window_start = end;
            }
            position += usize::from(unit_kept);
        }
        Self {
            total,
            kept,
            hidden: tail.hidden_steps(kept),
            window_start,
        }
    }

    /// The response's own window: one unit per top-level tool run.
    fn over_tool_runs(run: std::ops::Range<usize>, context: ProjectionContext<'_>) -> Self {
        let items = context.items;
        let mut units: Vec<(usize, bool)> = Vec::new();
        let mut k = run.start;
        while k < run.end {
            if !context.structure().top_level_tool(k) {
                k += 1;
                continue;
            }
            let span = context.structure().tool_run(k, run.end);
            k = span.end;
            // The span's own members decide, not everything it covers: an item
            // the run only passes over is not what the run has to show.
            let shows = context
                .structure()
                .group_members(span.clone())
                .any(|j| context.filter.matches(&items[j]));
            units.push((span.end, shows));
        }
        Self::of_units(units.iter().copied(), run.start, context.tail.steps)
    }

    /// One group's window: one unit per call it holds — the items it spans that
    /// own no row are not units, so they cannot spend a slot.
    fn over_group_calls(calls: &[usize], start: usize, context: ProjectionContext<'_>) -> Self {
        Self::of_units(
            calls
                .iter()
                .map(move |&j| (j + 1, context.filter.matches(&context.items[j]))),
            start,
            context.tail.calls,
        )
    }

    /// Whether the item at `ix` sits before the window's kept range.
    fn covers(self, ix: usize) -> bool {
        ix < self.window_start
    }
}

/// The two folds a group's children answer to. Kept apart rather than or-ed:
/// a running call escapes the group's own collapse, never the fold above it.
#[derive(Clone, Copy)]
struct GroupFolds {
    enclosing: bool,
    collapsed: bool,
}

/// What the step axis makes of one group's children.
#[derive(Clone, Copy)]
enum GroupWindow {
    /// A group the axis does not divide. Reasoning groups take this: a stretch
    /// of thoughts is one step's reasoning, not a run of steps, so the step
    /// axis has nothing to count inside it.
    Undivided,
    /// A tool group, whose calls are the units — with whether its own boundary
    /// row is open. The two travel together because a covered child's
    /// visibility is both answers at once.
    Divided { cut: UnitWindow, revealed: bool },
}

impl GroupWindow {
    /// A covered child the boundary is still holding back. Liveness is the
    /// caller's term: a running call stays surfaced through a shut boundary,
    /// exactly as a live run does one level up.
    fn withholds(self, ix: usize) -> bool {
        match self {
            Self::Undivided => false,
            Self::Divided { cut, revealed } => !revealed && cut.covers(ix),
        }
    }
}

/// Blocks the filter took from inside the run's tool cards. A nested child
/// owns no row, so the row walk cannot tally it — but the run's reveal admits
/// the whole card back, descendants included, so the number that reveal offers
/// has to name them or the cut is unreachable.
///
/// Summed over the run's *kept* top-level calls only: a call the filter dropped
/// is already one block on the tally and takes its card with it.
fn cut_inside_cards(context: ProjectionContext<'_>, run: std::ops::Range<usize>) -> usize {
    run.filter_map(|ix| match &context.items[ix] {
        ChatItem::ToolCall(tc)
            if !context.hierarchy.is_nested_child(tc) && context.filter.keeps_tool(tc) =>
        {
            Some(
                context
                    .hierarchy
                    .cut_below(tc.id.as_str(), |id| context.filter.keeps_id(id)),
            )
        }
        _ => None,
    })
    .sum()
}

pub(super) struct RunProjector<'items, 'rows> {
    context: ProjectionContext<'items>,
    spec: RunSpec,
    output: &'rows mut Vec<RenderRow>,
}

impl<'items, 'rows> RunProjector<'items, 'rows> {
    pub(super) fn new(
        context: ProjectionContext<'items>,
        spec: RunSpec,
        output: &'rows mut Vec<RenderRow>,
    ) -> Self {
        Self {
            context,
            spec,
            output,
        }
    }

    pub(super) fn project(self) {
        let context = self.context;
        let items = self.context.items;
        let fold = self.context.fold;
        let boundary = self.context.boundary;
        let hierarchy = self.context.hierarchy;
        let live_units = self.context.live_units;
        let filter = self.context.filter;
        let run = self.spec.run.clone();
        let base_indent = self.spec.base_indent;
        let response_collapsed = self.spec.response_collapsed;
        let last_prose = self.spec.last_prose;
        if run.is_empty() {
            return;
        }
        // Only a response the policy actually shut takes its answer with it; a
        // revealed one answers to the ordinary escape below.
        let history_fold = response_collapsed
            && fold.folds_history(fold_context_at(
                &FoldKey::Response(run.start),
                run.start,
                items,
                boundary,
                live_units,
            ));

        let tail_key = FoldKey::Tail(run.start);
        let tail_revealed = fold.is_expanded(
            &tail_key,
            fold_context_at(&tail_key, run.start, items, boundary, live_units),
        );
        let window = UnitWindow::over_tool_runs(run.clone(), context);
        let mut out = RunRows::new(
            self.output,
            self.spec.bar_ix,
            run.len(),
            self.spec.filter_revealed,
        );
        if window.total > 0 {
            out.push(
                RowKind::TailMore {
                    run_start: run.start,
                    hidden_steps: window.hidden,
                    kept_steps: window.kept - window.hidden,
                    collapsed: !tail_revealed,
                },
                response_collapsed || window.hidden == 0,
                false,
                base_indent,
            );
        }

        let mut k = run.start;
        while k < run.end {
            if !context.structure().owns_a_row(k) {
                k += 1;
                continue;
            }
            // A compaction belongs to this prompt's timeline but has its own
            // disclosure, independent of response, history and tail folds.
            if matches!(items[k], ChatItem::Compaction(_)) {
                out.push(RowKind::AgentItem(k), false, false, 0);
                k += 1;
                continue;
            }
            // A live covered run stays surfaced through a shut boundary, so the
            // window's range decides the fold and the boundary's state gates it.
            let covered = k < window.window_start;
            let folded = response_collapsed || (!tail_revealed && covered);
            if TranscriptStructure::new(items, hierarchy).top_level_tool(k) {
                // Every run earns a header, one call included: a turn's shape
                // must not change with how many calls happened to land next to
                // each other, and the bar is where the run's fold and its
                // summary live. A run always holds at least the call that
                // started it, so there is no shorter case to branch on.
                let structure = TranscriptStructure::new(items, hierarchy);
                let grun = structure.tool_run(k, run.end);
                k = grun.end;
                // Resolved once: past this line the group is its members, and
                // `grun` is only the walk's cursor — reading the span in their
                // place counts a nested child in two tallies.
                let calls: Vec<usize> = structure.group_calls(grun.clone()).collect();
                let thoughts: Vec<usize> = structure.group_thoughts(grun.clone()).collect();
                let members: Vec<usize> = structure.group_members(grun.clone()).collect();
                // Members, not calls: a filter that keeps only thoughts still
                // leaves the group something to show.
                let group = GroupFilter::of(members.iter().copied(), items, filter);
                // Two questions about one run: whether the header escapes the
                // fold above it (any call or its subtree still works), and what
                // the group's own `running` rule reads (its members).
                let escapes_enclosing_fold = run_is_live(items, calls.iter().copied(), live_units);
                {
                    let gid = tool_id(&items[grun.start]);
                    let group_key = FoldKey::ToolGroup(gid.clone());
                    // The same reading `fold_context_at` takes, asked of the
                    // structure this walk already built rather than a new one.
                    let group_active = structure.group_active(grun.clone());
                    // The fold setting is the only term: how many calls
                    // happened to land in one run must not change what the
                    // chip's rule for this block says.
                    let group_collapsed = !fold.is_expanded(
                        &group_key,
                        FoldContext::new(boundary.at(grun.start), group_active),
                    );
                    let group_tail_key = FoldKey::ToolGroupTail(gid.clone());
                    let group_tail_revealed = fold.is_expanded(
                        &group_tail_key,
                        fold_context_at(&group_tail_key, grun.start, items, boundary, live_units),
                    );
                    let group_cut = UnitWindow::over_group_calls(&calls, grun.start, context);
                    out.push(
                        RowKind::ToolGroupHeader {
                            gid: gid.clone(),
                            calls: calls.clone(),
                            thoughts,
                            collapsed: group_collapsed,
                        },
                        folded && !escapes_enclosing_fold,
                        group.hides_the_header(),
                        base_indent,
                    );
                    // Sits with the children it holds back, one indent in from
                    // the header — the same relation the response's boundary
                    // has to the run's blocks.
                    out.push(
                        RowKind::ToolGroupTailMore {
                            gid,
                            hidden_calls: group_cut.hidden,
                            kept_calls: group_cut.kept - group_cut.hidden,
                            collapsed: !group_tail_revealed,
                        },
                        folded || group_collapsed || group_cut.hidden == 0,
                        false,
                        base_indent + 1,
                    );
                    out.push_group_children(
                        context,
                        members.into_iter().map(|j| {
                            let kind = if structure.is_thought(j) {
                                RowKind::GroupThought(j)
                            } else {
                                RowKind::AgentItem(j)
                            };
                            (j, kind)
                        }),
                        GroupFolds {
                            enclosing: folded,
                            collapsed: group_collapsed,
                        },
                        base_indent,
                        group,
                        GroupWindow::Divided {
                            cut: group_cut,
                            revealed: group_tail_revealed,
                        },
                    );
                }
            } else if matches!(&items[k], ChatItem::Thinking { .. }) {
                let gstart = k;
                k += 1;
                // An empty streaming chunk gets no row of its own, so letting it
                // join a group would render a blank child and inflate the count.
                while k < run.end
                    && matches!(&items[k], ChatItem::Thinking { .. })
                    && !is_bodyless(&items[k])
                {
                    k += 1;
                }
                let grun = gstart..k;
                {
                    let group_key = FoldKey::ThinkingGroup(gstart);
                    let group_collapsed = !fold.is_expanded(
                        &group_key,
                        fold_context_at(&group_key, gstart, items, boundary, live_units),
                    );
                    let group = GroupFilter::of(grun.clone(), items, filter);
                    out.push(
                        RowKind::ThinkingGroupHeader {
                            first_ix: gstart,
                            count: grun.len(),
                            collapsed: group_collapsed,
                        },
                        folded,
                        group.hides_the_header(),
                        base_indent,
                    );
                    out.push_group_children(
                        context,
                        grun.map(|j| (j, RowKind::AgentItem(j))),
                        GroupFolds {
                            enclosing: folded,
                            collapsed: group_collapsed,
                        },
                        base_indent,
                        group,
                        GroupWindow::Undivided,
                    );
                }
            } else {
                // The run's last prose, its failures (why it stopped) and
                // actionable permissions escape enclosing folds. Only the
                // conclusion reading of that prose earns the conclusion's chrome:
                // a preamble written before work still in progress is not the
                // answer, and a bare-chevron fold announcing it read as a stray row.
                let is_last_prose = last_prose.map(LastProse::ix) == Some(k);
                let is_conclusion = last_prose == Some(LastProse::Conclusion(k));
                let is_failure = matches!(&items[k], ChatItem::Failure(_));
                let pending_permission =
                    matches!(&items[k], ChatItem::Permission(c) if c.resolved.is_none());
                let force_visible =
                    ((is_last_prose || is_failure) && !history_fold) || pending_permission;
                let kind = if is_conclusion && base_indent > 0 {
                    RowKind::ConclusionItem(k)
                } else {
                    RowKind::AgentItem(k)
                };
                out.push(
                    kind,
                    folded && !force_visible,
                    !filter.matches(&items[k]),
                    base_indent,
                );
                k += 1;
            }
        }
        out.filtered.revealable += cut_inside_cards(context, run.clone());
        out.finish();
    }
}

/// Whether any call in `run` is live — the run-wide reading of
/// [`tool_or_subtree_live`].
///
/// A group *header* speaks for its whole run, so it escapes an enclosing fold
/// while any member still works. A group *child* asks the per-call question
/// instead, and escapes only its own group's window rather than the fold above
/// it (see `RunRows::push_group_children`). The two granularities answer for
/// different subjects; they are not one rule stated twice.
fn run_is_live(
    items: &[ChatItem],
    calls: impl Iterator<Item = usize>,
    live_units: &LiveSubagentUnits,
) -> bool {
    calls.into_iter().any(
        |j| matches!(&items[j], ChatItem::ToolCall(tc) if tool_or_subtree_live(tc, live_units)),
    )
}

fn tool_id(item: &ChatItem) -> String {
    match item {
        ChatItem::ToolCall(tc) => tc.id.clone(),
        _ => String::new(),
    }
}
