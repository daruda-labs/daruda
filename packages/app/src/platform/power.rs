//! System-sleep inhibition behind one lease every platform implements.
//!
//! The lease only holds or releases; deciding when to hold it is policy that
//! lives outside this capability and never needs to know the OS.

#[cfg(unix)]
#[path = "power_unix.rs"]
mod unix;
#[cfg(windows)]
#[path = "power_windows.rs"]
mod windows;

/// A system-sleep inhibition that can be held and released.
pub(crate) trait SleepLease {
    /// Hold (`true`) or release (`false`) the system-sleep inhibition.
    fn set_active(&mut self, active: bool) -> Result<(), PowerError>;
    /// Whether an inhibition believed held is actually still in force
    /// (a child that exited on its own reads `false`).
    fn is_held(&mut self) -> bool;
}

#[derive(Debug)]
pub(crate) enum PowerError {
    /// The helper process that holds the inhibition could not be started.
    #[cfg(unix)]
    Spawn(std::io::Error),
    /// The OS refused or failed the inhibition change.
    Os(std::io::Error),
    /// The helper holding the inhibition exited on its own.
    #[cfg(unix)]
    Exited(std::process::ExitStatus),
}

impl std::fmt::Display for PowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(unix)]
            Self::Spawn(error) => write!(f, "sleep inhibitor failed to start: {error}"),
            Self::Os(error) => write!(f, "sleep inhibition change failed: {error}"),
            #[cfg(unix)]
            Self::Exited(status) => write!(f, "sleep inhibitor exited on its own: {status}"),
        }
    }
}

impl std::error::Error for PowerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            #[cfg(unix)]
            Self::Spawn(error) => Some(error),
            Self::Os(error) => Some(error),
            #[cfg(unix)]
            Self::Exited(_) => None,
        }
    }
}

/// The sleep lease for the OS this build runs on.
pub(crate) fn backend() -> Box<dyn SleepLease> {
    #[cfg(unix)]
    return Box::new(unix::ChildLease::for_this_os());
    #[cfg(windows)]
    return Box::new(windows::ExecutionStateLease::default());
    #[cfg(not(any(unix, windows)))]
    return Box::new(Unsupported);
}

#[cfg(not(any(unix, windows)))]
struct Unsupported;

#[cfg(not(any(unix, windows)))]
impl SleepLease for Unsupported {
    fn set_active(&mut self, active: bool) -> Result<(), PowerError> {
        if !active {
            return Ok(());
        }
        Err(PowerError::Os(std::io::ErrorKind::Unsupported.into()))
    }
    fn is_held(&mut self) -> bool {
        false
    }
}

/// Re-apply the lease after every resume from suspend.
#[cfg(windows)]
pub(crate) fn install(cx: &mut gpui::App) {
    windows::install(cx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error as _;

    #[test]
    fn error_names_its_stage_and_keeps_the_io_cause() {
        let os = PowerError::Os(std::io::ErrorKind::Unsupported.into());
        assert!(os.to_string().starts_with("sleep inhibition change failed"));
        assert!(os.source().is_some());
    }

    #[cfg(unix)]
    #[test]
    fn a_helper_that_cannot_start_names_its_stage() {
        let spawn = PowerError::Spawn(std::io::ErrorKind::NotFound.into());
        assert!(
            spawn
                .to_string()
                .starts_with("sleep inhibitor failed to start")
        );
        assert!(spawn.source().is_some());
    }

    #[cfg(unix)]
    #[test]
    fn an_exited_helper_names_its_status() {
        use std::os::unix::process::ExitStatusExt as _;
        let exited = PowerError::Exited(std::process::ExitStatus::from_raw(1 << 8));
        assert!(
            exited
                .to_string()
                .starts_with("sleep inhibitor exited on its own")
        );
        assert!(exited.source().is_none());
    }
}
