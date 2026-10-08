//! Native Windows toasts, with a per-profile Start menu identity.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock, mpsc};

use anyhow::Context as _;
use windows_api::Data::Xml::Dom::XmlDocument;
use windows_api::Foundation::TypedEventHandler;
use windows_api::UI::Notifications::{ToastNotification, ToastNotificationManager};
use windows_api::core::HSTRING;

#[path = "notifications_windows/identity.rs"]
mod identity;

pub(super) fn unregister() -> anyhow::Result<()> {
    identity::unregister()
}

use super::Target;

const RETAINED_TOASTS: usize = 64;
static ACTIVATIONS: OnceLock<mpsc::Sender<Option<Target>>> = OnceLock::new();
static TOASTS: Mutex<VecDeque<ToastNotification>> = Mutex::new(VecDeque::new());
static IDENTITY: OnceLock<String> = OnceLock::new();

pub(super) fn install(cx: &mut gpui::App) {
    let (sender, receiver) = mpsc::channel();
    if ACTIVATIONS.set(sender).is_err() {
        return;
    }
    super::report(identity::register_identity());
    crate::watcher_pumps::spawn_periodic_pump(
        std::time::Duration::from_millis(100),
        move |cx| {
            for target in receiver.try_iter() {
                super::super::desktop::reveal(target, cx);
            }
        },
        cx,
    );
}

pub(super) fn deliver(title: &str, body: &str, target: Option<Target>) -> anyhow::Result<()> {
    let identity = IDENTITY
        .get()
        .context("Windows notification identity unavailable")?;
    let document = XmlDocument::new()?;
    document.LoadXml(&HSTRING::from(format!(
        "<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>",
        escape_xml(title), escape_xml(body),
    )))?;
    let toast = ToastNotification::CreateToastNotification(&document)?;
    toast.Activated(&TypedEventHandler::new(move |_, _| {
        if let Some(sender) = ACTIVATIONS.get() {
            // SILENT-OK: receiver disappears only as the app exits.
            let _ = sender.send(target);
        }
        Ok(())
    }))?;
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(identity))?.Show(&toast)?;
    let mut retained = TOASTS
        .lock()
        .map_err(|_| anyhow::anyhow!("notification retention lock poisoned"))?;
    retained.push_back(toast);
    while retained.len() > RETAINED_TOASTS {
        retained.pop_front();
    }
    Ok(())
}

fn escape_xml(value: &str) -> String {
    value
        .chars()
        .filter(|c| matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}'))
        .collect::<String>()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "displays a native notification on an interactive Windows desktop"]
    fn native_toast_uses_registered_profile_identity() {
        super::identity::register_identity().unwrap();
        super::deliver("daruda", "Windows notification delivery check", None).unwrap();
    }

    #[test]
    fn toast_text_cannot_inject_markup_or_invalid_xml_controls() {
        assert_eq!(
            super::escape_xml("<&>\0\u{fffe}\u{ffff}한글\n"),
            "&lt;&amp;&gt;한글\n"
        );
    }
}
