use gpui::TestAppContext;

use super::*;
use crate::lane::port_attribution::AttributionConfidence;
use crate::workspace::sync::ports::{ListeningPort, PortKind, PortScanStatus};

#[gpui::test]
fn set_scanned_ports_classifies_orders_and_skips_unchanged_updates(cx: &mut TestAppContext) {
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

    let ports = vec![
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
    ];
    ws.update(cx, |ws, cx| ws.set_scanned_ports(ports.clone(), cx));

    ws.read_with(cx, |ws, _| {
        assert_eq!(ws.port_scan_status, PortScanStatus::Available);
        assert_eq!(
            ws.attributed_ports
                .iter()
                .map(|entry| entry.port)
                .collect::<Vec<_>>(),
            vec![3000, 5000, 4000, 7000]
        );
        assert!(matches!(
            &ws.attributed_ports[0].kind,
            PortKind::Workspace {
                lane_label,
                confidence: AttributionConfidence::Cwd,
            } if lane_label == &expected_label
        ));
        assert!(matches!(&ws.attributed_ports[1].kind, PortKind::Container));
        assert!(matches!(&ws.attributed_ports[2].kind, PortKind::External));
        assert!(matches!(&ws.attributed_ports[3].kind, PortKind::External));
    });

    let first = ws.read_with(cx, |ws, _| ws.attributed_ports.clone());
    ws.update(cx, |ws, cx| ws.set_scanned_ports(ports, cx));
    let second = ws.read_with(cx, |ws, _| ws.attributed_ports.clone());
    assert_eq!(first, second);
}
