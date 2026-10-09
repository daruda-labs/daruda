use super::agent_row_transport_error;
use crate::lane::session_host::{SessionHostError, SessionHostField};

fn agent_row_is_valid(kind: &str, host: &str, container: &str) -> bool {
    agent_row_transport_error(kind, host, container).is_none()
}

#[test]
fn agent_row_validation_cases() {
    let cases = [
        ("ssh empty host", "ssh", "", "", false),
        ("ssh blank host", "ssh", "   ", "", false),
        ("ssh host", "ssh", "vm-work", "", true),
        ("docker empty container", "docker", "", "", false),
        ("docker blank container", "docker", "", "   ", false),
        ("docker container", "docker", "", "ubuntu-dev", true),
        ("raw empty", "raw", "", "", true),
        (
            "raw ignores host and container",
            "raw",
            "irrelevant",
            "irrelevant",
            true,
        ),
        ("empty kind", "", "", "", true),
        ("bogus kind", "bogus", "", "", true),
    ];

    for (name, kind, host, container, expected) in cases {
        assert_eq!(
            agent_row_is_valid(kind, host, container),
            expected,
            "{name}"
        );
    }
}

/// A host/container is a bare word in the launch command daruda
/// assembles, so the field has to reject what `SessionHostModal`'s does —
/// non-emptiness alone would let a typed value carry its own `ssh` flags
/// or a `;` into that command line.
#[test]
fn a_host_that_would_break_the_launch_command_is_refused() {
    let cases = [
        (
            "ssh flags",
            "ssh",
            "vm -o ProxyCommand=touch /tmp/pwned",
            "",
            SessionHostError::Unsafe(SessionHostField::Target),
        ),
        (
            "ssh semicolon",
            "ssh",
            "vm; echo PWNED",
            "",
            SessionHostError::Unsafe(SessionHostField::Target),
        ),
        (
            "docker flags",
            "docker",
            "",
            "dev --privileged",
            SessionHostError::Unsafe(SessionHostField::Container),
        ),
    ];
    for (name, kind, host, container, expected) in cases {
        assert_eq!(
            agent_row_transport_error(kind, host, container),
            Some(expected),
            "{name}"
        );
    }
}
