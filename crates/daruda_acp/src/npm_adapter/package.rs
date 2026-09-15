//! Versioned installations, published only after their JS entry is validated.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, ensure};
use semver::Version;
use sha2::{Digest as _, Sha256};

use super::NpmAdapter;
use super::cache::{self, InstallationLease};
use super::runtime::NpmRuntime;
use crate::preparation::process::output;
use crate::preparation::{PreparationContext, PreparationError, PreparationKind};

const ADAPTER_DIRECTORY: &str = "adapters";
const MANIFEST: &str = "package.json";
const NODE_MODULES: &str = "node_modules";

pub(super) struct InstalledAdapter {
    pub entry: PathBuf,
    pub lease: InstallationLease,
}

pub(super) fn install(
    adapter: &NpmAdapter,
    runtime: &NpmRuntime,
    root: &Path,
    context: &PreparationContext<'_>,
) -> Result<InstalledAdapter, PreparationError> {
    let root = root.join(ADAPTER_DIRECTORY).join(digest(adapter.name));
    let _lock = cache::lock_package(&root, context)?;
    // The durable cache is the installed artifact, not another unbounded copy
    // of every historical dependency tarball in npm's content cache.
    let npm_cache = tempfile::Builder::new().prefix(".npm-").tempdir_in(&root)?;
    let (os, arch) = crate::node::node_platform().map_err(|error| {
        PreparationError::new(PreparationKind::Configuration, error.to_string())
    })?;
    let runtime_key = format!("{}:{os}:{arch}", runtime.version);
    let receipt = root.join(format!(
        "selector-{}",
        digest(&format!("{}:{runtime_key}", adapter.selector))
    ));
    let destination = |version: &str| {
        root.join(format!(
            "{}{}",
            cache::VERSION_PREFIX,
            digest(&format!("{version}:{runtime_key}"))
        ))
    };
    let prepared = prepare_or_cached(adapter, &receipt, &destination, context, || {
        let version = if let Ok(version) = Version::parse(&adapter.selector) {
            version.to_string()
        } else {
            let spec = format!("{}@{}", adapter.name, adapter.selector);
            let json = output(
                runtime
                    .npm(&root, npm_cache.path())?
                    .args(["view", &spec, "version", "--json"]),
                context,
            )?;
            resolved_version(&json)?
        };
        context.check()?;
        install_at(adapter, &version, &destination(&version), |staging| {
            let spec = format!("{}@{version}", adapter.name);
            output(
                runtime
                    .npm(&root, npm_cache.path())?
                    .args(["install", "--prefix"])
                    .arg(staging)
                    .args([
                        "--engine-strict",
                        "--include=optional",
                        "--save-exact",
                        "--json",
                        "--",
                        &spec,
                    ]),
                context,
            )?;
            context.check()?;
            Ok(())
        })?;
        Ok(version)
    })?;
    if let Err(error) = cache::sweep(&root, context) {
        context.check()?;
        context.notice(&format!("adapter cache cleanup deferred: {error}"));
    }
    Ok(prepared)
}

fn digest(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn prepare_or_cached(
    adapter: &NpmAdapter,
    receipt: &Path,
    destination: &impl Fn(&str) -> PathBuf,
    context: &PreparationContext<'_>,
    prepare: impl FnOnce() -> Result<String, PreparationError>,
) -> Result<InstalledAdapter, PreparationError> {
    let version = match prepare() {
        Ok(version) => {
            cache::write_receipt(receipt, &version)?;
            version
        }
        Err(error) if error.kind.allows_cached() => {
            context.check()?;
            let cached = fs::read_to_string(receipt)
                .ok()
                .filter(|version| Version::parse(version).is_ok())
                .filter(|version| validate_entry(adapter, version, &destination(version)).is_ok());
            let Some(version) = cached else {
                return Err(error);
            };
            context.notice(&format!(
                "using verified cached {}@{version}: {error}",
                adapter.name
            ));
            version
        }
        Err(error) => return Err(error),
    };
    context.check()?;
    let directory = destination(&version);
    let entry = validate_entry(adapter, &version, &directory).map_err(validation_error)?;
    let lease = InstallationLease::acquire(&directory)?;
    Ok(InstalledAdapter { entry, lease })
}

fn resolved_version(json: &str) -> anyhow::Result<String> {
    let value: serde_json::Value =
        serde_json::from_str(json).context("reading npm version response")?;
    let values = match &value {
        serde_json::Value::Array(versions) => versions.clone(),
        other => vec![other.clone()],
    };
    let version = values
        .iter()
        .filter_map(|v| v.as_str())
        .filter_map(|v| Version::parse(v).ok())
        .max()
        .context("npm returned no matching package version")?;
    Ok(version.to_string())
}

fn install_at(
    adapter: &NpmAdapter,
    version: &str,
    destination: &Path,
    install: impl FnOnce(&Path) -> anyhow::Result<()>,
) -> anyhow::Result<PathBuf> {
    if let Ok(entry) = validate_entry(adapter, version, destination) {
        return Ok(entry);
    }
    let root = destination
        .parent()
        .context("installation directory has no parent")?;
    let staging = tempfile::Builder::new()
        .prefix(".install-")
        .tempdir_in(root)?;
    install(staging.path()).with_context(|| format!("installing {}@{version}", adapter.name))?;
    validate_entry(adapter, version, staging.path()).map_err(validation_error)?;
    if destination.exists() {
        let _exclusive = cache::exclusive_lease(destination)?.context(
            "damaged adapter installation is still in use; close its sessions before retrying",
        )?;
        // Retain a damaged installation for diagnosis rather than deleting it.
        let quarantine = tempfile::Builder::new()
            .prefix(".invalid-")
            .tempdir_in(root)?
            .keep();
        fs::rename(destination, quarantine.join("package"))?;
    }
    fs::rename(staging.path(), destination).context("publishing adapter installation")?;
    validate_entry(adapter, version, destination)
        .map_err(validation_error)
        .map_err(Into::into)
}

fn validation_error(error: anyhow::Error) -> PreparationError {
    let invalid_entry = error.downcast_ref::<std::io::Error>().is_some_and(|error| {
        matches!(
            error.kind(),
            std::io::ErrorKind::NotFound | std::io::ErrorKind::InvalidData
        )
    });
    let mut failure = PreparationError::from(error);
    if invalid_entry {
        failure.kind = PreparationKind::InvalidPackage;
    }
    failure
}

fn validate_entry(
    adapter: &NpmAdapter,
    version: &str,
    directory: &Path,
) -> anyhow::Result<PathBuf> {
    let package = directory.join(NODE_MODULES).join(adapter.name);
    let manifest: serde_json::Value = serde_json::from_slice(&fs::read(package.join(MANIFEST))?)?;
    ensure!(
        manifest["name"] == adapter.name && manifest["version"] == version,
        "installed adapter identity does not match requested version"
    );
    let bin = &manifest["bin"];
    let relative = bin
        .as_str()
        .or_else(|| bin.get(adapter.bin).and_then(serde_json::Value::as_str))
        .context("installed adapter has no supported bin entry")?;
    let entry = package
        .join(relative)
        .canonicalize()
        .context("resolving adapter entry")?;
    ensure!(
        entry.starts_with(package.canonicalize()?),
        "adapter entry escapes its package directory"
    );
    ensure!(
        matches!(
            entry.extension().and_then(|s| s.to_str()),
            Some("js" | "mjs" | "cjs")
        ),
        "adapter entry is not a supported JS file"
    );
    ensure!(entry.is_file(), "adapter entry is not a file");
    // Node reads this file; executable mode and npm's .bin symlink are irrelevant.
    fs::File::open(&entry).context("reading adapter JS entry")?;
    Ok(entry)
}

#[cfg(test)]
mod tests;
