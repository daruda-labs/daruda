//! Values a `Workspace` holds only to keep something alive.
//!
//! Nothing here is read. A filesystem watch unregisters when its handle
//! drops, a pump stops when its `Task` drops, and a global observer
//! unsubscribes when its `Subscription` drops — so the whole contract is
//! "outlive the window, and be dropped in the right order". Kept as two
//! types rather than seventeen `_`-prefixed fields on `Workspace`, split by
//! the one way they differ: [`GlobalObservers`] is installed once at
//! construction, [`Pumps`] is restarted whenever the thing it watches moves.

use gpui::{Subscription, Task};

use crate::hooks::{flow_watcher, mcp_watcher, skills_watcher};

/// A filesystem watch and the pump draining its events.
///
/// The two start and stop together at every site that owns one, so they are
/// one value rather than two `Option`s that have to agree. Field order is the
/// drop order and is load-bearing: the handle unregisters the OS subscription
/// before the pump that would drain it goes away.
pub(in crate::workspace) struct Watch<H> {
    _handle: H,
    _pump: Task<()>,
}

impl<H> Watch<H> {
    pub(in crate::workspace) fn new(handle: H, pump: Task<()>) -> Self {
        Self {
            _handle: handle,
            _pump: pump,
        }
    }
}

/// Background watches, restarted as the lane, project or focused cwd moves.
/// Each `respawn_*` clears its field before building the replacement so the
/// old OS subscription is gone before the new one attaches.
pub(in crate::workspace) struct Pumps {
    pub(in crate::workspace) skills: Option<Watch<skills_watcher::SkillsWatcherHandle>>,
    pub(in crate::workspace) mcp: Option<Watch<mcp_watcher::McpWatcherHandle>>,
    pub(in crate::workspace) flow: Option<Watch<flow_watcher::FlowWatcherHandle>>,
    /// Drives the task list's pulse + elapsed labels; runs only while at least
    /// one task is `Running`, which is why this one is toggled rather than
    /// respawned.
    pub(in crate::workspace) task_live_tick: Option<Task<()>>,
    /// Port scanning has nothing to re-target, so it is started once and never
    /// touched again.
    _ports: Task<()>,
}

impl Pumps {
    /// Every watch starts unset; each `respawn_*` fills its own.
    pub(in crate::workspace) fn new(ports: Task<()>) -> Self {
        Self {
            skills: None,
            mcp: None,
            flow: None,
            task_live_tick: None,
            _ports: ports,
        }
    }
}

/// `cx.observe_global` subscriptions installed once in the constructor.
///
/// Private by construction: no caller has any business reaching one, and the
/// only reason they are fields at all is that dropping them unsubscribes.
pub(in crate::workspace) struct GlobalObservers {
    _accounts: Subscription,
    _agent_vocabulary: Subscription,
    _mcp: Subscription,
    _settings: Subscription,
    _skills: Subscription,
    _tasks: Subscription,
    _updater: Option<Subscription>,
}

impl GlobalObservers {
    #[allow(clippy::too_many_arguments)]
    pub(in crate::workspace) fn new(
        accounts: Subscription,
        agent_vocabulary: Subscription,
        mcp: Subscription,
        settings: Subscription,
        skills: Subscription,
        tasks: Subscription,
        updater: Option<Subscription>,
    ) -> Self {
        Self {
            _accounts: accounts,
            _agent_vocabulary: agent_vocabulary,
            _mcp: mcp,
            _settings: settings,
            _skills: skills,
            _tasks: tasks,
            _updater: updater,
        }
    }
}
