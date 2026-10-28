//! Path containment shared by flow loading, submission and execution.

use std::path::{Component, Path, PathBuf};

pub(crate) fn stays_relative(path: &Path) -> bool {
    path.components()
        .all(|part| matches!(part, Component::Normal(_) | Component::CurDir))
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum CwdError {
    #[error("could not resolve the working directory: {0}")]
    Io(#[from] std::io::Error),
    #[error("{} resolves outside {}", .cwd.display(), .root.display())]
    Outside { cwd: PathBuf, root: PathBuf },
}

pub(crate) fn check_cwd(root: &Path, cwd: &Path) -> Result<(), CwdError> {
    let root = daruda_core::path::canonicalize(root)?;
    let resolved = daruda_core::path::canonicalize(cwd)?;
    if !daruda_core::path::is_within(&resolved, &root) {
        return Err(CwdError::Outside {
            cwd: resolved,
            root,
        });
    }
    if !resolved.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotADirectory,
            "the working directory is not a directory",
        )
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
