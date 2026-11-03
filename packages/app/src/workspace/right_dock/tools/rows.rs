//! Stable native rows backed by the form's draft input. No shell execution.
use crate::ui::{Disableable as _, Selectable as _};
use crate::{
    surface::strings,
    ui::{FieldState, InputEvent, InputState, TextareaState, theme},
};
use gpui::{
    App, Context, Entity, Focusable, IntoElement, Render, Subscription, Window, div, prelude::*, px,
};

/// Rows a multi-line value grows to before it scrolls.
const MULTILINE_VALUE_MAX_ROWS: usize = 6;

#[derive(Clone, Copy)]
pub(super) enum Kind {
    Arguments,
    Pairs,
}
struct Row {
    id: usize,
    key: Entity<InputState>,
    /// A seeded value with a line break gets a multi-line field, since a
    /// one-line field drops line breaks and saving would rewrite the value.
    value: Option<FieldState>,
    /// Masking flattens text to one line, so a multi-line value is shown
    /// through this disabled one-line stand-in until it is revealed.
    masked_proxy: Option<Entity<InputState>>,
    masked: bool,
    _subscriptions: Vec<Subscription>,
}
pub(super) struct Rows {
    source: crate::ui::FieldState,
    kind: Kind,
    rows: Vec<Row>,
    next_id: usize,
    serialized: String,
    _source_subscription: Subscription,
    _disabled_subscription: Subscription,
}
impl Rows {
    pub fn new(
        source: impl Into<crate::ui::FieldState>,
        kind: Kind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let source = source.into();
        let (subscription, disabled_subscription) = match &source {
            crate::ui::FieldState::Line(state) => (
                Self::watch_source(state, window, cx),
                cx.observe(state, |this, _, cx| this.sync_disabled(cx)),
            ),
            crate::ui::FieldState::Text(state) => (
                Self::watch_source(state, window, cx),
                cx.observe(state, |this, _, cx| this.sync_disabled(cx)),
            ),
        };
        let mut this = Self {
            source,
            kind,
            rows: Vec::new(),
            next_id: 0,
            serialized: String::new(),
            _source_subscription: subscription,
            _disabled_subscription: disabled_subscription,
        };
        this.load(window, cx);
        this
    }
    fn watch_source<S: gpui::EventEmitter<InputEvent> + 'static>(
        state: &Entity<S>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(state, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::Change)
                && this.source.value(cx).as_str() != this.serialized
            {
                this.load(window, cx);
            }
        })
    }

    fn sync_disabled(&mut self, cx: &mut Context<Self>) {
        let disabled = self.source.is_disabled(cx);
        for row in &self.rows {
            row.key.update(cx, |i, cx| i.set_disabled(disabled, cx));
            if let Some(value) = &row.value {
                value.set_disabled(disabled, cx);
            }
        }
    }

    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.source.value(cx).to_string();
        self.serialized = text.clone();
        self.rows.clear();
        let values: Vec<(String, Option<String>)> = match self.kind {
            Kind::Arguments => super::modal_shared::split_args(&text)
                .unwrap_or_default()
                .into_iter()
                .map(|s| (s, None))
                .collect(),
            Kind::Pairs => text
                .lines()
                .filter(|s| !s.is_empty())
                .map(|s| {
                    let (k, v) = s.split_once('=').unwrap_or((s, ""));
                    (k.into(), Some(v.into()))
                })
                .collect(),
        };
        for (key, value) in values {
            self.push(&key, value.as_deref(), window, cx);
        }
        cx.notify();
    }
    fn push(
        &mut self,
        key: &str,
        value: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| {
            let input = InputState::new(window, cx).default_value(key);
            if matches!(self.kind, Kind::Pairs) {
                input.placeholder(strings::mcp::field_key())
            } else {
                input
            }
        });
        let masked_proxy = value
            .filter(|value| value.contains(['\n', '\r']))
            .map(|value| {
                cx.new(|cx| {
                    let mut proxy = InputState::new(window, cx)
                        .default_value(value)
                        .masked(true);
                    proxy.set_disabled(true, cx);
                    proxy
                })
            });
        let value = value.map(|value| -> FieldState {
            if masked_proxy.is_some() {
                cx.new(|cx| {
                    TextareaState::new(window, cx)
                        .auto_grow(1, MULTILINE_VALUE_MAX_ROWS)
                        .default_value(value)
                        .placeholder(strings::mcp::field_value())
                })
                .into()
            } else {
                cx.new(|cx| {
                    InputState::new(window, cx)
                        .default_value(value)
                        .placeholder(strings::mcp::field_value())
                        .masked(true)
                })
                .into()
            }
        });
        let subscriptions = std::iter::once(FieldState::from(input.clone()))
            .chain(value.iter().cloned())
            .map(|field| {
                field.subscribe_in(window, cx, |this, event, window, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.sync(window, cx);
                    }
                })
            })
            .collect();
        self.rows.push(Row {
            id: self.next_id,
            key: input,
            value,
            masked_proxy,
            masked: true,
            _subscriptions: subscriptions,
        });
        self.next_id += 1;
    }
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.serialized = match self.kind {
            Kind::Arguments => super::modal_shared::join_args(
                &self
                    .rows
                    .iter()
                    .map(|r| r.key.read(cx).value().to_string())
                    .collect::<Vec<_>>(),
            ),
            Kind::Pairs => self
                .rows
                .iter()
                .map(|r| {
                    format!(
                        "{}={}",
                        r.key.read(cx).value(),
                        r.value.as_ref().unwrap().value(cx)
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
        };
        self.source.set_value(self.serialized.clone(), window, cx);
        cx.notify();
    }
    pub fn seed_pairs(
        &mut self,
        pairs: &std::collections::BTreeMap<String, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.rows.clear();
        for (key, value) in pairs {
            self.push(key, Some(value), window, cx);
        }
        self.serialized = self.source.value(cx).to_string();
        cx.notify();
    }
    pub fn pairs(
        &self,
        headers: bool,
        cx: &App,
    ) -> Result<std::collections::BTreeMap<String, String>, gpui::SharedString> {
        let mut result = std::collections::BTreeMap::new();
        for (index, row) in self.rows.iter().enumerate() {
            let key = row.key.read(cx).value().trim().to_string();
            let value = row.value.as_ref().unwrap().value(cx).to_string();
            if super::modal_shared::parse_pairs(&format!("{key}="), headers).is_err()
                || key.is_empty()
                || (headers && value.contains(['\n', '\r']))
            {
                return Err(strings::mcp::pairs_invalid(index + 1).into());
            }
            if result.contains_key(&key)
                || (headers && result.keys().any(|k: &String| k.eq_ignore_ascii_case(&key)))
            {
                return Err(strings::mcp::pairs_duplicate(index + 1, &key).into());
            }
            result.insert(key, value);
        }
        Ok(result)
    }
    pub fn focus_handle(&self, headers: bool, cx: &App) -> Option<gpui::FocusHandle> {
        let mut seen = std::collections::HashSet::new();
        for row in &self.rows {
            let key = row.key.read(cx).value().trim().to_string();
            let comparison = if headers {
                key.to_ascii_lowercase()
            } else {
                key.clone()
            };
            if headers
                && let Some(value) = &row.value
                && value.value(cx).contains(['\n', '\r'])
            {
                return Some(value.focus_handle(cx));
            }
            if key.is_empty()
                || !seen.insert(comparison)
                || super::modal_shared::parse_pairs(&format!("{key}="), headers).is_err()
            {
                return Some(row.key.focus_handle(cx));
            }
        }
        self.rows.first().map(|row| row.key.focus_handle(cx))
    }
}
impl Render for Rows {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let disabled = self.source.is_disabled(cx);
        let mut body = div().flex().flex_col().gap(px(theme::GAP_LG));
        for row in &self.rows {
            let id = row.id;
            let mut line = div()
                .id(id)
                .flex()
                .items_center()
                .gap(px(theme::GAP_LG))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(crate::ui::form_input(&row.key, cx, 8)),
                );
            if let Some(value) = &row.value {
                let shown = match &row.masked_proxy {
                    Some(proxy) if row.masked => FieldState::from(proxy.clone()),
                    _ => value.clone(),
                };
                line = line.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(crate::ui::form_input(&shown, cx, 8)),
                );
                line = line.child(
                    crate::ui::button_icon(("reveal-row", id), crate::ui::icons::VISIBILITY, cx)
                        .tooltip(strings::mcp::reveal_values())
                        .selected(!row.masked)
                        .disabled(disabled)
                        .tab_stop(true)
                        .tab_index(8)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if let Some(row) = this.rows.iter_mut().find(|row| row.id == id) {
                                row.masked = !row.masked;
                                if let Some(FieldState::Line(value)) = &row.value {
                                    value.update(cx, |input, cx| {
                                        input.set_masked(row.masked, window, cx)
                                    });
                                }
                                cx.notify();
                            }
                        })),
                );
            }
            line = line.child(
                crate::ui::button_delete_glyph(("remove-row", id), cx)
                    .disabled(disabled)
                    .tooltip(strings::common::btn_delete())
                    .tab_stop(true)
                    .tab_index(8)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if this.source.is_disabled(cx) {
                            return;
                        }
                        let position = this.rows.iter().position(|r| r.id == id).unwrap_or(0);
                        this.rows.retain(|r| r.id != id);
                        this.sync(window, cx);
                        if let Some(row) = this.rows.get(position).or_else(|| this.rows.last()) {
                            window.focus(&row.key.focus_handle(cx), cx);
                        } else {
                            window.focus_next(cx);
                        }
                    })),
            );
            body = body.child(line);
        }
        body = body.child(
            crate::ui::button_with_icon(
                "add-row",
                strings::library::add_row(),
                crate::ui::icons::ADD,
            )
            .disabled(disabled)
            .border_1()
            .border_color(theme::current(cx).border)
            .tab_stop(true)
            .tab_index(8)
            .on_click(cx.listener(|this, _, window, cx| {
                if this.source.is_disabled(cx) {
                    return;
                }
                this.push(
                    "",
                    matches!(this.kind, Kind::Pairs).then_some(""),
                    window,
                    cx,
                );
                this.sync(window, cx);
                if let Some(row) = this.rows.last() {
                    window.focus(&row.key.focus_handle(cx), cx);
                }
            })),
        );
        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[gpui::test]
    async fn quick_entry_rows_preserve_multiline_values_and_empty_arguments(
        cx: &mut gpui::TestAppContext,
    ) {
        let (window, _) = crate::workspace::tests::build_workspace(cx);
        let mut vcx = gpui::VisualTestContext::from_window(window.into(), cx);
        vcx.update(|window, cx| {
            let source = cx.new(|cx| InputState::new(window, cx));
            let rows = cx.new(|cx| Rows::new(source.clone(), Kind::Pairs, window, cx));
            rows.update(cx, |rows, cx| {
                rows.seed_pairs(
                    &std::collections::BTreeMap::from([(
                        "MULTI".into(),
                        "first\nsecond=value".into(),
                    )]),
                    window,
                    cx,
                );
                let id = rows.rows[0].key.entity_id();
                rows.push("OTHER", Some("new"), window, cx);
                rows.sync(window, cx);
                assert_eq!(rows.rows[0].key.entity_id(), id);
                assert_eq!(
                    rows.pairs(false, cx).unwrap()["MULTI"],
                    "first\nsecond=value"
                );
                rows.push("OTHER", Some("duplicate"), window, cx);
                assert!(rows.pairs(false, cx).is_err());
            });
            let source = cx.new(|cx| InputState::new(window, cx));
            let rows = cx.new(|cx| Rows::new(source.clone(), Kind::Arguments, window, cx));
            rows.update(cx, |rows, cx| {
                for arg in ["", r"C:\path with space\file", "a\"b"] {
                    rows.push(arg, None, window, cx);
                }
                rows.sync(window, cx);
                assert_eq!(
                    super::super::modal_shared::split_args(&source.read(cx).value()).unwrap(),
                    vec!["", r"C:\path with space\file", "a\"b"]
                );
            });
        });
    }
}
