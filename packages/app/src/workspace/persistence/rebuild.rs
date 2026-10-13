//! `rebuild_layout`: materializes a serialized pane tree into live panes.

use crate::workspace::main_area::file_view_pane::{DiffSource, FileOrigin};
use std::collections::HashMap;

use daruda_store::project::PaneCwd;
use gpui::{Context, Window};

use super::{
    deserialize_chat_content_width, deserialize_chat_tail_window, deserialize_view_mode,
    effective_cwd, normalize_ratios, restored_agent_account, restored_terminal_account,
};
use crate::agent::launch_resolve::account_recipe_for_connect;
use crate::workspace::Workspace;
use crate::workspace::main_area::pane::{self, PaneSpawnError};
use crate::workspace::main_area::pane_tree::{self as pane_tree, PaneLayout, SplitDirection};
use daruda_chat::view::ChatPaneChoices;

impl Workspace {
    /// Recursively materialize a serialized layout into live panes. `None`
    /// when nothing of it is restored any more.
    /// `id_map` records the new PaneId per serialized pane_id so
    /// cross-references (focus) can be rewritten. `fallback_cwd` is used
    /// when a leaf's serialized cwd is missing — typically the owning
    /// lane's path, keeping each pane inside its lane.
    pub(in crate::workspace) fn rebuild_layout(
        &mut self,
        slayout: &daruda_store::project::SerializedLayout,
        fallback_cwd: Option<&std::path::Path>,
        id_map: &mut HashMap<u64, pane_tree::PaneId>,
        scratch: &mut Vec<pane::Pane>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Option<PaneLayout>, PaneSpawnError> {
        match slayout {
            daruda_store::project::SerializedLayout::Leaf { pane_id, content } => {
                use daruda_store::project::SerializedPaneContent as Content;
                let pane = match content {
                    Content::File(fc) => {
                        // File pane — `live_status` is not persisted; the git
                        // badge re-derives on the next `refresh_git_status`, via
                        // `sync_file_pane_statuses` (a range pane carries its
                        // own letter in its source). Content stays `Loading`
                        // until the owning lane becomes active and
                        // `load_pending_file_panes` fires.
                        // The lane is the one being restored, whatever id the
                        // file recorded. A path outside it is read for
                        // reference — older sessions did not mark those.
                        let inside = fallback_cwd
                            .is_some_and(|root| daruda_core::path::is_within(&fc.path, root));
                        let origin = if fc.reference || !inside {
                            FileOrigin::Reference
                        } else {
                            FileOrigin::Lane
                        };
                        self.create_file_pane(
                            origin,
                            fc.path.clone(),
                            DiffSource::from_serialized(fc),
                            None,
                            deserialize_view_mode(fc.view_mode),
                            window,
                            cx,
                        )
                    }
                    Content::AgentChat(ac) => {
                        // AgentChat pane — re-opens dormant (`Idle`) at the saved
                        // cwd, seeded with the persisted session id + title. The
                        // live session is not started here; `focus_pane` connects
                        // it lazily on first focus, resuming via `session/load`
                        // when a session id is present.
                        //
                        // Resolve which agent the pane relaunches under: its own
                        // owning agent when still in the catalog (session id kept),
                        // else the default agent (session id dropped — it belongs
                        // to a now-absent agent and could not resume).
                        let task_chat = ac.access.is_read_only()
                            || ac.session_id.is_some()
                                && cx
                                    .global::<crate::agent::tasks_global::GlobalTasks>()
                                    .tasks
                                    .iter()
                                    .any(|task| {
                                        task.agent_surface
                                            == daruda_store::tasks::TaskAgentSurface::AgentChat
                                            && task.execution.as_ref().is_some_and(|run| {
                                                ac.agent_id.as_ref() == Some(&run.agent_id)
                                                    && ac.session_id == run.session_id
                                                    && ac.account_id == run.account_id
                                                    && ac
                                                        .cwd
                                                        .as_ref()
                                                        .and_then(PaneCwd::as_local)
                                                        .is_some_and(|cwd| {
                                                            daruda_core::path::same_path(
                                                                cwd, &run.cwd,
                                                            )
                                                        })
                                            })
                                    });
                        // A task's conversation must never switch agent or account on restore.
                        let (agent_id, keep_session) = if task_chat {
                            // A record without an agent is hand-edited; the
                            // default agent is where every other pane lands.
                            let agent = ac.agent_id.clone().unwrap_or_else(|| {
                                daruda_config::AgentDefinition::claude_default().id
                            });
                            (agent, true)
                        } else {
                            self.resolve_restored_agent(ac.agent_id.clone())
                        };
                        let is_remote = matches!(ac.cwd, Some(PaneCwd::Remote(_)));
                        let pane_recipe = self
                            .agent_launch_for(&agent_id)
                            .and_then(|l| account_recipe_for_connect(&l.launch, is_remote));
                        let mut restored = self.create_agent_chat_pane(
                            ac.cwd.clone(),
                            if keep_session {
                                ac.session_id.clone()
                            } else {
                                None
                            },
                            agent_id,
                            ac.title.clone(),
                            window,
                            cx,
                        );
                        // The constructor seeds the domain default — patch in
                        // the persisted selection now that the pane exists.
                        let account = if task_chat {
                            daruda_store::accounts::AccountSelection::from_persisted(ac.account_id)
                        } else {
                            restored_agent_account(ac.account_id, pane_recipe, &self.accounts)
                        };
                        if let Some(content) = restored.agent_chat_content_mut() {
                            content
                                .view
                                .update(cx, |view, _| view.set_access(ac.access.clone()));
                            content.account = account;
                            let content_width =
                                ac.content_width.map(deserialize_chat_content_width);
                            // Missing pane choices retain the constructor's config
                            // seeds — which is also how a file written before the
                            // tail axis split restores: it states the step level
                            // and leaves the call level following config.
                            let tail_steps = ac.tail_window.map(deserialize_chat_tail_window);
                            let tail_calls = ac.tail_window_calls.map(deserialize_chat_tail_window);
                            // The current field wins; the superseded one is read
                            // only for a file written before the split, where an
                            // empty list meant "unfiltered" rather than "nothing".
                            use daruda_transcript::display_filter::DisplayFilter;
                            let display_filter = ac
                                .visible_kinds
                                .as_ref()
                                .map(|tokens| DisplayFilter::from_stored(tokens))
                                .or_else(|| {
                                    ac.display_filter
                                        .as_ref()
                                        .map(|tokens| DisplayFilter::from_legacy_tokens(tokens))
                                });
                            let fold_mode = ac.fold_mode.as_ref().map(|tokens| {
                                daruda_transcript::fold_mode::FoldMode::from_tokens(
                                    tokens.iter().map(String::as_str),
                                )
                            });
                            let choices = ChatPaneChoices {
                                mode_id: ac.picked_mode_id.clone(),
                                model_id: ac.model_id.clone(),
                                content_width,
                                tail_steps,
                                tail_calls,
                                display_filter,
                                fold_mode,
                            };
                            content
                                .view
                                .update(cx, |v, _| v.restore_pane_choices(choices));
                        }
                        restored
                    }
                    // A flow graph is the Flows page's detail now, not a
                    // lane's tab: a leaf an older session saved drops out,
                    // and its split closes up around it.
                    Content::FlowGraph(_) => return Ok(None),
                    Content::Terminal { cwd, account_id } => {
                        let effective = effective_cwd(cwd.clone(), fallback_cwd);
                        let account = restored_terminal_account(*account_id, &self.accounts);
                        // Terminal pane: no agent, so no required auth domain —
                        // the account's own recipe decides the env.
                        let prepared = self.resolve_account(account, pane::AccountDomain::Any);
                        self.create_pane_with_cwd(
                            effective,
                            account,
                            prepared.as_ref(),
                            window,
                            cx,
                        )?
                    }
                };
                let new_id = pane.id;
                id_map.insert(*pane_id, new_id);
                scratch.push(pane);
                Ok(Some(PaneLayout::Pane(new_id)))
            }
            daruda_store::project::SerializedLayout::Split {
                direction,
                children,
                ratios,
            } => {
                let dir = match direction {
                    daruda_store::project::SplitDirectionSerde::Horizontal => {
                        SplitDirection::Horizontal
                    }
                    daruda_store::project::SplitDirectionSerde::Vertical => {
                        SplitDirection::Vertical
                    }
                };
                if children.is_empty() {
                    // Degenerate serialization — materialize a fresh leaf
                    // so we never surface an empty Split to the renderer.
                    // A degenerate Split materializes a terminal, which has
                    // no agent and so no domain default to inherit.
                    let account = self.default_account_selection_for_new_pane(None);
                    let prepared = self.resolve_account(account, pane::AccountDomain::Any);
                    let pane = self.create_pane_with_cwd(
                        fallback_cwd.map(|p| p.to_path_buf()),
                        account,
                        prepared.as_ref(),
                        window,
                        cx,
                    )?;
                    let id = pane.id;
                    scratch.push(pane);
                    return Ok(Some(PaneLayout::Pane(id)));
                }
                // A dropped child takes its share with it, so the ratios are
                // those of the children that came back.
                let mut rebuilt = Vec::with_capacity(children.len());
                let mut kept_ratios = Vec::with_capacity(children.len());
                for (ix, c) in children.iter().enumerate() {
                    if let Some(layout) =
                        self.rebuild_layout(c, fallback_cwd, id_map, scratch, window, cx)?
                    {
                        rebuilt.push(layout);
                        if let Some(ratio) = ratios.get(ix) {
                            kept_ratios.push(*ratio);
                        }
                    }
                }
                match rebuilt.len() {
                    0 => Ok(None),
                    1 => Ok(rebuilt.pop()),
                    n => Ok(Some(PaneLayout::Split {
                        direction: dir,
                        children: rebuilt,
                        ratios: normalize_ratios(&kept_ratios, n),
                    })),
                }
            }
        }
    }
}
