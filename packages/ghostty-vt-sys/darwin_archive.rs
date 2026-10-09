use std::path::Path;
use std::process::Command;

pub fn repack(source: &Path, destination: &Path) {
    let objects_dir = destination.join("objects");
    std::fs::create_dir_all(&objects_dir).expect("create archive extraction directory");
    // This directory contains only members extracted by this build script.
    // Clear old members so a rebuilt library cannot retain removed objects.
    for entry in std::fs::read_dir(&objects_dir).expect("read extraction directory") {
        std::fs::remove_file(entry.expect("read archive member").path())
            .expect("remove stale archive member");
    }

    // Passing the Zig archive directly to Apple's libtool can silently drop
    // members. Extract first, then rebuild from individual Mach-O objects.
    let status = Command::new("xcrun")
        .args(["ar", "x"])
        .arg(source)
        .current_dir(&objects_dir)
        .status()
        .expect("invoke xcrun ar; install Xcode Command Line Tools");
    assert!(
        status.success(),
        "extract Ghostty archive: {}",
        source.display()
    );

    let mut objects = Vec::new();
    for entry in std::fs::read_dir(&objects_dir).expect("read extracted members") {
        let path = entry.expect("read archive member").path();
        if path.extension().is_some_and(|extension| extension == "o") {
            objects.push(path);
        }
    }
    objects.sort();
    assert!(
        !objects.is_empty(),
        "Ghostty archive contains no object members"
    );

    // Zig archive members can extract with mode 000; libtool must read them.
    let status = Command::new("chmod")
        .arg("644")
        .args(&objects)
        .status()
        .expect("set extracted object permissions");
    assert!(status.success(), "set Ghostty object permissions");

    let archive = destination.join("libghostty_vt.a");
    // Keep Apple's tools outside the Zig command's DEVELOPER_DIR override.
    let status = Command::new("xcrun")
        .args(["libtool", "-static", "-o"])
        .arg(&archive)
        .args(&objects)
        .status()
        .expect("invoke xcrun libtool");
    assert!(
        status.success(),
        "repack Ghostty archive: {}",
        archive.display()
    );

    let symbols = Command::new("xcrun")
        .args(["nm", "-gU"])
        .arg(&archive)
        .output()
        .expect("inspect repacked Ghostty symbols");
    assert!(
        symbols.status.success(),
        "inspect Ghostty archive: {}",
        String::from_utf8_lossy(&symbols.stderr)
    );
    let symbols = String::from_utf8_lossy(&symbols.stdout);
    for line in include_str!("src/lib.rs").lines() {
        if let Some(declaration) = line.trim().strip_prefix("pub fn ") {
            let name = declaration.split('(').next().expect("FFI function name");
            let symbol = format!("_{name}");
            assert!(
                symbols
                    .lines()
                    .any(|line| line.split_whitespace().last() == Some(symbol.as_str())),
                "repacked Ghostty archive is missing {symbol}: {}",
                archive.display()
            );
        }
    }
}
