//! Conservative effect rules for literal shell arguments.

use super::CommandEffect;

// Only commands whose normal options do not write user files belong here.
const READ_PROGRAMS: &[&str] = &[
    "cat", "ls", "head", "tail", "wc", "pwd", "stat", "du", "uname", "whoami", "which", "type",
    "echo", "printf", "true", "false", "test", "[", "grep", "egrep", "fgrep", "cut", "tr",
    "basename", "dirname", "readlink", "realpath", "id", "groups", "printenv", "nl", "od", "cmp",
    "diff",
];
const EDIT_PROGRAMS: &[&str] = &[
    "cp", "mv", "rm", "mkdir", "rmdir", "touch", "truncate", "install", "chmod", "chown", "ln",
    "patch", "tee", "unlink",
];
const GIT_READ_COMMANDS: &[&str] = &[
    "status",
    "diff",
    "log",
    "show",
    "ls-files",
    "ls-tree",
    "rev-parse",
    "rev-list",
    "diff-files",
    "diff-index",
    "diff-tree",
    "describe",
    "check-ignore",
    "count-objects",
];
const GIT_EDIT_COMMANDS: &[&str] = &[
    "add",
    "commit",
    "reset",
    "restore",
    "checkout",
    "switch",
    "merge",
    "rebase",
    "cherry-pick",
    "revert",
    "apply",
    "am",
    "clean",
    "rm",
    "mv",
    "fetch",
    "pull",
    "clone",
    "init",
];

pub(super) fn effect(program: &str, args: &[String]) -> Option<CommandEffect> {
    use CommandEffect::{Edit, Read};
    match program {
        "rg" if !args.iter().any(|arg| {
            matches!(arg.as_str(), "--pre" | "--hostname-bin")
                || arg.starts_with("--pre=")
                || arg.starts_with("--hostname-bin=")
        }) =>
        {
            Some(Read)
        }
        "sed" => sed(args),
        "find" => find(args),
        "sort" => sort(args),
        "git" => git(args),
        _ if READ_PROGRAMS.contains(&program) => Some(Read),
        _ if EDIT_PROGRAMS.contains(&program) => Some(Edit),
        _ => None,
    }
}

fn find(args: &[String]) -> Option<CommandEffect> {
    let mut args = args.iter().peekable();
    // Starting paths precede the expression; predicate operands are not actions.
    while let Some(arg) = args.peek() {
        match arg.as_str() {
            "-H" | "-L" | "-P" => {
                args.next();
            }
            path if !path.starts_with(['-', '!', '(']) => {
                args.next();
            }
            _ => break,
        }
    }
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-delete" | "-fprint" | "-fprint0" | "-fprintf" | "-fls" => {
                return Some(CommandEffect::Edit);
            }
            "-name" | "-iname" | "-path" | "-ipath" | "-type" | "-xtype" | "-maxdepth"
            | "-mindepth" | "-size" | "-mtime" | "-mmin" | "-atime" | "-ctime" | "-newer"
            | "-user" | "-group" | "-perm" | "-regex" | "-iregex" | "-printf" => {
                args.next()?;
            }
            "-print" | "-print0" | "-ls" | "-prune" | "-quit" | "-empty" | "-readable"
            | "-writable" | "-executable" | "-depth" | "-xdev" | "-mount" | "-true" | "-false"
            | "-a" | "-and" | "-o" | "-or" | "!" | "-not" | "(" | ")" | "," => {}
            // In particular, -exec/-execdir/-ok invoke arbitrary programs.
            _ => return None,
        }
    }
    Some(CommandEffect::Read)
}

fn sort(args: &[String]) -> Option<CommandEffect> {
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--" => break,
            "--output" => {
                args.next()?;
                return Some(CommandEffect::Edit);
            }
            option if option.starts_with("--output=") => return Some(CommandEffect::Edit),
            "--reverse"
            | "--unique"
            | "--numeric-sort"
            | "--general-numeric-sort"
            | "--human-numeric-sort"
            | "--version-sort"
            | "--ignore-case" => {}
            option if option.starts_with("--") => return None,
            option if option.starts_with('-') => {
                let mut flags = option[1..].chars().peekable();
                while let Some(flag) = flags.next() {
                    match flag {
                        'o' => {
                            if flags.peek().is_none() {
                                args.next()?;
                            }
                            return Some(CommandEffect::Edit);
                        }
                        'k' | 't' => {
                            if flags.peek().is_none() {
                                args.next()?;
                            }
                            break;
                        }
                        'b' | 'd' | 'f' | 'g' | 'h' | 'i' | 'M' | 'n' | 'r' | 'R' | 'V' | 'c'
                        | 'C' | 'm' | 's' | 'u' | 'z' => {}
                        _ => return None,
                    }
                }
            }
            _ => {}
        }
    }
    Some(CommandEffect::Read)
}

fn sed(args: &[String]) -> Option<CommandEffect> {
    if args
        .iter()
        .take_while(|arg| arg.as_str() != "--")
        .any(|arg| {
            arg == "--in-place"
                || arg.starts_with("--in-place=")
                || (arg.starts_with('-') && !arg.starts_with("--") && arg.contains('i'))
        })
    {
        return Some(CommandEffect::Edit);
    }
    // The common inspection form. Other sed programs may execute or write.
    let [flag, script, files @ ..] = args else {
        return None;
    };
    (flag == "-n"
        && script.ends_with('p')
        && script[..script.len() - 1]
            .bytes()
            .all(|b| b.is_ascii_digit() || b == b',' || b == b'$')
        && !files.iter().any(|file| file.starts_with('-')))
    .then_some(CommandEffect::Read)
}

fn git(args: &[String]) -> Option<CommandEffect> {
    let mut args = args;
    while let Some(first) = args.first() {
        match first.as_str() {
            "--no-pager" | "--no-optional-locks" => args = &args[1..],
            "-C" => args = args.get(2..)?,
            flag if flag.starts_with('-') => return None,
            _ => break,
        }
    }
    let (command, args) = args.split_first()?;
    if args
        .iter()
        .any(|arg| arg == "--output" || arg.starts_with("--output="))
    {
        return Some(CommandEffect::Edit);
    }
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "--ext-diff" | "--textconv"))
    {
        return None;
    }
    match command.as_str() {
        "branch" if args.is_empty() || args == ["--show-current"] || args == ["--list"] => {
            Some(CommandEffect::Read)
        }
        "stash"
            if args
                .first()
                .is_some_and(|arg| matches!(arg.as_str(), "list" | "show")) =>
        {
            Some(CommandEffect::Read)
        }
        "stash"
            if args.is_empty()
                || args.first().is_some_and(|arg| {
                    matches!(
                        arg.as_str(),
                        "push" | "pop" | "apply" | "drop" | "clear" | "save" | "store" | "branch"
                    )
                }) =>
        {
            Some(CommandEffect::Edit)
        }
        "worktree" if args.first().is_some_and(|arg| arg == "list") => Some(CommandEffect::Read),
        command if GIT_READ_COMMANDS.contains(&command) => Some(CommandEffect::Read),
        command if GIT_EDIT_COMMANDS.contains(&command) => Some(CommandEffect::Edit),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_can_change_the_effect() {
        let classify = |program, args: &[&str]| {
            effect(
                program,
                &args.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(),
            )
        };
        assert_eq!(classify("rg", &["--pre=script", "pattern"]), None);
        assert_eq!(
            classify("sed", &["-n", "1,20p", "file"]),
            Some(CommandEffect::Read)
        );
        assert_eq!(classify("sed", &["-n", "w out", "file"]), None);
        assert_eq!(
            classify("sed", &["-i", "s/a/b/", "file"]),
            Some(CommandEffect::Edit)
        );
        assert_eq!(
            classify("git", &["-C", "repo", "status"]),
            Some(CommandEffect::Read)
        );
        assert_eq!(classify("git", &["-c", "alias.foo=bar", "foo"]), None);
        assert_eq!(
            classify("git", &["diff", "--output=patch"]),
            Some(CommandEffect::Edit)
        );
    }
}
