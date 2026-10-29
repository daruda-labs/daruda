//! Load and normalize durable or legacy history without exposing mutable projections.

use super::*;

impl RunHistory {
    pub fn read(dir: &Path) -> Result<Self, HistoryError> {
        let inspected = reader::inspect(dir).map_err(HistoryError)?;
        let state = inspected.as_ref().map(|i| i.state.clone());
        let started_at = inspected.as_ref().and_then(|i| i.started_at);
        let elapsed = inspected.as_ref().and_then(|i| i.elapsed);
        let in_flight = inspected
            .as_ref()
            .map(|i| {
                i.state
                    .node_states()
                    .filter_map(|(node, state)| match state.last() {
                        crate::run_log::machine::AttemptState::Started {
                            attempt,
                            evidence,
                            ..
                        } => Some((node.clone(), *attempt, *evidence)),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let (replay, phase, sessions, source, current_outputs) = match inspected {
            Some(i) => (
                i.replay,
                Some(i.state.phase()),
                i.state.sessions().cloned().collect(),
                i.state.source().map(Path::to_path_buf),
                i.state.finished(),
            ),
            None => {
                let replay = reader::read(dir);
                let current = replay.passed.iter().cloned().collect();
                (replay, None, Vec::new(), None, current)
            }
        };
        let flow = std::fs::read_to_string(dir.join(crate::resume::RUN_SPEC_FILE))
            .ok()
            .and_then(|text| crate::load(&text, None).ok())
            .map(|loaded| loaded.flow().clone());
        let mut result = Self {
            flow,
            started_at,
            elapsed,
            spent: replay.spent,
            records: replay.records,
            sessions,
            source,
            phase,
            in_flight,
            current_outputs,
            state,
            evidence_paths: Default::default(),
            attempt_order: replay.attempt_order,
            torn: replay.torn,
        };
        let nodes: std::collections::BTreeSet<_> = result
            .records
            .iter()
            .map(|r| r.id.clone())
            .chain(result.sessions.iter().map(|s| s.node.clone()))
            .chain(result.in_flight.iter().map(|(node, _, _)| node.clone()))
            .collect();
        for node in nodes {
            for (attempt, evidence) in result.attempts(&node) {
                let logs = dir.join(crate::schedule::LOG_DIR_NAME);
                let command = result.is_command(&node);
                let path = if command {
                    logs.join(format!("{node}.attempt-{attempt}.evidence-{evidence}.log"))
                } else {
                    let [modern, legacy] =
                        crate::runner::acp::transcript_paths(&logs, &node, attempt, evidence);
                    if !modern.exists()
                        && legacy.exists()
                        && !result.legacy_transcript_is_output(&node, &legacy)
                    {
                        legacy
                    } else {
                        modern
                    }
                };
                result.evidence_paths.insert((node.clone(), evidence), path);
            }
        }
        Ok(result)
    }

    fn legacy_transcript_is_output(&self, node: &NodeId, path: &Path) -> bool {
        let output = self
            .flow
            .as_ref()
            .and_then(|flow| flow.nodes.iter().find(|n| &n.id == node))
            .and_then(|n| n.kind.agent_body())
            .map(|body| body.output.as_path());
        if output.is_some_and(|p| p.extension().is_none_or(|ext| ext != "md")) {
            return false;
        }
        // Old Markdown archives shared the transcript name. Without a snapshot,
        // an invalidation naming that path is ambiguous and cannot prove a conversation.
        self.records
            .iter()
            .flat_map(|r| &r.attempts)
            .any(|attempt| {
                attempt.invalidated.nodes.contains(node)
                    && path.file_name()
                        == Some(std::ffi::OsStr::new(&crate::archive::archive_name(
                            node,
                            &format!(
                                "attempt-{}.evidence-{}",
                                attempt.attempt, attempt.evidence_seq
                            ),
                            output.unwrap_or(Path::new("output.md")),
                        )))
                    && attempt.invalidated.archived.iter().any(|p| p == path)
            })
    }
}

#[cfg(test)]
mod tests {}
