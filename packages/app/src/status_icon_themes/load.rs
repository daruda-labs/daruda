//! Load bounded, self-contained SVG packages off the UI thread.
use super::model::*;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

const MAX_MANIFEST_BYTES: u64 = 128 * 1024;
const MAX_SVG_BYTES: u64 = 64 * 1024;
const MAX_FRAMES: usize = 32;
const MAX_THEMES: usize = 128;
const MAX_HOLD_TICKS: u64 = 40;

#[derive(Debug)]
pub(crate) struct LoadError {
    path: PathBuf,
    detail: String,
}
impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.detail)
    }
}
impl std::error::Error for LoadError {}

fn error(path: &Path, detail: impl ToString) -> LoadError {
    LoadError {
        path: path.into(),
        detail: detail.to_string(),
    }
}
fn read(path: &Path, max: u64) -> Result<Vec<u8>, LoadError> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| error(path, e))?
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| error(path, e))?;
    if bytes.len() as u64 > max {
        return Err(error(path, "file exceeds size limit"));
    }
    Ok(bytes)
}
fn color(value: &str) -> Result<u32, &'static str> {
    if value.len() != 7 || !value.starts_with('#') {
        return Err("colors must be #rrggbb");
    }
    u32::from_str_radix(&value[1..], 16).map_err(|_| "colors must be #rrggbb")
}
fn load_svg(path: &Path) -> Result<Frame, LoadError> {
    let bytes = read(path, MAX_SVG_BYTES)?;
    let text = std::str::from_utf8(&bytes).map_err(|e| error(path, e))?;
    let doc = usvg::roxmltree::Document::parse(text).map_err(|e| error(path, e))?;
    if doc.root_element().tag_name().name() != "svg" {
        return Err(error(path, "expected SVG root"));
    }
    for node in doc.descendants().filter(|n| n.is_element()) {
        if !matches!(
            node.tag_name().name(),
            "svg"
                | "g"
                | "path"
                | "rect"
                | "circle"
                | "ellipse"
                | "line"
                | "polyline"
                | "polygon"
                | "title"
                | "desc"
        ) {
            return Err(error(path, "only self-contained SVG shapes are supported"));
        }
        for attr in node.attributes() {
            if attr.name() == "href"
                || attr.name() == "style"
                || attr.name().starts_with("on")
                || attr.value().contains("url(")
            {
                return Err(error(
                    path,
                    "external references, styles and event handlers are not supported",
                ));
            }
        }
    }
    let tree =
        usvg::Tree::from_data(&bytes, &usvg::Options::default()).map_err(|e| error(path, e))?;
    if tree.root().children().is_empty() {
        return Err(error(path, "SVG has no visible shapes"));
    }
    let mut hash = DefaultHasher::new();
    bytes.hash(&mut hash);
    Ok(Frame {
        key: format!("status-theme/{:016x}", hash.finish()),
        svg: Arc::from(bytes),
    })
}

pub(super) fn package(manifest_path: &Path, namespace: &str) -> Result<Theme, LoadError> {
    let manifest: Manifest = serde_json::from_slice(&read(manifest_path, MAX_MANIFEST_BYTES)?)
        .map_err(|e| error(manifest_path, e))?;
    if manifest.version != 1
        || manifest.id.is_empty()
        || manifest.id.len() > 80
        || !manifest
            .id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(error(
            manifest_path,
            "expected version 1 and an alphanumeric theme id",
        ));
    }
    if manifest.name.trim().is_empty() || manifest.name.chars().count() > 80 {
        return Err(error(
            manifest_path,
            "theme name must contain 1–80 characters",
        ));
    }
    let root = manifest_path.parent().unwrap_or(Path::new("."));
    let canonical = daruda_core::path::canonicalize(root).map_err(|e| error(root, e))?;
    let mut states = std::collections::BTreeMap::new();
    for (state, spec) in manifest.states {
        if spec.frames.len() > MAX_FRAMES
            || !(1..=MAX_HOLD_TICKS).contains(&spec.ticks_per_frame)
            || (!state.allows_animation() && spec.frames.len() > 1)
        {
            return Err(error(
                manifest_path,
                "invalid frame count or timing for this state",
            ));
        }
        if spec.frames.is_empty() && spec.colors.is_none() {
            return Err(error(
                manifest_path,
                "state override must specify frames or colors",
            ));
        }
        let mut frames = Vec::new();
        for relative in spec.frames {
            let relative = Path::new(&relative);
            if relative.is_absolute()
                || relative
                    .components()
                    .any(|c| !matches!(c, std::path::Component::Normal(_)))
                || relative.extension().and_then(|s| s.to_str()) != Some("svg")
            {
                return Err(error(
                    manifest_path,
                    "frame must be a relative SVG path inside the package",
                ));
            }
            let file = daruda_core::path::canonicalize(root.join(relative))
                .map_err(|e| error(manifest_path, e))?;
            if !daruda_core::path::is_within(&file, &canonical) {
                return Err(error(manifest_path, "frame escapes package directory"));
            }
            frames.push(load_svg(&file)?);
        }
        let colors = spec
            .colors
            .map(|c| {
                Ok::<_, LoadError>((
                    color(&c.dark).map_err(|e| error(manifest_path, e))?,
                    color(&c.light).map_err(|e| error(manifest_path, e))?,
                ))
            })
            .transpose()?;
        states.insert(
            state,
            Artwork {
                frames,
                ticks_per_frame: spec.ticks_per_frame,
                colors,
            },
        );
    }
    Ok(Theme {
        id: format!("{namespace}/{}", manifest.id),
        name: manifest.name,
        base: manifest.extends,
        states,
    })
}

/// Each provider contributes immediate child directories containing theme.json.
pub(crate) fn catalog(sources: &[(String, PathBuf)]) -> (Catalog, Vec<LoadError>) {
    let mut result = Catalog::default();
    let mut errors = Vec::new();
    for (namespace, root) in sources {
        let entries = match std::fs::read_dir(root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                errors.push(error(root, e));
                continue;
            }
        };
        let mut paths = Vec::new();
        for entry in entries {
            match entry {
                Ok(entry) => paths.push(entry.path().join("theme.json")),
                Err(e) => errors.push(error(root, e)),
            }
        }
        paths.sort();
        for path in paths.into_iter().filter(|p| p.is_file()) {
            if result.themes.len() >= MAX_THEMES {
                errors.push(error(&path, "theme catalog limit reached"));
                break;
            }
            match package(&path, namespace) {
                Ok(theme) if result.themes.contains_key(&theme.id) => {
                    errors.push(error(&path, "duplicate theme id"))
                }
                Ok(theme) => {
                    result.themes.insert(theme.id.clone(), theme);
                }
                Err(e) => errors.push(e),
            }
        }
    }
    (result, errors)
}

#[cfg(test)]
mod tests;
