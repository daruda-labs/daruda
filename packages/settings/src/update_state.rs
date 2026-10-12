//! The self-updater as the About page shows it, and what its buttons ask
//! for. The updater itself is the host's; it reports its state here and
//! answers the commands.

/// One step of the update flow.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum UpdateState {
    /// No check has run, or the host has no updater.
    #[default]
    Idle,
    Checking,
    UpToDate,
    /// The last step failed, with the reason to show.
    Errored(String),
    Available {
        version: String,
        notes: String,
        install: UpdateInstall,
    },
    Downloading,
    Installing,
    /// Installed; a restart runs it. An installer-run update says so.
    ReadyToRestart {
        uses_installer: bool,
    },
}

/// Whether an available release can be installed from here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateInstall {
    InPlace,
    /// A system package: the package manager installs it.
    ManualPackage,
    /// A development build, which nothing replaces.
    Unpackaged,
}

/// A button on the About page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateCommand {
    Check,
    DownloadAndInstall,
    Restart,
}
