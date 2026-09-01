//! Where a catalog row's Fold / Recent steps / Filter pickers get their
//! options, and what each picked entry writes back into `[[agents]]`.
//!
//! Every axis leads with an unset entry — the `None` that lets the agent fall
//! through to the app-wide `[agent]` section — and offers the handful of values
//! a dropdown can state.
//!
//! Two of the three keys can hold values no dropdown can state: a fold matrix
//! with `"<turn>.<block>=<rule>"` cell overrides, or a partial visible facet
//! set. The tail size can be off the offered list. Such a value gets its own
//! selected entry and is written back verbatim while that entry stays picked,
//! mirroring what the catalog already does with an entry it cannot resolve —
//! kept as it stands rather than flattened.

use crate::surface::strings as s;
use crate::ui::select::{self, SelectOption, SelectState};
use daruda_config::{TAIL_WINDOW_ALL, TAIL_WINDOW_CHOICES};
use gpui::{AppContext as _, Entity, SharedString, Window};

use super::super::{AgentCatalogRow, AgentRowTranscript, SettingsWindow};

/// Picker value for "this row states nothing", so the axis follows `[agent]`.
const UNSET: &str = "";

/// Picker value for the entry standing in for a stored value none of the
/// offered choices can state. Not a token any axis produces, so it cannot
/// collide with a real choice.
pub(in crate::settings_window) const CUSTOM: &str = "__custom__";

/// The `display_filter` picker's one stated value: every kind visible. The
/// unset entry cannot say it — that follows `[agent]`, which may be narrowed.
pub(in crate::settings_window) const FILTER_EVERYTHING: &str = "everything";

/// The three transcript pickers a row renders, plus the stored values none of
/// them can state. Built as one unit so a preserved value and the entry that
/// stands for it cannot disagree about which axis is custom.
pub(in crate::settings_window) struct TranscriptRow {
    pub(in crate::settings_window) fold_mode_select: Entity<SelectState>,
    pub(in crate::settings_window) tail_window_select: Entity<SelectState>,
    pub(in crate::settings_window) display_filter_select: Entity<SelectState>,
    pub(in crate::settings_window) preserved: AgentRowTranscript,
}

/// Build the transcript half of a catalog row from the definition it loaded.
pub(in crate::settings_window) fn transcript_row(
    definition: &daruda_config::AgentDefinition,
    window: &mut Window,
    cx: &mut gpui::Context<SettingsWindow>,
) -> TranscriptRow {
    let fold = picker(fold_options(), definition.fold_mode.clone(), |tokens| {
        fold_value(tokens)
    });
    let tail = picker(tail_options(), definition.tail_window, |size| {
        tail_value(*size)
    });
    let filter = picker(
        filter_options(),
        definition.display_filter.clone(),
        |tokens| filter_value(tokens),
    );
    TranscriptRow {
        preserved: AgentRowTranscript {
            fold_mode: fold.preserved,
            tail_window: tail.preserved,
            display_filter: filter.preserved,
        },
        fold_mode_select: cx
            .new(|cx| select::state_with_options(fold.options, Some(&fold.selected), window, cx)),
        tail_window_select: cx
            .new(|cx| select::state_with_options(tail.options, Some(&tail.selected), window, cx)),
        display_filter_select: cx.new(|cx| {
            select::state_with_options(filter.options, Some(&filter.selected), window, cx)
        }),
    }
}

impl AgentCatalogRow {
    /// The `fold_mode` this row states, or `None` for the unset entry. The
    /// "configured elsewhere" entry writes back exactly what config held, so a
    /// matrix with cell overrides survives an edit to any other field.
    pub(in crate::settings_window) fn fold_mode(&self, cx: &gpui::App) -> Option<Vec<String>> {
        let value = picked(&self.fold_mode_select, cx)?;
        match value.as_str() {
            CUSTOM => self.transcript.fold_mode.clone(),
            token => Some(vec![token.to_owned()]),
        }
    }

    /// The `tail_window` this row states, same unset / preserved rules as
    /// [`Self::fold_mode`].
    pub(in crate::settings_window) fn tail_window(&self, cx: &gpui::App) -> Option<u8> {
        let value = picked(&self.tail_window_select, cx)?;
        match value.as_str() {
            CUSTOM => self.transcript.tail_window,
            size => size.parse().ok(),
        }
    }

    /// The `display_filter` this row states, same unset / preserved rules as
    /// [`Self::fold_mode`].
    pub(in crate::settings_window) fn display_filter(&self, cx: &gpui::App) -> Option<Vec<String>> {
        let value = picked(&self.display_filter_select, cx)?;
        // Named rather than wildcarded: unlike the other two axes a picked token
        // is not itself the value here, so a second `filter_options` entry has
        // to state what it writes rather than inherit the unfiltered set.
        match value.as_str() {
            CUSTOM => self.transcript.display_filter.clone(),
            FILTER_EVERYTHING => Some(everything_tokens()),
            _ => None,
        }
    }
}

/// One axis's picker as a row is built.
struct Picker<T> {
    selected: SharedString,
    options: Vec<SelectOption>,
    /// The stored value the [`CUSTOM`] entry stands for, `None` when the stored
    /// value is one of `options` (or absent).
    preserved: Option<T>,
}

/// Assemble one axis: the unset entry, then `offered`, then — only when the
/// stored value is none of them — the entry that keeps it verbatim.
fn picker<T>(
    offered: Vec<SelectOption>,
    stored: Option<T>,
    expressed_as: impl Fn(&T) -> Option<String>,
) -> Picker<T> {
    let mut options = vec![SelectOption::new(
        UNSET,
        s::settings_agent_transcript_default(),
    )];
    options.extend(offered);
    let selected = match stored.as_ref().map(&expressed_as) {
        None => UNSET.to_owned(),
        Some(Some(value)) => value,
        Some(None) => {
            options.push(SelectOption::new(
                CUSTOM,
                s::settings_agent_transcript_custom(),
            ));
            return Picker {
                selected: SharedString::from(CUSTOM),
                options,
                preserved: stored,
            };
        }
    };
    Picker {
        selected: SharedString::from(selected),
        options,
        preserved: None,
    }
}

/// A picker's value, or `None` for the unset entry (and for no selection at
/// all, which reads the same: the row states nothing).
fn picked(state: &Entity<SelectState>, cx: &gpui::App) -> Option<String> {
    let value = state.read(cx).selected_value()?.to_string();
    (value != UNSET).then_some(value)
}

/// The fold presets the picker offers. Destructured from the fold vocabulary
/// itself, so a preset added there is a compile error here rather than an
/// entry silently missing from Settings.
fn fold_options() -> Vec<SelectOption> {
    let [auto, summary, expanded] = crate::workspace::fold_preset_tokens();
    vec![
        SelectOption::new(auto, s::agent_chat_fold_mode_auto()),
        SelectOption::new(summary, s::agent_chat_fold_mode_summary()),
        SelectOption::new(expanded, s::agent_chat_fold_mode_expanded()),
    ]
}

/// The picker value for a stored `fold_mode`, or `None` when no offered entry
/// states it — a matrix with cell overrides, or a list that names no preset.
fn fold_value(tokens: &[String]) -> Option<String> {
    let [only] = tokens else {
        return None;
    };
    crate::workspace::fold_preset_tokens()
        .into_iter()
        .find(|token| only == token)
        .map(str::to_owned)
}

fn tail_options() -> Vec<SelectOption> {
    let mut options = vec![SelectOption::new(
        TAIL_WINDOW_ALL.to_string(),
        s::agent_chat_tail_window_all(),
    )];
    options.extend(TAIL_WINDOW_CHOICES.into_iter().map(|size| {
        SelectOption::new(
            size.to_string(),
            s::agent_chat_tail_window_last(usize::from(size)),
        )
    }));
    options
}

/// The picker value for a stored `tail_window`, or `None` for a size the chip
/// never offers — a hand-written `tail_window = 12` stays exactly that.
fn tail_value(size: u8) -> Option<String> {
    (size == TAIL_WINDOW_ALL || TAIL_WINDOW_CHOICES.contains(&size)).then(|| size.to_string())
}

fn filter_options() -> Vec<SelectOption> {
    vec![SelectOption::new(
        FILTER_EVERYTHING,
        s::settings_agent_display_filter_everything(),
    )]
}

/// The picker value for a stored `display_filter`, or `None` for any narrowed
/// set. Compared as a set: the reader ignores order, so a hand-written list in
/// another order still reads as unfiltered.
fn filter_value(tokens: &[String]) -> Option<String> {
    let mut stored: Vec<&str> = tokens.iter().map(String::as_str).collect();
    stored.sort_unstable();
    let mut everything = crate::workspace::show_everything_tokens();
    everything.sort_unstable();
    (stored == everything).then(|| FILTER_EVERYTHING.to_owned())
}

fn everything_tokens() -> Vec<String> {
    crate::workspace::show_everything_tokens()
        .into_iter()
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_owned()).collect()
    }

    fn values(options: &[SelectOption]) -> Vec<String> {
        options.iter().map(|o| o.value.to_string()).collect()
    }

    /// Every axis leads with the unset entry, because that is what a fresh row
    /// selects and what `None` in config means.
    #[test]
    fn the_unset_entry_leads_every_axis() {
        for offered in [fold_options(), tail_options(), filter_options()] {
            let picker = picker(offered, None::<u8>, |_| None);
            assert_eq!(picker.selected, SharedString::from(UNSET));
            assert_eq!(values(&picker.options)[0], UNSET);
            assert!(picker.preserved.is_none());
        }
    }

    /// A stored value the picker can state selects that entry and adds no
    /// "configured elsewhere" one — there is nothing left to preserve.
    #[test]
    fn an_offered_value_is_selected_directly() {
        let picker = picker(fold_options(), Some(strings(&["summary"])), |t| {
            fold_value(t)
        });
        assert_eq!(picker.selected, SharedString::from("summary"));
        assert!(picker.preserved.is_none());
        assert!(!values(&picker.options).contains(&CUSTOM.to_owned()));
    }

    /// The crux: a value no entry can state is offered as its own selected
    /// entry and kept verbatim, so an unrelated edit cannot flatten it.
    #[test]
    fn a_value_no_entry_states_is_kept_verbatim() {
        let matrix = strings(&["summary", "last.thinking=expanded"]);
        let picker = picker(fold_options(), Some(matrix.clone()), |t| fold_value(t));
        assert_eq!(picker.selected, SharedString::from(CUSTOM));
        assert_eq!(picker.preserved, Some(matrix));
        assert_eq!(values(&picker.options).last().unwrap(), CUSTOM);
    }

    #[test]
    fn every_fold_preset_is_offered_and_reads_back() {
        let offered = values(&fold_options());
        assert_eq!(offered, crate::workspace::fold_preset_tokens().to_vec());
        for token in offered {
            assert_eq!(fold_value(&strings(&[&token])), Some(token.clone()));
        }
    }

    /// A fold list that names no preset — including the empty list, which
    /// resolves to the built-in matrix but is not the same stored value as an
    /// absent key.
    #[test]
    fn a_fold_list_naming_no_preset_is_not_expressible() {
        assert_eq!(fold_value(&[]), None);
        assert_eq!(
            fold_value(&strings(&["summary", "last.tool=collapsed"])),
            None
        );
        assert_eq!(fold_value(&strings(&["bogus"])), None);
    }

    #[test]
    fn the_tail_picker_offers_all_plus_every_chip_size() {
        let mut expected = vec![TAIL_WINDOW_ALL.to_string()];
        expected.extend(TAIL_WINDOW_CHOICES.map(|n| n.to_string()));
        assert_eq!(values(&tail_options()), expected);
        for size in TAIL_WINDOW_CHOICES {
            assert_eq!(tail_value(size), Some(size.to_string()));
        }
        assert_eq!(tail_value(TAIL_WINDOW_ALL), Some("0".to_owned()));
    }

    /// The precedent case: a hand-written size the chip never offers.
    #[test]
    fn an_off_list_tail_size_is_not_expressible() {
        assert_eq!(tail_value(12), None);
        assert_eq!(tail_value(2), None);
    }

    /// The filter's only stated value is the unfiltered set, in any order.
    #[test]
    fn only_the_unfiltered_set_reads_as_show_everything() {
        let everything = everything_tokens();
        assert_eq!(
            filter_value(&everything),
            Some(FILTER_EVERYTHING.to_owned())
        );
        let mut reordered = everything.clone();
        reordered.reverse();
        assert_eq!(
            filter_value(&reordered),
            Some(FILTER_EVERYTHING.to_owned()),
            "the reader ignores order, so the picker must too"
        );
        assert_eq!(filter_value(&strings(&["prose", "tool_read"])), None);
        assert_eq!(filter_value(&[]), None, "an empty visible set is a value");
    }
}
