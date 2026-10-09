//! Linux Secret Service access through secret-tool.

use daruda_core::process::command;
use std::{
    io::{self, Write},
    process::Stdio,
};

pub(super) fn read(service: &str, account: &str) -> io::Result<Option<String>> {
    let output = command("secret-tool")
        .args(["lookup", "service", service, "account", account])
        .stderr(Stdio::null())
        .output()?;
    Ok(output
        .status
        .success()
        .then(|| super::normalize(&output.stdout))
        .flatten())
}

pub(super) fn write(service: &str, account: &str, value: &str) -> io::Result<()> {
    let mut child = command("secret-tool")
        .args([
            "store",
            "--label=daruda remote channel",
            "service",
            service,
            "account",
            account,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let write = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::new(io::ErrorKind::BrokenPipe, "Credential input is unavailable"))
        .and_then(|mut input| input.write_all(value.as_bytes()));
    if let Err(error) = write {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    super::status_result(child.wait()?, "write")
}

pub(super) fn delete(service: &str, account: &str) -> io::Result<()> {
    let status = command("secret-tool")
        .args(["clear", "service", service, "account", account])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    super::status_result(status, "delete")
}
