//! Data-only artwork contracts; activity state belongs to the session model.
use serde::Deserialize;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum State {
    Disconnected,
    Connecting,
    Working,
    ExecutingTool,
    NeedsAttention,
    Idle,
    Completed,
    Errored,
    Failed,
}

impl State {
    pub(crate) fn allows_animation(self) -> bool {
        matches!(
            self,
            Self::Connecting | Self::Working | Self::ExecutingTool | Self::NeedsAttention
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Base {
    Dots,
    #[default]
    Icons,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
    pub version: u32,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub extends: Base,
    pub states: BTreeMap<State, StateSpec>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StateSpec {
    #[serde(default)]
    pub frames: Vec<String>,
    #[serde(default = "one")]
    pub ticks_per_frame: u64,
    pub colors: Option<Colors>,
}
fn one() -> u64 {
    1
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Colors {
    pub dark: String,
    pub light: String,
}

#[derive(Clone, Debug)]
pub(crate) struct Frame {
    pub key: String,
    pub svg: Arc<[u8]>,
}

#[derive(Clone, Debug)]
pub(crate) struct Artwork {
    pub frames: Vec<Frame>,
    pub ticks_per_frame: u64,
    pub colors: Option<(u32, u32)>,
}
impl Artwork {
    pub(crate) fn frame(&self, tick: u64) -> Option<&Frame> {
        if self.frames.is_empty() {
            return None;
        }
        self.frames
            .get(((tick / self.ticks_per_frame) % self.frames.len() as u64) as usize)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Theme {
    pub id: String,
    pub name: String,
    pub base: Base,
    pub states: BTreeMap<State, Artwork>,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct Catalog {
    pub themes: BTreeMap<String, Theme>,
}

impl Catalog {
    pub(crate) fn get(&self, id: &daruda_config::StatusIconTheme) -> Option<&Theme> {
        self.themes.get(id.as_str())
    }
    pub(crate) fn base(&self, id: &daruda_config::StatusIconTheme) -> Base {
        self.get(id).map(|t| t.base).unwrap_or_else(|| {
            if *id == daruda_config::StatusIconTheme::Dots {
                Base::Dots
            } else {
                Base::Icons
            }
        })
    }
    pub(crate) fn animates(&self, id: &daruda_config::StatusIconTheme, state: State) -> bool {
        if !state.allows_animation() {
            return false;
        }
        if let Some(art) = self.get(id).and_then(|t| t.states.get(&state))
            && !art.frames.is_empty()
        {
            return art.frames.len() > 1;
        }
        state != State::NeedsAttention || self.base(id) == Base::Dots
    }
    pub(crate) fn choices(&self) -> Vec<(String, String)> {
        self.themes
            .values()
            .map(|t| (t.id.clone(), t.name.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests;
