//! Where each row sits: every page's cards and their rows, in page order.
//!
//! The pages render from this table and the search index walks it, so a
//! result lists in the order the page shows and a card title is written once.
//! Pages drawn wholly by hand (Keymap, Session Hosts, Accounts, Plugin) have
//! no entry here.

use daruda_config::{BuiltinSection as Section, StatusBarItem as Item};

use super::{BoolSetting as B, SelectSetting as S, TextSetting as T};
use crate::surface::strings as s;

/// The setting one row controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Target {
    Text(T),
    Select(S),
    Bool(B),
    StatusBarItem(Item),
}

/// One row of a card.
pub(super) enum Row {
    Setting(Target),
    /// Rows that only apply while the switch is on, indented under it. The
    /// switch itself is listed just before as a [`Row::Setting`].
    Under(B, &'static [Target]),
    Custom(CustomRow),
}

/// Rows a page draws by hand, named so the table can place them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CustomRow {
    CustomColors,
    ProjectShell,
    DarudaLink,
    RemoteLink,
    TelegramBody,
}

pub(super) enum Card {
    Rows {
        title: fn() -> String,
        rows: &'static [Row],
    },
    /// The page's folded low-traffic rows.
    Advanced(&'static [Row]),
    Custom(CustomCard),
}

/// Whole cards a page draws by hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CustomCard {
    AgentCatalog,
    RemoteIntegrations,
    AboutVersion,
}

const fn t(v: T) -> Row {
    Row::Setting(Target::Text(v))
}
const fn sel(v: S) -> Row {
    Row::Setting(Target::Select(v))
}
const fn b(v: B) -> Row {
    Row::Setting(Target::Bool(v))
}
const fn item(v: Item) -> Row {
    Row::Setting(Target::StatusBarItem(v))
}

const GENERAL: &[Card] = &[Card::Rows {
    title: s::settings_group_language,
    rows: &[sel(S::Language)],
}];

const APPEARANCE: &[Card] = &[
    Card::Rows {
        title: s::settings_group_themes,
        rows: &[
            sel(S::UiPreset),
            sel(S::TerminalPreset),
            sel(S::SyntaxTheme),
            Row::Custom(CustomRow::CustomColors),
        ],
    },
    Card::Rows {
        title: s::settings_group_window,
        rows: &[t(T::WindowOpacity), b(B::WindowBlur)],
    },
];

const FONT: &[Card] = &[
    Card::Rows {
        title: s::settings_font_domain_terminal,
        rows: &[
            sel(S::TerminalFontFamily),
            t(T::TerminalFontSize),
            t(T::TerminalLineHeight),
            t(T::TerminalCellWidth),
        ],
    },
    Card::Rows {
        title: s::settings_font_domain_editor,
        rows: &[
            sel(S::EditorFontFamily),
            t(T::EditorFontSize),
            t(T::EditorLineHeight),
        ],
    },
    Card::Rows {
        title: s::settings_font_domain_agent_chat,
        rows: &[
            sel(S::AgentChatFontFamily),
            t(T::AgentChatFontSize),
            t(T::AgentChatLineHeight),
        ],
    },
];

const TERMINAL: &[Card] = &[
    Card::Rows {
        title: s::settings_card_shell,
        rows: &[
            t(T::ShellProgram),
            b(B::ShellNaturalTextEditing),
            b(B::ShellClosePaneOnExit),
            Row::Custom(CustomRow::ProjectShell),
        ],
    },
    Card::Rows {
        title: s::settings_group_rendering,
        rows: &[t(T::ScrollbackMaxRows), sel(S::RenderMaxFps)],
    },
    Card::Rows {
        title: s::settings_group_insets,
        rows: &[t(T::TerminalInsetX), t(T::TerminalInsetY)],
    },
    Card::Rows {
        title: s::settings_card_cursor,
        rows: &[sel(S::CursorStyle)],
    },
    Card::Advanced(&[t(T::ClipboardStreamingMaxBytes)]),
];

const WORKSPACE: &[Card] = &[
    Card::Rows {
        title: s::settings_section_sidebar,
        rows: &[t(T::LeftDefaultWidth), b(B::LeftCollapsedByDefault)],
    },
    Card::Rows {
        title: s::settings_card_files,
        rows: &[
            b(B::FilesShowHidden),
            b(B::FilesUseGitignore),
            sel(S::FileIconColorMode),
            b(B::PreviewTab),
        ],
    },
    Card::Rows {
        title: s::settings_card_status_bar,
        rows: &[
            item(Item::ProjectBranch),
            item(Item::AccountSlot),
            item(Item::Ports),
            item(Item::ClaudeUsage),
            item(Item::Flow),
        ],
    },
    Card::Rows {
        title: s::settings_section_panels,
        rows: &[t(T::PanelsGridColumns)],
    },
    Card::Rows {
        title: s::settings_card_external_editor,
        rows: &[sel(S::PreferredEditor)],
    },
    Card::Advanced(&[
        t(T::UsageLimitsPollSecs),
        t(T::UsageStatusPollSecs),
        t(T::PortsPollSecs),
    ]),
];

const AGENT: &[Card] = &[
    Card::Rows {
        title: s::settings_group_chat,
        rows: &[
            b(B::AgentUseReadingWidth),
            Row::Under(
                B::AgentUseReadingWidth,
                &[Target::Text(T::AgentReadingWidth)],
            ),
            b(B::AgentUseModifierToSend),
            t(T::AgentInputMaxRows),
        ],
    },
    Card::Rows {
        title: s::settings_card_flows,
        rows: &[
            t(T::FlowTimeoutMinutes),
            t(T::FlowMaxNodeRuns),
            t(T::FlowMaxCost),
            t(T::FlowCostCurrency),
        ],
    },
    Card::Custom(CustomCard::AgentCatalog),
    Card::Rows {
        title: s::settings_section_claude_status,
        rows: &[b(B::ClaudeStatusEnabled)],
    },
    Card::Advanced(&[t(T::ClaudeStatusStaleSecs), t(T::ClaudeStatusFileTtlDays)]),
];

const ORCHESTRATOR: &[Card] = &[
    Card::Rows {
        title: s::settings_nav_orchestrator,
        rows: &[
            b(B::OrchestratorEnabled),
            Row::Under(
                B::OrchestratorEnabled,
                &[
                    Target::Select(S::OrchestratorAgent),
                    Target::Select(S::OrchestratorAccount),
                ],
            ),
        ],
    },
    Card::Rows {
        title: s::settings_card_used_by,
        rows: &[Row::Custom(CustomRow::RemoteLink)],
    },
];

const NOTIFICATIONS: &[Card] = &[
    Card::Rows {
        title: s::settings_card_terminal_programs,
        rows: &[
            b(B::NotifyOsc9),
            b(B::NotifyOsc777),
            b(B::NotifyAttention),
            b(B::NotifyLongRunning),
            Row::Under(
                B::NotifyLongRunning,
                &[Target::Text(T::NotifyLongRunningThresholdSecs)],
            ),
        ],
    },
    Card::Rows {
        title: s::settings_card_claude_code_terminals,
        rows: &[b(B::NotifyHook)],
    },
    Card::Rows {
        title: s::settings_card_agent_chat_notifications,
        rows: &[b(B::NotifyAgentCompletion), b(B::NotifyAgentWaiting)],
    },
    Card::Rows {
        title: s::settings_card_notify_behavior,
        rows: &[b(B::NotifySkipFocusedPane)],
    },
];

const REMOTE_CONTROL: &[Card] = &[
    Card::Rows {
        title: s::settings_card_daruda,
        rows: &[Row::Custom(CustomRow::DarudaLink)],
    },
    Card::Custom(CustomCard::RemoteIntegrations),
    Card::Rows {
        title: s::settings_telegram_heading,
        rows: &[
            b(B::TelegramEnabled),
            Row::Under(B::TelegramEnabled, &[Target::Bool(B::TelegramOnlyWhenAway)]),
            Row::Custom(CustomRow::TelegramBody),
        ],
    },
    Card::Advanced(&[
        t(T::PresenceGraceSecs),
        t(T::PresenceIdleSecs),
        t(T::PresenceIdleForegroundSecs),
    ]),
];

const ABOUT: &[Card] = &[
    Card::Custom(CustomCard::AboutVersion),
    Card::Rows {
        title: s::settings_card_updates,
        rows: &[b(B::UpdateAutoCheck)],
    },
    Card::Advanced(&[t(T::LogsRetentionDays), t(T::LogsMaxFileSizeMb)]),
];

/// `section`'s cards in page order; `None` for a page drawn wholly by hand.
pub(super) fn page(section: Section) -> Option<&'static [Card]> {
    match section {
        Section::General => Some(GENERAL),
        Section::Appearance => Some(APPEARANCE),
        Section::Font => Some(FONT),
        Section::Terminal => Some(TERMINAL),
        Section::Workspace => Some(WORKSPACE),
        Section::Agent => Some(AGENT),
        Section::Orchestrator => Some(ORCHESTRATOR),
        Section::Notifications => Some(NOTIFICATIONS),
        Section::RemoteControl => Some(REMOTE_CONTROL),
        Section::About => Some(ABOUT),
        Section::Keymap | Section::SessionHosts | Section::Accounts | Section::Plugin => None,
    }
}

impl CustomCard {
    /// The card's heading, on its page and over its search results. The
    /// version block carries none: its first line says what it is.
    pub(super) fn title(self) -> String {
        match self {
            CustomCard::AgentCatalog => s::settings_section_agent_catalog(),
            CustomCard::RemoteIntegrations => s::settings_group_integrations(),
            CustomCard::AboutVersion => String::new(),
        }
    }
}

impl Card {
    /// The title a search result groups under.
    pub(super) fn title(&self) -> String {
        match self {
            Card::Rows { title, .. } => title(),
            Card::Advanced(_) => s::settings_card_advanced(),
            Card::Custom(kind) => kind.title(),
        }
    }

    pub(super) fn rows(&self) -> &'static [Row] {
        match self {
            Card::Rows { rows, .. } | Card::Advanced(rows) => rows,
            Card::Custom(_) => &[],
        }
    }
}

/// What a page places at one spot of its layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Placed {
    Setting(Target),
    Card(CustomCard),
    Row(CustomRow),
}

/// The switch `target` is indented under, if it only applies while one is on.
pub(super) fn parent_of(target: Target) -> Option<B> {
    Section::ALL
        .iter()
        .filter_map(|section| page(*section))
        .flatten()
        .flat_map(Card::rows)
        .find_map(|row| match row {
            Row::Under(parent, children) if children.contains(&target) => Some(*parent),
            _ => None,
        })
}

/// Everything the pages place, with its page and card title, in page order.
pub(super) fn placed() -> Vec<(Section, String, Placed)> {
    let mut out = Vec::new();
    for section in Section::ALL {
        let Some(cards) = page(*section) else {
            continue;
        };
        for card in cards {
            if let Card::Custom(kind) = card {
                out.push((*section, kind.title(), Placed::Card(*kind)));
                continue;
            }
            let title = card.title();
            for row in card.rows() {
                match row {
                    Row::Setting(target) => {
                        out.push((*section, title.clone(), Placed::Setting(*target)));
                    }
                    Row::Under(_, children) => out.extend(
                        children
                            .iter()
                            .map(|t| (*section, title.clone(), Placed::Setting(*t))),
                    ),
                    Row::Custom(kind) => out.push((*section, title.clone(), Placed::Row(*kind))),
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests;
