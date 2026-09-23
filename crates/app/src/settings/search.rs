//! Settings search: an index of every row a page renders, and the query that
//! narrows it.
//!
//! Spec-backed rows come from [`super::copy`], the same table their pages
//! read, so a row cannot be renamed on screen without its search entry
//! following. Hand-drawn blocks (the agent catalog, tokens, plugins…) are
//! listed in [`HANDWRITTEN`] and land as a link to their page.

use daruda_config::{BuiltinSection as Section, Config, StatusBarItem};

use super::{BoolSetting, SelectSetting, TextSetting, copy, navigation, spec};
use crate::surface::strings as s;

/// What a search result renders as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Target {
    Text(TextSetting),
    Select(SelectSetting),
    Bool(BoolSetting),
    StatusBarItem(StatusBarItem),
    /// A block with no row of its own: the result links to its page.
    Page(Section),
}

#[derive(Clone)]
pub(super) struct Doc {
    pub(super) target: Target,
    pub(super) section: Section,
    pub(super) card: String,
    pub(super) label: String,
    pub(super) hint: String,
    path: &'static str,
    keywords: &'static [&'static str],
}

/// Rows that only take effect under a parent switch. A match on the child
/// also shows the parent, so its state is never read out of context.
const PARENTS: &[(Target, BoolSetting)] = &[
    (
        Target::Text(TextSetting::AgentReadingWidth),
        BoolSetting::AgentUseReadingWidth,
    ),
    (
        Target::Text(TextSetting::NotifyLongRunningThresholdSecs),
        BoolSetting::NotifyLongRunning,
    ),
    (
        Target::Bool(BoolSetting::TelegramOnlyWhenAway),
        BoolSetting::TelegramEnabled,
    ),
    (
        Target::Select(SelectSetting::OrchestratorAgent),
        BoolSetting::OrchestratorEnabled,
    ),
    (
        Target::Select(SelectSetting::OrchestratorAccount),
        BoolSetting::OrchestratorEnabled,
    ),
];

type Handwritten = (Section, fn() -> String, &'static [&'static str]);

/// Blocks a page draws by hand, indexed as links to that page.
const HANDWRITTEN: &[Handwritten] = &[
    (
        Section::Agent,
        s::settings_section_agent_catalog,
        &["agent", "preset", "command", "model", "mode", "acp"],
    ),
    (
        Section::SessionHosts,
        s::settings_session_host_add,
        &["ssh", "docker", "host", "remote"],
    ),
    (
        Section::Accounts,
        s::settings_nav_accounts,
        &["account", "login", "claude", "codex"],
    ),
    (
        Section::RemoteControl,
        s::settings_telegram_token_label,
        &["telegram", "bot", "token"],
    ),
    (
        Section::RemoteControl,
        s::settings_telegram_generate_code,
        &["telegram", "pair", "phone"],
    ),
    (
        Section::RemoteControl,
        s::remote_slack,
        &["slack", "token", "pair", "phone", "away"],
    ),
    (
        Section::RemoteControl,
        s::remote_discord,
        &["discord", "token", "pair", "phone", "away"],
    ),
    (
        Section::Plugin,
        s::settings_plugin_installed_header,
        &["plugin", "skill", "install"],
    ),
    (
        Section::Keymap,
        s::settings_section_keymap,
        &["shortcut", "keybinding", "key", "keymap"],
    ),
];

/// Retired page names and common words for a page, so a query for where a
/// setting used to live still finds where it lives now.
fn page_keywords(section: Section) -> &'static [&'static str] {
    match section {
        Section::Appearance => &["window", "theme"],
        Section::Terminal => &["shell", "cursor", "clipboard"],
        Section::Workspace => &["dock", "panels", "sidebar", "external editor"],
        Section::About => &["update", "version", "license"],
        _ => &[],
    }
}

/// Every searchable entry, in page order.
pub(super) fn docs() -> Vec<Doc> {
    let defaults = Config::default();
    let mut docs = Vec::new();
    for section in Section::ALL {
        docs.push(Doc {
            target: Target::Page(*section),
            section: *section,
            card: String::new(),
            label: navigation::label(*section),
            hint: navigation::description(*section),
            path: section.slug(),
            keywords: page_keywords(*section),
        });
    }
    let row = |target, row: copy::RowCopy, path| Doc {
        target,
        section: row.section,
        card: (row.card)(),
        label: (row.label)(),
        hint: (row.hint)(),
        path,
        keywords: &[],
    };
    for spec in spec::TEXT_SETTINGS {
        let path = (spec.current)(&defaults).field().path();
        docs.push(row(
            Target::Text(spec.setting),
            copy::text(spec.setting),
            path,
        ));
    }
    for spec in spec::SELECT_SETTINGS {
        let path = (spec.current)(&defaults).field().path();
        docs.push(row(
            Target::Select(spec.setting),
            copy::select(spec.setting),
            path,
        ));
    }
    for spec in spec::BOOL_SETTINGS {
        let path = (spec.patch)(false).field().path();
        docs.push(row(
            Target::Bool(spec.setting),
            copy::bool(spec.setting),
            path,
        ));
    }
    for item in StatusBarItem::ALL {
        docs.push(Doc {
            target: Target::StatusBarItem(*item),
            section: Section::Workspace,
            card: s::settings_card_status_bar(),
            label: super::sections::status_bar_item_label(*item),
            hint: String::new(),
            path: "status_bar.hidden_items",
            keywords: &[],
        });
    }
    for (section, label, keywords) in HANDWRITTEN {
        docs.push(Doc {
            target: Target::Page(*section),
            section: *section,
            card: navigation::label(*section),
            label: label(),
            hint: String::new(),
            path: section.slug(),
            keywords,
        });
    }
    docs
}

impl Doc {
    /// Every whitespace-separated term appears somewhere in the entry.
    fn matches(&self, terms: &[String]) -> bool {
        let haystack = format!(
            "{} {} {} {} {} {}",
            self.label,
            self.hint,
            self.card,
            navigation::label(self.section),
            self.path,
            self.keywords.join(" ")
        )
        .to_lowercase();
        terms.iter().all(|term| haystack.contains(term.as_str()))
    }
}

/// The entries `query` names, in page order, each child preceded by its
/// parent switch. Empty for a blank query.
pub(super) fn query(query: &str) -> Vec<Doc> {
    let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if terms.is_empty() {
        return Vec::new();
    }
    let all = docs();
    let hit: Vec<bool> = all.iter().map(|doc| doc.matches(&terms)).collect();
    let mut keep = hit.clone();
    for (i, doc) in all.iter().enumerate() {
        if !hit[i] {
            continue;
        }
        if let Some((_, parent)) = PARENTS.iter().find(|(child, _)| *child == doc.target)
            && let Some(p) = all.iter().position(|d| d.target == Target::Bool(*parent))
        {
            keep[p] = true;
        }
    }
    let mut out: Vec<Doc> = all
        .into_iter()
        .zip(keep)
        .filter_map(|(doc, keep)| keep.then_some(doc))
        .collect();
    // Group by page, keeping each page's own order; a parent sits before
    // its child because the row tables list it first or it is moved there.
    out.sort_by_key(|doc| Section::ALL.iter().position(|s| *s == doc.section));
    move_parents_first(&mut out);
    out
}

fn move_parents_first(docs: &mut Vec<Doc>) {
    for (child, parent) in PARENTS {
        let (Some(c), Some(p)) = (
            docs.iter().position(|d| d.target == *child),
            docs.iter().position(|d| d.target == Target::Bool(*parent)),
        ) else {
            continue;
        };
        if p > c {
            let parent_doc = docs.remove(p);
            docs.insert(c, parent_doc);
        }
    }
}

/// How many results fall on each page, in sidebar order.
pub(super) fn counts(results: &[Doc]) -> Vec<(Section, usize)> {
    Section::ALL
        .iter()
        .filter_map(|section| {
            let n = results.iter().filter(|d| d.section == *section).count();
            (n > 0).then_some((*section, n))
        })
        .collect()
}

#[cfg(test)]
mod tests;
