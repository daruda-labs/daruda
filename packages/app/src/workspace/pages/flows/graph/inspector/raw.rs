//! Lossless user input, independent of widgets and inactive-field visibility.
use super::{effective::EffectiveFields, fields::*};
use daruda_flow::parse::PermissionPolicyFile;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::workspace) struct RawInputs {
    pub id: String,
    pub deps: String,
    pub timeout: String,
    pub max_turns: String,
    pub cwd: String,
    pub agent_id: String,
    pub agent_model: String,
    pub agent_mode: String,
    pub agent_effort: String,
    pub prompt_inline: String,
    pub prompt_file: String,
    pub output: String,
    pub run: String,
    pub hint_inline: String,
    pub hint_file: String,
    pub fix: String,
    pub rerun: String,
    pub max_attempts: String,
    pub wait: String,
    pub kind: KindChoice,
    pub prompt_is_file: bool,
    pub hint_is_file: bool,
    pub acting: bool,
    pub permission: Option<PermissionPolicyFile>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::workspace) struct UiHints {
    pub agent_open: bool,
    pub settings_open: bool,
    pub agent_id_custom: bool,
    pub agent_model_custom: bool,
    pub agent_mode_custom: bool,
    pub timeout_custom: bool,
    pub turns_custom: bool,
}

impl RawInputs {
    pub(super) fn from_fields(fields: &NodeFields) -> Self {
        let source = |s: &SourceField| match s {
            SourceField::Inline(text) => (text.clone(), String::new(), false),
            SourceField::File(path) => (String::new(), path.clone(), true),
        };
        let (prompt_inline, prompt_file, prompt_is_file) = source(&fields.body.prompt);
        let mut raw = Self {
            id: fields.id.clone(),
            deps: fields.deps.join(", "),
            timeout: timeout_text(&fields.timeout),
            max_turns: match &fields.max_turns {
                TurnsField::Absent => String::new(),
                TurnsField::Set(n) => n.to_string(),
                TurnsField::Unreadable(s) => s.clone(),
            },
            cwd: fields.cwd.clone().unwrap_or_default(),
            agent_id: fields.agent.id.clone().unwrap_or_default(),
            agent_model: fields.agent.model.clone().unwrap_or_default(),
            agent_mode: fields.agent.mode.clone().unwrap_or_default(),
            agent_effort: fields.agent.effort.clone().unwrap_or_default(),
            permission: fields.agent.permission,
            kind: fields.body.kind,
            prompt_inline,
            prompt_file,
            prompt_is_file,
            output: fields.body.output.clone(),
            run: fields.body.run.clone(),
            hint_inline: String::new(),
            hint_file: String::new(),
            hint_is_file: false,
            fix: String::new(),
            rerun: String::new(),
            max_attempts: String::new(),
            wait: String::new(),
            acting: !matches!(fields.body.on_fail, FailFields::Halt),
        };
        match &fields.body.on_fail {
            FailFields::Halt => {}
            FailFields::Retry {
                hint,
                max_attempts,
                wait,
            } => {
                (raw.hint_inline, raw.hint_file, raw.hint_is_file) = source(hint);
                raw.max_attempts = attempts_text(max_attempts);
                raw.wait = timeout_text(wait);
            }
            FailFields::Repair {
                fix,
                rerun,
                max_attempts,
                wait,
            } => {
                raw.fix = fix.clone();
                raw.rerun = rerun.join(", ");
                raw.max_attempts = attempts_text(max_attempts);
                raw.wait = timeout_text(wait);
            }
        }
        raw
    }

    /// Only fields written for the active kind participate in modification and validation.
    pub(in crate::workspace::pages::flows::graph) fn effective(&self) -> EffectiveFields {
        let agent = self.kind == KindChoice::Agent;
        let source = |file: bool, inline: &String, path: &String| {
            if file {
                SourceField::File(path.trim().to_owned())
            } else {
                SourceField::Inline(inline.clone())
            }
        };
        let max_turns = if !agent || self.max_turns.trim().is_empty() {
            TurnsField::Absent
        } else {
            self.max_turns.trim().parse().map_or_else(
                |_| TurnsField::Unreadable(self.max_turns.trim().into()),
                TurnsField::Set,
            )
        };
        EffectiveFields::from_fields(NodeFields {
            id: self.id.trim().into(),
            deps: parse_deps(&self.deps),
            timeout: parse_timeout(self.timeout.trim()),
            max_turns,
            cwd: some_if_filled(self.cwd.trim()),
            agent: if agent {
                AgentFields {
                    id: some_if_filled(self.agent_id.trim()),
                    model: some_if_filled(self.agent_model.trim()),
                    effort: some_if_filled(self.agent_effort.trim()),
                    mode: some_if_filled(self.agent_mode.trim()),
                    permission: self.permission,
                }
            } else {
                AgentFields::default()
            },
            body: BodyFields {
                kind: self.kind,
                prompt: if agent {
                    source(self.prompt_is_file, &self.prompt_inline, &self.prompt_file)
                } else {
                    SourceField::Inline(String::new())
                },
                output: if agent {
                    self.output.trim().into()
                } else {
                    String::new()
                },
                run: if agent {
                    String::new()
                } else {
                    self.run.trim().into()
                },
                on_fail: if !self.acting {
                    FailFields::Halt
                } else if agent {
                    FailFields::Retry {
                        hint: source(self.hint_is_file, &self.hint_inline, &self.hint_file),
                        max_attempts: parse_attempts(self.max_attempts.trim()),
                        wait: parse_timeout(self.wait.trim()),
                    }
                } else {
                    FailFields::Repair {
                        fix: self.fix.clone(),
                        rerun: parse_deps(&self.rerun),
                        max_attempts: parse_attempts(self.max_attempts.trim()),
                        wait: parse_timeout(self.wait.trim()),
                    }
                },
            },
        })
    }
}

#[cfg(test)]
mod tests;
