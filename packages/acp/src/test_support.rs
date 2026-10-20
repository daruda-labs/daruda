//! Fixtures shared by this crate's unit tests.

use std::path::Path;

/// Write an executable script at `path` without this process ever holding it
/// open for writing.
///
/// Linux refuses to exec a file any process has open for writing
/// (`ETXTBSY`). A sibling test that forks between our `write` and its own
/// child's exec copies that descriptor into the child (`O_CLOEXEC` only closes
/// it at exec), so running a script this process just wrote fails at random
/// under the parallel test runner. A short-lived writer process owns the only
/// writable descriptor, and it has exited before this returns.
pub(crate) fn write_executable(path: &Path, contents: &str) {
    let status = daruda_core::process::command("/bin/sh")
        .arg("-c")
        .arg(r#"printf '%s' "$2" > "$1" && chmod 755 "$1""#)
        .arg("write_executable")
        .arg(path)
        .arg(contents)
        .status()
        .unwrap();
    assert!(
        status.success(),
        "writing {} failed: {status}",
        path.display()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_written_script_is_byte_exact_and_runs() {
        let directory = tempfile::tempdir().unwrap();
        let script = directory.path().join("dir with spaces").join("tool");
        std::fs::create_dir(script.parent().unwrap()).unwrap();
        let contents = "#!/bin/sh\nprintf '%s' \"it's $1\"\n";
        write_executable(&script, contents);
        assert_eq!(std::fs::read_to_string(&script).unwrap(), contents);
        let output = daruda_core::process::command(&script)
            .arg("ok")
            .output()
            .unwrap();
        assert_eq!(String::from_utf8(output.stdout).unwrap(), "it's ok");
    }
}
