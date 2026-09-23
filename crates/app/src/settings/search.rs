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

/// Where a hand-drawn block sits, so its result lists in page order.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Anchor {
    Card(super::layout::CustomCard),
    Row(super::layout::CustomRow),
    /// A page with no layout: the block is the page.
    Page(Section),
}

struct Handwritten {
    anchor: Anchor,
    section: Section,
    label: fn() -> String,
    keywords: &'static [&'static str],
}

const fn hand(
    anchor: Anchor,
    section: Section,
    label: fn() -> String,
    keywords: &'static [&'static str],
) -> Handwritten {
    Handwritten {
        anchor,
        section,
        label,
        keywords,
    }
}

use super::layout::{CustomCard as C, CustomRow as R};

/// Blocks a page draws by hand, indexed as links to that page.
const HANDWRITTEN: &[Handwritten] = &[
    hand(
        Anchor::Row(R::CustomColors),
        Section::Appearance,
        s::settings_label_custom_colors,
        &["colors", "palette", "ansi", "custom"],
    ),
    hand(
        Anchor::Card(C::AgentCatalog),
        Section::Agent,
        s::settings_section_agent_catalog,
        &["agent", "preset", "command", "model", "mode", "acp"],
    ),
    hand(
        Anchor::Card(C::RemoteIntegrations),
        Section::RemoteControl,
        s::remote_slack,
        &["slack", "token", "pair", "phone", "away"],
    ),
    hand(
        Anchor::Card(C::RemoteIntegrations),
        Section::RemoteControl,
        s::remote_discord,
        &["discord", "token", "pair", "phone", "away"],
    ),
    hand(
        Anchor::Row(R::TelegramBody),
        Section::RemoteControl,
        s::settings_telegram_token_label,
        &["telegram", "bot", "token"],
    ),
    hand(
        Anchor::Row(R::TelegramBody),
        Section::RemoteControl,
        s::settings_telegram_generate_code,
        &["telegram", "pair", "phone"],
    ),
    hand(
        Anchor::Page(Section::SessionHosts),
        Section::SessionHosts,
        s::settings_session_host_add,
        &["ssh", "docker", "host", "remote"],
    ),
    hand(
        Anchor::Page(Section::Plugin),
        Section::Plugin,
        s::settings_plugin_installed_header,
        &["plugin", "skill", "install"],
    ),
    hand(
        Anchor::Page(Section::Keymap),
        Section::Keymap,
        s::settings_section_keymap,
        &["shortcut", "keybinding", "key", "keymap"],
    ),
];

impl Handwritten {
    fn doc(&self, card: String) -> Doc {
        Doc {
            target: Target::Page(self.section),
            section: self.section,
            card,
            label: (self.label)(),
            hint: String::new(),
            path: self.section.slug(),
            keywords: self.keywords,
        }
    }
}

fn anchored(anchor: Anchor) -> impl Iterator<Item = &'static Handwritten> {
    HANDWRITTEN.iter().filter(move |h| h.anchor == anchor)
}

/// Retired page names and common words for a page, so a query for where a
/// setting used to live still finds where it lives now.
fn page_keywords(section: Section) -> &'static [&'static str] {
    match section {
        Section::Appearance => &["window", "theme"],
        Section::Terminal => &["shell", "cursor", "clipboard"],
        Section::Workspace => &["dock", "panels", "sidebar", "external editor"],
        Section::About => &["update", "version", "license"],
        Section::Accounts => &["account", "login", "claude", "codex"],
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
    // Rows in the order their pages show them; the layout places each once.
    for (section, card, placed) in super::layout::placed() {
        let target = match placed {
            super::layout::Placed::Setting(target) => target,
            super::layout::Placed::Card(kind) => {
                docs.extend(anchored(Anchor::Card(kind)).map(|h| h.doc(String::new())));
                continue;
            }
            super::layout::Placed::Row(kind) => {
                docs.extend(anchored(Anchor::Row(kind)).map(|h| h.doc(card.clone())));
                continue;
            }
        };
        let (label, hint, path) = match target {
            Target::Text(t) => {
                let copy = copy::text(t);
                (
                    (copy.label)(),
                    (copy.hint)(),
                    (spec::text_spec(t).current)(&defaults).field().path(),
                )
            }
            Target::Select(v) => {
                let copy = copy::select(v);
                (
                    (copy.label)(),
                    (copy.hint)(),
                    (spec::select_spec(v).current)(&defaults).field().path(),
                )
            }
            Target::Bool(b) => {
                let copy = copy::bool(b);
                (
                    (copy.label)(),
                    (copy.hint)(),
                    (spec::bool_spec(b).patch)(false).field().path(),
                )
            }
            Target::StatusBarItem(item) => (
                super::sections::status_bar_item_label(item),
                String::new(),
                "status_bar.hidden_items",
            ),
            Target::Page(_) => continue,
        };
        docs.push(Doc {
            target,
            section,
            card,
            label,
            hint,
            path,
            keywords: &[],
        });
    }
    for section in Section::ALL {
        docs.extend(anchored(Anchor::Page(*section)).map(|h| h.doc(String::new())));
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
