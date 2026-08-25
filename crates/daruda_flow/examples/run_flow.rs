//! Run a flow file against a real agent, from a terminal.
//!
//! Everything the engine refuses to decide for itself is decided here, which
//! makes this the smallest possible sketch of what the app owes it in P3: the
//! run id and directory, whether a pid is alive, what `git status` says, and
//! where to send the event stream.
//!
//! ```text
//! cargo run -p daruda_flow --example run_flow -- <flow.yaml> [cwd]
//! ```
//!
//! `DARUDA_FLOW_AGENT` overrides the adapter command (default: the ACP
//! client's own, which is what the app launches).

use daruda_flow::event::FlowEvent;
use daruda_flow::request::{Budget, RunRequest};
use daruda_flow::runner::{AcpRunner, CancelToken, ProcessRunner, Runners};
use daruda_flow::schedule::{RunOutcome, execute};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Long enough to watch a real agent work, short enough that a hung one does
/// not run all night. The app reads this from config.
const NODE_BUDGET: u32 = 50;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(flow_path) = args.next().map(PathBuf::from) else {
        eprintln!("usage: run_flow <flow.yaml> [cwd]");
        std::process::exit(2);
    };
    // Absolutised here, because the engine refuses a relative path — a run
    // resolves it a second time against whatever directory the process is
    // in, which is neither the lane nor the run. Making paths whole is the
    // host's job, and this is the smallest sketch of a host.
    let cwd = absolute(
        args.next()
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().expect("a working directory")),
    );

    let text = std::fs::read_to_string(&flow_path).expect("the flow file is readable");
    let loaded = match daruda_flow::load(&text, None) {
        Ok(loaded) => loaded,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    // The engine receives a finished path; inventing the id is the host's.
    // A real host uses a ULID — this is a sortable stand-in with the same
    // property retention relies on.
    let run_dir = cwd
        .join(".daruda/flow-runs")
        .join(format!("{:016x}", now_millis()));
    let flow_dir = absolute(
        flow_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from(".")),
    );

    let launch = daruda_acp::LaunchSpec {
        command: std::env::var("DARUDA_FLOW_AGENT")
            .unwrap_or_else(|_| daruda_acp::AdapterCommand::default().0),
        strip_env: Vec::new(),
    };
    let agents = HashMap::from([("claude".to_string(), launch.clone())]);
    // Outside the repository. A managed Node.js runtime is the host's, not
    // the run's — the `.gitignore` the engine writes covers `flow-runs/`
    // only, so a runtime unpacked beside it surfaces in `git status`. The
    // app puts this under its own data directory for the same reason.
    let node_install_dir = std::env::temp_dir().join("daruda-flow-node");

    let (tx, rx) = smol::channel::unbounded();
    let cancel = CancelToken::default();
    // Ctrl-C stops the run the way the app's stop button will: the token is
    // the whole of the engine's stop switch.
    {
        let cancel = cancel.clone();
        ctrlc_handler(move || cancel.cancel());
    }

    let request = RunRequest {
        loaded,
        until: None,
        pinned: Vec::new(),
        cwd: cwd.clone(),
        run_dir: run_dir.clone(),
        flow_dir,
        agents,
        node_install_dir: node_install_dir.clone(),
        budget: Budget {
            max_node_runs: Some(NODE_BUDGET),
            ..Budget::unlimited()
        },
        is_alive: Box::new(process_is_alive),
        // Asked about the directory the attempt ran in, which is the
        // node's own when it names one.
        git_status: Some(Box::new(git_status)),
        events: Some(tx),
        // No answering surface: this runner prints events, it does not
        // offer buttons. A flow that could reach `permission: ask` is
        // refused at submission rather than parking forever.
        ask: None,
        resume: None,
    };

    // Drained on another thread: the stream is unbounded and never awaited by
    // the engine, so a slow reader here cannot slow the run.
    let watcher = std::thread::spawn(move || {
        while let Ok(event) = smol::block_on(rx.recv()) {
            println!("{}", describe(&event));
        }
    });

    let runners = Runners {
        agent: AcpRunner::new(request.agents.clone(), node_install_dir),
        // Design §9: a command node inherits the environment but not the
        // ACP account credentials, and computing that list is the host's
        // job — the runner only unsets what it is given. `Vec::new()` here
        // would leak them into every shell line a committed flow names,
        // which is why this is spelled out even though the launch above
        // happens to strip nothing.
        command: ProcessRunner::new(strip_env_union(&request.agents)),
    };
    let report = execute(&request, &runners, &cancel);
    drop(request);
    let _ = watcher.join();

    println!("\n--- {} ---", run_dir.display());
    println!("{:?}", report.outcome);
    println!("budget units: {}", report.node_runs);
    for warning in report.warnings() {
        println!("warning: {warning}");
    }
    if matches!(report.outcome, RunOutcome::Done) {
        return;
    }
    std::process::exit(1);
}

/// One line per event, in the order the run made them — the same sequence a
/// UI would redraw from.
fn describe(event: &FlowEvent) -> String {
    match event {
        FlowEvent::RunStarted { nodes, .. } => format!("run started: {}", nodes.join(" → ")),
        FlowEvent::NodeStarted { node, attempt } => format!("  {node} (attempt {attempt})"),
        FlowEvent::NodePassed { node, .. } => format!("  {node} passed"),
        FlowEvent::NodeFailed { node, failure, .. } => format!("  {node} failed: {failure}"),
        FlowEvent::FixStarted { gate } => format!("  fixing for {gate}"),
        FlowEvent::FixEnded { gate, failure } => match failure {
            Some(failure) => format!("  fix for {gate} failed: {failure}"),
            None => format!("  fix for {gate} done"),
        },
        FlowEvent::Rerunning { gate, members } => {
            format!("  {gate} re-derives: {}", members.join(", "))
        }
        FlowEvent::RunEnded { end } => format!("run ended: {end:?}"),
    }
}

/// What the app answers with `sysinfo`. Signal 0 delivers nothing — it only
/// reports whether the pid is claimed.
#[cfg(unix)]
fn process_is_alive(pid: u32) -> bool {
    // SAFETY: any pid value is a valid argument to `kill`; signal 0 has no
    // effect beyond the existence check.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[cfg(not(unix))]
fn process_is_alive(pid: u32) -> bool {
    pid == std::process::id()
}

/// The engine deliberately does not manage the working tree, so it records
/// what the tree looked like instead. `None` when this is not a git repo.
fn git_status(cwd: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(cwd)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(unix)]
fn ctrlc_handler(on_stop: impl Fn() + Send + 'static) {
    // A signal handler may only call async-signal-safe functions, so it does
    // the least it can: flip an atomic that a watcher thread polls.
    static HIT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    extern "C" fn handle(_: libc::c_int) {
        HIT.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    // SAFETY: `handle` only stores to a static atomic, which is
    // async-signal-safe.
    unsafe {
        libc::signal(libc::SIGINT, handle as *const () as libc::sighandler_t);
    }
    std::thread::spawn(move || {
        while !HIT.load(std::sync::atomic::Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(50));
        }
        eprintln!("\nstopping…");
        on_stop();
    });
}

#[cfg(not(unix))]
fn ctrlc_handler(_on_stop: impl Fn() + Send + 'static) {}

/// A path the engine will accept. `std::path::absolute` normalises without
/// touching the filesystem, so a directory that does not exist yet — every
/// run directory, on a first run — still resolves.
/// Every env var to unset before a command node runs: the union of what
/// each agent's account strips. The app computes the same thing in
/// `workspace::flow_request::union_strip_env`.
fn strip_env_union(agents: &HashMap<String, daruda_acp::LaunchSpec>) -> Vec<String> {
    let mut names: Vec<String> = agents
        .values()
        .flat_map(|spec| spec.strip_env.iter().cloned())
        .collect();
    names.sort();
    names.dedup();
    names
}

fn absolute(path: PathBuf) -> PathBuf {
    std::path::absolute(&path).unwrap_or(path)
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}
