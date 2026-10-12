use super::*;
use daruda_acp::ToolStatusView;

#[test]
fn links_the_current_grammar_abi() {
    // Native grammar symbols must not resolve to an older transitive dependency.
    for language in [
        Language::from(tree_sitter_bash::LANGUAGE),
        Language::from(tree_sitter_python::LANGUAGE),
    ] {
        assert_eq!(language.abi_version(), 15);
    }
}

fn call(command: Option<&str>) -> ChatItem {
    ChatItem::ToolCall(ToolCallItem {
        id: "shell-1".into(),
        title: "Inspect the project".into(),
        kind: ToolKindView::Execute,
        tool_name: Some("Bash".into()),
        status: ToolStatusView::InProgress,
        diffs: Vec::new(),
        output: Vec::new(),
        raw_input: command.map(|command| serde_json::json!({ "command": command })),
        parent_tool_id: None,
        locations: Vec::new(),
        exit: None,
    })
}

#[test]
fn classifies_complete_shell_expressions() {
    let cases = [
        (
            "git status --short && git diff --stat",
            vec!["git"],
            Some(CommandEffect::Read),
        ),
        (
            "LC_ALL=C rg -n 'foo' src | head -20",
            vec!["rg", "head"],
            Some(CommandEffect::Read),
        ),
        ("cat input > output", vec!["cat"], Some(CommandEffect::Edit)),
        (
            "rg missing 2>/dev/null",
            vec!["rg"],
            Some(CommandEffect::Read),
        ),
        ("rg missing 2>&1", vec!["rg"], Some(CommandEffect::Read)),
        (
            "git status; rm file",
            vec!["git", "rm"],
            Some(CommandEffect::Edit),
        ),
        (
            "echo $(rm file)",
            vec!["echo", "rm"],
            Some(CommandEffect::Edit),
        ),
        (
            "cat <(rm file)",
            vec!["cat", "rm"],
            Some(CommandEffect::Edit),
        ),
        ("cat <(mystery)", vec!["cat", "mystery"], None),
        ("git status && ./script.sh", vec!["git", "script.sh"], None),
        ("./cat file", vec!["cat"], None),
        ("/usr/bin/ls -la", vec!["ls"], Some(CommandEffect::Read)),
        ("'git' status", vec!["git"], Some(CommandEffect::Read)),
        ("\"$TASK_COMMAND\" --workspace .", vec![], None),
        ("git $SUBCOMMAND", vec!["git"], None),
        ("python3 scripts/task.py", vec!["python3"], None),
        ("cargo test -p daruda", vec!["cargo"], None),
        ("f() { rm file; }; ls", vec!["ls"], None),
        (
            "cat <<'EOF' && rm file\nhello\nEOF",
            vec!["cat", "rm"],
            Some(CommandEffect::Edit),
        ),
        ("# only a comment", vec![], None),
    ];
    for (source, programs, effect) in cases {
        let actual = analyze(source);
        assert_eq!(actual.programs, programs, "programs: {source}");
        assert_eq!(actual.effect, effect, "effect: {source}");
    }
}

#[test]
fn python_heredoc_and_inline_code_are_inspected_as_python() {
    let script = "python3 - <<'EOF'\np = 'file.rs'\ns = open(p).read()\ns = s.replace('a', 'b')\nopen(p, 'w').write(s)\nEOF\ngit diff --stat";
    let analysis = analyze(script);
    assert_eq!(analysis.programs, ["python3", "git"]);
    assert_eq!(analysis.effect, Some(CommandEffect::Edit));
    assert_eq!(
        analyze("python3 -c \"open('file', 'w').write('x')\"").effect,
        Some(CommandEffect::Edit)
    );
    for source in [
        "python3 - <<'EOF'\nprint(\"open(p, 'w')\")\nEOF",
        "python3 - <<'EOF'\n# open(p, 'w')\nprint('hi')\nEOF",
        "python3 - <<EOF\nopen(p, '$MODE')\nEOF",
        "cat <<'EOF'\nopen(p, 'w')\nEOF",
        "python3 task.py <<'EOF'\nopen(p, 'w')\nEOF",
    ] {
        assert_ne!(
            analyze(source).effect,
            Some(CommandEffect::Edit),
            "{source}"
        );
    }
}

#[test]
fn option_sensitive_commands_do_not_confuse_operands_with_actions() {
    for source in [
        "find src -type f -name '*.rs' -print",
        "find . -name '-delete' -print",
        "find . -name '-exec' -print",
        "sort -nr file",
        "sort -t o -k 1 file",
        "sort -- -output-file",
        "git branch",
        "git stash list",
        "git stash show --stat",
        "git worktree list --porcelain",
        "git describe --always",
        "diff -u before after",
        "realpath src",
    ] {
        assert_eq!(
            analyze(source).effect,
            Some(CommandEffect::Read),
            "{source}"
        );
    }
    for source in [
        "find . -name '*.tmp' -delete",
        "find . -fprint results",
        "sort -o output input",
        "sort -nooutput input",
        "sort --output=output input",
        "git stash pop",
        "git stash",
        "git stash show --output=patch",
        "unlink file",
    ] {
        assert_eq!(
            analyze(source).effect,
            Some(CommandEffect::Edit),
            "{source}"
        );
    }
    for source in [
        "find . -exec script.sh '{}' ';'",
        "find . -unknown-predicate",
        "sort --compress-program=script file",
        "git stash show --ext-diff",
        "xargs rm",
        "awk '{ print $0 }' file",
        "curl https://example.com",
        "npm run build",
    ] {
        assert_eq!(analyze(source).effect, None, "{source}");
    }
}

#[test]
fn incomplete_and_oversized_inputs_have_no_metadata() {
    for source in [
        "echo 'unterminated".to_owned(),
        "x".repeat(MAX_COMMAND_BYTES + 1),
    ] {
        assert_eq!(analyze(&source), CommandAnalysis::default());
    }
}

#[test]
fn cache_tracks_raw_input_not_title_or_output() {
    let mut index = CommandAnalysisIndex::default();
    let mut item = call(None);
    index.reconcile(std::slice::from_ref(&item));
    assert!(index.get("shell-1").is_none());
    let ChatItem::ToolCall(tc) = &mut item else {
        panic!("tool fixture");
    };
    tc.raw_input = Some(serde_json::json!({ "command": "git status" }));
    tc.title = "rm file".into();
    index.reconcile(std::slice::from_ref(&item));
    assert_eq!(index.get("shell-1").unwrap().programs, ["git"]);
    let programs = index.get("shell-1").unwrap().programs.as_ptr();
    index.reconcile(std::slice::from_ref(&item));
    assert_eq!(index.get("shell-1").unwrap().programs.as_ptr(), programs);

    let ChatItem::ToolCall(tc) = &mut item else {
        panic!("tool fixture");
    };
    tc.raw_input = Some(serde_json::json!({ "command": "rm file" }));
    index.reconcile(std::slice::from_ref(&item));
    assert_eq!(
        index.get("shell-1").unwrap().effect,
        Some(CommandEffect::Edit)
    );

    let ChatItem::ToolCall(tc) = &mut item else {
        panic!("tool fixture");
    };
    tc.raw_input = Some(serde_json::json!({ "command": ["ls"] }));
    index.reconcile(std::slice::from_ref(&item));
    assert!(index.get("shell-1").is_none());
    index.reconcile(&[call(Some("ls"))]);
    index.reconcile(&[]);
    assert!(index.get("shell-1").is_none());
}

#[test]
fn non_execution_tools_are_not_treated_as_shell_code() {
    let mut item = call(Some("rm file"));
    let ChatItem::ToolCall(tc) = &mut item else {
        panic!("tool fixture");
    };
    tc.kind = ToolKindView::Other;
    let mut index = CommandAnalysisIndex::default();
    index.reconcile(&[item]);
    assert!(index.get("shell-1").is_none());
}
