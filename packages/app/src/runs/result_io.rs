//! Read-only enrichment of result snapshots, used off the UI thread by queries.
use super::result::{self, EndedAs, Pointers, RunKey, RunSnapshot, RunState};
use daruda_core::text::bound_agent_text;
use std::path::Path;

pub(super) fn flow_spec(dir: &Path) -> Option<daruda_flow::parse::FlowFile> {
    std::fs::read_to_string(dir.join(daruda_flow::resume::RUN_SPEC_FILE))
        .ok()
        .and_then(|text| daruda_flow::parse::parse_flow_file(&text).ok())
}

pub(super) fn flow_base(dir: &Path) -> RunSnapshot {
    let id = dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let what = flow_spec(dir)
        .and_then(|spec| spec.name)
        .unwrap_or_else(|| id.clone());
    RunSnapshot {
        key: RunKey::Flow(id),
        what,
        why: None,
        state: RunState::Unknown,
    }
}

pub(super) fn flow_pointers(
    dir: &Path,
    spec: Option<&daruda_flow::parse::FlowFile>,
    until: Option<&daruda_flow::NodeId>,
) -> Pointers {
    let outputs = spec
        .map(|file| {
            let mut selected = std::collections::HashSet::new();
            let mut pending = until.into_iter().cloned().collect::<Vec<_>>();
            while let Some(id) = pending.pop() {
                if selected.insert(id.clone())
                    && let Some(node) = file.nodes.iter().find(|node| node.id == id)
                {
                    pending.extend(node.deps.iter().cloned());
                }
            }
            let includes = |id: &daruda_flow::NodeId| until.is_none() || selected.contains(id);
            file.nodes
                .iter()
                .filter(|node| {
                    includes(&node.id)
                        && !file
                            .nodes
                            .iter()
                            .any(|other| includes(&other.id) && other.deps.contains(&node.id))
                })
                .filter_map(|node| match &node.kind {
                    daruda_flow::parse::NodeKindFile::Agent { output, .. } => {
                        Some(dir.join(output))
                    }
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    Pointers {
        run_dir: Some(dir.to_path_buf()),
        report: Some(dir.join(daruda_flow::record::RUN_REPORT_FILE)),
        outputs,
        ..Default::default()
    }
}

pub(super) fn enrich(mut snapshot: RunSnapshot) -> RunSnapshot {
    if let RunState::Ended { text, pointers, .. } = &mut snapshot.state
        && let Some(report) = &pointers.report
        && let Ok(report) = std::fs::read_to_string(report)
        && let Some(report) = bound_agent_text(&report)
    {
        *text = Some(report);
    }
    snapshot
}

#[cfg(test)]
pub(super) fn flow_disk(location: &super::result::FlowRunLocation) -> RunSnapshot {
    flow_observation(location).snapshot
}

pub(super) struct FlowObservation {
    pub snapshot: RunSnapshot,
    pub resumed_at: Option<u64>,
}

pub(super) fn flow_observation(location: &super::result::FlowRunLocation) -> FlowObservation {
    let dir = &location.run_dir;
    let mut snapshot = flow_base(dir);
    let mut resumed_at = None;
    if !dir.exists() {
        snapshot.state = RunState::Gone;
        return FlowObservation {
            snapshot,
            resumed_at,
        };
    }
    match daruda_flow::run_log::reader::inspect(dir) {
        Err(error) => {
            snapshot.state = RunState::Unreadable {
                reason: error.to_string(),
            }
        }
        Ok(Some(read)) => {
            resumed_at = Some(read.resumed_at);
            snapshot.why = result::flow_why(
                read.replay.until.as_ref(),
                read.replay.pinned.iter().map(ToString::to_string),
            );
            let pointers = flow_pointers(dir, flow_spec(dir).as_ref(), read.replay.until.as_ref());
            snapshot = if let Some(end) = read.end {
                result::flow_ended(snapshot, &end, pointers)
            } else {
                snapshot.state = open_state(dir, location.lock_dir.as_deref(), pointers);
                snapshot
            };
        }
        Ok(None) => {
            resumed_at = Some(0);
            let replay = daruda_flow::run_log::reader::read(dir);
            snapshot.why = result::flow_why(
                replay.until.as_ref(),
                replay.pinned.iter().map(ToString::to_string),
            );
            snapshot.state = open_state(
                dir,
                location.lock_dir.as_deref(),
                flow_pointers(dir, flow_spec(dir).as_ref(), replay.until.as_ref()),
            )
        }
    }
    FlowObservation {
        snapshot: enrich(snapshot),
        resumed_at,
    }
}
fn open_state(dir: &Path, lock_dir: Option<&Path>, pointers: Pointers) -> RunState {
    use daruda_flow::marker::RunStatus;
    let ended_as =
        match daruda_flow::marker::run_status(dir, lock_dir, &daruda_core::process::is_alive) {
            RunStatus::Running => {
                return RunState::Running {
                    progress: Default::default(),
                    waiting_on: None,
                };
            }
            RunStatus::Unknown => return RunState::Unknown,
            RunStatus::Done => EndedAs::Completed,
            RunStatus::Failed => EndedAs::Failed,
            RunStatus::Canceled => EndedAs::Canceled,
            RunStatus::Stalled => EndedAs::Stalled,
            RunStatus::Crashed => EndedAs::Crashed,
        };
    RunState::Ended {
        ended_as,
        text: None,
        pointers,
    }
}

#[cfg(test)]
mod tests;
