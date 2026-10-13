use super::*;
use daruda_config::BuiltinSection;
use gpui::{BorrowAppContext, Entity, TestAppContext, WindowHandle};

mod agent_page;
mod confirmation;
mod failure_reporting;
mod focus;
mod reset;
mod scrollbar;
mod terminal_page;
mod workspace_page;

use daruda_transcript::display_filter::FilterFacet;
use daruda_transcript::editor::range::RangeLevel;
use daruda_transcript::fold_mode::{BlockRule, FoldBlock, FoldMode, FoldPreset, TurnPosition};
use daruda_ui::test_support::init_gpui_component;

mod agent_cards;
mod agent_catalog;
mod agent_environment;
mod agent_transcript;
mod drafts_and_conflicts;
mod session_hosts;
mod toggles;

/// Construct a Settings window wrapped in `daruda_ui::Root` —
/// matches the production windowing path so the vendored `Input`'s
/// `TextElement::paint` can resolve `Root::read` without panicking.
/// The workspace draws the dialog layer over Settings, so a test window
/// has to as well for a confirm dialog's OK to be reachable.
struct DialogHost {
    view: Entity<SettingsView>,
}

impl gpui::Render for DialogHost {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{ParentElement as _, Styled as _};
        gpui::div()
            .size_full()
            .child(self.view.clone())
            .children(daruda_ui::Root::render_dialog_layer(window, cx))
    }
}

/// A host with no accounts in use and an empty credential store; with
/// `refuse_keychain`, a store that rejects every write.
#[derive(Default)]
struct FakeServices {
    refuse_keychain: bool,
}

impl SettingsServices for FakeServices {
    fn account_usage(&self, _: daruda_store::accounts::AccountId, _: &mut gpui::App) -> usize {
        0
    }

    fn telegram_token_stored(&self) -> bool {
        false
    }

    fn store_telegram_token(&self, _: &str) -> std::io::Result<()> {
        self.keychain()
    }

    fn forget_telegram_token(&self) -> std::io::Result<()> {
        self.keychain()
    }

    fn telegram_bot_held_elsewhere(&self, _: &gpui::App) -> bool {
        false
    }

    fn new_telegram_pair_code(&self, _: &mut gpui::App) -> String {
        "PAIRCODE".to_string()
    }
}

impl FakeServices {
    fn keychain(&self) -> std::io::Result<()> {
        if self.refuse_keychain {
            Err(std::io::Error::other("keychain said no"))
        } else {
            Ok(())
        }
    }
}

/// The host's channel settings, which these tests never open.
struct NoIntegrations;

impl gpui::Render for NoIntegrations {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        gpui::Empty
    }
}

/// A view on `section` with nothing shared yet, over `services`.
fn new_test_view(
    section: BuiltinSection,
    services: FakeServices,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<SettingsView>,
) -> SettingsView {
    let remote_integrations = cx.new(|_| NoIntegrations).into();
    SettingsView::new_with_section(
        section,
        SharedSnapshot::default(),
        std::rc::Rc::new(services),
        remote_integrations,
        window,
        cx,
    )
}

/// Press Enter in the window, which the open dialog takes as OK.
fn confirm_dialog(cx: &mut TestAppContext, wh: WindowHandle<daruda_ui::Root>) {
    let mut vcx = gpui::VisualTestContext::from_window(wh.into(), cx);
    vcx.run_until_parked();
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
}

fn build_window(cx: &mut TestAppContext) -> (WindowHandle<daruda_ui::Root>, Entity<SettingsView>) {
    build_window_with_config(cx, daruda_config::Config::default())
}

fn build_window_with_config(
    cx: &mut TestAppContext,
    config: daruda_config::Config,
) -> (WindowHandle<daruda_ui::Root>, Entity<SettingsView>) {
    build_window_with(cx, config, FakeServices::default())
}

fn build_window_with(
    cx: &mut TestAppContext,
    config: daruda_config::Config,
    services: FakeServices,
) -> (WindowHandle<daruda_ui::Root>, Entity<SettingsView>) {
    init_gpui_component(cx);
    cx.update(|cx| {
        crate::store::SettingsStore::init(cx);
        cx.update_global::<crate::store::SettingsStore, _>(|store, _| {
            store.set_user_for_testing(config);
        });
    });
    let settings_for_root = std::cell::RefCell::new(None);
    let wh = cx.add_window(|window, cx| {
        let settings = cx.new(|cx| new_test_view(BuiltinSection::default(), services, window, cx));
        *settings_for_root.borrow_mut() = Some(settings.clone());
        let host = cx.new(|_| DialogHost { view: settings });
        daruda_ui::Root::new(host, window, cx)
    });
    let entity = settings_for_root.borrow().clone().unwrap();
    (wh, entity)
}

/// Test-only — write `value` into one of the settings's inputs
/// through the real `InputState::set_value` pipeline. Tests don't hold
/// a live `&mut Window`, so re-enter via the window handle.
fn set_input(
    wh: &WindowHandle<daruda_ui::Root>,
    win: &Entity<SettingsView>,
    cx: &mut TestAppContext,
    field: fn(&SettingsView) -> Entity<InputState>,
    value: &str,
) {
    let state = win.read_with(cx, |w, _| field(w));
    wh.update(cx, |_root, window, cx| {
        state.update(cx, |i, cx_state| {
            i.set_value(value.to_string(), window, cx_state)
        });
    })
    .expect("settings window should still be open during the test");
}

/// Test-only — switch a built-in preset on from the catalog's available list.
fn enable_agent_preset(
    wh: &WindowHandle<daruda_ui::Root>,
    win: &Entity<SettingsView>,
    cx: &mut TestAppContext,
    preset_id: &str,
) {
    let win = win.clone();
    wh.update(cx, |_root, window, cx| {
        win.update(cx, |w, cx| w.enable_agent_preset(preset_id, window, cx));
    })
    .expect("settings window should still be open during the test");
}

/// Test-only — pick `value` in one catalog row's mode/model picker, including
/// the confirm event a real click emits (`set_selected_value` alone is silent).
fn confirm_agent_row_select(
    wh: &WindowHandle<daruda_ui::Root>,
    win: &Entity<SettingsView>,
    cx: &mut TestAppContext,
    index: usize,
    field: fn(&AgentCatalogRow) -> Entity<SelectState>,
    value: &str,
) {
    let state = win.read_with(cx, |w, _| field(w.agent_editable_row(index).unwrap()));
    let value = SharedString::from(value.to_owned());
    wh.update(cx, |_root, window, cx| {
        state.update(cx, |s, cx| {
            s.set_selected_value(&value, window, cx);
            cx.emit(daruda_ui::select::SelectEvent::Confirm(Some(value.clone())));
        });
    })
    .expect("settings window should still be open during the test");
}

/// Test-only — whether one catalog row's picker offers `value` at all.
/// `set_selected_value` clears the selection for a value the option list does
/// not carry, which is the only public read of what a `SelectState` holds.
fn agent_row_select_offers(
    wh: &WindowHandle<daruda_ui::Root>,
    win: &Entity<SettingsView>,
    cx: &mut TestAppContext,
    index: usize,
    field: fn(&AgentCatalogRow) -> Entity<SelectState>,
    value: &str,
) -> bool {
    let state = win.read_with(cx, |w, _| field(w.agent_editable_row(index).unwrap()));
    let value = SharedString::from(value.to_owned());
    wh.update(cx, |_root, window, cx| {
        state.update(cx, |s, cx| s.set_selected_value(&value, window, cx));
    })
    .expect("settings window should still be open during the test");
    win.read_with(cx, |_w, cx| state.read(cx).selected_value() == Some(&value))
}

/// Test-only — write `value` into one catalog row's input.
fn set_agent_row_input(
    wh: &WindowHandle<daruda_ui::Root>,
    win: &Entity<SettingsView>,
    cx: &mut TestAppContext,
    index: usize,
    field: fn(&AgentCatalogRow) -> Entity<InputState>,
    value: &str,
) {
    let state = win.read_with(cx, |w, _| field(w.agent_editable_row(index).unwrap()));
    wh.update(cx, |_root, window, cx| {
        state.update(cx, |i, cx_state| {
            i.set_value(value.to_owned(), window, cx_state)
        });
    })
    .expect("settings window should still be open during the test");
}

/// Emit the focus-loss the field sees when the user clicks away.
fn blur(wh: &WindowHandle<daruda_ui::Root>, state: &Entity<InputState>, cx: &mut TestAppContext) {
    wh.update(cx, |_root, _window, cx| {
        state.update(cx, |_, cx_state| cx_state.emit(InputEvent::Blur));
    })
    .expect("settings window should still be open during the test");
}

/// A one-row SSH catalog config — the seed for the tab-cycle tests.
fn one_ssh_row_config() -> daruda_config::Config {
    daruda_config::Config {
        session_hosts: vec![daruda_config::SessionHostEntry {
            id: daruda_store::project::SessionHostId::new(),
            label: "Build box".to_string(),
            kind: daruda_config::SessionHostKind::Ssh {
                target: "vm-work".to_string(),
            },
        }],
        ..daruda_config::Config::default()
    }
}
