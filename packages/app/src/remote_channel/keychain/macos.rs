//! macOS generic-password access through the security tool.

use daruda_core::process::command;
use std::{io, process::Stdio};

pub(super) fn read(service: &str, account: &str) -> io::Result<Option<String>> {
    let output = command("security")
        .args(["find-generic-password", "-s", service, "-a", account, "-w"])
        .stderr(Stdio::null())
        .output()?;
    Ok(output
        .status
        .success()
        .then(|| super::normalize(&output.stdout))
        .flatten())
}

pub(super) fn write(service: &str, account: &str, value: &str) -> io::Result<()> {
    let status = command("security")
        .args([
            "add-generic-password",
            "-U",
            "-s",
            service,
            "-a",
            account,
            "-w",
            value,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    super::status_result(status, "write")
}

pub(super) fn delete(service: &str, account: &str) -> io::Result<()> {
    let status = command("security")
        .args(["delete-generic-password", "-s", service, "-a", account])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    super::status_result(status, "delete")
}
