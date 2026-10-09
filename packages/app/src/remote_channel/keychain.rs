//! Profile-scoped credential policy over a native storage backend.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
mod unsupported;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
use linux as backend;
#[cfg(target_os = "macos")]
use macos as backend;
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
use unsupported as backend;
#[cfg(windows)]
use windows as backend;

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
    match backend::read(service, account) {
        Ok(secret) => secret,
        Err(error) => {
            crate::platform::report_error("credentials.read", "Credential read failed", &error);
            None
        }
    }
}

pub fn write(service: &str, account: &str, value: &str) -> std::io::Result<()> {
    if cfg!(test) {
        return Err(std::io::Error::other(
            "Credential writes are disabled in tests",
        ));
    }
    backend::write(service, account, value)
}

pub fn delete(service: &str, account: &str) -> std::io::Result<()> {
    if cfg!(test) {
        return Err(std::io::Error::other(
            "Credential deletes are disabled in tests",
        ));
    }
    backend::delete(service, account)
}

#[cfg(any(test, target_os = "macos", target_os = "linux"))]
fn normalize(bytes: &[u8]) -> Option<String> {
    let value = std::str::from_utf8(bytes).ok()?.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn status_result(status: std::process::ExitStatus, operation: &str) -> std::io::Result<()> {
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "OS credential store rejected the {operation}"
        )))
    }
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
