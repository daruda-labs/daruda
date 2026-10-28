//! A node's input and baseline move together; widgets never own its lifetime.
use super::{
    baseline::Baseline,
    effective::EffectiveFields,
    fields::{NodeFields, Refusal, fields_of, parse_deps},
    raw::{RawInputs, UiHints},
};
use daruda_flow::NodeId;
use std::{cell::RefCell, sync::Arc};

pub(super) struct NodeRef<'a> {
    file_id: &'a NodeId,
    edited_id: &'a str,
}
impl NodeRef<'_> {
    pub(super) fn matches(&self, id: &NodeId) -> bool {
        id == self.file_id || id.as_str() == self.edited_id
    }
}

#[derive(Clone)]
pub(in crate::workspace) struct NodeDraft {
    node: NodeId,
    base: Baseline,
    initial: EffectiveFields,
    initial_raw: RawInputs,
    effective: EffectiveFields,
    raw: RawInputs,
    ui: UiHints,
    match_memo: RefCell<Option<(Arc<str>, bool)>>,
}

impl NodeDraft {
    pub(super) fn new(base: Baseline, node: NodeId) -> Option<Self> {
        let initial = fields_of(base.node(&node)?);
        let raw = RawInputs::from_fields(&initial);
        let ui = UiHints {
            agent_open: !initial.agent.is_empty(),
            ..UiHints::default()
        };
        Some(Self {
            node,
            base,
            initial_raw: raw.clone(),
            initial: raw.effective(),
            effective: raw.effective(),
            raw,
            ui,
            match_memo: RefCell::default(),
        })
    }
    pub(super) fn node_ref(&self) -> NodeRef<'_> {
        NodeRef {
            file_id: &self.node,
            edited_id: self.raw.id.trim(),
        }
    }
    pub(super) fn baseline(&self) -> &Baseline {
        &self.base
    }
    pub(in crate::workspace) fn initial_fields(&self) -> NodeFields {
        self.initial.clone().into_fields()
    }
    pub(in crate::workspace) fn inherited_agent(&self) -> daruda_flow::parse::AgentOverride {
        self.base
            .file()
            .and_then(|file| file.defaults.agent.clone())
            .unwrap_or_default()
    }
    pub(in crate::workspace) fn node(&self) -> &NodeId {
        &self.node
    }
    #[cfg(test)]
    pub(in crate::workspace) fn source(&self) -> &str {
        self.base.text()
    }
    pub(in crate::workspace) fn raw(&self) -> &RawInputs {
        &self.raw
    }
    pub(in crate::workspace) fn ui(&self) -> &UiHints {
        &self.ui
    }
    pub(super) fn fields(&self) -> NodeFields {
        self.effective.clone().into_fields()
    }
    pub(super) fn is_modified(&self) -> bool {
        self.effective != self.initial
    }
    pub(super) fn has_input(&self) -> bool {
        self.raw != self.initial_raw
    }
    pub(super) fn introduced_refusal(&self) -> Option<Refusal> {
        let refusal = self.effective.refusal()?;
        (self.initial.refusal().as_ref() != Some(&refusal)).then_some(refusal)
    }
    pub(super) fn matches(&self, next: &Baseline) -> bool {
        if let Some((text, answer)) = &*self.match_memo.borrow()
            && Arc::ptr_eq(text, next.text())
        {
            return *answer;
        }
        let answer = self.base.matches(next, &self.node);
        *self.match_memo.borrow_mut() = Some((next.text().clone(), answer));
        answer
    }
    pub(super) fn edit(&mut self, raw: RawInputs, ui: UiHints) {
        self.effective = raw.effective();
        self.raw = raw;
        self.ui = ui;
    }
    pub(super) fn adopt(&mut self, base: Baseline) {
        self.initial_raw = RawInputs::from_fields(&fields_of(
            base.node(&self.node)
                .expect("draft node exists in adopted baseline"),
        ));
        self.initial = self.initial_raw.effective();
        self.match_memo = RefCell::default();
        self.base = base;
    }
    pub(super) fn rebase_onto(&mut self, before: &Baseline, after: &Baseline, deleted: &[NodeId]) {
        let already_moved = self.base.text() == after.text();
        if !already_moved && !self.matches(before) {
            return;
        }
        let removed = |id: &String| deleted.iter().any(|node| node.as_str() == id);
        let deps = |base: &Baseline| {
            base.node(&self.node)
                .map(|node| {
                    node.deps
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        let old = deps(before);
        let new = deps(after);
        if old != new || !deleted.is_empty() {
            let original = parse_deps(&self.raw.deps);
            let mut typed = original.clone();
            typed.retain(|id| (!old.contains(id) || new.contains(id)) && !removed(id));
            for id in new {
                if !old.contains(&id) && !typed.contains(&id) {
                    typed.push(id);
                }
            }
            if original != typed {
                self.raw.deps = typed.join(", ");
            }
            let mut rerun = parse_deps(&self.raw.rerun);
            if rerun.iter().any(removed) {
                rerun.retain(|id| !removed(id));
                self.raw.rerun = rerun.join(", ");
            }
        }
        self.effective = self.raw.effective();
        self.adopt(after.clone());
    }
}

#[cfg(test)]
mod tests {}
