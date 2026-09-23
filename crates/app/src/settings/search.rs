//! Settings search: an index of every row the pages render, in page order,
//! and the query that narrows it. Rows come from the tables their pages read
//! (`copy`, `layout`); hand-drawn blocks are listed in [`HANDWRITTEN`] and
//! land as a link to their page.

use daruda_config::{BuiltinSection as Section, Config, SettingsPatch};

use super::layout::{self, CustomCard as C, CustomRow as R, Target};
use super::{copy, navigation, spec};
use crate::surface::strings as s;

/// What a search result renders as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Hit {
    /// The row's own live control.
    Setting(Target),
    /// A block with no row of its own: the result links to its page.
    Page(Section),
}

#[derive(Clone)]
pub(super) struct Doc {
    pub(super) target: Hit,
    pub(super) section: Section,
    pub(super) card: String,
    pub(super) label: String,
    pub(super) hint: String,
    path: &'static str,
    keywords: &'static [&'static str],
}

/// Where a hand-drawn block sits, so its result lists in page order.
/// The page comes from where the layout places the anchor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Anchor {
    Card(C),
    Row(R),
    /// A page with no layout: the block is the page.
    Page(Section),
}

struct Handwritten {
    anchor: Anchor,
    label: fn() -> String,
    keywords: &'static [&'static str],
}

const fn hand(
    anchor: Anchor,
    label: fn() -> String,
    keywords: &'static [&'static str],
) -> Handwritten {
    Handwritten {
        anchor,
        label,
        keywords,
    }
}

/// Blocks a page draws by hand, indexed as links to that page.
const HANDWRITTEN: &[Handwritten] = &[
    hand(
        Anchor::Row(R::CustomColors),
        s::settings_label_custom_colors,
        &["colors", "palette", "ansi", "custom"],
    ),
    hand(
        Anchor::Card(C::AgentCatalog),
        s::settings_section_agent_catalog,
        &["agent", "preset", "command", "model", "mode", "acp"],
    ),
    hand(
        Anchor::Card(C::RemoteIntegrations),
        s::remote_slack,
        &["slack", "token", "pair", "phone", "away"],
    ),
    hand(
        Anchor::Card(C::RemoteIntegrations),
        s::remote_discord,
        &["discord", "token", "pair", "phone", "away"],
    ),
    hand(
        Anchor::Row(R::TelegramBody),
        s::settings_telegram_token_label,
        &["telegram", "bot", "token"],
    ),
    hand(
        Anchor::Row(R::TelegramBody),
        s::settings_telegram_generate_code,
        &["telegram", "pair", "phone"],
    ),
    hand(
        Anchor::Page(Section::SessionHosts),
        s::settings_session_host_add,
        &["ssh", "docker", "host", "remote"],
    ),
    hand(
        Anchor::Page(Section::Plugin),
        s::settings_plugin_installed_header,
        &["plugin", "skill", "install"],
    ),
    hand(
        Anchor::Page(Section::Keymap),
        s::settings_section_keymap,
        &["shortcut", "keybinding", "key", "keymap"],
    ),
];

impl Handwritten {
    fn doc(&self, section: Section, card: String) -> Doc {
        Doc {
            target: Hit::Page(section),
            section,
            card,
            label: (self.label)(),
            hint: String::new(),
            path: section.slug(),
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
            target: Hit::Page(*section),
            section: *section,
            card: String::new(),
            label: navigation::label(*section),
            hint: navigation::description(*section),
            path: section.slug(),
            keywords: page_keywords(*section),
        });
    }
    // Rows in the order their pages show them; the layout places each once.
    for (section, card, placed) in layout::placed() {
        let target = match placed {
            layout::Placed::Setting(target) => target,
            layout::Placed::Card(kind) => {
                docs.extend(anchored(Anchor::Card(kind)).map(|h| h.doc(section, card.clone())));
                continue;
            }
            layout::Placed::Row(kind) => {
                docs.extend(anchored(Anchor::Row(kind)).map(|h| h.doc(section, card.clone())));
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
                SettingsPatch::StatusBarHiddenItems(Vec::new())
                    .field()
                    .path(),
            ),
        };
        docs.push(Doc {
            target: Hit::Setting(target),
            section,
            card,
            label,
            hint,
            path,
            keywords: &[],
        });
    }
    for section in Section::ALL {
        docs.extend(anchored(Anchor::Page(*section)).map(|h| h.doc(*section, String::new())));
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
        let Hit::Setting(target) = doc.target else {
            continue;
        };
        if let Some(parent) = layout::parent_of(target)
            && let Some(p) = all
                .iter()
                .position(|d| d.target == Hit::Setting(Target::Bool(parent)))
        {
            keep[p] = true;
        }
    }
    let mut out: Vec<Doc> = all
        .into_iter()
        .zip(keep)
        .filter_map(|(doc, keep)| keep.then_some(doc))
        .collect();
    // Group by page, keeping each page's own order; the layout lists a
    // parent switch just before the rows under it.
    out.sort_by_key(|doc| Section::ALL.iter().position(|s| *s == doc.section));
    out
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
