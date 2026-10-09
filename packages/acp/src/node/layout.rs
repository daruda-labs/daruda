//! Pure layout rules for Node distributions, independent of the build host.

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NodeLayout {
    Windows,
    Unix,
}

impl NodeLayout {
    pub(super) fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else {
            Self::Unix
        }
    }
    pub(super) fn bin_dir(self, root: &Path) -> PathBuf {
        match self {
            Self::Windows => root.to_path_buf(),
            Self::Unix => root.join("bin"),
        }
    }
    pub(super) fn launcher(self, root: &Path, launcher: &str) -> PathBuf {
        let name = match (self, launcher) {
            (Self::Windows, "node") => "node.exe",
            (Self::Windows, "npx") => "npx.cmd",
            _ => launcher,
        };
        self.bin_dir(root).join(name)
    }
    pub(super) fn npm_entry(self, bin: &Path) -> PathBuf {
        bin.join(match self {
            Self::Windows => "node_modules/npm/bin/npm-cli.js",
            Self::Unix => "npm",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn node_and_npm_are_paired_in_both_distribution_layouts() {
        let root = Path::new("runtime");
        assert_eq!(
            NodeLayout::Windows.launcher(root, "node"),
            root.join("node.exe")
        );
        assert_eq!(
            NodeLayout::Unix.launcher(root, "node"),
            root.join("bin/node")
        );
        assert_eq!(
            NodeLayout::Windows.npm_entry(&NodeLayout::Windows.bin_dir(root)),
            root.join("node_modules/npm/bin/npm-cli.js")
        );
        assert_eq!(
            NodeLayout::Unix.npm_entry(&NodeLayout::Unix.bin_dir(root)),
            root.join("bin/npm")
        );
    }
}
