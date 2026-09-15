//! Select and retain one concrete Node/npm pair for JS adapter preparation.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{
    NodeError, NodeProgress, NodeRuntime, managed_cache_valid_with_context, managed_node_dir,
    node_platform,
};
use crate::preparation::process::output;
use crate::preparation::{PreparationContext, PreparationError, PreparationKind};

const IDENTITY: &str =
    "JSON.stringify({path:process.execPath,version:process.versions.node,arch:process.arch})";
const MIN_ADAPTER_NODE: semver::Version = semver::Version::new(22, 0, 0);

pub(crate) struct ResolvedNode {
    pub node: PathBuf,
    pub version: String,
    pub npm: PathBuf,
}

pub(crate) fn resolve(
    root: &Path,
    env: &BTreeMap<String, String>,
    strip_env: &[String],
    progress: &mut dyn FnMut(NodeProgress),
    context: &PreparationContext<'_>,
) -> Result<ResolvedNode, PreparationError> {
    context.check()?;
    if let Some(node) = system_node(env, strip_env) {
        match probe(&node, env, strip_env, context) {
            Ok(resolved) => {
                progress(NodeProgress::UsingSystemNode);
                return Ok(resolved);
            }
            Err(error) if error.kind == PreparationKind::Canceled => return Err(error),
            Err(_) => {} // An unusable system toolchain is replaced by the managed one.
        }
    }
    let (os, arch) = node_platform().map_err(configuration)?;
    let directory = managed_node_dir(root, os, arch);
    progress(NodeProgress::CheckingCache);
    if !managed_cache_valid_with_context(&directory, context).map_err(runtime_error)? {
        context.check()?;
        let result = super::install_managed_with_context(root, os, arch, progress, context);
        context.check()?;
        let runtime = result.map_err(runtime_error)?;
        debug_assert!(matches!(runtime, NodeRuntime::Managed { .. }));
    }
    context.check()?;
    probe(&directory.join("bin/node"), env, strip_env, context)
}

fn runtime_error(error: NodeError) -> PreparationError {
    let kind = match &error {
        NodeError::Canceled => PreparationKind::Canceled,
        NodeError::Download(_) => PreparationKind::Network,
        NodeError::Checksum { .. } => PreparationKind::Integrity,
        NodeError::Extract(_) => PreparationKind::Io,
        NodeError::UnsupportedPlatform(_) => PreparationKind::Configuration,
    };
    PreparationError::new(kind, error.to_string())
}

fn system_node(env: &BTreeMap<String, String>, strip_env: &[String]) -> Option<PathBuf> {
    if strip_env.iter().any(|name| name == "PATH") {
        return None;
    }
    let path = env
        .get("PATH")
        .map(std::ffi::OsString::from)
        .or_else(|| std::env::var_os("PATH"));
    which::which_in("node", path, std::env::current_dir().ok()?).ok()
}

fn configuration(error: impl std::fmt::Display) -> PreparationError {
    PreparationError::new(PreparationKind::Configuration, error.to_string())
}

fn probe(
    node: &Path,
    env: &BTreeMap<String, String>,
    strip_env: &[String],
    context: &PreparationContext<'_>,
) -> Result<ResolvedNode, PreparationError> {
    let mut command = Command::new(node);
    command.args(["-p", IDENTITY]).envs(env);
    for name in strip_env {
        command.env_remove(name);
    }
    let identity: serde_json::Value =
        serde_json::from_str(&output(&mut command, context)?).map_err(configuration)?;
    from_identity(identity)
}

fn from_identity(identity: serde_json::Value) -> Result<ResolvedNode, PreparationError> {
    let node = PathBuf::from(
        identity["path"]
            .as_str()
            .ok_or_else(|| configuration("missing Node path"))?,
    );
    let version = identity["version"]
        .as_str()
        .ok_or_else(|| configuration("missing Node version"))?;
    let parsed = semver::Version::parse(version).map_err(configuration)?;
    let (_, arch) = node_platform().map_err(configuration)?;
    if parsed < MIN_ADAPTER_NODE || identity["arch"] != arch {
        return Err(configuration(
            "Node version or architecture does not support this adapter",
        ));
    }
    let npm = npm_entry(
        node.parent()
            .ok_or_else(|| configuration("Node path has no parent"))?,
    )?;
    Ok(ResolvedNode {
        node,
        version: version.to_owned(),
        npm,
    })
}

fn npm_entry(bin: &Path) -> Result<PathBuf, PreparationError> {
    // Do not pair this Node with an unrelated npm installation found elsewhere.
    let npm = bin.join("npm").canonicalize().map_err(configuration)?;
    if npm.file_name().is_none_or(|name| name != "npm-cli.js") {
        return Err(configuration(
            "npm-cli.js not found alongside the selected Node runtime",
        ));
    }
    Ok(npm)
}

#[cfg(test)]
mod tests;
