//! Windows' sleep lease, and the resume subscription that re-applies it:
//! the thread execution state does not survive a suspend.

use super::{PowerError, SleepLease};
use gpui::{App, Global};
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::System::Power::*;
use windows_sys::Win32::UI::WindowsAndMessaging::PBT_APMRESUMEAUTOMATIC;

/// Set by the OS callback on its own thread; drained on the GUI thread.
static RESUMED: AtomicBool = AtomicBool::new(false);

/// Keeps the resume subscription registered for the process lifetime.
struct ResumeSubscription {
    registration: HPOWERNOTIFY,
}
impl Global for ResumeSubscription {}

impl Drop for ResumeSubscription {
    fn drop(&mut self) {
        // SAFETY: the callback refers only to a static atomic, so nothing it
        // touches outlives the registration.
        unsafe {
            if self.registration != 0 {
                PowerUnregisterSuspendResumeNotification(self.registration);
            }
        }
    }
}

/// Thread execution state; the API has no query, so the held flag is tracked.
/// Must be driven from the GUI thread, which owns the execution state.
#[derive(Default)]
pub(super) struct ExecutionStateLease {
    held: bool,
}

impl SleepLease for ExecutionStateLease {
    fn set_active(&mut self, active: bool) -> Result<(), PowerError> {
        // SAFETY: called consistently on the UI thread. No display or
        // global power-plan changes are requested.
        if unsafe { SetThreadExecutionState(execution_flags(active)) } == 0 {
            return Err(PowerError::Os(std::io::Error::last_os_error()));
        }
        self.held = active;
        Ok(())
    }

    fn is_held(&mut self) -> bool {
        self.held
    }
}

impl Drop for ExecutionStateLease {
    fn drop(&mut self) {
        if self.held {
            // SAFETY: the lease is dropped on the same GUI thread that
            // acquired the execution state.
            unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
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
    cx.set_global(ResumeSubscription {
        registration: registration as HPOWERNOTIFY,
    });
    // Only an atomic swap per tick; the lease itself belongs to keep-awake.
    crate::watcher_pumps::spawn_periodic_pump(
        std::time::Duration::from_secs(2),
        |cx| {
            if !RESUMED.swap(false, Ordering::AcqRel) {
                return;
            }
            crate::keep_awake::reassert(cx);
            crate::app_presence::observe(cx);
            if crate::settings_store::SettingsStore::global(cx)
                .user()
                .update
                .auto_check
                && let Some(updater) = crate::update::Updater::get(cx)
            {
                updater.update(cx, |updater, cx| updater.check(cx));
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
