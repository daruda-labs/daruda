//! Kind-specific values used for modification checks and the save boundary.
use super::fields::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace::pages::flows::graph) struct EffectiveFields {
    id: String,
    deps: Vec<String>,
    timeout: TimeoutField,
    cwd: Option<String>,
    body: EffectiveBody,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum EffectiveBody {
    Agent {
        agent: AgentFields,
        max_turns: TurnsField,
        prompt: SourceField,
        output: String,
        on_fail: FailFields,
    },
    Command {
        run: String,
        on_fail: FailFields,
    },
}

impl EffectiveFields {
    pub(super) fn from_fields(fields: NodeFields) -> Self {
        let body = match fields.body.kind {
            KindChoice::Agent => EffectiveBody::Agent {
                agent: fields.agent,
                max_turns: fields.max_turns,
                prompt: fields.body.prompt,
                output: fields.body.output,
                on_fail: fields.body.on_fail,
            },
            KindChoice::Command => EffectiveBody::Command {
                run: fields.body.run,
                on_fail: fields.body.on_fail,
            },
        };
        Self {
            id: fields.id,
            deps: fields.deps,
            timeout: fields.timeout,
            cwd: fields.cwd,
            body,
        }
    }

    pub(in crate::workspace::pages::flows::graph) fn into_fields(self) -> NodeFields {
        let (agent, max_turns, body) = match self.body {
            EffectiveBody::Agent {
                agent,
                max_turns,
                prompt,
                output,
                on_fail,
            } => (
                agent,
                max_turns,
                BodyFields {
                    kind: KindChoice::Agent,
                    prompt,
                    output,
                    run: String::new(),
                    on_fail,
                },
            ),
            EffectiveBody::Command { run, on_fail } => (
                AgentFields::default(),
                TurnsField::Absent,
                BodyFields {
                    kind: KindChoice::Command,
                    prompt: SourceField::Inline(String::new()),
                    output: String::new(),
                    run,
                    on_fail,
                },
            ),
        };
        NodeFields {
            id: self.id,
            deps: self.deps,
            timeout: self.timeout,
            cwd: self.cwd,
            agent,
            max_turns,
            body,
        }
    }

    pub(super) fn refusal(&self) -> Option<Refusal> {
        refusal_of(&self.clone().into_fields())
    }
}

#[cfg(test)]
mod tests {}
