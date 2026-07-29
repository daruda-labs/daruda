//! Pure menu description. No GPUI context is needed to build or inspect
//! these values — only to *run* an [`Activate`], which the adapter does.

use gpui::{Action, Context, SharedString, Window};

use crate::workspace::Workspace;

pub(super) enum MenuEntry {
    Item(MenuItemSpec),
    Separator,
    Submenu {
        label: SharedString,
        entries: Vec<MenuEntry>,
    },
}

pub(super) type WorkspaceMenuOp = dyn Fn(&mut Workspace, &mut Window, &mut Context<Workspace>);

/// An item either acts or explains why it cannot. Separate variants so a
/// disabled item can never carry an [`Activate`] that will not run — the
/// invalid pairing is unrepresentable rather than merely unused.
pub(super) enum MenuItemSpec {
    Enabled {
        label: SharedString,
        activate: Activate,
    },
    Disabled {
        label: SharedString,
        reason: Option<SharedString>,
    },
}

#[allow(dead_code, reason = "label/is_disabled are the test-side accessors")]
impl MenuItemSpec {
    pub(super) fn label(&self) -> &SharedString {
        match self {
            MenuItemSpec::Enabled { label, .. } | MenuItemSpec::Disabled { label, .. } => label,
        }
    }

    pub(super) fn is_disabled(&self) -> bool {
        matches!(self, MenuItemSpec::Disabled { .. })
    }
}

/// Constructor input for call sites whose `Activate` is the same either way
/// and only the enablement varies (e.g. every Split item shares one op but is
/// gated on lane access).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ItemState {
    Enabled,
    Disabled(Option<SharedString>),
}

pub(super) enum Activate {
    Op(Box<WorkspaceMenuOp>),
    Action(Box<dyn Action>),
    Clipboard(String),
}

pub(super) fn item(
    label: impl Into<SharedString>,
    state: ItemState,
    activate: Activate,
) -> MenuEntry {
    match state {
        ItemState::Enabled => MenuEntry::Item(MenuItemSpec::Enabled {
            label: label.into(),
            activate,
        }),
        ItemState::Disabled(reason) => MenuEntry::Item(MenuItemSpec::Disabled {
            label: label.into(),
            reason,
        }),
    }
}

/// Explicitly-disabled item — for call sites that already branch and would
/// otherwise have to invent a never-run [`Activate`].
pub(super) fn disabled_item(
    label: impl Into<SharedString>,
    reason: Option<SharedString>,
) -> MenuEntry {
    MenuEntry::Item(MenuItemSpec::Disabled {
        label: label.into(),
        reason,
    })
}

pub(super) fn state_if(enabled: bool, reason: Option<SharedString>) -> ItemState {
    if enabled {
        ItemState::Enabled
    } else {
        ItemState::Disabled(reason)
    }
}

/// Collapse separators produced by conditionally-empty sections: no repeats,
/// none leading or trailing, and an empty submenu drops entirely.
pub(super) fn normalize_entries(entries: Vec<MenuEntry>) -> Vec<MenuEntry> {
    let mut normalized = Vec::new();
    for entry in entries {
        match entry {
            MenuEntry::Separator => {
                if !normalized.is_empty()
                    && !matches!(normalized.last(), Some(MenuEntry::Separator))
                {
                    normalized.push(MenuEntry::Separator);
                }
            }
            MenuEntry::Submenu { label, entries } => {
                let entries = normalize_entries(entries);
                if !entries.is_empty() {
                    normalized.push(MenuEntry::Submenu { label, entries });
                }
            }
            MenuEntry::Item(spec) => normalized.push(MenuEntry::Item(spec)),
        }
    }
    while matches!(normalized.last(), Some(MenuEntry::Separator)) {
        normalized.pop();
    }
    normalized
}
