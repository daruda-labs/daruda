//! Provider-native optional fields; unchanged values retain their original shape.
use crate::workspace::right_dock::form_layout;
use crate::{
    surface::strings,
    ui::{InputState, theme},
};
use gpui::{App, AppContext, Entity, IntoElement, SharedString, Window, div, prelude::*, px};
use serde_json::Value;
use std::collections::BTreeMap;

const KEYS: [&str; 5] = [
    "startup_timeout_sec",
    "tool_timeout_sec",
    "enabled_tools",
    "disabled_tools",
    "env_http_headers",
];

pub(super) struct CodexOptions {
    pub inputs: Vec<Entity<InputState>>,
    original: Vec<String>,
}
impl CodexOptions {
    pub fn new(window: &mut Window, cx: &mut App) -> Self {
        Self {
            inputs: KEYS
                .iter()
                .map(|_| cx.new(|cx| InputState::new(window, cx).auto_grow(1, 4)))
                .collect(),
            original: vec![String::new(); KEYS.len()],
        }
    }
    pub fn seed(&mut self, extra: &BTreeMap<String, Value>, window: &mut Window, cx: &mut App) {
        for (index, key) in KEYS.iter().enumerate() {
            let text = match extra.get(*key) {
                Some(Value::Number(value)) => value.to_string(),
                Some(Value::Array(values)) => values
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join("\n"),
                Some(Value::Object(values)) => values
                    .iter()
                    .filter_map(|(k, v)| v.as_str().map(|v| format!("{k}={v}")))
                    .collect::<Vec<_>>()
                    .join("\n"),
                _ => String::new(),
            };
            self.inputs[index].update(cx, |input, cx| input.set_value(text.clone(), window, cx));
            self.original[index] = text;
        }
    }
    pub fn values(&self, cx: &App) -> Vec<String> {
        self.inputs
            .iter()
            .map(|i| i.read(cx).value().to_string())
            .collect()
    }
    pub fn apply(
        &self,
        extra: &mut BTreeMap<String, Value>,
        cx: &App,
    ) -> Result<(), (usize, SharedString)> {
        for (index, text) in self.values(cx).iter().enumerate() {
            if *text == self.original[index] {
                continue;
            }
            let key = KEYS[index];
            if text.trim().is_empty() {
                extra.remove(key);
                continue;
            }
            let value = if index < 2 {
                let value = text
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite() && *v >= 0.0)
                    .ok_or_else(|| {
                        (
                            index,
                            SharedString::from(strings::library::options_invalid()),
                        )
                    })?;
                if index == 0 {
                    extra.remove("startup_timeout_ms");
                }
                serde_json::json!(value)
            } else if index < 4 {
                Value::Array(
                    text.lines()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(|s| Value::String(s.into()))
                        .collect(),
                )
            } else {
                let pairs =
                    super::modal_shared::parse_pairs(text, true).map_err(|error| (index, error))?;
                if pairs
                    .values()
                    .any(|v| !crate::agent::mcp::import::valid_variable(v))
                {
                    return Err((index, strings::library::options_invalid().into()));
                }
                serde_json::json!(pairs)
            };
            extra.insert(key.into(), value);
        }
        Ok(())
    }
    pub fn render(&self, cx: &App) -> impl IntoElement {
        let labels = [
            strings::library::startup_timeout(),
            strings::library::tool_timeout(),
            strings::library::enabled_tools(),
            strings::library::disabled_tools(),
            strings::library::env_headers(),
        ];
        let mut body = div()
            .flex()
            .flex_col()
            .gap(px(theme::FORM_MODAL_FIELD_SPACING));
        for (index, label) in labels.into_iter().enumerate() {
            body = body.child(form_layout::field(
                super::modal_shared::field_label(label, theme::current(cx)),
                crate::ui::form_input(&self.inputs[index], cx, 13 + index as isize),
            ));
        }
        body
    }
}
