//! Shell-aware quoting — a path dropped into the terminal input has to
//! reach the shell as one token, spaces and metacharacters included.
//!
//! Which rules apply is read from the shell's own name, so the caller passes
//! whichever shell it spawned rather than asking what platform it is on.

use std::path::{Path, PathBuf};

/// Shell flavour for quoting. Falls back to [`Shell::Posix`] for unknown
/// programs so a typo or custom wrapper still produces a quoted token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(clippy::enum_variant_names)] // PowerShell is the product name; renaming to Pwsh hurts readability
pub enum Shell {
    /// `bash`, `zsh`, `sh`, `dash`, `ksh` — POSIX rules. Inside `'…'` an
    /// embedded `'` is escaped as `'\''`.
    #[default]
    Posix,
    /// `fish` — single-quoted strings are literal; only `'` and `\` are
    /// escaped (with a leading backslash).
    Fish,
    /// `pwsh`, `powershell` — single-quoted strings are literal; an embedded
    /// `'` is doubled (`''`).
    PowerShell,
    /// Windows `cmd.exe` — wraps in `"…"`; an embedded `"` is doubled (`""`).
    Cmd,
}

impl Shell {
    /// Detect the shell flavour from a program path or name. The match is
    /// case-insensitive and uses the file stem so `/opt/homebrew/bin/fish`,
    /// `fish`, and `Fish.exe` all resolve to [`Shell::Fish`].
    ///
    /// Robust against:
    /// - Windows-style paths captured on a Unix host (splits on both `/`
    ///   and `\`) — `Path::file_stem` only treats `/` as a separator on
    ///   Unix and would otherwise glue the whole string into one filename.
    /// - Programs carrying arguments (`/bin/bash -l`, `fish --no-config`)
    ///   — the first whitespace-delimited token is treated as the program.
    pub fn detect_from_program(program: &str) -> Self {
        let program = program.split_ascii_whitespace().next().unwrap_or(program);
        let basename = program.rsplit(['/', '\\']).next().unwrap_or("");
        let stem = basename
            .rsplit_once('.')
            .map(|(s, _)| s)
            .unwrap_or(basename);
        match stem.to_ascii_lowercase().as_str() {
            "fish" => Shell::Fish,
            "pwsh" | "powershell" => Shell::PowerShell,
            "cmd" => Shell::Cmd,
            _ => Shell::Posix,
        }
    }
}

/// Quote `path` so the active shell receives it as a single token.
pub fn quote_path(path: &Path, shell: Shell) -> String {
    quote_str(&path.to_string_lossy(), shell)
}

/// Quote an arbitrary string using [`Shell`]-specific rules.
pub fn quote_str(s: &str, shell: Shell) -> String {
    match shell {
        Shell::Posix => quote_posix(s),
        Shell::Fish => quote_fish(s),
        Shell::PowerShell => quote_pwsh(s),
        Shell::Cmd => quote_cmd(s),
    }
}

/// Format multiple dropped paths for insertion at the cursor. Paths are
/// quoted individually and joined with a single space so the shell tokenizer
/// sees them as separate arguments.
pub fn format_paths_for_drop(paths: &[PathBuf], shell: Shell) -> String {
    paths
        .iter()
        .map(|p| quote_path(p, shell))
        .collect::<Vec<_>>()
        .join(" ")
}

const POSIX_SAFE: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_./";

fn is_posix_safe(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| POSIX_SAFE.contains(&b))
}

fn quote_posix(s: &str) -> String {
    if is_posix_safe(s) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

fn quote_fish(s: &str) -> String {
    if is_posix_safe(s) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        match ch {
            '\'' => out.push_str("\\'"),
            '\\' => out.push_str("\\\\"),
            other => out.push(other),
        }
    }
    out.push('\'');
    out
}

fn quote_pwsh(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

fn quote_cmd(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        if ch == '"' {
            out.push_str("\"\"");
        } else {
            out.push(ch);
        }
    }
    // Trailing backslashes immediately before the closing `"` are
    // interpreted as escaping it under C runtime parsing
    // (CommandLineToArgvW), turning `"C:\Users\foo\"` into an unterminated
    // string and leaking the next argv token into our path. Double every
    // trailing backslash so each ends up as a literal `\` instead.
    let trailing = out.len() - out.trim_end_matches('\\').len();
    for _ in 0..trailing {
        out.push('\\');
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posix_safe_set_excludes_non_ascii() {
        assert!(!is_posix_safe("café"));
        assert!(!is_posix_safe("한"));
    }
}
