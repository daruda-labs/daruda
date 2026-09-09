//! The tool table daruda advertises. GPUI-free.
//!
//! Ten tools, all for the orchestrator. A lane's own agent gets none — it
//! reads untrusted text and could be steered into calling them — so there is
//! no per-caller filtering here.
//! Descriptions and schemas are English and not localized: a tool definition
//! is machine data an LLM reads, and a stable wording is worth more than a
//! translated one.
//!
//! The words here are the user's, not the code's. A `Lane` is a **worktree**
//! to anyone outside the app (the same rule the UI follows), and the tab a
//! chat opens in is a **tab** — so a person can say "add a tab to daruda main"
//! and the model has the vocabulary to resolve it. `ToolId`'s variants keep
//! the internal names because that is what the types are called.
//!
//! Every identifier a tool accepts is one daruda handed out earlier in the
//! same session (`daruda_chat_list`, `daruda_worktree_list`) — runtime ids, not
//! persisted uuids. One kind of handle, so a value read from one tool can be
//! passed to another without conversion.

/// Which tool a call named. The table is static, so this is the whole
/// vocabulary — a name outside it is a tool error, not a protocol one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToolId {
    ChatList,
    ChatSend,
    ChatStop,
    ChatRead,
    Status,
    LaneList,
    LaneCreate,
    ChatNew,
    FlowList,
    FlowRun,
}

/// Whether a tool's effect needs the user's say-so before it runs.
///
/// Carried beside the description rather than decided at the call site, so
/// the sentence the model reads and the gate the call meets cannot disagree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Gate {
    /// Reads, or acts on something that already exists.
    Open,
    /// Brings something new into being. Asks the phone first.
    NeedsApproval,
}

pub(crate) struct Tool {
    pub id: ToolId,
    pub name: &'static str,
    pub description: &'static str,
    pub gate: Gate,
    /// JSON Schema `properties` for the call arguments.
    pub properties: fn() -> serde_json::Value,
    pub required: &'static [&'static str],
}

pub(crate) struct ToolTable(&'static [Tool]);

impl ToolTable {
    pub(crate) fn all() -> Self {
        Self(TABLE)
    }

    #[cfg(test)]
    pub(crate) fn empty() -> Self {
        Self(&[])
    }

    pub(crate) fn describe(&self) -> Vec<serde_json::Value> {
        self.0
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "inputSchema": {
                        "type": "object",
                        "properties": (t.properties)(),
                        "required": t.required,
                    },
                })
            })
            .collect()
    }

    /// Resolve a called name. `None` is answered as a tool error so the model
    /// can recover by listing again.
    ///
    /// Reads `self`, like `describe`: a table that advertised one set of tools
    /// and resolved another would be two vocabularies wearing one type.
    pub(crate) fn lookup(&self, name: &str) -> Option<ToolId> {
        self.0.iter().find(|t| t.name == name).map(|t| t.id)
    }

    /// Whether `id` has to clear the approval gate. Unknown ids fail closed —
    /// unreachable through [`Self::lookup`], but a gate that defaulted open
    /// would be the wrong way to be wrong.
    pub(crate) fn gate(&self, id: ToolId) -> Gate {
        self.0
            .iter()
            .find(|t| t.id == id)
            .map_or(Gate::NeedsApproval, |t| t.gate)
    }
}

/// The `target` argument every pane-addressed tool takes: a `PaneRef` exactly
/// as `daruda_chat_list` reported it.
fn pane_ref_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "description": "A chat pane, copied verbatim from daruda_chat_list's `target`.",
        "properties": {
            "workspace": { "type": "string", "description": "Window uuid." },
            "pane": { "type": "integer", "description": "Pane id within that window." },
        },
        "required": ["workspace", "pane"],
    })
}

/// The `worktree` argument, as `daruda_worktree_list` reported it.
///
/// "Worktree" is the word for this everywhere a user can see it; `Lane` is the
/// internal type's name and stays inside the app.
fn lane_ref_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "description": "A worktree, copied verbatim from daruda_worktree_list's `target`.",
        "properties": {
            "workspace": { "type": "string", "description": "Window uuid." },
            "project": { "type": "integer", "description": "Project id, scoped to that window." },
            "worktree": { "type": "integer", "description": "Worktree id, scoped to that window." },
        },
        "required": ["workspace", "project", "worktree"],
    })
}

fn no_properties() -> serde_json::Value {
    serde_json::json!({})
}

fn chat_send_properties() -> serde_json::Value {
    serde_json::json!({
        "target": pane_ref_schema(),
        "text": { "type": "string", "description": "The prompt to send." },
    })
}

fn chat_stop_properties() -> serde_json::Value {
    serde_json::json!({ "target": pane_ref_schema() })
}

fn chat_read_properties() -> serde_json::Value {
    serde_json::json!({ "target": pane_ref_schema() })
}

fn lane_create_properties() -> serde_json::Value {
    serde_json::json!({
        "workspace": {
            "type": "string",
            "description": "Window uuid, from daruda_worktree_list's `target.workspace`.",
        },
        "project": {
            "type": "integer",
            "description": "Project id, from the same row's `target.project`.",
        },
        "name": {
            "type": "string",
            "description": "Name for a branch to create. It must not exist yet — a worktree \
                            owns its branch, so an existing one cannot be checked out twice. \
                            Git's rules apply: no spaces, no `..`, none of \
                            `: ~ ^ ? * [ \\`.",
        },
        "base_ref": {
            "type": "string",
            "description": "Ref to branch from. Omit for the project's default.",
        },
        "agent": {
            "type": "string",
            "description": "Agent id for the worktree's first chat pane. Omit for the default.",
        },
        "prompt": {
            "type": "string",
            "description": "First prompt to send in the new pane, if any.",
        },
    })
}

fn chat_new_properties() -> serde_json::Value {
    serde_json::json!({
        "worktree": lane_ref_schema(),
        "agent": {
            "type": "string",
            "description": "Agent id to open under. Omit for the default.",
        },
    })
}

fn flow_run_properties() -> serde_json::Value {
    serde_json::json!({
        "name": {
            "type": "string",
            "description": "Flow file name, from daruda_flow_list. The extension is optional.",
        },
    })
}

/// The table, for a test that has to walk every row.
#[cfg(test)]
pub(crate) fn table_for_test() -> &'static [Tool] {
    TABLE
}

/// The advertised vocabulary. Order is the order `tools/list` reports.
static TABLE: &[Tool] = &[
    Tool {
        id: ToolId::ChatList,
        name: "daruda_chat_list",
        description: "List every open agent chat — one per tab: where it lives, what it is \
                      doing, and the `target` handle the other chat tools take.",
        gate: Gate::Open,
        properties: no_properties,
        required: &[],
    },
    Tool {
        id: ToolId::ChatSend,
        name: "daruda_chat_send",
        description: "Send a prompt to one agent chat. Returns whether it went out now or \
                      queued behind the turn in flight — not the agent's answer, which \
                      arrives in that chat.",
        gate: Gate::Open,
        properties: chat_send_properties,
        required: &["target", "text"],
    },
    Tool {
        id: ToolId::ChatStop,
        name: "daruda_chat_stop",
        description: "Stop whatever one agent chat has in flight. Reports if it was \
                      already idle.",
        gate: Gate::Open,
        properties: chat_stop_properties,
        required: &["target"],
    },
    Tool {
        id: ToolId::ChatRead,
        name: "daruda_chat_read",
        description: "Read what one agent chat last said — the most recent message it \
                      finished writing, long ones cut in the middle. A message still being \
                      written does not count, so a chat that is working reports what it said \
                      before — check `activity` from daruda_chat_list to tell the two apart. \
                      Nothing is returned when it has not spoken yet.",
        gate: Gate::Open,
        properties: chat_read_properties,
        required: &["target"],
    },
    Tool {
        id: ToolId::Status,
        name: "daruda_status",
        description: "One-line count across every open chat: working, waiting on a \
                      permission, failed, total.",
        gate: Gate::Open,
        properties: no_properties,
        required: &[],
    },
    Tool {
        id: ToolId::LaneList,
        name: "daruda_worktree_list",
        description: "List every worktree, including ones with no agent chat in them, and \
                      the `target` handle daruda_chat_new takes. `name` is the branch it is \
                      on, which is how a person refers to it — resolve \"main\" or \
                      \"feat/x\" against this list rather than guessing a handle.",
        gate: Gate::Open,
        properties: no_properties,
        required: &[],
    },
    Tool {
        id: ToolId::LaneCreate,
        name: "daruda_worktree_create",
        description: "Create a worktree on a new branch and open its first chat. To add a \
                      tab to a worktree that already exists — including the one a project \
                      is currently on — use daruda_chat_new instead. Needs the user's \
                      approval: the call waits for them to tap, and fails if they refuse or \
                      do not answer.",
        gate: Gate::NeedsApproval,
        properties: lane_create_properties,
        required: &["workspace", "project", "name"],
    },
    Tool {
        id: ToolId::ChatNew,
        name: "daruda_chat_new",
        description: "Open one more agent chat — a new tab — in a worktree that already \
                      exists, from daruda_worktree_list. Any number can share one \
                      worktree. Needs the user's approval: the call waits for them to tap, \
                      and fails if they refuse or do not answer.",
        gate: Gate::NeedsApproval,
        properties: chat_new_properties,
        required: &["worktree"],
    },
    Tool {
        id: ToolId::FlowList,
        name: "daruda_flow_list",
        description: "List the flows that can be run right now: every open window's \
                      active worktree contributes its own, so the same name can appear \
                      twice with a different `lane`.",
        gate: Gate::Open,
        properties: no_properties,
        required: &[],
    },
    Tool {
        id: ToolId::FlowRun,
        name: "daruda_flow_run",
        description: "Start a flow by name, in whichever open window's active worktree \
                      has it — the answer says which. Reports that it started, not that \
                      it finished; the outcome arrives when the run ends.",
        gate: Gate::Open,
        properties: flow_run_properties,
        required: &["name"],
    },
];

#[cfg(test)]
mod tests;
