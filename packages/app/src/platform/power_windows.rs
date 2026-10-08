//! Keep system sleep inhibited only while configured work is actually active.

use gpui::{App, Global};
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::System::Power::*;
use windows_sys::Win32::UI::WindowsAndMessaging::PBT_APMRESUMEAUTOMATIC;

static RESUMED: AtomicBool = AtomicBool::new(false);

struct Power {
    registration: HPOWERNOTIFY,
    active: bool,
}
impl Global for Power {}

impl Drop for Power {
    fn drop(&mut self) {
        // SAFETY: this Global is dropped on the same GUI thread that acquired
        // the execution state. The callback refers only to a static atomic.
        unsafe {
            if self.active {
                SetThreadExecutionState(ES_CONTINUOUS);
            }
            if self.registration != 0 {
                PowerUnregisterSuspendResumeNotification(self.registration);
            }
        }
    }
}

unsafe extern "system" fn resume(
    _context: *const core::ffi::c_void,
    event: u32,
    _setting: *const core::ffi::c_void,
) -> u32 {
    if event == PBT_APMRESUMEAUTOMATIC {
        RESUMED.store(true, Ordering::Release);
    }
    0
}

pub(super) fn install(cx: &mut App) {
    if cfg!(test) {
        return;
    }
    let mut registration = std::ptr::null_mut();
    let mut parameters = DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
        Callback: Some(resume),
        Context: std::ptr::null_mut(),
    };
    // SAFETY: Windows copies the subscription parameters; the callback has
    // process lifetime and does not dereference its null context.
    let status = unsafe {
        PowerRegisterSuspendResumeNotification(
            2,
            (&mut parameters as *mut DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS).cast(),
            &mut registration,
        )
    };
    if status != 0 {
        super::super::report_error(
            "desktop.power.subscription",
            "Power resume subscription failed",
            &std::io::Error::from_raw_os_error(status as i32),
        );
    }
    cx.set_global(Power {
        registration: registration as HPOWERNOTIFY,
        active: false,
    });
    crate::watcher_pumps::spawn_periodic_pump(
        std::time::Duration::from_secs(2),
        |cx| {
            let mut working = false;
            crate::window_registry::WindowRegistry::for_each_workspace(cx, |ws, _, cx| {
                working |= ws.has_active_desktop_work(cx);
            });
            let enabled = crate::settings_store::SettingsStore::global(cx)
                .user()
                .desktop
                .keep_awake_while_working;
            let resumed = RESUMED.swap(false, Ordering::AcqRel);
            let desired = enabled && working;
            let power = cx.global_mut::<Power>();
            if power.active != desired || resumed {
                let flags = execution_flags(desired);
                // SAFETY: called consistently on the UI thread. No display or
                // global power-plan changes are requested.
                if unsafe { SetThreadExecutionState(flags) } == 0 {
                    super::super::report_error(
                        "desktop.power.lease",
                        "Power execution state update failed",
                        &std::io::Error::last_os_error(),
                    );
                } else {
                    power.active = desired;
                }
            }
            if resumed {
                crate::app_presence::observe(cx);
                if crate::settings_store::SettingsStore::global(cx)
                    .user()
                    .update
                    .auto_check
                    && let Some(updater) = crate::update::Updater::get(cx)
                {
                    updater.update(cx, |updater, cx| updater.check(cx));
                }
            }
        },
        cx,
    );
}

fn execution_flags(active: bool) -> EXECUTION_STATE {
    if active {
        ES_CONTINUOUS | ES_SYSTEM_REQUIRED
    } else {
        ES_CONTINUOUS
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn idle_releases_sleep_lease_without_holding_display_awake() {
        use super::*;
        assert_eq!(execution_flags(false), ES_CONTINUOUS);
        assert_ne!(execution_flags(true) & ES_SYSTEM_REQUIRED, 0);
        assert_eq!(execution_flags(true) & ES_DISPLAY_REQUIRED, 0);
    }
}
