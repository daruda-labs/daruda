//! What a catalog row's Fold / Recent steps / Filter controls hold, and what
//! each writes back into `[[agents]]`.
//!
//! This row is the default — there is no layer under it but the built-in value.
//! So each axis states its value directly, and the one that *is* the built-in
//! writes nothing back: an unwritten key and a key stating the built-in value
//! resolve alike, and the shorter of the two keeps the file clean.
//!
//! Fold and Filter carry the same editors the chat pane opens (see
//! [`crate::transcript::editor`]), so every value those keys can hold is one
//! the row can state and edit — including a matrix with `"<turn>.<block>=<rule>"`
//! cell overrides or a partial facet set. Recent steps is still a dropdown, and
//! still the one axis that can load a size it cannot offer: a hand-written
//! `tail_window = 12` gets its own selected entry and is written back verbatim
//! while that entry stays picked.

use crate::surface::strings as s;
use crate::transcript::display_filter::DisplayFilter;
use crate::transcript::fold_mode::FoldMode;
use crate::ui::select::{self, SelectOption, SelectState};
use daruda_config::{TAIL_WINDOW_ALL, TAIL_WINDOW_CHOICES, TAIL_WINDOW_DEFAULT};
use gpui::{AppContext as _, Entity, SharedString, Window};

use super::super::{AgentCatalogRow, SettingsWindow};

pub(in crate::settings_window) mod editor;

/// Picker value for the entry standing in for a stored size none of the offered
/// choices can state. Not a token the axis produces, so it cannot collide with
/// a real choice.
pub(in crate::settings_window) const CUSTOM: &str = "__custom__";

/// The transcript controls a row renders, plus the one stored value the
/// remaining dropdown cannot state.
pub(in crate::settings_window) struct TranscriptRow {
    pub(in crate::settings_window) fold_mode: Option<FoldMode>,
    pub(in crate::settings_window) fold_mode_loaded: Option<Vec<String>>,
    pub(in crate::settings_window) display_filter: Option<DisplayFilter>,
    pub(in crate::settings_window) display_filter_loaded: Option<Vec<String>>,
    pub(in crate::settings_window) tail_window_select: Entity<SelectState>,
    pub(in crate::settings_window) tail_window_loaded: Option<u8>,
}

/// Build the transcript half of a catalog row from the definition it loaded.
pub(in crate::settings_window) fn transcript_row(
    definition: &daruda_config::AgentDefinition,
    window: &mut Window,
    cx: &mut gpui::Context<SettingsWindow>,
) -> TranscriptRow {
    let tail = picker(
        tail_options(),
        &tail_built_in(),
        definition.tail_window,
        |size| tail_value(*size),
    );
    TranscriptRow {
        // An absent key is the built-in, which the editor would otherwise be
        // unable to tell apart from a key stating it — so the override stays an
        // `Option` rather than collapsing into a `FoldMode`.
        fold_mode: definition
            .fold_mode
            .as_ref()
            .map(|tokens| FoldMode::from_tokens(tokens.iter().map(String::as_str))),
        fold_mode_loaded: definition.fold_mode.clone(),
        display_filter: definition
            .display_filter
            .as_ref()
            .map(|tokens| DisplayFilter::from_stored(tokens)),
        display_filter_loaded: definition.display_filter.clone(),
        tail_window_loaded: tail.preserved,
        tail_window_select: cx
            .new(|cx| select::state_with_options(tail.options, Some(&tail.selected), window, cx)),
    }
}

impl AgentCatalogRow {
    /// The `fold_mode` this row writes, or `None` to write no key at all —
    /// which is what "follow the built-in" means, since an absent key resolves
    /// to exactly that value.
    pub(in crate::settings_window) fn fold_mode(&self) -> Option<Vec<String>> {
        let mode = self.fold_mode?;
        Some(
            untouched(self.fold_mode_loaded.as_deref(), mode, |tokens| {
                FoldMode::from_tokens(tokens.iter().map(String::as_str))
            })
            .unwrap_or_else(|| mode.tokens()),
        )
    }

    /// The value the fold editor edits: the row's own override, or the built-in
    /// it would otherwise start from.
    pub(in crate::settings_window) fn fold_mode_value(&self) -> FoldMode {
        self.fold_mode.unwrap_or_default()
    }

    /// The `tail_window` this row writes, same built-in / preserved rules as
    /// [`Self::fold_mode`].
    pub(in crate::settings_window) fn tail_window(&self, cx: &gpui::App) -> Option<u8> {
        let value = picked(&self.tail_window_select, cx)?;
        match value.as_str() {
            CUSTOM => self.tail_window_loaded,
            size if size == tail_built_in() => None,
            size => size.parse().ok(),
        }
    }

    /// The `display_filter` this row writes, same built-in / preserved rules as
    /// [`Self::fold_mode`].
    pub(in crate::settings_window) fn display_filter(&self) -> Option<Vec<String>> {
        let filter = self.display_filter?;
        Some(
            untouched(self.display_filter_loaded.as_deref(), filter, |tokens| {
                DisplayFilter::from_stored(tokens)
            })
            .unwrap_or_else(|| filter.tokens().into_iter().map(str::to_owned).collect()),
        )
    }

    /// The value the filter editor edits — see [`Self::fold_mode_value`].
    pub(in crate::settings_window) fn display_filter_value(&self) -> DisplayFilter {
        self.display_filter.unwrap_or_default()
    }
}

impl SettingsWindow {
    /// Write a fold value onto a row. `None` drops the key, which is what the
    /// editor's reset footer hands back.
    ///
    /// Recording the move is part of writing, not of the caller: a path that
    /// wrote without recording would dead-end its own `Custom` segment, which
    /// is what the chat pane's [`AgentChatView::set_fold_mode`] avoids by
    /// remembering here too.
    pub(in crate::settings_window) fn set_agent_row_fold_mode(
        &mut self,
        catalog_index: usize,
        mode: Option<FoldMode>,
        cx: &mut gpui::Context<Self>,
    ) {
        // An edit that lands on the built-in states nothing: an absent key and
        // a key stating that value resolve alike.
        let stored = mode.filter(|mode| *mode != FoldMode::default());
        let Some(row) = self.agent_editable_row_mut(catalog_index) else {
            return;
        };
        if row.fold_mode == stored {
            return;
        }
        // Recorded against what was asked for, not what gets stored: landing on
        // the built-in is still a departure from a matrix worth recalling, and
        // the normalization above would otherwise swallow it.
        if let Some(next) = mode {
            row.fold_editor.remember(row.fold_mode_value(), next);
        }
        let previous = std::mem::replace(&mut row.fold_mode, stored);
        // Nothing reached disk, so the row must not go on claiming it did —
        // same hand-back as `remove_agent_catalog_item`. The recall above
        // stands either way: it names where the user came from, not what
        // config holds.
        if !self.persist_agent_catalog(cx)
            && let Some(row) = self.agent_editable_row_mut(catalog_index)
        {
            row.fold_mode = previous;
        }
        cx.notify();
    }

    /// Pick a segment of the fold editor's preset strip — the same dispatch the
    /// chat pane makes, against this row's own editor state.
    pub(in crate::settings_window) fn select_agent_row_fold_preset(
        &mut self,
        catalog_index: usize,
        preset: Option<crate::transcript::fold_mode::FoldPreset>,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(row) = self.agent_editable_row_mut(catalog_index) else {
            return;
        };
        let Some(target) = row.fold_editor.segment_target(preset) else {
            return;
        };
        self.set_agent_row_fold_mode(catalog_index, Some(target), cx);
    }

    /// Hand the fold axis back to the built-in, keeping the hand-edited matrix
    /// recallable through the `Custom` segment — the chat pane's reset does the
    /// same, and the two sit in the same editor.
    pub(in crate::settings_window) fn reset_agent_row_fold_mode(
        &mut self,
        catalog_index: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(row) = self.agent_editable_row_mut(catalog_index) else {
            return;
        };
        row.fold_editor
            .remember_before_reset(row.fold_mode_value(), FoldMode::default());
        self.set_agent_row_fold_mode(catalog_index, None, cx);
    }

    /// Move the fold editor's turn column. A view switch, not a value, so it
    /// neither persists nor marks the catalog dirty.
    pub(in crate::settings_window) fn set_agent_row_fold_turn(
        &mut self,
        catalog_index: usize,
        turn: crate::transcript::fold_mode::TurnPosition,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(row) = self.agent_editable_row_mut(catalog_index) else {
            return;
        };
        if row.fold_editor.set_turn(turn) {
            cx.notify();
        }
    }

    pub(in crate::settings_window) fn toggle_agent_row_filter_facet(
        &mut self,
        catalog_index: usize,
        facet: crate::transcript::display_filter::FilterFacet,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(row) = self.agent_editable_row_mut(catalog_index) else {
            return;
        };
        let next = row.display_filter_value().toggled(facet);
        self.set_agent_row_display_filter(catalog_index, Some(next), cx);
    }

    pub(in crate::settings_window) fn set_agent_row_filter_section(
        &mut self,
        catalog_index: usize,
        parent: crate::transcript::display_filter::FilterFacet,
        on: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(row) = self.agent_editable_row_mut(catalog_index) else {
            return;
        };
        let next = row.display_filter_value().with_section(parent, on);
        self.set_agent_row_display_filter(catalog_index, Some(next), cx);
    }

    /// Hand the filter axis back to the built-in — see
    /// [`Self::reset_agent_row_fold_mode`].
    pub(in crate::settings_window) fn reset_agent_row_display_filter(
        &mut self,
        catalog_index: usize,
        cx: &mut gpui::Context<Self>,
    ) {
        self.set_agent_row_display_filter(catalog_index, None, cx);
    }

    fn set_agent_row_display_filter(
        &mut self,
        catalog_index: usize,
        filter: Option<DisplayFilter>,
        cx: &mut gpui::Context<Self>,
    ) {
        // The unfiltered set is the built-in one — see
        // [`Self::set_agent_row_fold_mode`].
        let filter = filter.filter(|f| *f != DisplayFilter::default());
        let Some(row) = self.agent_editable_row_mut(catalog_index) else {
            return;
        };
        if row.display_filter == filter {
            return;
        }
        let previous = std::mem::replace(&mut row.display_filter, filter);
        // See [`Self::set_agent_row_fold_mode`].
        if !self.persist_agent_catalog(cx)
            && let Some(row) = self.agent_editable_row_mut(catalog_index)
        {
            row.display_filter = previous;
        }
        cx.notify();
    }
}

/// The tokens an axis loaded, when they still spell the value the row holds.
///
/// An untouched axis is written back exactly as it was read, so a save that
/// edits some other field rewrites nothing here — including a token this build
/// does not know, which [`FoldMode::from_tokens`] and
/// [`DisplayFilter::from_stored`] both drop on the way in. Editing the axis
/// moves the value off what the tokens spell, and from then on the row states
/// the value.
fn untouched<T: PartialEq>(
    loaded: Option<&[String]>,
    current: T,
    parse: impl Fn(&[String]) -> T,
) -> Option<Vec<String>> {
    let tokens = loaded?;
    (parse(tokens) == current).then(|| tokens.to_vec())
}

/// The tail picker as a row is built.
struct Picker {
    selected: SharedString,
    options: Vec<SelectOption>,
    /// The stored size the [`CUSTOM`] entry stands for, `None` when the stored
    /// size is one of `options` (or absent).
    preserved: Option<u8>,
}

/// Assemble one axis: the values it offers, then — only when the stored value
/// is none of them — the entry that keeps it verbatim. An absent key selects
/// `built_in`, the entry that resolves to the same thing.
fn picker(
    offered: Vec<SelectOption>,
    built_in: &str,
    stored: Option<u8>,
    expressed_as: impl Fn(&u8) -> Option<String>,
) -> Picker {
    let mut options = offered;
    let selected = match stored.as_ref().map(&expressed_as) {
        None => built_in.to_owned(),
        Some(Some(value)) => value,
        Some(None) => {
            options.push(SelectOption::new(
                CUSTOM,
                s::settings_agent_tail_window_off_list(),
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

/// A picker's value. `None` only when nothing is selected at all, which reads
/// as "this row writes no key".
fn picked(state: &Entity<SelectState>, cx: &gpui::App) -> Option<String> {
    Some(state.read(cx).selected_value()?.to_string())
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

/// The window an absent `tail_window` resolves to.
fn tail_built_in() -> String {
    TAIL_WINDOW_DEFAULT.to_string()
}

/// The picker value for a stored `tail_window`, or `None` for a size the chip
/// never offers — a hand-written `tail_window = 12` stays exactly that.
fn tail_value(size: u8) -> Option<String> {
    (size == TAIL_WINDOW_ALL || TAIL_WINDOW_CHOICES.contains(&size)).then(|| size.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::display_filter::FilterFacet;
    use crate::transcript::fold_mode::{BlockRule, FoldBlock, FoldPreset, TurnPosition};

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_owned()).collect()
    }

    fn values(options: &[SelectOption]) -> Vec<String> {
        options.iter().map(|o| o.value.to_string()).collect()
    }

    /// An absent key selects the entry that resolves to the same thing, so a
    /// fresh row reads as the size a pane would actually get.
    #[test]
    fn an_absent_tail_key_selects_the_built_in_entry() {
        let built_in = tail_built_in();
        let picker = picker(tail_options(), &built_in, None, |_| None);
        assert_eq!(picker.selected, SharedString::from(built_in.clone()));
        assert!(
            values(&picker.options).contains(&built_in),
            "the built-in entry has to be one the axis offers"
        );
        assert!(picker.preserved.is_none());
    }

    /// The tail picker offers no entry standing for "states nothing" — this row
    /// is the default, so such an entry would duplicate the built-in one.
    #[test]
    fn the_tail_picker_offers_no_unset_entry() {
        assert!(
            values(&tail_options())
                .iter()
                .all(|value| !value.is_empty()),
            "an empty value is the shape the removed unset entry had"
        );
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

    /// Tail is the one axis left that can load a value it cannot state, so it
    /// keeps the preserved entry the other two shed.
    #[test]
    fn an_off_list_tail_size_is_kept_verbatim() {
        assert_eq!(tail_value(12), None);
        let picker = picker(tail_options(), &tail_built_in(), Some(12u8), |s| {
            tail_value(*s)
        });
        assert_eq!(picker.selected, SharedString::from(CUSTOM));
        assert_eq!(picker.preserved, Some(12));
        assert_eq!(values(&picker.options).last().unwrap(), CUSTOM);
    }

    fn row_transcript(
        fold: Option<&[&str]>,
        filter: Option<&[&str]>,
    ) -> (Option<FoldMode>, Option<DisplayFilter>) {
        (
            fold.map(|t| FoldMode::from_tokens(t.iter().copied())),
            filter.map(|t| DisplayFilter::from_stored(&strings(t))),
        )
    }

    /// An absent key stays absent: the editor opens on the value it resolves
    /// to, but the row writes nothing until something is edited.
    #[test]
    fn an_absent_key_reads_as_the_built_in_without_writing_one() {
        let (fold, filter) = row_transcript(None, None);
        assert_eq!(fold, None);
        assert_eq!(filter, None);
        assert_eq!(fold.unwrap_or_default(), FoldMode::default());
        assert_eq!(filter.unwrap_or_default(), DisplayFilter::default());
    }

    /// The crux of dropping the preserved entry: every value these keys can
    /// hold is one the editor can state, so it round-trips instead of needing
    /// to be kept verbatim.
    #[test]
    fn a_matrix_with_cell_overrides_round_trips_through_the_row() {
        let stored = [
            "summary",
            "last.thinking=expanded",
            "past.tool.read=collapsed",
        ];
        let (fold, _) = row_transcript(Some(&stored), None);
        let mode = fold.expect("a stated key");
        assert_eq!(mode.preset(), None, "no preset states this matrix");
        assert_eq!(
            mode.rule(TurnPosition::Last, FoldBlock::Thinking),
            BlockRule::Expanded
        );
        assert_eq!(
            FoldMode::from_tokens(mode.tokens().iter().map(String::as_str)),
            mode,
            "what the row writes back reads as the same matrix"
        );
    }

    #[test]
    fn a_partial_facet_set_round_trips_through_the_row() {
        let narrowed = DisplayFilter::default().toggled(FilterFacet::Thinking);
        let tokens: Vec<String> = narrowed.tokens().into_iter().map(str::to_owned).collect();
        let (_, filter) = row_transcript(None, Some(&[]));
        assert!(
            filter.is_some(),
            "a stated key is an override even if empty"
        );
        let reread = DisplayFilter::from_stored(&tokens);
        assert_eq!(reread, narrowed);
        assert!(!reread.shows_everything());
    }

    /// Every preset survives the round trip the row does on commit.
    #[test]
    fn every_fold_preset_round_trips_through_the_row() {
        for preset in FoldPreset::ALL {
            let mode = preset.mode();
            let tokens = mode.tokens();
            assert_eq!(tokens.len(), 1, "a preset writes back as its own name");
            let reread = FoldMode::from_tokens(tokens.iter().map(String::as_str));
            assert_eq!(reread, mode);
            assert_eq!(reread.preset(), Some(preset));
        }
    }
}
