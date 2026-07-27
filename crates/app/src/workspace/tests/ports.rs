use gpui::TestAppContext;

use super::*;
use crate::lane::port_attribution::AttributionConfidence;
use crate::workspace::sync::ports::{ListeningPort, PortKind, PortScanStatus};

#[gpui::test]
fn set_scanned_ports_attributes_to_the_bootstrapped_lane(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().unwrap();
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path(temp.path());
    let (_wh, ws) = build_workspace_with(cx, &config, Some(project));

    let (expected_label, lane_path) = ws.read_with(cx, |ws, _| {
        let project = &ws.projects[0];
        let lane = &project.lanes[0];
        (
            format!("{}/{}", project.name, lane.display_name()),
            lane.path.clone(),
        )
    });

    ws.update(cx, |ws, cx| {
        ws.set_scanned_ports(
            vec![ListeningPort {
                port: 3000,
                address: "*:3000".to_string(),
                pid: 1,
                process_name: None,
                cwd: Some(lane_path),
                command: None,
            }],
            cx,
        )
    });

    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.port_scan_status, PortScanStatus::Available);
        assert_eq!(ws.attributed_ports.len(), 1);
        assert_eq!(ws.attributed_ports[0].port, 3000);
        assert!(matches!(
            &ws.attributed_ports[0].kind,
            PortKind::Workspace {
                lane_label,
                confidence: AttributionConfidence::Cwd,
            } if lane_label == &expected_label
        ));
    });
}

#[gpui::test]
fn set_scanned_ports_leaves_unmatched_port_unattributed(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().unwrap();
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path(temp.path());
    let (_wh, ws) = build_workspace_with(cx, &config, Some(project));

    ws.update(cx, |ws, cx| {
        ws.set_scanned_ports(
            vec![ListeningPort {
                port: 4000,
                address: "*:4000".to_string(),
                pid: 1,
                process_name: None,
                cwd: Some(std::path::PathBuf::from("/completely/unrelated")),
                command: None,
            }],
            cx,
        )
    });

    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.port_scan_status, PortScanStatus::Available);
        assert_eq!(ws.attributed_ports.len(), 1);
        assert!(matches!(&ws.attributed_ports[0].kind, PortKind::External));
    });
}

#[gpui::test]
fn set_scanned_ports_classifies_container_processes_separately(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().unwrap();
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path(temp.path());
    let (_wh, ws) = build_workspace_with(cx, &config, Some(project));

    ws.update(cx, |ws, cx| {
        ws.set_scanned_ports(
            vec![ListeningPort {
                port: 5000,
                address: "*:5000".to_string(),
                pid: 1,
                process_name: Some("com.docker.backend".to_string()),
                cwd: Some(std::path::PathBuf::from("/Applications/Docker.app")),
                command: None,
            }],
            cx,
        )
    });

    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.port_scan_status, PortScanStatus::Available);
        assert_eq!(ws.attributed_ports.len(), 1);
        assert!(matches!(&ws.attributed_ports[0].kind, PortKind::Container));
    });
}

#[gpui::test]
fn set_scanned_ports_stores_entries_in_stable_display_order(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().unwrap();
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path(temp.path());
    let (_wh, ws) = build_workspace_with(cx, &config, Some(project));

    let lane_path = ws.read_with(cx, |ws, _| ws.projects[0].lanes[0].path.clone());

    ws.update(cx, |ws, cx| {
        ws.set_scanned_ports(
            vec![
                ListeningPort {
                    port: 7000,
                    address: "*:7000".to_string(),
                    pid: 1,
                    process_name: Some("redis".to_string()),
                    cwd: Some(std::path::PathBuf::from("/outside")),
                    command: None,
                },
                ListeningPort {
                    port: 3000,
                    address: "127.0.0.1:3000".to_string(),
                    pid: 2,
                    process_name: Some("node".to_string()),
                    cwd: Some(lane_path),
                    command: None,
                },
                ListeningPort {
                    port: 5000,
                    address: "*:5000".to_string(),
                    pid: 3,
                    process_name: Some("com.docker.backend".to_string()),
                    cwd: Some(std::path::PathBuf::from("/Applications/Docker.app")),
                    command: None,
                },
                ListeningPort {
                    port: 4000,
                    address: "*:4000".to_string(),
                    pid: 4,
                    process_name: Some("postgres".to_string()),
                    cwd: Some(std::path::PathBuf::from("/outside")),
                    command: None,
                },
            ],
            cx,
        )
    });

    ws.read_with(cx, |ws, _| {
        assert_eq!(
            ws.attributed_ports
                .iter()
                .map(|entry| entry.port)
                .collect::<Vec<_>>(),
            vec![3000, 5000, 4000, 7000]
        );
    });
}

#[gpui::test]
fn set_scanned_ports_skips_notify_when_unchanged(cx: &mut TestAppContext) {
    let temp = tempfile::tempdir().unwrap();
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path(temp.path());
    let (_wh, ws) = build_workspace_with(cx, &config, Some(project));

    let ports = vec![ListeningPort {
        port: 4000,
        address: "*:4000".to_string(),
        pid: 1,
        process_name: None,
        cwd: Some(std::path::PathBuf::from("/completely/unrelated")),
        command: None,
    }];
    ws.update(cx, |ws, cx| ws.set_scanned_ports(ports.clone(), cx));
    let first = ws.read_with(cx, |ws, _| ws.attributed_ports.clone());
    ws.update(cx, |ws, cx| ws.set_scanned_ports(ports, cx));
    let second = ws.read_with(cx, |ws, _| ws.attributed_ports.clone());
    assert_eq!(first, second);
}
