//! Optional text history kept outside the workspace's structural transaction.

use super::{WorkspaceStore, WorkspaceUuid};
use crate::persistence::{delete_recoverable_json, load_recoverable_json, save_recoverable_json};
use std::{collections::BTreeMap, io, path::PathBuf};

const PER_PANE: usize = 64 * 1024;
const TOTAL: usize = 8 * 1024 * 1024;

impl WorkspaceStore {
    fn terminal_snapshot_path(&self, workspace: WorkspaceUuid) -> PathBuf {
        super::persistence::workspaces_dir_in(&self.root)
            .join(format!("{}.terminal.json", workspace.as_inner()))
    }

    pub fn save_terminal_snapshots(
        &self,
        workspace: WorkspaceUuid,
        snapshots: &BTreeMap<u64, String>,
    ) -> io::Result<()> {
        validate(snapshots)?;
        let path = self.terminal_snapshot_path(workspace);
        save_recoverable_json(
            path.parent().expect("Snapshot has a parent"),
            &path,
            snapshots,
        )
    }

    pub fn load_terminal_snapshots(
        &self,
        workspace: WorkspaceUuid,
    ) -> io::Result<BTreeMap<u64, String>> {
        let snapshots = load_recoverable_json(
            "terminal-snapshots",
            &self.terminal_snapshot_path(workspace),
        )?
        .unwrap_or_default();
        validate(&snapshots)?;
        Ok(snapshots)
    }

    pub fn delete_terminal_snapshots(&self, workspace: WorkspaceUuid) -> io::Result<()> {
        delete_recoverable_json(&self.terminal_snapshot_path(workspace))
    }
}

fn validate(snapshots: &BTreeMap<u64, String>) -> io::Result<()> {
    if snapshots.len() > 512
        || snapshots.values().any(|text| text.len() > PER_PANE)
        || snapshots.values().map(String::len).sum::<usize>() > TOTAL
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Terminal snapshots exceeded their storage limit",
        ));
    }
    Ok(())
}
