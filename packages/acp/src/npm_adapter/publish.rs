//! Bounded publication retries for Windows installation-directory contention.

use std::path::Path;

use anyhow::Context as _;

use crate::preparation::PreparationContext;

const RETRY_DELAYS_MS: [u64; 5] = [50, 100, 150, 200, 250];

pub(super) fn rename(
    source: &Path,
    target: &Path,
    context: &PreparationContext<'_>,
) -> anyhow::Result<()> {
    let mut attempts = 0;
    loop {
        context.check()?;
        attempts += 1;
        match std::fs::rename(source, target) {
            Ok(()) => return Ok(()),
            Err(error) => {
                if let Some(delay) = retry_delay(&error, attempts) {
                    // Windows scanners can briefly hold files beneath this directory.
                    // This runs on the preparation worker; cancellation is checked each attempt.
                    std::thread::sleep(delay);
                    continue;
                }
                return Err(error).with_context(|| {
                    format!(
                        "renaming adapter directory {} to {} after {attempts} attempt(s)",
                        source.display(),
                        target.display()
                    )
                });
            }
        }
    }
}

fn retry_delay(error: &std::io::Error, attempts: usize) -> Option<std::time::Duration> {
    use daruda_core::path::io_error::{FileAccessFailure, classify};
    if matches!(
        classify(error),
        FileAccessFailure::AccessDenied
            | FileAccessFailure::SharingViolation
            | FileAccessFailure::LockViolation
    ) {
        return attempts
            .checked_sub(1)
            .and_then(|index| RETRY_DELAYS_MS.get(index))
            .map(|delay| std::time::Duration::from_millis(*delay));
    }
    None
}

#[cfg(test)]
mod tests;
