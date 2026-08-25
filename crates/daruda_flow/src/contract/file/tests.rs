//! What is on disk, asked as the scheduler asks it. Every fixture builds a
//! real run directory, because a symlink is the thing under test.

use super::*;

/// The contract asked the way a caller asks it, reported the way a run
/// reports it — so these assertions cover the conversion too.
fn met(run_dir: &Path, output: &Path) -> Result<(), NodeFailure> {
    FileContract::new(run_dir, output, None)
        .check()
        .map_err(NodeFailure::from)
}

/// The same, for a node that also declared a shape.
fn met_shaped(run_dir: &Path, output: &Path, schema: &str) -> Result<(), NodeFailure> {
    let schema: SchemaSubset = yaml_serde::from_str(schema).expect("the fixture is a schema");
    FileContract::new(run_dir, output, Some(&schema))
        .check()
        .map_err(NodeFailure::from)
}

const VERDICT: &str = "\
type: object
required: [verdict]
properties:
  verdict: { type: string, enum: [pass, fail] }
";

/// A run directory with `run_dir` made and nothing in it, plus a
/// non-empty file outside it for a link to aim at.
fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let run_dir = dir.path().join("run");
    std::fs::create_dir_all(&run_dir).expect("mkdir");
    let elsewhere = dir.path().join("elsewhere.md");
    std::fs::write(&elsewhere, "someone else's work\n").expect("write");
    (dir, run_dir, elsewhere)
}

/// The first run's normal state: nothing exists yet on the path, which
/// is not something to refuse.
#[test]
fn an_output_path_with_nothing_on_it_yet_passes_the_preflight() {
    let (_dir, run_dir, _) = fixture();
    assert_eq!(preflight(&run_dir, &run_dir.join("reports/out.md")), Ok(()));
}

/// A directory an earlier node made is a directory, not a redirection.
#[test]
fn a_real_parent_directory_passes_the_preflight() {
    let (_dir, run_dir, _) = fixture();
    std::fs::create_dir_all(run_dir.join("reports")).expect("mkdir");
    assert_eq!(preflight(&run_dir, &run_dir.join("reports/out.md")), Ok(()));
}

/// The hole the preflight exists for: a link where the output's parent
/// goes sends the write out of the run directory, and `create_dir_all`
/// would follow it before the agent is even started.
#[test]
fn a_linked_parent_is_refused_and_the_refusal_names_the_target() {
    let (dir, run_dir, _) = fixture();
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(&outside).expect("mkdir");
    std::os::unix::fs::symlink(&outside, run_dir.join("reports")).expect("symlink");

    let output = run_dir.join("reports/out.md");
    let Err(NodeFailure::OutputEscapes { expected, resolved }) = preflight(&run_dir, &output)
    else {
        panic!("a linked parent must be refused");
    };
    assert_eq!(expected, output);
    assert_eq!(
        resolved,
        std::fs::canonicalize(&outside)
            .expect("real")
            .join("out.md"),
        "the message has to name where the write would land"
    );
}

/// A link standing on the output itself is refused, and refused as the
/// same failure the post-turn check reports — where it points does not
/// change what it is, and neither does when it was planted. The defect
/// underneath: a node under `bypassPermissions` can aim its output at any
/// non-empty file and satisfy a size check having written nothing.
#[test]
fn a_linked_output_is_refused_as_not_a_file_wherever_it_points() {
    let (_dir, run_dir, elsewhere) = fixture();
    let inside = run_dir.join("review.md");
    std::fs::write(&inside, "another node's work\n").expect("write");
    for target in [&elsewhere, &inside] {
        let output = run_dir.join("design.md");
        std::os::unix::fs::symlink(target, &output).expect("symlink");
        assert_eq!(
            preflight(&run_dir, &output),
            Err(NodeFailure::OutputNotAFile {
                expected: output.clone()
            }),
            "a link to {}",
            target.display()
        );
        assert_eq!(met(&run_dir, &output), preflight(&run_dir, &output));
        std::fs::remove_file(&output).expect("unlink");
    }
}

/// What the contract is for, and the one shape that meets it.
#[test]
fn a_plain_non_empty_file_meets_the_contract() {
    let (_dir, run_dir, _) = fixture();
    let output = run_dir.join("design.md");
    std::fs::write(&output, "the design\n").expect("write");
    assert_eq!(met(&run_dir, &output), Ok(()));
}

/// Nothing written, and an empty file, are the same thing to a reader.
#[test]
fn an_absent_or_empty_output_reads_as_nothing_written() {
    let (_dir, run_dir, _) = fixture();
    let output = run_dir.join("design.md");
    assert_eq!(
        met(&run_dir, &output),
        Err(NodeFailure::NoOutput {
            expected: output.clone()
        })
    );
    std::fs::write(&output, "").expect("write");
    assert_eq!(
        met(&run_dir, &output),
        Err(NodeFailure::NoOutput { expected: output })
    );
}

/// A plain file, written through a directory that is a link — the case
/// the preflight prevents and this catches if it is planted mid-turn.
#[test]
fn a_plain_file_under_a_linked_directory_escapes_the_run() {
    let (dir, run_dir, _) = fixture();
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(&outside).expect("mkdir");
    std::os::unix::fs::symlink(&outside, run_dir.join("reports")).expect("symlink");
    let output = run_dir.join("reports/out.md");
    std::fs::write(&output, "outside the run\n").expect("write");

    let Err(NodeFailure::OutputEscapes { expected, resolved }) = met(&run_dir, &output) else {
        panic!("a file outside the run directory must be refused");
    };
    assert_eq!(expected, output);
    assert_eq!(
        resolved,
        std::fs::canonicalize(outside.join("out.md")).expect("real")
    );
}

/// A node that declares a shape owes the file *and* the shape.
#[test]
fn json_matching_the_declared_shape_meets_the_contract() {
    let (_dir, run_dir, _) = fixture();
    let output = run_dir.join("verdict.json");
    std::fs::write(&output, r#"{"verdict": "pass", "notes": "extra"}"#).expect("write");
    assert_eq!(met_shaped(&run_dir, &output, VERDICT), Ok(()));
}

/// The likeliest miss by a long way: the node writes the prose it would
/// have written anyway, and the file is there and non-empty.
#[test]
fn prose_where_json_was_declared_is_a_schema_failure() {
    let (_dir, run_dir, _) = fixture();
    let output = run_dir.join("verdict.json");
    std::fs::write(&output, "The verdict is: pass.\n").expect("write");
    let Err(NodeFailure::OutputSchema {
        expected,
        problem,
        more,
    }) = met_shaped(&run_dir, &output, VERDICT)
    else {
        panic!("prose is not the declared shape");
    };
    assert_eq!(expected, output);
    assert!(problem.contains("not a single JSON value"), "{problem}");
    assert_eq!(more, 0, "one problem, not a list of them");
}

/// Valid JSON of the wrong shape, and the count the failure line carries —
/// the whole list is the correction prompt's, not `run.md`'s.
#[test]
fn json_of_the_wrong_shape_reports_one_line_and_a_count() {
    let (_dir, run_dir, _) = fixture();
    let output = run_dir.join("verdict.json");
    std::fs::write(&output, r#"{"verdict": "maybe", "n": 1}"#).expect("write");
    let Err(NodeFailure::OutputSchema { problem, more, .. }) =
        met_shaped(&run_dir, &output, VERDICT)
    else {
        panic!("`maybe` is not one of the listed verdicts");
    };
    assert_eq!(
        problem,
        r#"$.verdict: expected one of ["pass","fail"], found "maybe""#
    );
    assert_eq!(more, 0);
    assert!(
        format!(
            "{}",
            NodeFailure::OutputSchema {
                expected: output.clone(),
                problem,
                more: 2
            }
        )
        .ends_with("(and 2 more)"),
        "the line says how much it is not saying"
    );
}

/// A shape question is asked last: the file being absent is a different
/// failure, and reporting it as a schema problem would send a correction
/// after the wrong thing.
#[test]
fn a_missing_file_is_missing_rather_than_misshapen() {
    let (_dir, run_dir, _) = fixture();
    let output = run_dir.join("verdict.json");
    assert_eq!(
        met_shaped(&run_dir, &output, VERDICT),
        Err(NodeFailure::NoOutput {
            expected: output.clone()
        })
    );
    std::os::unix::fs::symlink(run_dir.join("elsewhere.json"), &output).expect("symlink");
    std::fs::write(run_dir.join("elsewhere.json"), r#"{"verdict": "pass"}"#).expect("write");
    assert_eq!(
        met_shaped(&run_dir, &output, VERDICT),
        Err(NodeFailure::OutputNotAFile { expected: output }),
        "a link to conforming JSON is still not this node's work"
    );
}

/// On macOS the run directory is itself reached through a link (`/var`
/// and `/tmp` both are), so comparing a resolved output against an
/// unresolved root would refuse every output on the primary platform.
#[test]
fn a_run_directory_reached_through_a_link_is_not_an_escape() {
    let (_dir, run_dir, _) = fixture();
    let linked_root = run_dir.parent().expect("parent").join("linked");
    std::os::unix::fs::symlink(&run_dir, &linked_root).expect("symlink");
    let output = linked_root.join("design.md");
    std::fs::write(&output, "the design\n").expect("write");
    assert_eq!(met(&linked_root, &output), Ok(()));
}

/// The bypass the two-sided walk exists for, and the one a resolved-path
/// comparison cannot see: the link points *back inside* the run directory,
/// at a sibling node's real output. Every check made on the resolved path —
/// plain file, non-empty, under the run root — says yes, and the node is
/// credited with work another node did.
///
/// `preflight` cannot catch it either: it walked a real `notes/` before the
/// turn, and the swap happened during one.
#[test]
fn a_parent_swapped_for_a_link_to_a_sibling_output_is_not_this_nodes_work() {
    let (_dir, run_dir, _) = fixture();
    std::fs::create_dir_all(run_dir.join("reports")).expect("mkdir");
    std::fs::write(run_dir.join("reports/summary.md"), "node A wrote this\n").expect("write");

    // Before the turn: a real directory, which preflight passes.
    let planted = run_dir.join("notes");
    std::fs::create_dir_all(&planted).expect("mkdir");
    assert_eq!(preflight(&run_dir, &planted.join("summary.md")), Ok(()));

    // During the turn: swapped for a link at the parent.
    std::fs::remove_dir(&planted).expect("rmdir");
    std::os::unix::fs::symlink("reports", &planted).expect("symlink");

    let output = planted.join("summary.md");
    assert!(
        output.is_file(),
        "the fixture only bites if the link resolves"
    );
    assert!(
        matches!(
            met(&run_dir, &output),
            Err(NodeFailure::OutputEscapes { .. })
        ),
        "resolving inside the run directory is not the same as being written there"
    );
}
