//! Native launch of an NSIS update after the running application exits.

use std::path::Path;

pub(crate) fn launch(package: &Path, root: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        command(package, root, std::process::id()).spawn()?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (package, root);
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "NSIS updates require Windows",
        ))
    }
}

#[cfg(windows)]
fn command(package: &Path, root: &Path, pid: u32) -> std::process::Command {
    use std::os::windows::process::CommandExt as _;
    let mut command = daruda_core::process::command(package);
    command.arg(format!("/WAITPID={pid}")).arg("/RELAUNCH");
    // NSIS requires /D last and unquoted, including when its path has spaces.
    // No shell interprets this argument; Windows paths cannot contain quotes.
    let mut directory = std::ffi::OsString::from("/D=");
    directory.push(root);
    command.raw_arg(directory);
    command
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn installer_command_waits_and_preserves_a_space_containing_directory() {
        let command = command(
            Path::new("C:/Temp/verified setup.exe"),
            Path::new("C:/Users/me/My Apps/daruda"),
            1234,
        );
        let debug = format!("{command:?}");
        assert!(debug.contains("/WAITPID=1234"));
        assert!(debug.contains("/RELAUNCH"));
        assert!(debug.contains("/D=C:/Users/me/My Apps/daruda"));
        assert!(!debug.contains("/S"));
    }
}
