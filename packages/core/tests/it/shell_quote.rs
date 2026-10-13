//! Public API tests for shell quote.

use daruda_core::shell::quote::*;
use std::path::PathBuf;

#[test]
fn detect_shell_flavors() {
    let cases = [
        ("/bin/zsh", Shell::Posix),
        ("/bin/bash", Shell::Posix),
        ("sh", Shell::Posix),
        ("/usr/bin/dash", Shell::Posix),
        ("", Shell::Posix),
        ("/usr/bin/myshell", Shell::Posix),
        ("/bin/bash5", Shell::Posix),
        ("/usr/bin/bash-5.2", Shell::Posix),
        ("zsh-5.9", Shell::Posix),
        ("/bin/bash -l", Shell::Posix),
        ("/opt/homebrew/bin/fish", Shell::Fish),
        ("FISH", Shell::Fish),
        ("/usr/bin/fish --no-config", Shell::Fish),
        ("pwsh", Shell::PowerShell),
        ("pwsh.exe", Shell::PowerShell),
        ("pwsh -NoLogo -NoProfile", Shell::PowerShell),
        ("C:\\Windows\\System32\\powershell.exe", Shell::PowerShell),
        ("cmd.exe", Shell::Cmd),
        ("C:\\Windows\\cmd.exe", Shell::Cmd),
    ];

    for (program, expected) in cases {
        assert_eq!(Shell::detect_from_program(program), expected, "{program}");
    }
}

#[test]
fn posix_quote_cases() {
    let cases = [
        ("/Users/me/foo.txt", "/Users/me/foo.txt"),
        ("relative/path-1.rs", "relative/path-1.rs"),
        ("-rf", "-rf"),
        ("--help", "--help"),
        ("/Users/me/My File.txt", "'/Users/me/My File.txt'"),
        ("/x/it's mine.txt", "'/x/it'\\''s mine.txt'"),
        ("", "''"),
        ("/Users/me/한글파일.txt", "'/Users/me/한글파일.txt'"),
        ("/x/файл", "'/x/файл'"),
        ("/x/🦀.rs", "'/x/🦀.rs'"),
        ("a\nb.txt", "'a\nb.txt'"),
        ("a\tb", "'a\tb'"),
        ("a\\b", "'a\\b'"),
        ("~/foo.txt", "'~/foo.txt'"),
        ("$HOME", "'$HOME'"),
        ("$HOME/foo", "'$HOME/foo'"),
        ("   ", "'   '"),
        ("a*", "'a*'"),
        ("a?b", "'a?b'"),
        ("[abc]", "'[abc]'"),
        ("{a,b}", "'{a,b}'"),
        ("a>b", "'a>b'"),
        ("a|b", "'a|b'"),
        ("a;b", "'a;b'"),
        ("a&b", "'a&b'"),
        ("a`b", "'a`b'"),
        ("a\"b", "'a\"b'"),
        ("/x/can't open this.txt", "'/x/can'\\''t open this.txt'"),
    ];

    for (input, expected) in cases {
        assert_eq!(quote_str(input, Shell::Posix), expected, "{input:?}");
    }
}

#[test]
fn fish_quote_cases() {
    let cases = [
        ("it's", "'it\\'s'"),
        ("a\\b c", "'a\\\\b c'"),
        ("C:\\Users", "'C:\\\\Users'"),
    ];

    for (input, expected) in cases {
        assert_eq!(quote_str(input, Shell::Fish), expected, "{input:?}");
    }
}

#[test]
fn powershell_quote_cases() {
    let cases = [
        ("it's", "'it''s'"),
        ("a''b", "'a''''b'"),
        ("C:\\Users\\me\\My File.txt", "'C:\\Users\\me\\My File.txt'"),
    ];

    for (input, expected) in cases {
        assert_eq!(quote_str(input, Shell::PowerShell), expected, "{input:?}");
    }
}

#[test]
fn cmd_quote_cases() {
    let cases = [
        ("C:\\Program Files\\foo", "\"C:\\Program Files\\foo\""),
        (
            "C:\\Program Files\\app.exe",
            "\"C:\\Program Files\\app.exe\"",
        ),
        ("a\"b", "\"a\"\"b\""),
        ("C:\\Users\\me\\", "\"C:\\Users\\me\\\\\""),
        ("C:\\dir\\\\\\", "\"C:\\dir\\\\\\\\\\\\\""),
    ];

    for (input, expected) in cases {
        assert_eq!(quote_str(input, Shell::Cmd), expected, "{input:?}");
    }
}

#[test]
fn format_paths_for_drop_cases() {
    let paths = vec![PathBuf::from("/a/foo.txt"), PathBuf::from("/a/bar baz.txt")];
    assert_eq!(
        format_paths_for_drop(&paths, Shell::Posix),
        "/a/foo.txt '/a/bar baz.txt'"
    );

    let paths: Vec<PathBuf> = Vec::new();
    assert_eq!(format_paths_for_drop(&paths, Shell::Posix), "");

    let paths = vec![PathBuf::from(""), PathBuf::from("/a/b.txt")];
    assert_eq!(format_paths_for_drop(&paths, Shell::Posix), "'' /a/b.txt");
}

#[test]
fn quote_path_cases() {
    assert_eq!(
        quote_path(&PathBuf::from("/Users/me/My Photos"), Shell::Posix),
        "'/Users/me/My Photos'"
    );
    assert_eq!(
        quote_path(&PathBuf::from("/usr/bin/ls"), Shell::Posix),
        "/usr/bin/ls"
    );
}
