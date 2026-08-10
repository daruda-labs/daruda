//! `model` / `effort` / `mode`: a node runs on what it asked for or it
//! does not run. The fixtures here build an adapter that advertises and
//! journals settings, which no other group does.

use super::*;

/// One advertised select option, in the shape the schema actually parses:
/// `options[].value` (not `id`), and `type` a sibling key rather than a
/// nested object. Get either wrong and the whole set deserializes to an
/// empty list — silently, which reads exactly like "advertises nothing".
fn select(id: &str, category: &str, current: &str, choices: &[&str]) -> String {
    let choices: Vec<String> = choices
        .iter()
        .map(|c| format!(r#"{{"value":"{c}","name":"{c}"}}"#))
        .collect();
    format!(
        r#"{{"id":"{id}","name":"{id}","category":"{category}","type":"select","currentValue":"{current}","options":[{}]}}"#,
        choices.join(",")
    )
}

fn model_select(current: &str) -> String {
    select("model", "model", current, &["sonnet", "opus"])
}

fn effort_select(current: &str) -> String {
    select("effort", "thought_level", current, &["low", "high"])
}

/// What the adapter answers each `set_config_option` with, keyed by the
/// config id it answers. A reply carries the *whole* option set — the
/// protocol replaces it wholesale — so an arm spells out what the adapter
/// looks like afterwards, not just the value that moved.
fn replying(arms: &[(&str, &[String])]) -> String {
    let arms: Vec<String> = arms
        .iter()
        .map(|(cfg, set)| {
            format!(
                r#"{cfg}) printf '{{"jsonrpc":"2.0","id":"%s","result":{{"configOptions":[{}]}}}}\n' "$id" ;;"#,
                set.join(",")
            )
        })
        .collect();
    format!("case \"$cfg\" in\n  {}\n  esac", arms.join("\n  "))
}

/// An adapter that advertises `advertised` at `session/new` and answers a
/// config change out of `replies`. Every method it serves is appended to
/// `journal`, which is the only place the order the agent saw them in is
/// observable — and that order is the whole point of this task.
fn settings_adapter(advertised: &[String], journal: &Path, replies: &str) -> String {
    let journal = journal.display();
    let advertised = advertised.join(",");
    format!(
        r#"while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  case "$line" in
{INITIALIZE}
*'"method":"session/new"'*)
  printf '{{"jsonrpc":"2.0","id":"%s","result":{{"sessionId":"{SESSION}","configOptions":[{advertised}]}}}}\n' "$id" ;;
*'"method":"session/set_config_option"'*)
  cfg=$(printf '%s' "$line" | sed -n 's/.*"configId":"\([^"]*\)".*/\1/p')
  want=$(printf '%s' "$line" | sed -n 's/.*"value":"\([^"]*\)".*/\1/p')
  printf 'set %s=%s\n' "$cfg" "$want" >> "{journal}"
  {replies} ;;
*'"method":"session/prompt"'*)
  printf 'prompt\n' >> "{journal}"
  printf '{{"jsonrpc":"2.0","id":"%s","result":{{"stopReason":"end_turn"}}}}\n' "$id" ;;
  esac
done
"#
    )
}

/// The fixture agent, pinned to a model and/or an effort.
fn pinned(model: Option<&str>, effort: Option<&str>) -> AgentSpec {
    AgentSpec {
        model: model.map(str::to_string),
        effort: effort.map(str::to_string),
        ..spec(AGENT)
    }
}

/// What the adapter was asked to do, in order. Empty when it was asked
/// nothing at all.
fn served(journal: &Path) -> Vec<String> {
    std::fs::read_to_string(journal)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// The defect this task exists to close: without it the record says one
/// model and the session used another, and nothing anywhere disagrees.
///
/// The order is the assertion. A setting that lands after the prompt runs
/// the first turn on the adapter's default, and a test that only checked
/// `set_config_option` was sent would call that a pass.
#[test]
fn a_node_naming_a_model_sets_it_before_prompting() {
    let (_probe, journal) = probe("methods.log");
    let fixture = Fixture::with_script(&settings_adapter(
        &[model_select("sonnet")],
        &journal,
        &replying(&[("model", &[model_select("opus")])]),
    ));

    let result = fixture.run(&pinned(Some("opus"), None));

    assert_eq!(result.outcome, Ok(()), "{:?}", result.outcome);
    assert_eq!(served(&journal), ["set model=opus", "prompt"]);
}

/// Effort travels the same channel under a different category. Setting one
/// and not the other would be invisible — both are optional, so a missing
/// one looks like "not requested".
#[test]
fn a_node_naming_an_effort_sets_the_thought_level() {
    let (_probe, journal) = probe("methods.log");
    let fixture = Fixture::with_script(&settings_adapter(
        &[model_select("sonnet"), effort_select("low")],
        &journal,
        &replying(&[("effort", &[model_select("sonnet"), effort_select("high")])]),
    ));

    let result = fixture.run(&pinned(None, Some("high")));

    assert_eq!(result.outcome, Ok(()), "{:?}", result.outcome);
    assert_eq!(served(&journal), ["set effort=high", "prompt"]);
}

/// Each confirmation replaces the option set wholesale, so the next axis
/// has to be read out of the reply rather than out of what the adapter
/// said at connect. The Claude adapter rebuilds its lists per model, so an
/// effort only exists once the model is settled — here the effort selector
/// appears only in the model's reply.
#[test]
fn each_setting_is_read_against_the_set_the_agent_last_advertised() {
    let (_probe, journal) = probe("methods.log");
    let fixture = Fixture::with_script(&settings_adapter(
        &[model_select("sonnet")],
        &journal,
        &replying(&[
            ("model", &[model_select("opus"), effort_select("low")]),
            ("effort", &[model_select("opus"), effort_select("high")]),
        ]),
    ));

    let result = fixture.run(&pinned(Some("opus"), Some("high")));

    assert_eq!(result.outcome, Ok(()), "{:?}", result.outcome);
    assert_eq!(
        served(&journal),
        ["set model=opus", "set effort=high", "prompt"]
    );
}

/// §8's rule: an axis the adapter never advertised cannot be honoured, and
/// running anyway would silently produce a different run than the record
/// claims.
#[test]
fn a_model_the_adapter_does_not_advertise_fails_the_node() {
    let (_probe, journal) = probe("methods.log");
    let fixture = Fixture::with_script(&settings_adapter(
        &[model_select("sonnet")],
        &journal,
        &replying(&[]),
    ));

    let result = fixture.run(&pinned(Some("haiku"), None));

    assert_eq!(
        result.outcome,
        Err(NodeFailure::UnsupportedSetting {
            field: "model",
            value: "haiku".to_string(),
            available: vec!["sonnet".to_string(), "opus".to_string()],
        })
    );
    assert!(
        served(&journal).is_empty(),
        "an unofferable value was asked for anyway, and the node ran: {:?}",
        served(&journal)
    );
}

/// A whole category the adapter is silent about. Distinct from the case
/// above — there is nothing to list as available, and asking for it would
/// be asking an option that does not exist.
#[test]
fn an_axis_the_adapter_never_offers_at_all_fails_the_node() {
    let (_probe, journal) = probe("methods.log");
    let fixture = Fixture::with_script(&settings_adapter(
        &[model_select("sonnet")],
        &journal,
        &replying(&[]),
    ));

    let result = fixture.run(&pinned(None, Some("high")));

    assert_eq!(
        result.outcome,
        Err(NodeFailure::UnsupportedSetting {
            field: "effort",
            value: "high".to_string(),
            available: Vec::new(),
        })
    );
    assert!(served(&journal).is_empty(), "the node ran unconfigured");
}

/// Asking is not applying. An adapter that accepts the request and then
/// reports the old value has not honoured it, and the node must not
/// proceed as though it had.
#[test]
fn a_setting_the_adapter_accepts_but_does_not_apply_fails_the_node() {
    let (_probe, journal) = probe("methods.log");
    let fixture = Fixture::with_script(&settings_adapter(
        &[model_select("sonnet")],
        &journal,
        // Accepted, acknowledged with a full option set — and still on the
        // value it started with.
        &replying(&[("model", &[model_select("sonnet")])]),
    ));

    let result = fixture.run(&pinned(Some("opus"), None));

    assert_eq!(
        result.outcome,
        Err(NodeFailure::UnsupportedSetting {
            field: "model",
            value: "opus".to_string(),
            available: vec!["sonnet".to_string(), "opus".to_string()],
        })
    );
    assert_eq!(
        served(&journal),
        ["set model=opus"],
        "the turn ran on a model the node did not ask for"
    );
}

/// A confirmation that never comes is "could not apply", not a turn that
/// hangs — so the wait has a budget of its own, well inside the node's.
#[test]
fn a_setting_the_adapter_never_confirms_fails_within_its_own_budget() {
    let (_probe, journal) = probe("methods.log");
    let mut fixture = Fixture::with_script(&settings_adapter(
        &[model_select("sonnet")],
        &journal,
        // Recorded, never answered.
        "",
    ));
    fixture.settings_budget = Duration::from_millis(200);
    fixture.timeout = Duration::from_secs(5);

    let started = Instant::now();
    let result = fixture.run(&pinned(Some("opus"), None));
    let elapsed = started.elapsed();

    assert!(
        matches!(
            result.outcome,
            Err(NodeFailure::UnsupportedSetting { field: "model", .. })
        ),
        "{:?}",
        result.outcome
    );
    assert!(
        elapsed < fixture.timeout,
        "a silent adapter read as the node hanging, not as an unapplied setting: {elapsed:?}"
    );
    assert_eq!(served(&journal), ["set model=opus"]);
}

/// A node naming neither must not wait for a confirmation that will never
/// come — the common case is no override at all.
#[test]
fn a_node_naming_no_model_prompts_immediately() {
    let (_probe, journal) = probe("methods.log");
    let fixture = Fixture::with_script(&settings_adapter(
        &[model_select("sonnet"), effort_select("low")],
        &journal,
        &replying(&[]),
    ));

    let started = Instant::now();
    let result = fixture.run(&pinned(None, None));

    assert_eq!(result.outcome, Ok(()), "{:?}", result.outcome);
    assert_eq!(served(&journal), ["prompt"]);
    assert!(
        started.elapsed() < HARNESS_GUARD,
        "the node waited on a setting it never asked for"
    );
}

/// The mirror of `ProcessRunner::run_agent`: each runner refuses the other
/// kind of node by name rather than pretending to have run it.
#[test]
fn a_command_node_is_not_this_runners() {
    let fixture = Fixture::with_script(&adapter_script("", &stops_with("end_turn")));
    let runner = fixture.runner();

    let result = smol::block_on(runner.run_command(&fixture.context(), "true"));
    assert!(
        matches!(result.outcome, Err(NodeFailure::SessionError(_))),
        "{:?}",
        result.outcome
    );
}

/// `session/new` that advertises modes and reports which one it settled in.
fn adapter_in_mode(current: &str, available: &[&str]) -> String {
    let modes: Vec<String> = available
        .iter()
        .map(|id| format!(r#"{{"id":"{id}","name":"{id}"}}"#))
        .collect();
    format!(
        r#"while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
  case "$line" in
{INITIALIZE}
    *'"method":"session/new"'*)
      printf '{{"jsonrpc":"2.0","id":"%s","result":{{"sessionId":"{SESSION}","modes":{{"currentModeId":"{current}","availableModes":[{modes}]}}}}}}\n' "$id" ;;
    *'"method":"session/set_mode"'*)
      printf '{{"jsonrpc":"2.0","id":"%s","result":{{}}}}\n' "$id" ;;
    *'"method":"session/prompt"'*)
      printf '{{"jsonrpc":"2.0","id":"%s","result":{{"stopReason":"end_turn"}}}}\n' "$id" ;;
  esac
done"#,
        modes = modes.join(",")
    )
}

fn ran_in_mode(want: &str, current: &str, available: &[&str]) -> Result<(), NodeFailure> {
    let fixture = Fixture::with_script(&adapter_in_mode(current, available));
    let mut agent = spec(AGENT);
    agent.mode = Some(want.to_string());
    fixture.run(&agent).outcome
}

/// `daruda_acp` degrades an unavailable or rejected mode to a fallback and
/// only emits a `Notice`. Left unchecked, a flow claiming
/// `bypassPermissions` runs in `auto` — and that axis decides what the agent
/// is *allowed* to do, so it is the one that must not drift silently.
#[test]
fn a_node_that_did_not_get_the_mode_it_asked_for_fails() {
    assert_eq!(
        ran_in_mode("bypassPermissions", "auto", &["auto", "plan"]),
        Err(NodeFailure::UnsupportedSetting {
            field: "mode",
            value: "bypassPermissions".to_string(),
            available: vec!["auto".to_string(), "plan".to_string()],
        })
    );
}

#[test]
fn a_node_that_got_its_mode_runs() {
    assert_eq!(
        ran_in_mode("bypassPermissions", "bypassPermissions", &["auto"]),
        Ok(())
    );
}

/// An agent that advertises no modes at all cannot be in the one a node
/// pinned, so the node must not run as though it were.
#[test]
fn a_node_pinning_a_mode_against_an_agent_with_none_fails() {
    let fixture = Fixture::with_script(&adapter_script("", &stops_with("end_turn")));
    let mut agent = spec(AGENT);
    agent.mode = Some("plan".to_string());
    assert!(matches!(
        fixture.run(&agent).outcome,
        Err(NodeFailure::UnsupportedSetting { field: "mode", .. })
    ));
}

/// An adapter that *advertises* a value and then refuses to set it is the
/// case the settings budget was covering by accident: the runner had
/// nothing to observe but the clock, so it waited out the whole budget to
/// conclude what the refusal already said.
///
/// The outcome was always right — only the wait was wrong, and on a real
/// adapter that wait is 30 seconds of a node's time per rejected axis.
#[test]
fn a_setting_the_adapter_refuses_fails_at_once_rather_than_on_the_clock() {
    let (_probe, journal) = probe("methods.log");
    let mut fixture = Fixture::with_script(&settings_adapter(
        &[model_select("sonnet")],
        &journal,
        // A JSON-RPC error, which is what a refusal is on the wire.
        r#"printf '{"jsonrpc":"2.0","id":"%s","error":{"code":-32602,"message":"no such model"}}\n' "$id""#,
    ));
    fixture.settings_budget = Duration::from_secs(4);
    fixture.timeout = Duration::from_secs(8);

    let started = Instant::now();
    let result = fixture.run(&pinned(Some("opus"), None));
    let elapsed = started.elapsed();

    assert!(
        matches!(result.outcome, Err(NodeFailure::SettingRejected { .. })),
        "{:?}",
        result.outcome
    );
    assert!(
        elapsed < fixture.settings_budget,
        "waited out the budget to learn what the adapter had already said: {elapsed:?}"
    );
    // The node never ran: a turn on a model the record claims but the
    // session does not have is the thing this refuses to produce.
    assert_eq!(served(&journal), ["set model=opus"]);
}
