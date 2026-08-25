//! What the node pinned, negotiated with the adapter that has to honour it.
//!
//! Every axis is confirmed before the next is applied, because an adapter
//! rebuilding its effort list per model only offers the right one once the
//! model has landed — so this is a conversation, not a batch of settings.
//! `runner::acp::tests::settings` is the other half of it.

use super::{AcpRunner, ENDED_EARLY};
use crate::model::AgentSpec;
use crate::runner::{NodeFailure, sleep};
use daruda_acp::{
    AcpEvent, AcpSessionHandle, ConfigOptionCategoryView, ConfigOptionKindView, ConfigOptionView,
    ConfigValueView, ModeStateView, UsageView,
};
use smol::stream::{Stream, StreamExt};
use std::cell::RefCell;

impl AcpRunner {
    /// Apply each axis the node pinned, in order, and confirm each before
    /// moving on. Every failure here is [`NodeFailure::UnsupportedSetting`]:
    /// not advertised, not among the offered values, or requested and not
    /// applied all leave the node running as something other than its record.
    pub(super) async fn apply(
        &self,
        events: &mut (impl Stream<Item = AcpEvent> + Unpin),
        session: &AcpSessionHandle,
        mut options: Vec<ConfigOptionView>,
        agent: &AgentSpec,
        usage: &RefCell<Option<UsageView>>,
    ) -> Result<(), NodeFailure> {
        for want in requested(agent) {
            let unsupported = |available: Vec<String>| NodeFailure::UnsupportedSetting {
                field: want.field,
                value: want.value.to_string(),
                available,
            };
            let Some(offer) = advertised(&options, want.category) else {
                return Err(unsupported(Vec::new()));
            };
            if !offer.choices.iter().any(|c| c == want.value) {
                return Err(unsupported(offer.choices));
            }
            session.set_config_option(offer.id, ConfigValueView::Id(want.value.to_string()));

            // The reply carries the agent's whole option set, so it replaces
            // what the next axis reads: an adapter that rebuilds its effort
            // list per model only offers the right one after the model lands.
            //
            // The budget is the backstop, not the mechanism: an adapter that
            // refuses says so, and waiting out 30s to conclude what it
            // already told us is the whole of this node's remaining time
            // spent learning nothing.
            let confirmed = smol::future::or(
                async { Some(announced(events, usage, Announcement::OptionsChanged).await) },
                async {
                    sleep(self.settings_budget).await;
                    None
                },
            )
            .await;
            options = match confirmed {
                Some(announced) => announced?.options,
                None => return Err(unsupported(offer.choices)),
            };
            match advertised(&options, want.category) {
                Some(now) if now.current == want.value => {}
                Some(now) => return Err(unsupported(now.choices)),
                None => return Err(unsupported(Vec::new())),
            }
        }
        Ok(())
    }
}

/// One axis a node pinned, and the category the adapter advertises it under.
/// `mode` is absent: it travels as `initial_modes` at connect time, and
/// `daruda_acp` strips the mode option out of every set the host sees.
struct Requested<'a> {
    field: &'static str,
    category: ConfigOptionCategoryView,
    value: &'a str,
}

/// A node that pins a mode must actually be in it. `daruda_acp` degrades an
/// unavailable or rejected mode to a fallback and only emits a `Notice`
/// (`session.rs`'s connect path), so a flow claiming `bypassPermissions`
/// can otherwise run in `auto` with nothing disagreeing.
///
/// Checked ahead of model and effort because this is the axis that decides
/// what the agent is allowed to do, not merely how well it does it.
pub(super) fn check_mode(
    agent: &AgentSpec,
    modes: Option<&ModeStateView>,
) -> Result<(), NodeFailure> {
    let Some(want) = agent.mode.as_deref() else {
        return Ok(());
    };
    let unsupported = |available: Vec<String>| NodeFailure::UnsupportedSetting {
        field: "mode",
        value: want.to_string(),
        available,
    };
    // No mode state at all is an agent that advertises none — it cannot be
    // in the requested one.
    let Some(state) = modes else {
        return Err(unsupported(Vec::new()));
    };
    if state.current == want {
        return Ok(());
    }
    Err(unsupported(
        state.available.iter().map(|m| m.id.clone()).collect(),
    ))
}

/// What one announcement carried. `modes` only ever arrives with
/// `Connected` — `ConfigOptionsChanged` replaces options, not modes.
pub(super) struct Announced {
    pub(super) options: Vec<ConfigOptionView>,
    pub(super) modes: Option<ModeStateView>,
}

/// What the node asked for, in the order it is applied.
fn requested(agent: &AgentSpec) -> Vec<Requested<'_>> {
    [
        (
            "model",
            ConfigOptionCategoryView::Model,
            agent.model.as_deref(),
        ),
        (
            "effort",
            ConfigOptionCategoryView::ThoughtLevel,
            agent.effort.as_deref(),
        ),
    ]
    .into_iter()
    .filter_map(|(field, category, value)| {
        Some(Requested {
            field,
            category,
            value: value?,
        })
    })
    .collect()
}

/// What the adapter offers on one axis. Owned because the option set it was
/// read from is replaced by the agent's next reply.
struct Advertised {
    id: String,
    current: String,
    choices: Vec<String>,
}

/// The selectable option advertised for `category`. A boolean where a choice
/// of named values belongs is as unusable as a missing option, so both read as
/// "not advertised" rather than as something to coerce.
fn advertised(
    options: &[ConfigOptionView],
    category: ConfigOptionCategoryView,
) -> Option<Advertised> {
    let option = options.iter().find(|o| o.category == category)?;
    let ConfigOptionKindView::Select {
        current_value,
        options: choices,
    } = &option.kind
    else {
        return None;
    };
    Some(Advertised {
        id: option.id.clone(),
        current: current_value.clone(),
        choices: choices.iter().map(|c| c.value.clone()).collect(),
    })
}

/// Which announcement the runner is waiting on before it can go further.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Announcement {
    /// The session is up, and the adapter has said what it offers.
    Connected,
    /// The agent re-advertised its options — the only confirmation a
    /// `set_config_option` gets, and a full replacement of the set.
    OptionsChanged,
}

/// Read events until the agent announces `wanted`, carrying its option set
/// out. Everything else describes a conversation nobody is here to watch —
/// except usage, the run's only cost meter, and a terminal error.
pub(super) async fn announced(
    events: &mut (impl Stream<Item = AcpEvent> + Unpin),
    usage: &RefCell<Option<UsageView>>,
    wanted: Announcement,
) -> Result<Announced, NodeFailure> {
    loop {
        let Some(event) = events.next().await else {
            return Err(NodeFailure::SessionError(ENDED_EARLY.to_string()));
        };
        match event {
            AcpEvent::Connected {
                config_options,
                modes,
                ..
            } if wanted == Announcement::Connected => {
                return Ok(Announced {
                    options: config_options,
                    modes,
                });
            }
            AcpEvent::ConfigOptionsChanged(options) if wanted == Announcement::OptionsChanged => {
                return Ok(Announced {
                    options,
                    modes: None,
                });
            }
            AcpEvent::UsageChanged(reported) => *usage.borrow_mut() = Some(reported),
            // The adapter said no. Waiting for a confirmation it has
            // already refused to send is the whole of the settings budget
            // spent learning what this event just said.
            AcpEvent::ConfigOptionRejected { config_id, reason }
                if wanted == Announcement::OptionsChanged =>
            {
                return Err(NodeFailure::SettingRejected { config_id, reason });
            }
            // `NodeFailure` still carries a message; the classified failure is
            // rendered through it. Threading the classification into the
            // retry policy is a follow-up, not this change.
            AcpEvent::Error(failure) => {
                return Err(NodeFailure::SessionError(failure.to_string()));
            }
            _ => {}
        }
    }
}
