//! Native file access failures relevant to bounded publication retries.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileAccessFailure {
    AccessDenied,
    SharingViolation,
    LockViolation,
    Other,
}

pub fn classify(error: &std::io::Error) -> FileAccessFailure {
    classify_for(error.raw_os_error(), cfg!(windows))
}

fn classify_for(code: Option<i32>, windows: bool) -> FileAccessFailure {
    if !windows {
        return FileAccessFailure::Other;
    }
    match code {
        Some(5) => FileAccessFailure::AccessDenied,
        Some(32) => FileAccessFailure::SharingViolation,
        Some(33) => FileAccessFailure::LockViolation,
        _ => FileAccessFailure::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_error_numbers_do_not_leak_between_platforms() {
        assert_eq!(
            classify_for(Some(32), true),
            FileAccessFailure::SharingViolation
        );
        assert_eq!(
            classify_for(Some(33), true),
            FileAccessFailure::LockViolation
        );
        assert_eq!(classify_for(Some(5), true), FileAccessFailure::AccessDenied);
        assert_eq!(classify_for(Some(32), false), FileAccessFailure::Other);
        assert_eq!(classify_for(None, true), FileAccessFailure::Other);
    }
}
