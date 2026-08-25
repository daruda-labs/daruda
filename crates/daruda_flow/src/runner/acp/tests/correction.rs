//! The one correction turn, and the three gates that refuse it.
//!
//! Driven through the real adapter, never through a fake `NodeRunner`: a
//! scripted "misses once, then writes" step would be this module's logic
//! restated inside the fake, and would keep passing with `settle` sending
//! nothing at all. What every test here asserts is how many prompts reached
//! the wire.

use super::*;

/// Shell that writes the fixture's output — what an agent doing as it was
/// asked the second time looks like from outside.
fn writes(output: &Path) -> String {
    format!(r#"printf 'the design\n' > "{}""#, output.display())
}

/// Shell that writes JSON meeting [`VERDICT`] — an agent that read the schema
/// the second time.
fn writes_json(output: &Path) -> String {
    format!(
        r#"printf '{{"verdict":"pass"}}
' > "{}""#,
        output.display()
    )
}

const VERDICT: &str = "\
type: object
required: [verdict]
properties:
  verdict: { type: string, enum: [pass, fail] }
";

/// **The shape is correctable, and this is why.** The seeded output is prose:
/// the file is there and non-empty, so nothing but the declared shape refuses
/// it — and the session being asked again is the one the schema was stated in.
#[test]
fn a_turn_that_wrote_the_wrong_shape_is_asked_again_and_then_writes_json() {
    let (_probe, counter) = probe("prompts");
    let mut fixture =
        Fixture::with_script_for_output(|output| counting_adapter(&counter, &writes_json(output)));
    fixture.wants_json(VERDICT);
    assert!(
        matches!(fixture.judged(), Err(NodeFailure::OutputSchema { .. })),
        "the seeded prose has to be what the shape refuses"
    );

    let result = fixture.run(&spec(AGENT));

    assert_eq!(result.outcome, Ok(()), "{:?}", result.outcome);
    assert_eq!(prompts_sent(&counter), 2, "the correction never went out");
    assert_eq!(result.turns, 2);
    assert_eq!(fixture.judged(), Ok(()), "the correction did not land");
}

/// And the correction says why the shape was refused in the check's own
/// words — the one thing the agent cannot derive from being told it failed.
#[test]
fn a_shape_correction_carries_the_checks_own_reason() {
    let (_probe, counter) = probe("prompts");
    let mut fixture = Fixture::with_script(&counting_adapter(&counter, ""));
    fixture.wants_json(VERDICT);

    let result = fixture.run(&spec(AGENT));

    let transcript = result
        .artifacts
        .first()
        .unwrap_or_else(|| panic!("no artifact: {:?}", result.artifacts));
    let text = std::fs::read_to_string(transcript).expect("the transcript is on disk");
    assert!(
        text.contains("not a single JSON value"),
        "the reason is the check's own: {text}"
    );
}

/// The correction is a second prompt on the *same* session: the contract is
/// already in that session's context, which a fresh session next attempt
/// would not have.
#[test]
fn a_turn_that_wrote_nothing_is_asked_again_and_then_writes_it() {
    let (_probe, counter) = probe("prompts");
    let fixture =
        Fixture::with_script_for_output(|output| counting_adapter(&counter, &writes(output)));
    fixture.owes_its_output();

    let result = fixture.run(&spec(AGENT));

    assert_eq!(result.outcome, Ok(()), "{:?}", result.outcome);
    assert_eq!(prompts_sent(&counter), 2, "the correction never went out");
    assert_eq!(result.turns, 2, "the second turn went unrecorded");
    assert_eq!(fixture.judged(), Ok(()), "the correction did not land");
}

/// The correction is told what the check will say, in the check's own
/// words — and the transcript has to show it, or a reader cannot tell a
/// corrected attempt from one that simply ran twice.
#[test]
fn the_correction_is_in_the_transcript_with_the_reason_it_was_sent() {
    let (_probe, counter) = probe("prompts");
    let fixture = Fixture::with_script(&counting_adapter(&counter, ""));
    fixture.owes_its_output();

    let result = fixture.run(&spec(AGENT));

    let transcript = result
        .artifacts
        .first()
        .unwrap_or_else(|| panic!("no artifact: {:?}", result.artifacts));
    let text = std::fs::read_to_string(transcript).expect("the transcript is on disk");
    assert!(
        text.contains("Your previous turn ended without satisfying the OUTPUT CONTRACT"),
        "{text}"
    );
    assert!(
        text.contains("the file is absent or empty"),
        "the reason is the check's own: {text}"
    );
}

/// A second turn that also writes nothing changes only the cost. What the
/// attempt is judged on is still what is on disk.
#[test]
fn a_correction_that_changes_nothing_still_reports_what_was_owed() {
    let (_probe, counter) = probe("prompts");
    let fixture = Fixture::with_script(&counting_adapter(&counter, ""));
    fixture.owes_its_output();

    let result = fixture.run(&spec(AGENT));

    assert_eq!(prompts_sent(&counter), 2);
    assert_eq!(result.turns, 2);
    assert_eq!(result.outcome, Ok(()), "the turn itself ended cleanly");
    assert_eq!(
        fixture.judged(),
        Err(NodeFailure::NoOutput {
            expected: fixture.output.clone()
        }),
        "the node still owes its file"
    );
}

/// **Gate A.** A stop does not go missing — `interrupted` races the whole
/// turn — but this is what keeps an about-to-be-dropped future from paying
/// for one more prompt on its way out.
#[test]
fn a_stopped_run_pays_for_no_correction() {
    let (_probe, counter) = probe("prompts");
    let (_answers, answered) = probe("answer.json");
    let mut fixture = Fixture::with_script(&asking_counting_adapter(
        &counter,
        ALLOW_ONCE_OPTION,
        &answered,
        "",
    ));
    fixture.owes_its_output();
    let cancel = fixture.cancel.clone();

    let (_result, _asked) = fixture.run_answered(&spec(AGENT), Duration::ZERO, move |_| {
        cancel.cancel();
        Person::Answers(PermissionDecision::Allow {
            option_id: "once".to_string(),
        })
    });

    assert_eq!(
        prompts_sent(&counter),
        1,
        "a stopped run paid for another turn"
    );
}

/// **Gate B is wired in.** What a stubbed `reserve` can show is that it is
/// consulted before anything reaches the wire, and that a `false` stops the
/// send and leaves the attempt unmarked. When a real budget answers `false`
/// is `schedule::budget`'s, and `schedule::tests::budgets` asks it.
#[test]
fn a_refused_reservation_stops_the_correction_before_it_is_sent() {
    let (_probe, counter) = probe("prompts");
    let mut fixture = Fixture::with_script(&counting_adapter(&counter, ""));
    fixture.owes_its_output();
    fixture.reserve = Box::new(|| false);

    let result = fixture.run(&spec(AGENT));

    assert_eq!(prompts_sent(&counter), 1, "the run paid past its ceiling");
    assert!(
        result.turns == 1,
        "a refused correction must not be recorded as one"
    );
}

/// **Gate C is wired in, against the node's own timeout.** A budget under
/// the floor has no room for a correction whatever the clock says, so this
/// does not depend on elapsed time and does not claim to: that the gate
/// weighs elapsed time against the floor at all is
/// `correction::tests::a_turn_that_has_nearly_spent_its_budget_does_not_start_one`'s,
/// and the paused clock it subtracts from that is
/// `a_long_wait_for_a_person_still_gets_its_correction`'s below.
#[test]
fn a_budget_smaller_than_the_correction_floor_sends_none() {
    let (_probe, counter) = probe("prompts");
    let mut fixture = Fixture::with_script(&counting_adapter(&counter, ""));
    fixture.owes_its_output();
    // Under the floor a correction needs, so there is no room for one from
    // the moment the turn starts.
    fixture.timeout = Duration::from_secs(5);

    let result = fixture.run(&spec(AGENT));

    assert_eq!(result.outcome, Ok(()), "{:?}", result.outcome);
    assert_eq!(prompts_sent(&counter), 1, "a doomed correction was sent");
    assert_eq!(result.turns, 1);
}

/// An adapter that counts prompts *and* asks for permission on the first
/// one, so a correction can be posed after a long wait for a person.
fn asking_counting_adapter(
    prompts: &Path,
    options: &str,
    answer_file: &Path,
    on_second: &str,
) -> String {
    let counter = prompts.display();
    let answer = answer_file.display();
    format!(
        r#"count=0
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  case "$line" in
{INITIALIZE}
{NEW_SESSION}
*'"method":"session/prompt"'*)
  prompt_id="$id"
  count=$((count+1))
  printf '%s' "$count" > "{counter}"
  if [ "$count" -ge 2 ]; then
    {on_second}
    printf '{{"jsonrpc":"2.0","id":"%s","result":{{"stopReason":"end_turn"}}}}\n' "$id"
  else
    printf '{{"jsonrpc":"2.0","id":"perm-1","method":"session/request_permission","params":{{"sessionId":"{SESSION}","toolCall":{{"toolCallId":"t1","title":"write a file"}},"options":[{options}]}}}}\n'
  fi ;;
*'"id":"perm-1"'*)
  printf '%s\n' "$line" > "{answer}"
  printf '{{"jsonrpc":"2.0","id":"%s","result":{{"stopReason":"end_turn"}}}}\n' "$prompt_id" ;;
  esac
done
"#
    )
}

/// **The sign inside gate C.** The node's clock stops while a person is
/// waited on, so what the gate compares is work time. On `elapsed + paused`
/// this turn — a moment of work and two seconds of somebody thinking — is
/// refused a correction for the time that person took.
#[test]
fn a_long_wait_for_a_person_still_gets_its_correction() {
    let (_probe, counter) = probe("prompts");
    let (_answers, answered) = probe("answer.json");
    let mut fixture = Fixture::with_script(&asking_counting_adapter(
        &counter,
        ALLOW_ONCE_OPTION,
        &answered,
        "",
    ));
    fixture.owes_its_output();
    // Just over the correction floor, so a second of the person's time
    // charged to the node is the difference between sending and not.
    fixture.timeout = Duration::from_secs(31);

    let (result, asked) = fixture.run_answered(&spec(AGENT), Duration::from_secs(2), |_| {
        Person::Answers(PermissionDecision::Allow {
            option_id: "once".to_string(),
        })
    });

    assert_eq!(asked.len(), 1, "the person was asked once");
    assert!(
        result.waiting.total >= Duration::from_secs(2),
        "the wait was not accounted for: {:?}",
        result.waiting.total
    );
    assert_eq!(
        prompts_sent(&counter),
        2,
        "the person's time was charged to the node"
    );
    assert_eq!(result.turns, 2);
}

/// A link where the output belongs is a refusal, not a mistake to point
/// out: the bytes it names are not this node's work, and asking the agent
/// that pointed there to try again checks nothing.
#[test]
fn a_link_where_the_output_belongs_is_not_asked_again() {
    let (_probe, counter) = probe("prompts");
    let fixture = Fixture::with_script(&counting_adapter(&counter, ""));
    fixture.owes_its_output();
    // Somebody else's work, which a size check would have accepted.
    let elsewhere = fixture.cwd.join("elsewhere.md");
    std::fs::write(&elsewhere, "work this node did not do\n").expect("write");
    std::os::unix::fs::symlink(&elsewhere, &fixture.output).expect("symlink");

    let result = fixture.run(&spec(AGENT));

    assert_eq!(prompts_sent(&counter), 1, "a refusal was argued with");
    assert_eq!(result.turns, 1);
    assert!(matches!(
        fixture.judged(),
        Err(NodeFailure::OutputNotAFile { .. })
    ));
}

/// **The hole the correction opened.** The second turn writes and then runs
/// out of context, so the file on disk is non-empty and the contract is met —
/// exactly the half-written output `failure_for` refuses a first turn for.
/// Discarding the correction's own verdict would pass the node.
#[test]
fn a_correction_that_ran_out_of_context_fails_the_node() {
    let (_probe, counter) = probe("prompts");
    let fixture = Fixture::with_script_for_output(|output| {
        counting_adapter_stopping(&counter, &writes(output), "max_tokens")
    });
    fixture.owes_its_output();

    let result = fixture.run(&spec(AGENT));

    assert_eq!(prompts_sent(&counter), 2, "the correction never went out");
    assert_eq!(
        fixture.judged(),
        Ok(()),
        "the file has to be there, or the node would fail on the contract instead"
    );
    assert_eq!(
        result.outcome,
        Err(NodeFailure::ContextExhausted),
        "a truncated correction passed the node"
    );
}

/// And the complement, which is why the filter is a filter and not a `?`:
/// `forbids_retry` counts `Refused`, so a *correction's* refusal reaching
/// the node would cap the attempts of a node whose own failure retries.
#[test]
fn a_refused_correction_leaves_the_nodes_own_retryable_failure() {
    let (_probe, counter) = probe("prompts");
    let fixture = Fixture::with_script(&counting_adapter_stopping(&counter, "", "refusal"));
    fixture.owes_its_output();

    let result = fixture.run(&spec(AGENT));

    assert_eq!(prompts_sent(&counter), 2, "the correction never went out");
    assert_eq!(result.outcome, Ok(()), "the node's own turn ended cleanly");
    let judged = fixture.judged();
    assert_eq!(
        judged,
        Err(NodeFailure::NoOutput {
            expected: fixture.output.clone()
        }),
        "the node still owes its file"
    );
    assert!(
        !judged.expect_err("owed").forbids_retry(),
        "a correction's refusal must not cap the node's attempts"
    );
}

/// The regression the correction must not cost: a turn that met its
/// contract is one turn, and pays for nothing else.
#[test]
fn a_turn_that_wrote_its_output_is_never_asked_twice() {
    let (_probe, counter) = probe("prompts");
    let fixture = Fixture::with_script(&counting_adapter(&counter, ""));

    let result = fixture.run(&spec(AGENT));

    assert_eq!(result.outcome, Ok(()), "{:?}", result.outcome);
    assert_eq!(prompts_sent(&counter), 1, "a met contract bought a turn");
    assert_eq!(result.turns, 1);
}

const STATED: &str = "\
type: object
required: [state]
properties:
  state: { type: string, enum: [in_progress, done] }
";

/// Shell that writes `in_progress` the first time it runs and `done` the
/// second — an agent that reports partial work and carries on, which is the
/// behaviour "continue" was being clicked for.
///
/// `counting_adapter` only runs a body from the *second* prompt, so the first
/// turn writes nothing at all. That makes this fixture pass through both
/// reasons to go round: a missing output, then an unfinished one.
fn finishes_on_its_second_run(output: &Path, counter: &Path) -> String {
    format!(
        r#"n=$(cat "{c}" 2>/dev/null || echo 0)
n=$((n+1)); printf '%s' "$n" > "{c}"
if [ "$n" -ge 2 ]; then s=done; else s=in_progress; fi
printf '{{"state":"%s"}}\n' "$s" > "{o}""#,
        c = counter.display(),
        o = output.display()
    )
}

/// The click this whole feature removes. Three turns, two reasons to go round
/// — nothing written, then written but unfinished — and nobody asked whether
/// to continue.
#[test]
fn a_node_that_reports_partial_work_is_carried_on_until_it_says_done() {
    let (_probe, counter) = probe("prompts");
    let (_turns, turn_file) = probe("turns");
    let mut fixture = Fixture::with_script_for_output(|output| {
        counting_adapter(&counter, &finishes_on_its_second_run(output, &turn_file))
    });
    fixture.wants_done_when(STATED, "state", "done");
    fixture.wants_turns(4);

    let result = fixture.run(&spec(AGENT));

    assert_eq!(result.outcome, Ok(()), "{:?}", result.outcome);
    assert_eq!(
        prompts_sent(&counter),
        3,
        "one prompt per turn, and no fourth once it said done"
    );
    assert_eq!(fixture.judged(), Ok(()));
}

/// `max_turns: 1` is a node that gets no second chance — not even the
/// correction turn that existed before this loop did.
#[test]
fn a_node_allowed_one_turn_is_never_asked_again() {
    let (_probe, counter) = probe("prompts");
    let mut fixture =
        Fixture::with_script_for_output(|output| counting_adapter(&counter, &writes(output)));
    fixture.wants_json(VERDICT);
    fixture.wants_turns(1);

    let result = fixture.run(&spec(AGENT));

    assert_eq!(prompts_sent(&counter), 1, "a second prompt went out");
    assert_eq!(result.turns, 1);
    assert!(
        matches!(fixture.judged(), Err(NodeFailure::OutputSchema { .. })),
        "and the node is judged on what one turn left behind"
    );
}

/// Running out of turns with the work unfinished fails the node. Passing it
/// would hand a half-written output to whatever reads it next, and the run
/// would look like it worked.
#[test]
fn running_out_of_turns_still_unfinished_fails_the_node() {
    let (_probe, counter) = probe("prompts");
    let (_turns, turn_file) = probe("turns");
    let mut fixture = Fixture::with_script_for_output(|output| {
        // Needs three prompts to reach `done` and is allowed two.
        counting_adapter(&counter, &finishes_on_its_second_run(output, &turn_file))
    });
    fixture.wants_done_when(STATED, "state", "done");
    fixture.wants_turns(2);

    let result = fixture.run(&spec(AGENT));

    assert_eq!(prompts_sent(&counter), 2, "it spent exactly what it had");
    assert_eq!(result.outcome, Ok(()), "the turns themselves ended cleanly");
    assert!(
        matches!(fixture.judged(), Err(NodeFailure::OutputSchema { .. })),
        "and the node fails on what is on disk: {:?}",
        fixture.judged()
    );
}

/// **A refusal ends the loop.** The design said the opposite — that a refused
/// turn should not come out of the cap, because a person saying no is not the
/// agent failing to progress — and implemented that way it does not
/// terminate: a policy refusing every turn decrements nothing, and the loop
/// runs until the clock or the budget stops it, having spent both on turns
/// that were always going to be refused.
///
/// Ending is also what the rest of the engine already does with a refusal —
/// `NodeFailure::forbids_retry` counts it for the same reason.
#[test]
fn a_refused_turn_ends_the_loop_rather_than_being_exempt_from_the_cap() {
    let (_probe, counter) = probe("prompts");
    let (_answers, answered) = probe("answer.json");
    let mut fixture = Fixture::with_script(&asking_counting_adapter(
        &counter,
        REJECT_ONCE_OPTION,
        &answered,
        "",
    ));
    fixture.owes_its_output();
    // Room for many turns, so what stops the loop is the refusal and not the
    // cap or the clock.
    fixture.wants_turns(9);
    fixture.timeout = Duration::from_secs(120);

    let (result, asked) = fixture.run_answered(&spec(AGENT), Duration::from_millis(1), |_| {
        Person::Answers(PermissionDecision::Reject {
            option_id: "no".to_string(),
        })
    });

    assert_eq!(asked.len(), 1, "asked once, refused once");
    assert_eq!(
        prompts_sent(&counter),
        1,
        "the refusal stopped it before a second prompt went out"
    );
    assert_eq!(result.turns, 1);
    assert!(
        result.waiting.any_refused(),
        "and the record says a person refused, so the failure below is not \
         read as the agent simply not writing"
    );
    assert!(
        matches!(fixture.judged(), Err(NodeFailure::NoOutput { .. })),
        "{:?}",
        fixture.judged()
    );
}
