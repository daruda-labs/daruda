//! What the person has filled into an open question, and the answer it makes.
//!
//! Plain data with no widgets: the view keeps text boxes, commits their
//! contents here, and asks this type whether the form is complete.

use std::collections::{BTreeMap, BTreeSet};

use daruda_acp::{QuestionField, QuestionFieldKind, QuestionRequest, QuestionValue};

#[derive(Debug, Default, Clone)]
pub(crate) struct QuestionDraft {
    /// Picked option values per select field, in pick order.
    picks: BTreeMap<String, Vec<String>>,
    flags: BTreeMap<String, bool>,
    /// Committed free text by field key; never holds an empty string.
    texts: BTreeMap<String, String>,
    /// Set once the person acts, so seeded defaults alone never count.
    touched: bool,
    /// Primary fields the person chose to leave blank, so a walk through
    /// the unanswered ones does not come back to them.
    left_blank: BTreeSet<String>,
    /// Primary fields the person answered or accepted as shown. A field
    /// holding only its schema default is answered but not confirmed, so a
    /// surface that never showed it still asks.
    confirmed: BTreeSet<String>,
}

impl QuestionDraft {
    /// A draft pre-filled with the schema's defaults. Defaults do not mark
    /// it touched: the person has not yet acted on the form.
    pub(crate) fn seeded(request: &QuestionRequest) -> Self {
        let mut draft = Self::default();
        for field in &request.fields {
            let Some(default) = &field.default else {
                continue;
            };
            match (&field.kind, default) {
                (QuestionFieldKind::SingleSelect(options), QuestionValue::Text(v))
                    if options.iter().any(|o| o.value == *v) =>
                {
                    draft.picks.insert(field.key.clone(), vec![v.clone()]);
                }
                (QuestionFieldKind::MultiSelect(options), QuestionValue::Choices(values)) => {
                    // The schema does not vouch for multi-select defaults.
                    let offered: Vec<String> = values
                        .iter()
                        .filter(|v| options.iter().any(|o| o.value == **v))
                        .cloned()
                        .collect();
                    if !offered.is_empty() {
                        draft.picks.insert(field.key.clone(), offered);
                    }
                }
                (QuestionFieldKind::Boolean, QuestionValue::Bool(b)) => {
                    draft.flags.insert(field.key.clone(), *b);
                }
                (QuestionFieldKind::Text, QuestionValue::Text(t)) => {
                    let t = t.trim();
                    if !t.is_empty() {
                        draft.texts.insert(field.key.clone(), t.to_owned());
                    }
                }
                _ => {}
            }
        }
        draft
    }

    /// Pick `value` in `field`: a single select keeps one pick, a
    /// multi-select toggles it.
    pub(crate) fn pick(&mut self, field: &QuestionField, value: &str) {
        self.touched = true;
        self.confirmed.insert(field.key.clone());
        let picks = self.picks.entry(field.key.clone()).or_default();
        if matches!(field.kind, QuestionFieldKind::MultiSelect(_)) {
            match picks.iter().position(|p| p == value) {
                Some(at) => {
                    picks.remove(at);
                }
                None => picks.push(value.to_owned()),
            }
        } else {
            *picks = vec![value.to_owned()];
        }
    }

    pub(crate) fn set_flag(&mut self, key: &str, value: bool) {
        self.touched = true;
        self.confirmed.insert(key.to_owned());
        self.flags.insert(key.to_owned(), value);
    }

    /// Store the trimmed text for `key`; blank text removes it. Only a
    /// change counts as the person acting: committing blank over nothing
    /// is what moving through an empty field does.
    pub(crate) fn commit_text(&mut self, key: &str, text: &str) {
        let text = text.trim();
        let changed = if text.is_empty() {
            self.texts.remove(key).is_some()
        } else {
            self.texts
                .insert(key.to_owned(), text.to_owned())
                .as_deref()
                != Some(text)
        };
        self.touched |= changed;
    }

    /// The person acted on the question outside the form, by typing its
    /// answer in the bound composer.
    pub(crate) fn touch(&mut self) {
        self.touched = true;
    }

    /// The person answered primary field `primary_key`, or accepted what it
    /// holds, without changing it.
    pub(crate) fn confirm(&mut self, primary_key: &str) {
        self.touched = true;
        self.confirmed.insert(primary_key.to_owned());
    }

    /// Whether primary field `primary_key` needs nothing more from the
    /// person: answered by them, or left blank on purpose.
    pub(crate) fn is_settled_by_person(
        &self,
        request: &QuestionRequest,
        primary_key: &str,
    ) -> bool {
        self.left_blank.contains(primary_key)
            || (self.confirmed.contains(primary_key) && self.is_answered(request, primary_key))
    }

    pub(crate) fn text(&self, key: &str) -> Option<&str> {
        self.texts.get(key).map(String::as_str)
    }

    pub(crate) fn is_picked(&self, key: &str, value: &str) -> bool {
        self.picks
            .get(key)
            .is_some_and(|picks| picks.iter().any(|p| p == value))
    }

    pub(crate) fn flag(&self, key: &str) -> Option<bool> {
        self.flags.get(key).copied()
    }

    /// Whether the person has acted on the question, as opposed to defaults.
    /// Never reset: the answer timeout stops for good once this is set.
    pub(crate) fn touched(&self) -> bool {
        self.touched
    }

    /// Leave primary field `primary_key` blank: drop whatever it holds,
    /// its "Other" text included, so it goes as skipped.
    pub(crate) fn leave_blank(&mut self, request: &QuestionRequest, primary_key: &str) {
        self.touched = true;
        self.picks.remove(primary_key);
        self.flags.remove(primary_key);
        self.texts.remove(primary_key);
        if let Some(companion) = companion_key(request, primary_key) {
            self.texts.remove(companion);
        }
        self.left_blank.insert(primary_key.to_owned());
    }

    /// Whether `primary_key` has a pick, flag, own text, or companion text.
    pub(crate) fn is_answered(&self, request: &QuestionRequest, primary_key: &str) -> bool {
        self.picks.get(primary_key).is_some_and(|p| !p.is_empty())
            || self.flags.contains_key(primary_key)
            || self.texts.contains_key(primary_key)
            || self.companion_typed(request, primary_key)
    }

    /// The filled form, or `None` while a required field is unanswered. A
    /// select counts as answered by its own "Other" box.
    pub(crate) fn answer(
        &self,
        request: &QuestionRequest,
    ) -> Option<BTreeMap<String, QuestionValue>> {
        let mut values = BTreeMap::new();
        for field in &request.fields {
            let value = match &field.kind {
                QuestionFieldKind::SingleSelect(_) => self
                    .picks
                    .get(&field.key)
                    .and_then(|p| p.first())
                    .map(|v| QuestionValue::Text(v.clone())),
                QuestionFieldKind::MultiSelect(_) => self
                    .picks
                    .get(&field.key)
                    .filter(|p| !p.is_empty())
                    .map(|p| QuestionValue::Choices(p.clone())),
                QuestionFieldKind::Boolean => {
                    self.flags.get(&field.key).map(|b| QuestionValue::Bool(*b))
                }
                QuestionFieldKind::Text => self
                    .text(&field.key)
                    .map(|t| QuestionValue::Text(t.to_owned())),
            };
            match value {
                Some(value) => {
                    values.insert(field.key.clone(), value);
                }
                None if field.required && !self.companion_typed(request, &field.key) => {
                    return None;
                }
                None => {}
            }
        }
        (!values.is_empty()).then_some(values)
    }

    fn companion_typed(&self, request: &QuestionRequest, key: &str) -> bool {
        request
            .fields
            .iter()
            .filter(|f| f.companion_of.as_deref() == Some(key))
            .any(|f| self.texts.contains_key(&f.key))
    }
}

/// The fields shown as questions of their own: everything that is not an
/// "Other" box under a select.
pub(crate) fn primary_fields(request: &QuestionRequest) -> impl Iterator<Item = &QuestionField> {
    request.fields.iter().filter(|f| f.companion_of.is_none())
}

/// How a question headed by primary field `field` reads: the field's
/// title, or the request's message when it has none.
pub(crate) fn field_header<'a>(
    request: &'a QuestionRequest,
    field: Option<&'a QuestionField>,
) -> &'a str {
    field
        .and_then(|f| f.title.as_deref())
        .filter(|title| !title.is_empty())
        .unwrap_or(&request.message)
}

/// The key of the free-text field supplementing `primary_key`, if any.
pub(crate) fn companion_key<'a>(
    request: &'a QuestionRequest,
    primary_key: &str,
) -> Option<&'a str> {
    request
        .fields
        .iter()
        .find(|f| f.companion_of.as_deref() == Some(primary_key))
        .map(|f| f.key.as_str())
}

/// One select and nothing but its own "Other" box: picking an option is
/// the whole answer, so a pick sends it.
pub(crate) fn picks_answer(request: &QuestionRequest) -> bool {
    let mut selects = request
        .fields
        .iter()
        .filter(|f| matches!(f.kind, QuestionFieldKind::SingleSelect(_)));
    let Some(select) = selects.next() else {
        return false;
    };
    selects.next().is_none()
        && request
            .fields
            .iter()
            .all(|f| f.key == select.key || f.companion_of.as_deref() == Some(select.key.as_str()))
}

#[cfg(test)]
pub(super) mod tests;
