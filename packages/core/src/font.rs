//! Shared factory monospace families; font objects belong to the UI layer.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontPlatform {
    Macos,
    Windows,
    Linux,
}

impl FontPlatform {
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::Macos
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Linux
        }
    }
    pub fn monospace_family(self) -> &'static str {
        match self {
            Self::Macos => "Monaco",
            Self::Windows => "Consolas",
            Self::Linux => "DejaVu Sans Mono",
        }
    }
}

pub fn default_monospace_family() -> &'static str {
    FontPlatform::current().monospace_family()
}
