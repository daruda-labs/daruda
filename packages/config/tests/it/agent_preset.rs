//! Public API tests for agent preset.

use daruda_config::AgentLaunch;
use daruda_config::agent::preset::{PresetLaunchability, presets};

/// `definition()` is what decides whether a preset can become a catalog row
/// at all, so the Add button's behaviour rides on this being exactly
/// "Some for Runnable, None for NeedsManualInstall".
///
/// `install_url` is checked here rather than in the generator because it
/// comes from the registry's own `website`/`repository` — upstream data we
/// hand to `open_url` without reading it first.
#[test]
fn definition_follows_launchability() {
    for preset in presets() {
        match preset.launchability {
            PresetLaunchability::Runnable { command } => {
                let definition = preset
                    .definition()
                    .unwrap_or_else(|| panic!("{} is runnable so it has a definition", preset.id));
                assert_eq!(definition.launch, AgentLaunch::Raw(command.to_string()));
            }
            PresetLaunchability::NeedsManualInstall { install_url } => {
                assert!(
                    install_url.starts_with("https://"),
                    "{} install_url is not https: {install_url}",
                    preset.id
                );
                assert_eq!(preset.definition(), None, "{} cannot launch", preset.id);
            }
        }
    }
}
