//! Public API tests for port attribution.

use daruda_project::lane::port_attribution::*;
use std::path::PathBuf;

#[test]
fn matches_exact_cwd() {
    let lanes = vec![lane("/repo/app", "app")];
    let ports = vec![scanned(3000, Some("/repo/app"), None)];
    let result = attribute(&ports, &lanes);
    let owner = result[0].owner.as_ref().expect("expected an owner");
    assert_eq!(owner.lane_label, "app");
    assert_eq!(owner.confidence, AttributionConfidence::Cwd);
}

#[test]
fn matches_subdirectory_cwd() {
    let lanes = vec![lane("/repo/app", "app")];
    let ports = vec![scanned(3000, Some("/repo/app/src/server"), None)];
    let result = attribute(&ports, &lanes);
    assert_eq!(result[0].owner.as_ref().unwrap().lane_label, "app");
}

#[test]
fn does_not_match_sibling_with_shared_prefix() {
    let lanes = vec![lane("/repo/app", "app")];
    let ports = vec![scanned(3000, Some("/repo/app-2"), None)];
    let result = attribute(&ports, &lanes);
    assert_eq!(result[0].owner, None);
}

#[test]
fn nested_worktree_picks_deepest_match() {
    let lanes = vec![
        lane("/repo", "repo-main"),
        lane("/repo/nested-worktree", "repo-feature"),
    ];
    let ports = vec![scanned(3000, Some("/repo/nested-worktree/src"), None)];
    let result = attribute(&ports, &lanes);
    let owner = result[0].owner.as_ref().expect("expected an owner");
    assert_eq!(owner.lane_label, "repo-feature");
    assert_eq!(owner.confidence, AttributionConfidence::Cwd);
}

#[test]
fn falls_back_to_command_line_when_cwd_unmatched() {
    let lanes = vec![lane("/repo/app", "app")];
    let ports = vec![scanned(
        3000,
        Some("/tmp"),
        Some("node /repo/app/server.js"),
    )];
    let result = attribute(&ports, &lanes);
    let owner = result[0].owner.as_ref().expect("expected an owner");
    assert_eq!(owner.lane_label, "app");
    assert_eq!(owner.confidence, AttributionConfidence::Command);
}

#[test]
fn command_line_fallback_picks_deepest_match() {
    let lanes = vec![
        lane("/repo", "repo-main"),
        lane("/repo/worktrees/feature", "repo-feature"),
    ];
    let ports = vec![scanned(
        3000,
        Some("/tmp"),
        Some("node /repo/worktrees/feature/server.js"),
    )];
    let result = attribute(&ports, &lanes);
    let owner = result[0].owner.as_ref().expect("expected an owner");
    assert_eq!(owner.lane_label, "repo-feature");
    assert_eq!(owner.confidence, AttributionConfidence::Command);
}

#[test]
fn command_line_fallback_respects_word_boundary() {
    let lanes = vec![lane("/repo/app", "app")];
    let ports = vec![scanned(
        3000,
        Some("/tmp"),
        Some("node /repo/app-2/server.js"),
    )];
    let result = attribute(&ports, &lanes);
    assert_eq!(result[0].owner, None);
}

#[test]
fn no_cwd_no_command_is_unattributed() {
    let lanes = vec![lane("/repo/app", "app")];
    let ports = vec![scanned(3000, None, None)];
    let result = attribute(&ports, &lanes);
    assert_eq!(result[0].owner, None);
}

#[test]
fn unmatched_port_returns_none_rather_than_dropped() {
    let lanes = vec![lane("/repo/app", "app")];
    let ports = vec![
        scanned(3000, Some("/repo/app"), None),
        scanned(4000, Some("/unrelated"), None),
    ];
    let result = attribute(&ports, &lanes);
    assert_eq!(result.len(), 2);
    assert_eq!(result[1].port, 4000);
    assert_eq!(result[1].owner, None);
}

fn lane(path: &str, label: &str) -> LaneCandidate {
    LaneCandidate {
        path: PathBuf::from(path),
        label: label.to_string(),
    }
}

fn scanned(port: u16, cwd: Option<&str>, command: Option<&str>) -> ScannedPort {
    ScannedPort {
        port,
        cwd: cwd.map(PathBuf::from),
        command: command.map(str::to_string),
    }
}
