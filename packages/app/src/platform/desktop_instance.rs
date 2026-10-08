//! One desktop process per data profile; later launches forward open requests.

use std::fs::File;
use std::io::{Read as _, Write as _};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail, ensure};
use fs4::fs_std::FileExt as _;
use serde::{Deserialize, Serialize};

const LIMIT: u64 = 16 * 1024;
const TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(crate) struct Request {
    pub paths: Vec<PathBuf>,
}

#[derive(Deserialize, Serialize)]
struct Endpoint {
    port: u16,
    token: String,
    pid: u32,
}

#[derive(Deserialize, Serialize)]
struct Envelope {
    token: String,
    request: Request,
}

pub(crate) enum Launch {
    Primary(Instance),
    Forwarded,
}

pub(crate) struct Instance {
    _lock: File,
    endpoint: PathBuf,
    stopped: Arc<AtomicBool>,
    receiver: mpsc::Receiver<Request>,
}

impl gpui::Global for Instance {}

impl Drop for Instance {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        if let Err(error) = std::fs::remove_file(&self.endpoint)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            log_error("Desktop endpoint cleanup failed", &error);
        }
    }
}

pub(crate) fn start(
    dir: &Path,
    args: impl IntoIterator<Item = std::ffi::OsString>,
) -> anyhow::Result<Launch> {
    let args: Vec<_> = args.into_iter().collect();
    let exclusive = args.iter().any(|arg| {
        let arg = arg.to_string_lossy();
        let flag = arg.split('=').next().unwrap_or_default();
        matches!(flag, "--smoke" | "--screenshot" | "--replay-acp-log")
            || arg.starts_with("--screenshot-")
    });
    acquire(dir, request_from_args(args)?, exclusive)
}

fn acquire(dir: &Path, request: Request, exclusive: bool) -> anyhow::Result<Launch> {
    daruda_core::path::create_owner_only_dir(dir)?;
    let lock = File::options()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join("desktop.lock"))?;
    let endpoint = dir.join("desktop.json");
    if !lock.try_lock_exclusive()? {
        ensure!(
            !exclusive,
            "validation/replay requires an unused data profile; choose a separate {}",
            daruda_core::process_env::DATA_DIR.name()
        );
        forward(&endpoint, request)?;
        return Ok(Launch::Forwarded);
    }
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    listener.set_nonblocking(true)?;
    let info = Endpoint {
        port: listener.local_addr()?.port(),
        token: uuid::Uuid::new_v4().to_string(),
        pid: std::process::id(),
    };
    let mut staging = tempfile::NamedTempFile::new_in(dir)?;
    serde_json::to_writer(&mut staging, &info)?;
    staging.as_file().sync_all()?;
    staging
        .persist(&endpoint)
        .context("publishing desktop endpoint")?;
    let (sender, receiver) = mpsc::channel();
    if !request.paths.is_empty() {
        sender.send(request)?;
    }
    let stopped = Arc::new(AtomicBool::new(false));
    let stop = stopped.clone();
    std::thread::Builder::new()
        .name("desktop-requests".into())
        .spawn(move || {
            while !stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        if let Err(error) = receive(stream, &info.token, &sender) {
                            log_error("Desktop request rejected", error.as_ref());
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    Err(error) => {
                        log_error("Desktop request listener failed", &error);
                        break;
                    }
                }
            }
        })?;
    Ok(Launch::Primary(Instance {
        _lock: lock,
        endpoint,
        stopped,
        receiver,
    }))
}

fn receive(
    mut stream: TcpStream,
    token: &str,
    sender: &mpsc::Sender<Request>,
) -> anyhow::Result<()> {
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    let mut bytes = Vec::new();
    (&mut stream).take(LIMIT + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= LIMIT,
        "desktop request exceeds size limit"
    );
    let envelope: Envelope = serde_json::from_slice(&bytes)?;
    ensure!(
        envelope.token == token,
        "desktop request authentication failed"
    );
    ensure!(
        envelope.request.paths.iter().all(|path| path.is_absolute()),
        "desktop paths must be absolute"
    );
    sender.send(envelope.request)?;
    stream.write_all(b"ok")?;
    Ok(())
}

fn forward(endpoint: &Path, request: Request) -> anyhow::Result<()> {
    let start = Instant::now();
    loop {
        let attempt = || -> anyhow::Result<()> {
            let file = File::open(endpoint)?;
            let mut bytes = Vec::new();
            file.take(LIMIT + 1).read_to_end(&mut bytes)?;
            ensure!(
                bytes.len() as u64 <= LIMIT,
                "desktop endpoint exceeds size limit"
            );
            let info: Endpoint = serde_json::from_slice(&bytes)?;
            let mut stream = TcpStream::connect_timeout(
                &(std::net::Ipv4Addr::LOCALHOST, info.port).into(),
                TIMEOUT,
            )?;
            stream.set_write_timeout(Some(TIMEOUT))?;
            stream.set_read_timeout(Some(TIMEOUT))?;
            allow_foreground(info.pid);
            let bytes = serde_json::to_vec(&Envelope {
                token: info.token,
                request: request.clone(),
            })?;
            ensure!(
                bytes.len() as u64 <= LIMIT,
                "desktop request exceeds size limit"
            );
            stream.write_all(&bytes)?;
            stream.shutdown(Shutdown::Write)?;
            let mut reply = [0; 2];
            stream.read_exact(&mut reply)?;
            ensure!(&reply == b"ok", "desktop request was not acknowledged");
            Ok(())
        };
        match attempt() {
            Ok(()) => return Ok(()),
            Err(error) if start.elapsed() >= TIMEOUT => {
                return Err(error.context("existing desktop did not accept this launch"));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

fn allow_foreground(pid: u32) {
    #[cfg(windows)]
    // SAFETY: grants only the endpoint owner's process foreground permission.
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(pid);
    }
    #[cfg(not(windows))]
    let _ = pid;
}

pub(crate) fn request_from_args(
    args: impl IntoIterator<Item = std::ffi::OsString>,
) -> anyhow::Result<Request> {
    let mut request = Request::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--open" {
            let path = args.next().context("--open needs a directory")?;
            request.paths.push(daruda_core::path::canonicalize(path)?);
        } else if !arg.to_string_lossy().starts_with('-') {
            let path = daruda_core::path::canonicalize(&arg)?;
            ensure!(
                path.is_dir(),
                "desktop open target is not a directory: {}",
                path.display()
            );
            request.paths.push(path);
        } else if arg == "--smoke" {
            continue;
        } else {
            // Feature-specific switches are interpreted by their existing parsers.
            break;
        }
    }
    if request.paths.iter().any(|path| !path.is_dir()) {
        bail!("desktop open targets must be existing directories");
    }
    Ok(request)
}

pub(crate) fn install(instance: Instance, cx: &mut gpui::App) {
    cx.set_global(instance);
    crate::watcher_pumps::spawn_periodic_pump(
        Duration::from_millis(100),
        |cx| {
            let requests: Vec<_> = cx.global::<Instance>().receiver.try_iter().collect();
            for request in requests {
                if request.paths.is_empty() {
                    super::desktop::reveal(None, cx);
                } else {
                    let config = crate::settings_store::SettingsStore::global(cx).user_arc();
                    for path in request.paths {
                        crate::windows::open_requested_directory(config.clone(), path, cx);
                    }
                }
            }
        },
        cx,
    );
}

fn log_error(message: &str, error: &dyn std::error::Error) {
    super::report_error("desktop.instance", message, error);
}

#[cfg(test)]
mod tests;
