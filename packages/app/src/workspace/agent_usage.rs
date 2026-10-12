//! Provider-neutral usage, activity, session titles, and polling state.

use std::collections::HashMap;

use daruda_store::accounts::{AccountRecipeId, AccountSelection};
use gpui::{Context, Task};

use crate::workspace::Workspace;

/// Which account's usage a cache entry belongs to. The auth domain is part of
/// the key because [`AccountSelection::SystemDefault`] names a *different*
/// login in each domain — the ambient Claude login and the ambient Codex login
/// would otherwise collide on one slot and overwrite each other every tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::workspace) struct UsageKey {
    pub recipe: AccountRecipeId,
    pub account: AccountSelection,
}

/// Per-account cache of the two usage quantities the Usage tab renders —
/// plan-rate limits and locally-aggregated activity. Encapsulates the maps
/// behind typed getters/setters so callers never reach into a raw `HashMap`;
/// the setters return whether a *visible* field changed, letting the caller
/// decide `cx.notify()` (keeps GPUI out of this plain-data type).
#[derive(Default)]
pub(in crate::workspace) struct PerAccountUsage {
    usage: HashMap<UsageKey, daruda_agent::UsageOutcome>,
    /// Keyed the same way `usage` is — a plain `AccountSelection` key would
    /// let Claude's and Codex's `SystemDefault` ambient logins collide on
    /// one slot (see [`UsageKey`]'s own doc).
    activity: HashMap<UsageKey, daruda_agent::ActivityStats>,
    pub(in crate::workspace) session_titles: Vec<daruda_store::project::types::SessionTitle>,
}

impl PerAccountUsage {
    pub(in crate::workspace) fn session_title(
        &self,
        key: UsageKey,
        session_id: &str,
    ) -> Option<&str> {
        self.session_titles
            .iter()
            .find(|entry| {
                entry.recipe == key.recipe
                    && entry.account == key.account.to_persisted()
                    && entry.session_id == session_id
            })
            .map(|entry| entry.title.as_str())
    }

    pub(in crate::workspace) fn set_session_title(
        &mut self,
        key: UsageKey,
        session_id: &str,
        title: Option<&str>,
    ) -> bool {
        let title = title.map(str::trim).filter(|title| !title.is_empty());
        if self.session_title(key, session_id) == title {
            return false;
        }
        self.session_titles.retain(|entry| {
            entry.recipe != key.recipe
                || entry.account != key.account.to_persisted()
                || entry.session_id != session_id
        });
        if let Some(title) = title {
            self.session_titles
                .push(daruda_store::project::types::SessionTitle {
                    recipe: key.recipe,
                    account: key.account.to_persisted(),
                    session_id: session_id.to_owned(),
                    title: title.to_owned(),
                });
        }
        true
    }

    /// What the last poll established for `key`. `Pending` before the first
    /// one lands, so callers never have to treat "absent" separately.
    pub(in crate::workspace) fn usage(&self, key: UsageKey) -> daruda_agent::UsageOutcome {
        self.usage.get(&key).cloned().unwrap_or_default()
    }

    /// The cached activity aggregate for `key`, if any has landed.
    pub(in crate::workspace) fn activity(
        &self,
        key: UsageKey,
    ) -> Option<&daruda_agent::ActivityStats> {
        self.activity.get(&key)
    }

    /// Fold a fetch result into `key`'s outcome; returns `true` when what is on
    /// screen changed, so only a meaningful change triggers a repaint.
    /// `UsageOutcome::rendered_key` is what "on screen" means, so a newly
    /// rendered field can't be left out of this comparison by accident.
    pub(in crate::workspace) fn advance_usage(
        &mut self,
        key: UsageKey,
        result: Result<daruda_agent::ProviderUsage, daruda_agent::FetchError>,
    ) -> bool {
        let prev = self.usage(key);
        let next = prev.clone().advance(result);
        let visible_changed = prev.rendered_key() != next.rendered_key();
        self.usage.insert(key, next);
        visible_changed
    }

    /// Insert/replace the activity aggregate for `key`; returns `true` when
    /// it differs from the prior entry (a quiet tick that found no new JSONL
    /// lines returns `false`, so an idle pane doesn't repaint — Pitfall #10).
    pub(in crate::workspace) fn set_activity(
        &mut self,
        key: UsageKey,
        activity: daruda_agent::ActivityStats,
    ) -> bool {
        if self.activity.get(&key) == Some(&activity) {
            return false;
        }
        self.activity.insert(key, activity);
        true
    }

    /// Drop every cached quantity, across all domains, for a managed account
    /// `state` no longer lists — so no usage lingers under a dangling
    /// selection. The system-default entries are never touched.
    pub(in crate::workspace) fn retain_known(
        &mut self,
        state: &daruda_store::accounts::AccountsState,
    ) {
        let known = |account: AccountSelection| match account {
            AccountSelection::Managed(id) => state.find(id).is_some(),
            AccountSelection::SystemDefault => true,
        };
        self.usage.retain(|key, _| known(key.account));
        self.activity.retain(|key, _| known(key.account));
        self.session_titles
            .retain(|entry| known(AccountSelection::from_persisted(entry.account)));
    }
}

#[derive(Default)]
pub(in crate::workspace) struct AgentUsageState {
    /// Per-account cache of the Usage tab's two quantities (plan-rate
    /// limits + locally-aggregated activity), keyed by [`UsageKey`].
    /// Empty before the first fetch/aggregation, which reads back as
    /// `UsageOutcome::Pending`. Reads/writes go through [`PerAccountUsage`]'s
    /// typed getters/setters (`advance_usage` / `set_activity_stats` decide
    /// `cx.notify()` from the returned "visible changed" flag).
    pub(in crate::workspace) usage_by_account: PerAccountUsage,

    /// Latest service-status indicator per auth domain. Account-independent —
    /// a status page reports its provider's health, not a per-account
    /// quantity — but not domain-independent: each provider hosts its own
    /// page. Absent before that domain's first fetch lands;
    /// `set_service_status` updates and bumps `cx.notify()`.
    pub(in crate::workspace) service_status: HashMap<AccountRecipeId, daruda_agent::ServiceStatus>,

    /// Guards `refresh_usage_now` against overlapping manual refreshes —
    /// the button's one-shot fetch sets this on entry and clears it when
    /// the background fetch resolves, so a double-click can't fan out
    /// two concurrent `/api/oauth/usage` round-trips.
    pub(in crate::workspace) usage_refresh_in_flight: bool,

    /// Background-poll tasks driving the Usage tab: plan-rate windows and
    /// service status for each auth domain, plus the local JSONL activity
    /// aggregation. Each task re-reads the workspace's poll cadence every tick
    /// so live config edits flow without restart. Dropping the tasks (when
    /// Workspace is dropped) cancels the loops; held in one field so the set
    /// grows with the domain list rather than the struct.
    #[allow(dead_code)]
    pub(in crate::workspace) _limits_pumps: Vec<Task<()>>,

    /// Sticky per-domain focus: the last [`FocusedAccount`] observed while a
    /// pane of that domain (`AccountDomain::for_pane`) was actually focused.
    /// Refreshed once per frame (`Workspace::prepare_frame`) via
    /// `sync::limits::observe_focus` — the single writer. Reading a domain nobody has focused yet (a fresh
    /// workspace, or a domain the user never switched to) falls back to
    /// `FocusedAccount::SystemDefault`. Without this, focusing an unrelated
    /// pane (a terminal, or another domain's agent) would snap every other
    /// domain's Usage-tab section back to its ambient login every frame.
    pub(in crate::workspace) sticky_focus_by_recipe:
        HashMap<AccountRecipeId, crate::workspace::main_area::pane::FocusedAccount>,

    /// User's manual pick for which domain's section the Usage tab shows when
    /// the focused pane doesn't name one exactly (a terminal, or an agent
    /// daruda can't resolve a domain for). Ignored while the focused pane
    /// does name one — auto mode always wins there. Volatile: not part of
    /// persisted `WorkspaceState`, and never explicitly cleared — a pick for
    /// a domain that later signs out (or was never signed in) just stops
    /// matching anything in `usage_sections`, and the resolver falls through
    /// to its default.
    pub(in crate::workspace) usage_domain_override: Option<AccountRecipeId>,
}

impl Workspace {
    /// Fold a plan-rate fetch result into `key`'s cached outcome. Called by the
    /// limits pump and by `refresh_usage_now` — including on failure, since
    /// which failure it was is what tells a signed-out domain from a domain
    /// whose refresh merely broke. Skips `cx.notify()` when nothing visible
    /// moved.
    pub(in crate::workspace) fn advance_usage(
        &mut self,
        key: UsageKey,
        result: Result<daruda_agent::ProviderUsage, daruda_agent::FetchError>,
        cx: &mut Context<Self>,
    ) {
        if self.agent_usage.usage_by_account.advance_usage(key, result) {
            cx.notify();
        }
    }

    /// Replace one provider's cached service-status snapshot. Called by the
    /// status pump after a successful fetch of that provider's status page.
    pub(in crate::workspace) fn set_service_status(
        &mut self,
        recipe: AccountRecipeId,
        status: daruda_agent::ServiceStatus,
        cx: &mut Context<Self>,
    ) {
        let prev = self.agent_usage.service_status.get(&recipe);
        let visible_changed = prev.is_none_or(|prev| {
            prev.indicator != status.indicator || prev.description != status.description
        });
        self.agent_usage.service_status.insert(recipe, status);
        if visible_changed {
            cx.notify();
        }
    }

    /// Replace the cached activity aggregate for `key` (the domain + account
    /// it was aggregated for). Called by the per-domain `Activity` pumps and
    /// by `refresh_usage_now` after `update_activity` lands. Skips the redraw
    /// when the stats are unchanged vs. that key's prior entry (a quiet tick
    /// that found no new JSONL lines) so an idle pane doesn't repaint the
    /// whole window tree (Pitfall #10).
    pub(in crate::workspace) fn set_activity_stats(
        &mut self,
        key: UsageKey,
        activity: daruda_agent::ActivityStats,
        cx: &mut Context<Self>,
    ) {
        if self
            .agent_usage
            .usage_by_account
            .set_activity(key, activity)
        {
            cx.notify();
        }
    }

    /// Manual-refresh backend for the Usage tab's ⟳ button. The tab shows every
    /// auth domain at once, so this refreshes every domain — limits, status,
    /// and local activity, each per domain — through the pump's own routing
    /// (`sync::limits::refresh_round`), which is what keeps the two from
    /// filing a domain's data under different accounts.
    ///
    /// The `usage_refresh_in_flight` guard collapses rapid clicks into a single
    /// in-flight round; it is cleared when the round resolves (or with the
    /// workspace if it is gone).
    pub(in crate::workspace) fn refresh_usage_now(&mut self, cx: &mut Context<Self>) {
        if self.agent_usage.usage_refresh_in_flight {
            return;
        }
        self.agent_usage.usage_refresh_in_flight = true;
        cx.notify();

        let sticky = self.agent_usage.sticky_focus_by_recipe.clone();

        cx.spawn(async move |this, cx| {
            let round = cx
                .background_executor()
                .spawn(async move { crate::workspace::sync::limits::refresh_round(&sticky) })
                .await;

            // The workspace can close mid-refresh; nothing to update or
            // unset then, so the dropped guard dies with the entity.
            // SILENT-OK: workspace gone mid-refresh — no state to clear.
            let _ = this.update(cx, |ws, cx| {
                for (key, result) in round.limits {
                    ws.advance_usage(key, result, cx);
                }
                for (recipe, result) in round.status {
                    if let Ok(status) = result {
                        ws.set_service_status(recipe, status, cx);
                    }
                }
                for (key, stats) in round.activity {
                    ws.set_activity_stats(key, stats, cx);
                }
                ws.agent_usage.usage_refresh_in_flight = false;
                cx.notify();
            });
        })
        .detach();
    }
}
