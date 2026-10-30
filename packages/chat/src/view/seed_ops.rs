//! Transcript seeding for the `--screenshot` and `--replay-acp-log` front-ends:
//! a fixed transcript stood up without an ACP session.

#[cfg(feature = "screenshot")]
use daruda_acp::ChatItem;
use gpui::{Context, Window};

use super::super::window_access::WindowAccess;
use super::AgentChatView;
use super::AgentSessionStatus;

/// Fixed turn facts the capture seed files, so the answer row's trailing slot
/// reads the same in every shot rather than drifting with the machine.
#[cfg(feature = "devtools")]
const SHOT_TURN_SECS: u64 = 72;
#[cfg(feature = "devtools")]
const SHOT_TURN_OUTPUT_TOKENS: u64 = 14_313;
/// A fixed wall clock for the capture seed. `Local::now()` would put a
/// different minute in every shot, which is the opposite of what a
/// pixel-comparable capture needs.
#[cfg(feature = "devtools")]
fn shot_turn_finished_at() -> chrono::DateTime<chrono::Local> {
    use chrono::TimeZone as _;
    chrono::Local
        .with_ymd_and_hms(2026, 1, 2, 11, 16, 0)
        .single()
        .unwrap_or_else(chrono::Local::now)
}

impl AgentChatView {
    /// Replace the conversation with a fixed transcript — a `--screenshot`
    /// scenario or a `--replay-acp-log` capture — then run the same aftermath a
    /// live event does. The one seeding entry point, so no caller can leave
    /// `rows`, the virtualized list or the embed caches out of step with
    /// `items`; without the embed pass a seeded pane renders diffs and verbatim
    /// output through fallbacks no live session takes.
    #[cfg(feature = "devtools")]
    pub fn seed_transcript(
        &mut self,
        items: Vec<daruda_acp::ChatItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.items = items;
        self.overage_stop_markers.clear();
        self.settle_prompts();
        // The seed stands in for a turn that already settled, so give it the
        // record a settled turn would have. Without one the answer row renders
        // bare and a capture cannot judge the facts it is supposed to carry.
        if let Some(run_start) = Self::run_start_of(&self.items) {
            self.activity.turn_records.insert(
                run_start,
                super::TurnRecord {
                    worked_for: std::time::Duration::from_secs(SHOT_TURN_SECS),
                    finished_at: shot_turn_finished_at(),
                    output_tokens: Some(SHOT_TURN_OUTPUT_TOKENS),
                },
            );
        }
        // A seeded pane stands in for a session it does not have. Parking it out
        // of `Idle` here is what keeps `maybe_connect_agent_chat` from spawning
        // a real adapter behind it on first focus. `handle` stays `None`, which
        // is what the send path already checks, so the pane reads as connected
        // and behaves as inert rather than half-live.
        self.status = AgentSessionStatus::Connected;
        // Seeding runs inside the opener's window update cycle — see [`WindowAccess`].
        let mut access = WindowAccess::Live(window);
        self.reconcile_all_embeds(&mut access, cx);
        self.reproject(cx);
    }

    /// Reproject after a capture-only model edit.
    #[cfg(feature = "screenshot")]
    pub fn reproject_for_shot(&mut self, cx: &mut Context<Self>) {
        self.reproject(cx);
    }

    /// Put the transcript's last tool call back in flight for a capture, aged by
    /// `age` so the badge shows a real number. Model-only: the caller opens the
    /// fold the card needs and reprojects.
    #[cfg(feature = "screenshot")]
    pub fn start_last_tool_for_shot(&mut self, age: std::time::Duration) {
        let Some(id) = self.items.iter_mut().rev().find_map(|item| match item {
            ChatItem::ToolCall(tc) => {
                tc.status = daruda_acp::ToolStatusView::InProgress;
                Some(tc.id.clone())
            }
            _ => None,
        }) else {
            return;
        };
        let started = std::time::Instant::now()
            .checked_sub(age)
            .unwrap_or_else(std::time::Instant::now);
        self.activity.tool_started_at.insert(id, started);
    }

    /// Seed the plan region for a capture, expanded so its entries are on
    /// screen (a settled seed would otherwise arrive collapsed).
    #[cfg(feature = "screenshot")]
    pub fn seed_plan(&mut self, plan: Vec<daruda_acp::PlanEntryView>, cx: &mut Context<Self>) {
        self.plan = plan;
        self.plan_collapsed = false;
        cx.notify();
    }

    /// Seed a transcript captured while the foreground turn is still active.
    #[cfg(feature = "screenshot")]
    pub fn seed_working_transcript(
        &mut self,
        items: Vec<daruda_acp::ChatItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let now = std::time::Instant::now();
        self.queue.turn.start(now);
        let _ = self.reconcile_activity(now);
        self.seed_transcript(items, window, cx);
    }
}
