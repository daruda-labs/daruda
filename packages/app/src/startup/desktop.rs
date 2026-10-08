//! Desktop ownership, GUI initialization, and first-window lifecycle.

use crate::{
    bind_keys, bootstrap, globals, platform, surface, watchers_lifecycle, window_startup, windows,
};
use gpui::App;

pub(super) fn run() {
    let desktop = platform::desktop_instance::start(
        &daruda_store::persistence::default_data_dir(),
        std::env::args_os().skip(1),
    );
    let instance = match desktop {
        Ok(platform::desktop_instance::Launch::Primary(instance)) => instance,
        Ok(platform::desktop_instance::Launch::Forwarded) => return,
        Err(error) => {
            super::failure::exit("desktop.startup", "Desktop startup failed", error.as_ref());
        }
    };

    let workspace_store = match daruda_store::project::WorkspaceStore::open_current() {
        Ok(store) => store,
        Err(error) => {
            super::failure::exit(
                "storage.workspace",
                "Workspace storage preparation failed",
                &error,
            );
        }
    };
    let app = bootstrap::new_application();
    app.run(move |cx: &mut App| {
        crate::workspace_storage::install(workspace_store, cx);
        globals::init_all(cx);
        platform::desktop_instance::install(instance, cx);
        platform::notifications::install(cx);
        platform::desktop::install(cx);
        bind_keys::register_static_bindings(cx);

        // SettingsStore is the single source of truth — read the
        // user layer directly instead of re-reading disk.
        let config = crate::settings_store::SettingsStore::global(cx).user_arc();
        surface::action_map::apply_keybinding_overrides(&config.keybindings.bindings, cx);

        let window_opts = windows::build_window_options(&config);

        bind_keys::register_global_actions(cx, config.clone());
        crate::register_recent_actions(cx, config.clone());

        // Canonical startup install of the app-wide managed-accounts Global
        // (the single source of truth every window mirrors). Window
        // constructors also install it idempotently, so this is belt-and-
        // suspenders for the first window plus the authoritative install
        // point for a window with no Workspace (Settings).
        crate::workspace::accounts_global::install_if_absent(
            cx,
            daruda_store::accounts::load_accounts().unwrap_or_default(),
        );

        #[cfg(feature = "replay")]
        let replay_loaded = super::inspection::load_replay();

        window_startup::open_first_window(config, window_opts, cx);

        #[cfg(feature = "replay")]
        if let Some(loaded) = replay_loaded {
            crate::replay::schedule_seed(loaded, cx);
        }

        watchers_lifecycle::spawn_all(cx);
        crate::telegram::global::install(cx);
        crate::remote_channel::global::install(cx);

        super::inspection::after_startup(cx);
    });
}
