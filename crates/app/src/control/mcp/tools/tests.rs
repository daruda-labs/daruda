use super::*;

/// A catalog shaped like the one that produced the `codex` / `codex-acp`
/// mismatch: an id nobody would guess from the display name.
fn agents() -> Vec<daruda_config::AgentDefinition> {
    vec![
        daruda_config::AgentDefinition::claude_default(),
        daruda_config::AgentDefinition::registry_preset("codex-acp").expect("runnable"),
    ]
}

#[test]
fn twelve_tools_are_exposed() {
    assert_eq!(ToolTable::all(&agents()).describe().len(), 12);
}

#[test]
fn every_tool_has_a_name_description_and_object_schema() {
    for t in ToolTable::all(&agents()).describe() {
        let name = t["name"].as_str().expect("name");
        assert!(name.starts_with("daruda_"), "{name} must be namespaced");
        assert!(
            t["description"].as_str().is_some_and(|d| !d.is_empty()),
            "{name} needs a description"
        );
        assert_eq!(
            t["inputSchema"]["type"], "object",
            "{name} schema must be an object"
        );
    }
}

#[test]
fn names_are_unique() {
    let names: Vec<String> = ToolTable::all(&agents())
        .describe()
        .iter()
        .map(|t| t["name"].as_str().expect("name").to_owned())
        .collect();
    let mut sorted = names.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), names.len(), "duplicate tool name");
}

#[test]
fn lookup_round_trips_every_id() {
    for t in ToolTable::all(&agents()).describe() {
        let name = t["name"].as_str().expect("name");
        assert!(
            ToolTable::all(&agents()).lookup(name).is_some(),
            "{name} not resolvable"
        );
    }
    assert!(ToolTable::all(&agents()).lookup("daruda_nope").is_none());
    assert!(
        ToolTable::empty().lookup("daruda_chat_list").is_none(),
        "an empty table resolves nothing it does not advertise"
    );
}

/// Every declared variant has to be in the table, or `lookup` could never
/// produce it and the gate lookup would fall back to refusing it.
#[test]
fn every_tool_id_has_a_row() {
    for id in [
        ToolId::ChatList,
        ToolId::ChatSend,
        ToolId::ChatStop,
        ToolId::ChatRead,
        ToolId::ChatAsk,
        ToolId::Status,
        ToolId::LaneList,
        ToolId::LaneCreate,
        ToolId::ChatNew,
        ToolId::FlowList,
        ToolId::FlowRun,
        ToolId::FlowStop,
    ] {
        assert!(TABLE.iter().any(|t| t.id == id), "{id:?} has no row");
    }
}

#[test]
fn creation_tools_declare_that_they_need_approval() {
    // The description is what the model reads before calling; a gated tool
    // saying so up front avoids a surprised retry loop on a refusal.
    for name in ["daruda_worktree_create", "daruda_chat_new"] {
        let described = ToolTable::all(&agents()).describe();
        let t = described
            .iter()
            .find(|t| t["name"] == name)
            .expect("present");
        let desc = t["description"]
            .as_str()
            .expect("description")
            .to_lowercase();
        assert!(desc.contains("approval"), "{name} must advertise its gate");
    }
}

/// The sentence and the gate are two statements of one fact, so they have
/// to agree in both directions.
#[test]
fn the_gate_and_the_description_agree() {
    for t in TABLE {
        let advertises = t.description.to_lowercase().contains("approval");
        assert_eq!(
            advertises,
            t.gate == Gate::NeedsApproval,
            "{} says {advertises} but is gated {:?}",
            t.name,
            t.gate
        );
    }
}

/// Only the two creation tools are gated. A read that asked for approval
/// would make the orchestrator useless without a phone in hand.
#[test]
fn exactly_the_creation_tools_are_gated() {
    let gated: Vec<&str> = TABLE
        .iter()
        .filter(|t| t.gate == Gate::NeedsApproval)
        .map(|t| t.name)
        .collect();
    assert_eq!(gated, vec!["daruda_worktree_create", "daruda_chat_new"]);
}

/// A required key the properties do not define cannot be supplied, so the
/// tool would be uncallable.
#[test]
fn every_required_argument_is_a_declared_property() {
    for t in TABLE {
        let properties = (t.properties)();
        for key in t.required {
            assert!(
                properties.get(key).is_some(),
                "{}: required `{key}` is not a property",
                t.name
            );
        }
    }
}

/// An optional argument is fine; an undocumented one is not — the model
/// only knows what the schema says.
#[test]
fn every_property_carries_a_type_and_a_description() {
    for t in TABLE {
        let properties = (t.properties)();
        let Some(map) = properties.as_object() else {
            panic!("{}: properties must be an object", t.name);
        };
        for (key, schema) in map {
            assert!(schema["type"].is_string(), "{}.{key} has no type", t.name);
            assert!(
                schema["description"]
                    .as_str()
                    .is_some_and(|d| !d.is_empty()),
                "{}.{key} has no description",
                t.name
            );
        }
    }
}

/// Every tool that takes an `agent` advertises the ids this daruda will
/// actually accept, each next to the name a person says. Without it the model
/// infers an id from a display name — "Codex" reads as `codex` when the
/// catalog calls it `codex-acp` — and a miss silently opens the default agent.
#[test]
fn an_agent_argument_advertises_the_real_catalog() {
    let described = ToolTable::all(&agents()).describe();
    let taking_agent: Vec<&serde_json::Value> = described
        .iter()
        .filter(|t| t["inputSchema"]["properties"].get("agent").is_some())
        .collect();
    assert_eq!(
        taking_agent.len(),
        2,
        "daruda_chat_new and daruda_worktree_create take one"
    );
    for t in taking_agent {
        let name = t["name"].as_str().expect("name");
        let agent = &t["inputSchema"]["properties"]["agent"];
        assert_eq!(
            agent["enum"],
            serde_json::json!(["claude", "codex-acp"]),
            "{name} must offer the catalog's ids"
        );
        let desc = agent["description"].as_str().expect("description");
        assert!(
            desc.contains("Claude Code (claude)") && desc.contains("Codex (codex-acp)"),
            "{name} must pair each id with the name a person says: {desc}"
        );
    }
}

/// A table with no catalog leaves the static schema alone rather than
/// advertising an empty `enum`, which matches nothing and would refuse every
/// call. Reachable only before the settings store is up.
#[test]
fn an_empty_catalog_advertises_no_choices() {
    let described = ToolTable::all(&[]).describe();
    let t = described
        .iter()
        .find(|t| t["name"] == "daruda_chat_new")
        .expect("present");
    let agent = &t["inputSchema"]["properties"]["agent"];
    assert_eq!(agent["type"], "string");
    assert!(agent.get("enum").is_none(), "{agent}");
}
