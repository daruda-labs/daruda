//! Path syntax as a value rather than a `cfg`.
//!
//! `std::path` answers with the rules of the OS daruda was built for. That is
//! the wrong answer for a path another machine wrote (a remote shell, an agent
//! on another host), and it makes the Windows rules untestable anywhere but
//! Windows. A [`PathStyle`] names the rules explicitly, so both styles are
//! asserted from any host. Pure: string rules only, never the filesystem.

/// Which platform's path syntax a string is written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PathStyle {
    Posix,
    Windows,
}

impl PathStyle {
    /// The syntax of the machine daruda is running on.
    pub const fn local() -> Self {
        if cfg!(windows) {
            PathStyle::Windows
        } else {
            PathStyle::Posix
        }
    }

    /// Windows accepts `/` as well as its own `\`.
    pub fn is_separator(self, c: char) -> bool {
        match self {
            PathStyle::Posix => c == '/',
            PathStyle::Windows => c == '/' || c == '\\',
        }
    }

    /// The separator this style writes when it builds a path.
    pub const fn primary_separator(self) -> char {
        match self {
            PathStyle::Posix => '/',
            PathStyle::Windows => '\\',
        }
    }

    /// Whether `path` is absolute under this style's rules. On Windows that
    /// means a drive root (`C:\`, `C:/`) or a UNC / root-relative `\` start; a
    /// bare `C:x` is drive-relative and so is not.
    pub fn is_absolute(self, path: &str) -> bool {
        match self {
            PathStyle::Posix => path.starts_with('/'),
            PathStyle::Windows => {
                path.starts_with(['/', '\\'])
                    || matches!(
                        path.as_bytes(),
                        [drive, b':', sep, ..]
                            if drive.is_ascii_alphabetic() && (*sep == b'/' || *sep == b'\\')
                    )
            }
        }
    }

    /// Whether `path` names a network share or a device rather than a local
    /// disk: on Windows a UNC path (`\\host\share`, `//host/share`,
    /// `\\?\UNC\…`) or the device namespace (`\\.\pipe\…`). Merely probing
    /// a UNC path authenticates to its host, so an untrusted link must be
    /// refused *before* any filesystem call. A verbatim disk path
    /// (`\\?\C:\…`) is local. Never true under Posix.
    pub fn is_network_or_device(self, path: &str) -> bool {
        match self {
            PathStyle::Posix => false,
            PathStyle::Windows => {
                let mut chars = path.chars();
                let leads_with_two = chars.next().is_some_and(|c| self.is_separator(c))
                    && chars.next().is_some_and(|c| self.is_separator(c));
                if !leads_with_two {
                    return false;
                }
                // `\\?\C:` — the verbatim spelling of an ordinary drive.
                let rest = &path[2..];
                let verbatim_disk = rest.strip_prefix('?').is_some_and(|rest| {
                    matches!(
                        rest.as_bytes(),
                        [sep, drive, b':', ..]
                            if (*sep == b'\\' || *sep == b'/') && drive.is_ascii_alphabetic()
                    )
                });
                !verbatim_disk
            }
        }
    }
}
