//! Profile-scoped OS credential storage. Secrets never enter config or logs.

use std::process::{Command, Stdio};

pub fn service(base: &str) -> String {
    match daruda_store::persistence::profile_suffix() {
        Some(profile) => format!("{base}-{profile}"),
        None => base.to_owned(),
    }
}

pub fn channel_service(id: &str) -> String {
    service(&format!("daruda-remote-{id}"))
}

pub fn read(service: &str, account: &str) -> Option<String> {
    if cfg!(test) {
        return None;
    }
    #[cfg(target_os = "macos")]
    let output = Command::new("security")
        .args(["find-generic-password", "-s", service, "-a", account, "-w"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    #[cfg(target_os = "linux")]
    let output = Command::new("secret-tool")
        .args(["lookup", "service", service, "account", account])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    return output
        .status
        .success()
        .then(|| normalize(&output.stdout))
        .flatten();
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    None
}

fn normalize(bytes: &[u8]) -> Option<String> {
    let value = std::str::from_utf8(bytes).ok()?.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

pub fn write(service: &str, account: &str, value: &str) -> std::io::Result<()> {
    if cfg!(test) {
        return Err(std::io::Error::other(
            "Credential writes are disabled in tests",
        ));
    }
    #[cfg(target_os = "macos")]
    let status = Command::new("security")
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
    #[cfg(target_os = "linux")]
    let status = {
        use std::io::Write;
        let mut child = Command::new("secret-tool")
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
        if let Some(mut input) = child.stdin.take() {
            input.write_all(value.as_bytes())?;
        }
        child.wait()?
    };
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    return if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(
            "OS credential store rejected the write",
        ))
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    Err(std::io::Error::other(
        "OS credential storage is unavailable",
    ))
}

pub fn delete(service: &str, account: &str) -> std::io::Result<()> {
    if cfg!(test) {
        return Err(std::io::Error::other(
            "Credential deletes are disabled in tests",
        ));
    }
    #[cfg(target_os = "macos")]
    let status = Command::new("security")
        .args(["delete-generic-password", "-s", service, "-a", account])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    #[cfg(target_os = "linux")]
    let status = Command::new("secret-tool")
        .args(["clear", "service", service, "account", account])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    return if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(
            "OS credential store rejected the delete",
        ))
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    Err(std::io::Error::other(
        "OS credential storage is unavailable",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_and_connection_names_do_not_collide() {
        assert_ne!(service("daruda-telegram-bot"), "daruda-telegram-bot");
        assert_ne!(channel_service("work"), channel_service("personal"));
        assert_ne!(channel_service("work"), service("daruda-telegram-bot"));
    }

    #[test]
    fn reads_are_hermetic_and_empty_values_are_missing() {
        assert!(read("test", "bot_token").is_none());
        assert_eq!(normalize(b" \n"), None);
        assert_eq!(normalize(b" token \n"), Some("token".into()));
        assert_eq!(normalize(&[0xff]), None);
    }
}
