//! String in, string out. The assertions that matter are what *stayed*: the
//! comments, the key order, the block style, and every line nobody asked to
//! change being byte-identical.

use super::*;
use daruda_flow::parse::{NodeKindFile, PromptSource};

const FLOW: &str = "\
version: 1
# how every node runs unless it says otherwise
defaults:
  agent:
    id: claude
    mode: bypassPermissions
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: |
      write the design
      in two lines
  - id: build
    kind: agent
    deps: [design]
    output: build.md
    prompt: build it
";

fn edited(text: &str, update: impl FnOnce(&mut FlowFile)) -> String {
    let edits = edits_for_update(text, update).expect("the edit is representable");
    apply(text, &edits)
}

/// Every line the edit did not name is byte-identical, and the lines it did are
/// the ones expected.
fn assert_only_lines_changed(before: &str, after: &str, expected: &[(usize, &str)]) {
    let old: Vec<&str> = before.lines().collect();
    let new: Vec<&str> = after.lines().collect();
    assert_eq!(old.len(), new.len(), "line count changed:\n{after}");
    for (ix, (o, n)) in old.iter().zip(new.iter()).enumerate() {
        match expected.iter().find(|(want_ix, _)| *want_ix == ix) {
            Some((_, want)) => assert_eq!(n, want, "line {ix}"),
            None => assert_eq!(o, n, "line {ix} was not supposed to change"),
        }
    }
}

#[test]
fn an_update_that_changes_nothing_is_no_edit_at_all() {
    let edits = edits_for_update(FLOW, |_| {}).expect("no-op");
    assert!(edits.is_empty());
}

#[test]
fn one_changed_scalar_changes_one_line_and_keeps_the_comment() {
    let after = edited(FLOW, |file| {
        file.nodes[1].kind = NodeKindFile::Agent {
            agent: None,
            prompt: PromptSource::Prompt("build it".into()),
            output: "artifact.md".into(),
            on_fail: Default::default(),
        };
    });
    assert_only_lines_changed(FLOW, &after, &[(16, "    output: artifact.md")]);
    assert!(
        after.contains("# how every node runs unless it says otherwise"),
        "the comment survived"
    );
}

/// A key the file does not have is written where its siblings are, not at the
/// top of the mapping.
#[test]
fn an_absent_key_is_inserted_at_the_end_of_its_mapping() {
    let after = edited(FLOW, |file| {
        file.nodes[0].timeout = Some(std::time::Duration::from_secs(90));
    });
    assert!(
        after.contains("      in two lines\n    timeout: 1m 30s\n  - id: build"),
        "inserted after the node's last field:\n{after}"
    );
    assert_eq!(
        after.replace("    timeout: 1m 30s\n", ""),
        FLOW,
        "and nothing else moved"
    );
}

#[test]
fn a_key_that_went_away_takes_its_whole_line_with_it() {
    let after = edited(FLOW, |file| {
        file.defaults.agent.as_mut().unwrap().mode = None;
    });
    assert!(!after.contains("mode:"), "{after}");
    assert_eq!(
        after,
        FLOW.replace("    mode: bypassPermissions\n", ""),
        "one line, exactly"
    );
}

/// The headline of the whole design: adding a node is not a feature of its own,
/// it falls out of comparing the two trees.
#[test]
fn a_new_node_is_appended_under_the_dash_its_siblings_use() {
    let after = edited(FLOW, |file| {
        let mut ship = file.nodes[1].clone();
        ship.id = "ship".into();
        ship.deps = vec!["build".into()];
        ship.kind = NodeKindFile::Agent {
            agent: None,
            prompt: PromptSource::Prompt("ship it".into()),
            output: "ship.md".into(),
            on_fail: Default::default(),
        };
        file.nodes.push(ship);
    });
    assert!(after.starts_with(FLOW), "the file was only added to");
    // `on_fail: halt` is written out because the wire type spells its default —
    // `run.yaml` is produced by that same `Serialize` and records the resolved
    // spec in full. An added node is therefore explicit about what it does on
    // failure, which is noisier than the file was and not wrong.
    assert_eq!(
        &after[FLOW.len()..],
        "  - id: ship\n    deps:\n      - build\n    kind: agent\n    prompt: ship it\n    output: ship.md\n    on_fail: halt\n",
        "the new node reads like the ones above it"
    );
    daruda_flow::load(&after, None).expect("and it still loads");
}

#[test]
fn a_node_that_went_away_takes_its_lines_and_nothing_else() {
    let after = edited(FLOW, |file| {
        file.nodes.remove(0);
    });
    assert_eq!(
        after,
        "\
version: 1
# how every node runs unless it says otherwise
defaults:
  agent:
    id: claude
    mode: bypassPermissions
nodes:
  - id: build
    kind: agent
    deps: [design]
    output: build.md
    prompt: build it
"
    );
}

/// D3: a value written as `|` is rewritten as `|`, at the column the file chose.
#[test]
fn a_block_scalar_stays_a_block_scalar() {
    let after = edited(FLOW, |file| {
        file.nodes[0].kind = NodeKindFile::Agent {
            agent: None,
            prompt: PromptSource::Prompt("write the design\nin three lines\nnow\n".into()),
            output: "design.md".into(),
            on_fail: Default::default(),
        };
    });
    assert!(
        after.contains("    prompt: |\n      write the design\n      in three lines\n      now\n"),
        "{after}"
    );
    assert!(after.contains("# how every node"), "the comment survived");
}

/// A single-line value that grows a newline cannot stay plain.
#[test]
fn a_plain_scalar_that_grew_a_newline_is_promoted_to_a_block_scalar() {
    let after = edited(FLOW, |file| {
        file.nodes[1].kind = NodeKindFile::Agent {
            agent: None,
            prompt: PromptSource::Prompt("build it\nthen check it\n".into()),
            output: "build.md".into(),
            on_fail: Default::default(),
        };
    });
    assert!(
        after.contains("    prompt: |\n      build it\n      then check it\n"),
        "{after}"
    );
    daruda_flow::load(&after, None).expect("still loads");
}

/// A prompt whose first line is itself indented — pasted code, a nested list.
/// Without an indentation indicator the written file does not parse at all
/// (measured), so the second gate would refuse a change that is perfectly
/// reasonable, and blame the flow for it.
#[test]
fn a_prompt_whose_first_line_is_indented_still_loads_and_says_the_same_thing() {
    let prompt = "  fn main() {}\nexplain the above\n";
    let after = edited(FLOW, |file| {
        file.nodes[1].kind = NodeKindFile::Agent {
            agent: None,
            prompt: PromptSource::Prompt(prompt.into()),
            output: "build.md".into(),
            on_fail: Default::default(),
        };
    });
    let reparsed = daruda_flow::parse::parse_flow_file(&after).expect("what we wrote parses");
    let NodeKindFile::Agent {
        prompt: PromptSource::Prompt(back),
        ..
    } = &reparsed.nodes[1].kind
    else {
        panic!("the node is still an agent with an inline prompt");
    };
    assert_eq!(back, prompt, "byte for byte");
    daruda_flow::load(&after, None).expect("and it loads");
}

/// A list the file wrote on one line stays on one line.
///
/// Splicing *inside* `[design]` is still refused — what happens instead is that
/// the whole value is replaced, in the style it was written in. Reformatting it
/// into a block list would be a change to a line nobody asked about.
#[test]
fn a_flow_style_list_is_replaced_in_flow_style() {
    let after = edited(FLOW, |file| {
        file.nodes[1].deps = vec!["design".into(), "review".into()];
    });
    assert_only_lines_changed(FLOW, &after, &[(15, "    deps: [design, review]")]);

    // Replacing an element, not just adding one — the case that walks *into* the
    // list and has to come back out to the value above it.
    let after = edited(FLOW, |file| {
        file.nodes[1].deps = vec!["review".into()];
    });
    assert_only_lines_changed(FLOW, &after, &[(15, "    deps: [review]")]);
}

/// Two elements of one flow list changing is **two** changes from the differ —
/// same length, so it compares element by element — and both resolve to
/// replacing the same whole value. That has to come out as one edit; two
/// identical ranges would read as an overlap and be refused.
#[test]
fn two_elements_of_one_flow_list_still_make_one_edit() {
    let two_deps = FLOW.replace("deps: [design]", "deps: [design, review]");
    let edits = edits_for_update(&two_deps, |file| {
        file.nodes[1].deps = vec!["audit".into(), "sign".into()];
    })
    .expect("representable");
    assert_eq!(edits.len(), 1, "{edits:?}");
    assert_eq!(edits[0].1, "[audit, sign]");
    assert!(
        apply(&two_deps, &edits).contains("    deps: [audit, sign]\n"),
        "and it lands as one line"
    );
}

/// The refusal that stays: a flow *mapping* is not a list, and there is no
/// equivalent one-line answer for it.
#[test]
fn an_edit_touching_a_flow_mapping_is_refused() {
    let flow_mapping = "\
version: 1
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write it
    agent: {id: claude, mode: bypassPermissions}
";
    let err = edits_for_update(flow_mapping, |file| {
        let NodeKindFile::Agent { agent, .. } = &mut file.nodes[0].kind else {
            panic!("agent node");
        };
        agent.as_mut().expect("has an override").model = Some("opus".into());
    })
    .expect_err("refused");
    assert!(matches!(err, FlowEditError::FlowStyle(_)), "{err:?}");
}

/// And a `>` folded scalar, for the same reason: rewriting one moves the line
/// breaks, which is a change nobody asked for.
#[test]
fn an_edit_to_a_folded_scalar_is_refused() {
    let folded = "\
version: 1
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: >
      write the design
";
    let err = edits_for_update(folded, |file| {
        file.nodes[0].kind = NodeKindFile::Agent {
            agent: None,
            prompt: PromptSource::Prompt("something else".into()),
            output: "design.md".into(),
            on_fail: Default::default(),
        };
    })
    .expect_err("refused");
    assert!(matches!(err, FlowEditError::FoldedScalar(_)), "{err:?}");
}

#[test]
fn a_file_that_does_not_parse_is_refused_before_anything_is_touched() {
    let err = edits_for_update("nodes: [\n", |_| {}).expect_err("refused");
    assert!(matches!(err, FlowEditError::Unparsable(_)), "{err:?}");
}

/// D2: the indentation of an inserted line is the file's, not ours.
#[test]
fn an_insertion_follows_the_indentation_the_file_uses() {
    let four = "\
version: 1
defaults:
    agent:
        id: claude
nodes:
    -   id: design
        kind: agent
        output: design.md
        prompt: write it
";
    let after = edited(four, |file| {
        file.defaults.agent.as_mut().unwrap().model = Some("opus".into());
    });
    assert!(
        after.contains("        id: claude\n        model: opus\n"),
        "{after}"
    );
}

/// Two new fields in one mapping land on the same byte. One edit has to carry
/// both, or the two ranges would overlap and the second would be dropped.
#[test]
fn two_new_fields_in_one_mapping_are_one_edit() {
    let edits = edits_for_update(FLOW, |file| {
        file.nodes[0].timeout = Some(std::time::Duration::from_secs(30));
        file.nodes[0].cwd = Some("sub".into());
    })
    .expect("representable");
    assert_eq!(edits.len(), 1, "{edits:?}");
    let after = apply(FLOW, &edits);
    assert!(after.contains("    cwd: sub\n"), "{after}");
    assert!(after.contains("    timeout: 30s\n"), "{after}");
    daruda_flow::load(&after, None).expect("still loads");
}

/// A nested block the file has never held is written whole, indented in.
#[test]
fn a_new_nested_mapping_is_written_as_a_block() {
    let after = edited(FLOW, |file| {
        file.nodes[0].kind = NodeKindFile::Agent {
            agent: Some(daruda_flow::parse::AgentOverride {
                id: Some("codex".into()),
                model: Some("gpt".into()),
                mode: Some("bypassPermissions".into()),
                ..Default::default()
            }),
            prompt: PromptSource::Prompt("write the design\nin two lines\n".into()),
            output: "design.md".into(),
            on_fail: Default::default(),
        };
    });
    assert!(
        after.contains(
            "    agent:\n      id: codex\n      model: gpt\n      mode: bypassPermissions\n"
        ),
        "{after}"
    );
    daruda_flow::load(&after, None).expect("still loads");
}

/// Every edit is a range into the *original* text, so applying them back to
/// front is the caller's whole job — and that is what `apply` documents.
#[test]
fn edits_come_back_in_reading_order() {
    let edits = edits_for_update(FLOW, |file| {
        file.version = 2;
        file.nodes[1].cwd = Some("sub".into());
    })
    .expect("representable");
    assert!(
        edits.windows(2).all(|w| w[0].0.start <= w[1].0.start),
        "{edits:?}"
    );
}

/// Deleting one node of several takes out its lines and the mentions of it, and
/// **nothing else** — the comment and the one-line list of the node that stays
/// are exactly as they were.
///
/// This is what pairing sequence elements by `id` buys: a deletion also edits a
/// surviving element (its `deps` loses a name), so matching by equality found
/// neither a common prefix nor a common suffix and rewrote the whole block.
#[test]
fn deleting_one_node_leaves_the_others_byte_identical() {
    let after = edited(FLOW, |file| {
        file.nodes.retain(|n| n.id != "design");
        for node in file.nodes.iter_mut() {
            node.deps.retain(|dep| dep != "design");
        }
    });
    assert_eq!(
        after,
        "\
version: 1
# how every node runs unless it says otherwise
defaults:
  agent:
    id: claude
    mode: bypassPermissions
nodes:
  - id: build
    kind: agent
    output: build.md
    prompt: build it
"
    );
}

/// And a rename is not a deletion plus an addition: the node keeps its place in
/// the file. Pairing by id would have moved it to the end.
#[test]
fn a_renamed_node_keeps_its_place() {
    let after = edited(FLOW, |file| {
        file.nodes[0].id = "spec".into();
        for node in file.nodes.iter_mut() {
            for dep in node.deps.iter_mut() {
                if dep == "design" {
                    *dep = "spec".into();
                }
            }
        }
    });
    assert_eq!(
        after,
        FLOW.replace("- id: design", "- id: spec")
            .replace("deps: [design]", "deps: [spec]"),
        "one name in two places, and no lines moved"
    );
}

/// The whole-list rewrite: no common prefix, no common suffix of equal length,
/// and no `id` to pair by, so the value is replaced entire. Its first line's
/// indentation belongs to the value, and a naive replacement writes it twice —
/// the list's own two spaces plus the rendered ones.
#[test]
fn a_rewritten_block_list_is_not_indented_twice() {
    let block_deps = "\
version: 1
nodes:
  - id: build
    kind: agent
    deps:
      - a
      - b
      - keep
    output: build.md
    prompt: build it
";
    let after = edited(block_deps, |file| {
        file.nodes[0].deps = vec!["p".into(), "q".into(), "r".into(), "keep".into()];
    });
    assert_eq!(
        after,
        block_deps.replace(
            "      - a\n      - b\n      - keep\n",
            "      - p\n      - q\n      - r\n      - keep\n"
        ),
        "rewritten at the indentation the list already had"
    );
}

/// A key that goes away while another arrives in the same mapping: the new one
/// takes the old one's line.
///
/// The insertion anchor is the end of the mapping's last entry, so when that
/// entry is the one being removed the two edits overlap — and an overlap is
/// refused, which is how switching a prompt to a `prompt_file` came to fail
/// outright.
#[test]
fn a_new_key_takes_the_line_of_the_one_it_replaces() {
    let after = edited(FLOW, |file| {
        file.nodes[1].kind = NodeKindFile::Agent {
            agent: None,
            prompt: PromptSource::PromptFile("brief.md".into()),
            output: "build.md".into(),
            on_fail: Default::default(),
        };
    });
    assert_eq!(
        after,
        FLOW.replace("    prompt: build it\n", "    prompt_file: brief.md\n"),
        "one line becomes the other, and nothing else moves"
    );
}

/// A block scalar this module wrote has to be readable by it again. An indented
/// first line makes the writer state the indentation (`|2`), and a reader that
/// only knew the bare headers called its own output a folded scalar — so the
/// second edit of such a node was refused, for ever.
#[test]
fn a_block_scalar_with_an_indentation_indicator_is_still_editable() {
    let text = "\
version: 1
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: |2
      hello
";
    let after = edited(text, |file| {
        if let NodeKindFile::Agent { prompt, .. } = &mut file.nodes[0].kind {
            *prompt = PromptSource::Prompt("new text\n".into());
        }
    });
    assert!(after.contains("new text"), "{after}");
    // And again on the result: reading back what it just wrote is the whole
    // property, so once is not enough to prove it.
    assert!(
        edits_for_update(&after, |file| {
            if let NodeKindFile::Agent { prompt, .. } = &mut file.nodes[0].kind {
                *prompt = PromptSource::Prompt("third\n".into());
            }
        })
        .is_ok(),
        "the result is editable too:\n{after}"
    );
}

/// `|+` keeps every trailing newline and this module cannot say that, so it is
/// refused rather than quietly rewritten as `|` — which would keep one.
#[test]
fn a_block_scalar_that_keeps_its_newlines_is_refused_not_collapsed() {
    let text = "\
version: 1
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: |+
      hello

";
    let err = edits_for_update(text, |file| {
        if let NodeKindFile::Agent { prompt, .. } = &mut file.nodes[0].kind {
            *prompt = PromptSource::Prompt("new text\n".into());
        }
    })
    .expect_err("refused");
    assert!(
        matches!(err, FlowEditError::Unrepresentable(_)),
        "and not called folded: {err:?}"
    );
}

/// A comment beside a key whose block value collapses to one line. The edit
/// wants the whole gap between the two, and the comment is in it.
#[test]
fn a_comment_beside_a_collapsing_block_is_refused_not_eaten() {
    let text = "\
version: 1
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: hi
    on_fail: # retry twice before giving up
      retry:
        hint: try again
        max_attempts: 2
";
    let err = edits_for_update(text, |file| {
        if let NodeKindFile::Agent { on_fail, .. } = &mut file.nodes[0].kind {
            *on_fail = Default::default();
        }
    })
    .expect_err("refused rather than written");
    assert!(
        format!("{err}").contains("retry twice before giving up"),
        "and it says which comment: {err}"
    );
}

/// A comment trailing a block's last entry belongs to the block, so an edit
/// that replaces the block would take it. Refused instead — and it says which
/// comment, because the person has to go move it themselves.
#[test]
fn a_comment_trailing_a_rewritten_list_is_refused_not_eaten() {
    let text = "\
version: 1
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: hi
  - id: gate
    kind: agent
    output: gate.md
    prompt: check
    deps:
      - design
      - other
      # keep both for now
";
    let err = edits_for_update(text, |file| {
        file.nodes[1].deps = vec!["a".into(), "b".into(), "c".into()];
    })
    .expect_err("refused rather than written");
    assert!(format!("{err}").contains("keep both for now"), "{err}");
}

/// The same, one level down: the comment sits at the outer block's indentation
/// but inside the inner one, so the entry's own range reaches past it.
#[test]
fn a_comment_trailing_a_nested_block_is_refused_too() {
    let text = "\
version: 1
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: hi
    on_fail:
      retry:
        hint: try again
        max_attempts: 2
      # keep retrying
";
    let err = edits_for_update(text, |file| {
        if let NodeKindFile::Agent { on_fail, .. } = &mut file.nodes[0].kind {
            *on_fail = Default::default();
        }
    })
    .expect_err("refused rather than written");
    assert!(format!("{err}").contains("keep retrying"), "{err}");
}

/// A header may carry a comment, and it is still a literal scalar.
#[test]
fn a_block_scalar_header_may_carry_a_comment() {
    let text = "\
version: 1
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: | # note
      hello
";
    let out = edits_for_update(text, |file| {
        file.nodes[0].cwd = Some("sub".into());
    });
    assert!(out.is_ok(), "{:?}", out.err());
}

/// Two removals at once, and the node that survives keeps *its own* values.
///
/// The trap: an edit's path indexes the **text**, and the value it writes came
/// from the **new** tree read at that same index — which is a different element
/// once anything before it is gone. Here it gave `n4` the deps of `n6`, and the
/// flow only failed to load because that happened to make a cycle. A pair that
/// did not would have been written silently.
#[test]
fn removing_two_nodes_leaves_the_survivors_own_values() {
    let text = "\
version: 1
defaults:
  agent:
    id: claude
    mode: bypassPermissions
nodes:
  - id: n1
    kind: agent
    output: n1.md
    prompt: one
  - id: n2
    kind: agent
    deps: [n1]
    output: n2.md
    prompt: two
  - id: n3
    kind: agent
    deps: [n2]
    output: n3.md
    prompt: three
  - id: n4
    kind: agent
    deps: [n3]
    output: n4.md
    prompt: four
  - id: n5
    kind: agent
    deps: [n4]
    output: n5.md
    prompt: five
  - id: n6
    kind: agent
    deps: [n5]
    output: n6.md
    prompt: six
";
    let after = edited(text, |file| {
        for target in ["n2", "n3"] {
            crate::workspace::main_area::flow_graph_pane::form::apply::remove_node(
                file,
                &target.into(),
            );
        }
    });
    let file = daruda_flow::parse::parse_flow_file(&after).expect("still a flow");
    let deps: Vec<(&str, Vec<&str>)> = file
        .nodes
        .iter()
        .map(|n| {
            (
                n.id.as_str(),
                n.deps.iter().map(daruda_flow::NodeId::as_str).collect(),
            )
        })
        .collect();
    assert_eq!(
        deps,
        vec![
            ("n1", vec![]),
            ("n4", vec![]),
            ("n5", vec!["n4"]),
            ("n6", vec!["n5"]),
        ],
        "n4 lost the dep on a node that is gone, and nobody else moved:\n{after}"
    );
    assert!(
        daruda_flow::load(&after, None).is_ok(),
        "and it loads:\n{after}"
    );
}

/// The coordinate pair, asserted where it is read.
///
/// `value_at_path` is the only reader of a step's `value`, and the whole point
/// of the pair is that a removed element has no place in the new tree. Both
/// sequence differs build removals that way now; the keyed one was fixed after
/// it wrote a survivor's `deps` with another node's value, and the positional
/// one was still claiming a place it did not have.
#[test]
fn a_removed_elements_step_reads_nothing_from_the_new_tree() {
    let tree: Value = yaml_serde::from_str("nodes: [a, b]").expect("fixture");
    let kept = vec![
        Step::Key("nodes".to_string()),
        Step::Index {
            text: 1,
            value: Some(1),
        },
    ];
    assert_eq!(
        value_at_path(&tree, &kept).and_then(Value::as_str),
        Some("b"),
        "an element that is still there reads as itself"
    );

    let going = vec![
        Step::Key("nodes".to_string()),
        Step::Index {
            text: 1,
            value: None,
        },
    ];
    assert!(
        value_at_path(&tree, &going).is_none(),
        "and one that is going reads as nothing — not as whatever sits at its old index"
    );
}
