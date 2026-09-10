use super::*;

/// A settled instant, so a record rendered in a test reads the same on
/// every machine and at every hour. `Instant::now()` in a fixture is a
/// test that passes because nobody looked at the line it produced.
const FIXED_INSTANT: std::time::SystemTime = std::time::SystemTime::UNIX_EPOCH;

fn attempt(n: u32, seq: u32, outcome: AttemptOutcome) -> AttemptRecord {
    AttemptRecord {
        tools: Vec::new(),
        attempt: n,
        evidence_seq: seq,
        at: FIXED_INSTANT,
        took: Duration::from_secs(0),
        outcome,
        invalidated: Invalidation::default(),
        git_status: None,
        waited: Waiting::default(),
        turns: 1,
        usage: None,
    }
}

fn spent(runs: u32) -> Spent {
    Spent {
        node_runs: runs,
        ..Spent::default()
    }
}

/// A journal an earlier build left behind still reads. The line is
/// written out here by hand rather than round-tripped, because a
/// round-trip through our own writer agrees with itself no matter what
/// shape it picked — and what has to hold is that `node` and
/// `invalidated` are still bare strings, which is how every journal
/// already on disk spells them.
#[test]
fn a_journal_written_before_an_id_was_a_type_still_reads() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(JOURNAL_FILE);
    std::fs::write(
        &path,
        concat!(
            r#"{"kind":"started","v":1}"#,
            "\n",
            r#"{"kind":"attempt","v":1,"node":"design","attempt":1,"#,
            r#""evidence_seq":1,"outcome":{"result":"passed"},"#,
            r#""invalidated":["stale"],"spent":{"node_runs":1}}"#,
            "\n",
        ),
    )
    .expect("write");

    let replay = read(dir.path());
    assert_eq!(replay.passed, vec![NodeId::from("design")]);
    assert_eq!(
        replay.records[0].attempts[0].invalidated.nodes,
        vec![NodeId::from("stale")]
    );
    assert!(!replay.torn, "a line an older build wrote is not damage");
}

/// The other half of the same contract: what we write now is still what
/// an earlier build would recognise.
#[test]
fn an_id_reaches_the_file_as_a_bare_string() {
    let dir = tempfile::tempdir().expect("tempdir");
    start(dir.path(), None, None, &[]).expect("start");
    append_attempt(
        dir.path(),
        &"design".into(),
        &attempt(1, 1, AttemptOutcome::Passed),
        &spent(1),
    )
    .expect("append");
    let text = std::fs::read_to_string(dir.path().join(JOURNAL_FILE)).expect("read");
    assert!(text.contains(r#""node":"design""#), "{text}");
}

/// The whole point: which nodes passed survives the process. Nothing
/// else on disk can say it — a command node's pass writes no file.
#[test]
fn what_passed_reads_back() {
    let dir = tempfile::tempdir().expect("tempdir");
    start(dir.path(), Some("cheap"), None, &[]).expect("start");
    append_attempt(
        dir.path(),
        &"design".into(),
        &attempt(1, 1, AttemptOutcome::Passed),
        &spent(1),
    )
    .expect("append");
    append_attempt(
        dir.path(),
        &"gate".into(),
        &attempt(
            1,
            2,
            AttemptOutcome::Reported("failed: no output".to_string()),
        ),
        &spent(2),
    )
    .expect("append");

    let replay = read(dir.path());
    assert_eq!(replay.passed, vec![NodeId::from("design")]);
    assert_eq!(replay.profile.as_deref(), Some("cheap"));
    assert_eq!(replay.spent.node_runs, 2);
    assert_eq!(replay.records.len(), 2, "both nodes have a history");
    assert!(!replay.torn);
}

/// A repair failure invalidates outputs that used to be good. If the
/// process dies after that line, a resume must re-run those nodes rather
/// than skipping them as already passed.
#[test]
fn invalidation_takes_nodes_back_out_of_the_passed_set() {
    let dir = tempfile::tempdir().expect("tempdir");
    start(dir.path(), None, None, &[]).expect("start");
    append_attempt(
        dir.path(),
        &"design".into(),
        &attempt(1, 1, AttemptOutcome::Passed),
        &spent(1),
    )
    .expect("append");
    let mut gate = attempt(
        1,
        2,
        AttemptOutcome::Reported("failed: exit status 1".to_string()),
    );
    gate.invalidated.nodes = vec!["design".into(), "gate".into()];
    append_attempt(dir.path(), &"gate".into(), &gate, &spent(2)).expect("append");

    assert!(
        read(dir.path()).passed.is_empty(),
        "an invalidated node was still treated as passed"
    );
}

/// Once currencies mix, later costs are ignored. That is state, not a
/// warning alone, so a resume has to keep carrying it.
#[test]
fn mixed_cost_accounting_reads_back() {
    let dir = tempfile::tempdir().expect("tempdir");
    start(dir.path(), None, None, &[]).expect("start");
    append_attempt(
        dir.path(),
        &"design".into(),
        &attempt(1, 1, AttemptOutcome::Passed),
        &Spent {
            node_runs: 1,
            cost: Some(CostLimit {
                amount: 2.0,
                currency: "USD".to_string(),
            }),
            cost_mixed: true,
            ..Spent::default()
        },
    )
    .expect("append");

    assert!(read(dir.path()).spent.cost_mixed);
}

/// The evidence counter continues. Restarting it would make the first
/// attempt after a resume write `evidence-1` over a log that is still
/// the only account of what happened the first time.
#[test]
fn the_evidence_counter_continues_where_it_stopped() {
    let dir = tempfile::tempdir().expect("tempdir");
    start(dir.path(), None, None, &[]).expect("start");
    for seq in 1..=4 {
        append_attempt(
            dir.path(),
            &"a".into(),
            &attempt(seq, seq, AttemptOutcome::Passed),
            &spent(seq),
        )
        .expect("append");
    }
    assert_eq!(read(dir.path()).next_seq, 5);
}

/// The writer is a process that can be killed mid-write, so the last
/// line can be half a line. It is dropped, everything before it stands,
/// and the reader says so rather than pretending the run stopped there.
#[test]
fn a_line_torn_by_the_kill_costs_only_itself() {
    let dir = tempfile::tempdir().expect("tempdir");
    start(dir.path(), None, None, &[]).expect("start");
    append_attempt(
        dir.path(),
        &"design".into(),
        &attempt(1, 1, AttemptOutcome::Passed),
        &spent(1),
    )
    .expect("append");
    let path = dir.path().join(JOURNAL_FILE);
    let mut text = std::fs::read_to_string(&path).expect("read");
    text.push_str("{\"kind\":\"attempt\",\"v\":1,\"node\":\"ga");
    std::fs::write(&path, text).expect("write");

    let replay = read(dir.path());
    assert_eq!(replay.passed, vec![NodeId::from("design")]);
    assert_eq!(replay.spent.node_runs, 1, "the torn line was counted");
    assert!(replay.torn, "the tear was swallowed");
}

/// A build reading a newer journal skips what it does not know instead
/// of refusing the whole file — the shape is expected to change, and a
/// resume that dies on an unknown line is worse than one that resumes
/// from what it understood.
#[test]
fn an_entry_from_a_newer_build_is_skipped_not_fatal() {
    let dir = tempfile::tempdir().expect("tempdir");
    start(dir.path(), None, None, &[]).expect("start");
    append_attempt(
        dir.path(),
        &"design".into(),
        &attempt(1, 1, AttemptOutcome::Passed),
        &spent(1),
    )
    .expect("append");
    let path = dir.path().join(JOURNAL_FILE);
    let mut text = std::fs::read_to_string(&path).expect("read");
    text.push_str("{\"kind\":\"from_the_future\",\"v\":99}\n");
    text.push_str("{\"kind\":\"attempt\",\"v\":99,\"node\":\"x\"}\n");
    std::fs::write(&path, text).expect("write");

    let replay = read(dir.path());
    assert_eq!(replay.passed, vec![NodeId::from("design")]);
    assert!(!replay.torn, "a newer entry is not damage");
}

/// **The trap this closes.** `absorb` destructures with a trailing `..`,
/// so a field added to the line compiles clean and is silently dropped on
/// resume — an attempt that spent four turns would read back as one that
/// spent one, and the record of a run continued after a crash would
/// understate what it cost. Nothing but a round trip catches that.
#[test]
fn a_turn_count_survives_the_round_trip() {
    let dir = tempfile::tempdir().expect("tempdir");
    start(dir.path(), None, None, &[]).expect("start");
    let mut carried_on = attempt(1, 1, AttemptOutcome::Passed);
    carried_on.turns = 4;
    append_attempt(dir.path(), &"design".into(), &carried_on, &spent(2)).expect("append");
    append_attempt(
        dir.path(),
        &"review".into(),
        &attempt(1, 2, AttemptOutcome::Passed),
        &spent(3),
    )
    .expect("append");

    let replay = read(dir.path());
    assert_eq!(
        replay.records[0].attempts[0].turns, 4,
        "the count was dropped on the way back"
    );
    assert_eq!(
        replay.records[1].attempts[0].turns, 1,
        "and a one-turn attempt does not inherit it"
    );
}

/// A journal an older build wrote knows only `corrected: true` — that the
/// attempt spent more than one turn, not how many. It reads back as the
/// floor that claim allows, which is the most an old line can honestly
/// assert. Dropping the key instead would turn every corrected attempt in
/// an existing journal into an ordinary one.
#[test]
fn an_older_journals_corrected_flag_reads_as_two_turns() {
    let dir = tempfile::tempdir().expect("tempdir");
    start(dir.path(), None, None, &[]).expect("start");
    // Hand-written, because the point is a line this build no longer
    // writes: only `corrected`, with no `turns` at all.
    let line = "{\"kind\":\"attempt\",\"v\":1,\"node\":\"design\",\"attempt\":1,\
                    \"evidence_seq\":1,\"outcome\":{\"result\":\"passed\"},\
                    \"corrected\":true,\"spent\":{\"node_runs\":2}}\n";
    let path = dir.path().join(JOURNAL_FILE);
    let mut text = std::fs::read_to_string(&path).expect("started");
    text.push_str(line);
    std::fs::write(&path, text).expect("append by hand");

    let replay = read(dir.path());
    assert_eq!(replay.records[0].attempts[0].turns, 2);
}

/// The key is skipped at one turn, so every line a run that never carried
/// a node on writes is unchanged — and `corrected` is never written at
/// all, because nothing reads it but an older journal's own lines.
#[test]
fn a_one_turn_attempt_writes_no_turn_key() {
    let dir = tempfile::tempdir().expect("tempdir");
    append_attempt(
        dir.path(),
        &"design".into(),
        &attempt(1, 1, AttemptOutcome::Passed),
        &spent(1),
    )
    .expect("append");
    let text = std::fs::read_to_string(dir.path().join(JOURNAL_FILE)).expect("read");
    assert!(!text.contains("corrected"), "{text}");
}

/// A directory with no journal is a run that never got past setup, not
/// an error — `read` answers with an empty replay and `exists` is how a
/// caller tells the two apart.
#[test]
fn a_run_with_no_journal_reads_empty() {
    let dir = tempfile::tempdir().expect("tempdir");
    assert!(!exists(dir.path()));
    let replay = read(dir.path());
    assert!(replay.passed.is_empty() && replay.records.is_empty());
    assert_eq!(replay.next_seq, 0);
}

/// The selection has to survive a crash, or the continuation runs the
/// nodes the first process was told to skip.
#[test]
fn the_selection_reads_back() {
    let dir = tempfile::tempdir().expect("tempdir");
    let target = NodeId::from("design");
    start(dir.path(), None, Some(&target), &[]).expect("start");
    assert_eq!(read(dir.path()).until.as_ref(), Some(&target));
}

#[test]
fn a_run_with_no_selection_reads_back_none() {
    let dir = tempfile::tempdir().expect("tempdir");
    start(dir.path(), None, None, &[]).expect("start");
    assert_eq!(read(dir.path()).until, None);
}
