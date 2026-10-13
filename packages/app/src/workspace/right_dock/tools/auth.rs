//! A small auth editor that preserves custom headers until explicitly changed.
use crate::surface::strings;
use crate::ui::{Disableable, InputState, Selectable, button, button_group, theme};
use crate::workspace::right_dock::form_layout;
use gpui::{App, Context, Entity, SharedString, Window, div, prelude::*, px};
use std::collections::BTreeMap;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    None,
    Variable,
    Token,
    Custom,
}

pub(super) struct Auth {
    mode: Mode,
    pub input: Entity<InputState>,
}

impl Auth {
    pub fn new(window: &mut Window, cx: &mut App) -> Self {
        Self {
            mode: Mode::None,
            input: cx.new(|cx| InputState::new(window, cx).masked(true)),
        }
    }
    pub fn seed(&mut self, headers: &BTreeMap<String, String>, window: &mut Window, cx: &mut App) {
        let authorization = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
            .map(|(_, v)| v.as_str());
        let (mode, value) = match authorization {
            None => (Mode::None, ""),
            Some(v)
                if v.starts_with("Bearer ${")
                    && v.ends_with('}')
                    && crate::agent::mcp::import::valid_variable(&v[9..v.len() - 1]) =>
            {
                (Mode::Variable, &v[9..v.len() - 1])
            }
            Some(v) if v.starts_with("Bearer ") && !v.contains("${") => (Mode::Token, &v[7..]),
            Some(_) => (Mode::Custom, ""),
        };
        self.mode = mode;
        self.input.update(cx, |input, cx| {
            input.set_value(value, window, cx);
            input.set_masked(mode == Mode::Token, window, cx);
        });
    }
    pub fn seed_codex(
        &mut self,
        draft: &crate::agent::mcp::McpServerDraft,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.seed(&draft.headers, window, cx);
        if let Some(value) = draft
            .extra
            .get("bearer_token_env_var")
            .and_then(serde_json::Value::as_str)
        {
            self.mode = Mode::Variable;
            self.input.update(cx, |input, cx| {
                input.set_value(value, window, cx);
                input.set_masked(false, window, cx);
            });
        } else if draft.extra.contains_key("env_http_headers")
            || draft.headers.values().any(|v| v.contains("${"))
        {
            self.mode = Mode::Custom;
        }
    }
    pub fn apply_codex(
        &self,
        headers: &mut BTreeMap<String, String>,
        extra: &mut BTreeMap<String, serde_json::Value>,
        cx: &App,
    ) -> Result<(), SharedString> {
        if self.mode == Mode::Custom {
            return Ok(());
        }
        extra.remove("bearer_token_env_var");
        if let Some(map) = extra
            .get_mut("env_http_headers")
            .and_then(serde_json::Value::as_object_mut)
        {
            map.retain(|key, _| !key.eq_ignore_ascii_case("authorization"));
        }
        if self.mode == Mode::Variable {
            let value = self.input.read(cx).value().to_string();
            if !crate::agent::mcp::import::valid_variable(&value) {
                return Err(strings::mcp::auth_invalid().into());
            }
            headers.retain(|key, _| !key.eq_ignore_ascii_case("authorization"));
            extra.insert("bearer_token_env_var".into(), value.into());
            Ok(())
        } else {
            self.apply(headers, cx)
        }
    }
    pub fn values(&self, cx: &App) -> Vec<String> {
        vec![
            (self.mode as u8).to_string(),
            self.input.read(cx).value().to_string(),
        ]
    }
    pub fn apply(
        &self,
        headers: &mut BTreeMap<String, String>,
        cx: &App,
    ) -> Result<(), SharedString> {
        let value = self.input.read(cx).value().to_string();
        if self.mode == Mode::Custom {
            return Ok(());
        }
        let replacement = match self.mode {
            Mode::Variable if crate::agent::mcp::import::valid_variable(&value) => {
                Some(format!("Bearer ${{{value}}}"))
            }
            Mode::Token
                if !value.is_empty() && !value.contains(['\n', '\r']) && !value.contains("${") =>
            {
                Some(format!("Bearer {value}"))
            }
            Mode::None => None,
            _ => return Err(strings::mcp::auth_invalid().into()),
        };
        headers.retain(|k, _| !k.eq_ignore_ascii_case("authorization"));
        if let Some(value) = replacement {
            headers.insert("Authorization".into(), value);
        }
        Ok(())
    }
    pub fn render<E: 'static>(
        &self,
        busy: bool,
        get: fn(&mut E) -> &mut Auth,
        cx: &mut Context<E>,
    ) -> gpui::AnyElement {
        let modes = [
            (Mode::None, strings::mcp::auth_none()),
            (Mode::Variable, strings::mcp::auth_variable()),
            (Mode::Token, strings::mcp::auth_token()),
            (Mode::Custom, strings::mcp::auth_custom()),
        ];
        let mode_values = [Mode::None, Mode::Variable, Mode::Token, Mode::Custom];
        let group = button_group("auth-mode", cx)
            .children(modes.into_iter().enumerate().map(|(ix, (mode, label))| {
                button(("auth", ix), label)
                    .selected(self.mode == mode)
                    .disabled(busy)
                    .tab_stop(true)
                    .tab_index(5)
            }))
            .on_click(cx.listener(move |this, ixs: &Vec<usize>, window, cx| {
                if busy {
                    return;
                }
                if let Some(ix) = ixs.first() {
                    let auth = get(this);
                    let next = mode_values[*ix];
                    if next != auth.mode {
                        auth.input
                            .update(cx, |input, cx| input.set_value("", window, cx));
                    }
                    auth.mode = next;
                    auth.input.update(cx, |input, cx| {
                        input.set_masked(auth.mode == Mode::Token, window, cx)
                    });
                    cx.notify();
                }
            }));
        div()
            .flex()
            .flex_col()
            .gap(px(theme::FORM_MODAL_FIELD_SPACING))
            .child(form_layout::field(
                super::modal_shared::field_label(strings::mcp::field_auth(), theme::current(cx)),
                group,
            ))
            .when(matches!(self.mode, Mode::Token | Mode::Variable), |body| {
                body.child(crate::ui::input(&self.input, cx, 6))
            })
            .when(self.mode == Mode::Custom, |body| {
                body.child(super::modal_shared::field_label(
                    strings::mcp::auth_custom_hint(),
                    theme::current(cx),
                ))
            })
            .into_any_element()
    }
}
