//! Result descriptions come from the original submission facts, including resumed pins.
use super::FlowSource;
use crate::runs::result::{Pointers, RunKey, RunSnapshot, RunState, flow_why};
use daruda_flow::request::RunRequest;
use std::path::Path;

pub(super) fn for_request(request: &RunRequest, source: &FlowSource) -> (RunSnapshot, Pointers) {
    let run_dir = &request.run_dir;
    let until = request.until.as_ref().or_else(|| {
        request
            .resume
            .as_ref()
            .and_then(|resume| resume.until.as_ref())
    });
    let selected = daruda_flow::graph::Selection::of(request.loaded.flow(), until);
    let outputs = request
        .loaded
        .flow()
        .nodes
        .iter()
        .filter(|node| {
            selected.includes(&node.id)
                && !request
                    .loaded
                    .flow()
                    .nodes
                    .iter()
                    .any(|other| selected.includes(&other.id) && other.deps.contains(&node.id))
        })
        .filter_map(|node| match &node.kind {
            daruda_flow::model::NodeKind::Agent(body) => Some(run_dir.join(&body.output)),
            _ => None,
        })
        .collect();
    let what = request
        .loaded
        .name()
        .map(str::to_owned)
        .unwrap_or_else(|| source_label(source, run_dir));
    let pinned = request.resume.as_ref().map_or_else(
        || {
            request
                .pinned
                .iter()
                .map(|pin| pin.node.to_string())
                .collect()
        },
        |resume| {
            resume
                .pinned
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        },
    );
    let why = flow_why(until, pinned.into_iter());
    let snapshot = RunSnapshot {
        key: RunKey::Flow(
            run_dir
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
        ),
        what,
        why,
        state: RunState::Unknown,
    };
    let pointers = Pointers {
        run_dir: Some(run_dir.to_path_buf()),
        report: Some(run_dir.join(daruda_flow::record::RUN_REPORT_FILE)),
        outputs,
        ..Default::default()
    };
    (snapshot, pointers)
}

#[cfg(any(test, feature = "screenshot"))]
pub(super) fn flow_metadata(
    dir: &Path,
    source: &FlowSource,
    _selection: Option<()>,
) -> (RunSnapshot, Pointers) {
    let spec = crate::runs::result_io::flow_spec(dir);
    let what = spec
        .as_ref()
        .and_then(|spec| spec.name.clone())
        .unwrap_or_else(|| source_label(source, dir));
    let snapshot = RunSnapshot {
        key: RunKey::Flow(
            dir.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
        ),
        what,
        why: None,
        state: RunState::Unknown,
    };
    (
        snapshot,
        crate::runs::result_io::flow_pointers(dir, spec.as_ref(), None),
    )
}

fn source_label(source: &FlowSource, dir: &Path) -> String {
    match source {
        FlowSource::File(path) => path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        _ => dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unnamed_sources_use_the_file_stem_or_resumed_directory() {
        let dir = Path::new("/runs/resumed");
        assert_eq!(
            source_label(&FlowSource::File("/flows/build.yaml".into()), dir),
            "build"
        );
        assert_eq!(
            source_label(
                &FlowSource::Resumed {
                    run_dir: dir.into()
                },
                dir
            ),
            "resumed"
        );
    }
}
