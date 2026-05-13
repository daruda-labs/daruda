//! Extension trait for `std::path::Path`.

use std::path::Path;

pub trait PathExt {
    /// Returns `true` when the path is a directory this process can read.
    ///
    /// Unlike `is_dir()`, this also rejects TCC-restricted directories on
    /// macOS (e.g. Desktop/Downloads without Full Disk Access) where
    /// `stat()` succeeds but `readdir()` is denied.
    fn is_accessible_dir(&self) -> bool;

    /// File extension as a UTF-8 `&str`, or `""` when absent or non-UTF-8.
    fn extension_str(&self) -> &str;

    /// File extension lowercased to a `String`, or `None` when absent.
    fn extension_lower(&self) -> Option<String>;

    /// Last path component as a lossily-decoded `String`. Falls back to
    /// the full path when there is no file-name component (e.g. `"/"`).
    fn file_name_lossy(&self) -> String;

    /// Parent directory. Falls back to `"."` for paths with no parent
    /// (root or single-component relative paths).
    fn parent_or_current(&self) -> &Path;

    /// Strip `base` prefix. Returns `self` unchanged when the path does not
    /// start with `base` — avoids the `strip_prefix(...).unwrap_or(self)`
    /// boilerplate at display-path call sites.
    fn strip_prefix_or_self<'a>(&'a self, base: &Path) -> &'a Path;
}

impl PathExt for Path {
    fn is_accessible_dir(&self) -> bool {
        self.read_dir().is_ok()
    }

    fn extension_str(&self) -> &str {
        self.extension().and_then(|e| e.to_str()).unwrap_or("")
    }

    fn extension_lower(&self) -> Option<String> {
        self.extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
    }

    fn file_name_lossy(&self) -> String {
        self.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.to_string_lossy().into_owned())
    }

    fn parent_or_current(&self) -> &Path {
        self.parent().unwrap_or(Path::new("."))
    }

    fn strip_prefix_or_self<'a>(&'a self, base: &Path) -> &'a Path {
        self.strip_prefix(base).unwrap_or(self)
    }
}
