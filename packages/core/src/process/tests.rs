use std::time::Duration;

use super::*;

#[test]
fn the_last_path_assignment_is_the_childs() {
    let env = [("HOME", "/h"), ("PATH", "/a"), ("X", "1"), ("PATH", "/b")];
    assert_eq!(
        child_path(env.iter().map(|(k, v)| (k, v))),
        Some(OsStr::new("/b"))
    );
    let none = [("HOME", "/h")];
    assert_eq!(child_path(none.iter().map(|(k, v)| (k, v))), None);
}

#[test]
fn a_path_or_an_unknown_name_resolves_to_itself() {
    let path = std::path::Path::new("some/dir/tool");
    assert_eq!(resolve_program(path.as_os_str(), None), path);
    let unknown = OsStr::new("daruda-no-such-program-anywhere");
    assert_eq!(
        resolve_program(unknown, None),
        std::path::Path::new(unknown)
    );
}

/// What Windows needs PATHEXT for: `npx` is `npx.cmd`, found in the PATH
/// the child will get — not this process's.
#[cfg(windows)]
#[test]
fn a_bare_name_finds_a_cmd_shim_in_the_given_path() {
    let dir = tempfile::tempdir().unwrap();
    let shim = dir.path().join("daruda-probe-tool.cmd");
    std::fs::write(&shim, "@echo off\r\n").unwrap();
    assert_eq!(
        resolve_program(
            OsStr::new("daruda-probe-tool"),
            Some(dir.path().as_os_str())
        ),
        shim
    );
}

#[test]
fn this_process_is_alive() {
    assert!(is_alive(std::process::id()));
    assert!(!is_alive(0));
}

#[test]
fn another_live_process_is_not_mistaken_for_a_stale_lock() {
    let mut child = command(test_process::executable())
        .args(["--sleep-ms", "30000"])
        .spawn()
        .unwrap();
    let alive = is_alive(child.id());
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(alive);
}

#[test]
fn command_keeps_the_program_it_was_given() {
    let built = command("some-program");

    assert_eq!(built.get_program(), "some-program");
}

/// The reason this module exists: a tear-down has to reach what the child
/// forked, not only the child. The fixture leaves a grandchild behind and
/// exits, which is exactly what `child.kill()` cannot clean up.
///
/// Runs everywhere, because the Windows half — a job object — has no
/// other way to be checked from this machine.
#[test]
fn kill_tree_reaches_a_grandchild_the_child_left_behind() {
    let dir = tempfile::tempdir().expect("tempdir");
    let pid_file = dir.path().join("grandchild.pid");

    let mut cmd = command(test_process::executable());
    cmd.arg("--orphan").arg(&pid_file);
    lead_own_group(&mut cmd);
    let mut child = cmd.spawn().expect("spawn");
    let group = Group::try_adopt(child.id()).expect("adopt the suspended child");

    let grandchild = read_pid(&pid_file);
    assert!(is_alive(grandchild), "the fixture must leave one behind");

    group
        .try_terminate_child(&mut child)
        .expect("terminate and reap the whole group");

    assert!(
        gone_within(grandchild, Duration::from_secs(5)),
        "the grandchild outlived the kill — it reached only the child"
    );
    let _ = child.wait();
}

/// The fixture writes the pid before exiting, but the write races this
/// read on a loaded machine.
fn read_pid(path: &std::path::Path) -> u32 {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(text) = std::fs::read_to_string(path)
            && let Ok(pid) = text.trim().parse()
        {
            return pid;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the fixture never recorded a grandchild"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The kill is asynchronous and the pid is released on reap, so "gone" is
/// a bounded wait rather than an instant.
fn gone_within(pid: u32, budget: Duration) -> bool {
    let deadline = std::time::Instant::now() + budget;
    while std::time::Instant::now() < deadline {
        if !is_alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

/// The contract's other half: killing the group must not take this
/// process with it. A child that never called `lead_own_group` shares
/// our group, and killing *that* would be suicide — so the call is only
/// ever made with a pid that led its own.
#[cfg(unix)]
#[test]
fn a_group_leader_is_the_only_thing_killed() {
    let mut cmd = command("sh");
    cmd.arg("-c").arg("sleep 30");
    lead_own_group(&mut cmd);
    let mut child = cmd.spawn().expect("spawn");

    Group::adopt(child.id()).kill_tree();

    let status = child.wait().expect("wait");
    assert!(!status.success(), "the child was signalled");
}
