//! Host restore policy survives cold restore without persisting navigation state.

use crate::workspace::flow_navigation::RunOrigin;

#[derive(Default)]
pub(in crate::workspace) enum SessionRestore {
    #[default]
    Ordinary,
    Exact {
        flow: Option<RunOrigin>,
    },
}

impl SessionRestore {
    pub fn from_required(required: bool) -> Self {
        if required {
            Self::Exact { flow: None }
        } else {
            Self::Ordinary
        }
    }
    pub fn requires_exact(&self) -> bool {
        matches!(self, Self::Exact { .. })
    }
    pub fn flow_origin(&self) -> Option<&RunOrigin> {
        match self {
            Self::Exact { flow } => flow.as_ref(),
            Self::Ordinary => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cold_restore_keeps_the_policy_without_inventing_a_backlink() {
        let restored = SessionRestore::from_required(true);
        assert!(restored.requires_exact());
        assert!(restored.flow_origin().is_none());
        assert!(!SessionRestore::from_required(false).requires_exact());
    }
}
