//! npm and ACP share the concrete Node selected by the runtime resolver.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::node::resolved::ResolvedNode;
use crate::preparation::{PreparationError, PreparationKind};

const NPM_FETCH_TIMEOUT: &str = "30000";

pub(super) struct NpmRuntime {
    pub node: PathBuf,
    pub version: String,
    npm: PathBuf,
    pub env: BTreeMap<String, String>,
    strip_env: Vec<String>,
}

impl NpmRuntime {
    pub fn resolve(
        node: ResolvedNode,
        env: &BTreeMap<String, String>,
        strip_env: &[String],
    ) -> Result<Self, PreparationError> {
        let bin = node.node.parent().expect("resolved Node has a parent");
        let mut env = env.clone();
        let original_path = env
            .get("PATH")
            .cloned()
            .unwrap_or_else(|| std::env::var("PATH").unwrap_or_default());
        let paths = std::iter::once(bin.to_path_buf()).chain(std::env::split_paths(&original_path));
        env.insert(
            "PATH".into(),
            std::env::join_paths(paths)
                .map_err(|error| {
                    PreparationError::new(PreparationKind::Configuration, error.to_string())
                })?
                .to_string_lossy()
                .into_owned(),
        );
        Ok(Self {
            node: node.node,
            version: node.version,
            npm: node.npm,
            env,
            strip_env: strip_env.to_vec(),
        })
    }

    pub fn npm(&self, root: &Path, cache: &Path) -> Result<Command, PreparationError> {
        let (os, arch) = crate::node::node_platform().map_err(|error| {
            PreparationError::new(PreparationKind::Configuration, error.to_string())
        })?;
        let mut command = Command::new(&self.node);
        // App-managed dependencies do not consume an arbitrary workspace .npmrc.
        // User/global npm settings and explicit launch environment still apply.
        command.arg(&self.npm).current_dir(root).envs(&self.env);
        for name in &self.strip_env {
            command.env_remove(name);
        }
        command.env("npm_config_cpu", arch).env("npm_config_os", os);
        if !self.env.contains_key("npm_config_cache") {
            command.env("npm_config_cache", cache);
        }
        command.args([
            "--fetch-retries=0",
            "--fetch-timeout",
            NPM_FETCH_TIMEOUT,
            "--no-audit",
            "--no-fund",
        ]);
        Ok(command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installs_and_launches_with_the_same_runtime_and_preserves_overrides() {
        let node = ResolvedNode {
            node: "/runtime/bin/node".into(),
            version: "24.11.0".into(),
            npm: "/runtime/lib/npm-cli.js".into(),
        };
        let env = BTreeMap::from([
            ("PATH".into(), "/custom/bin".into()),
            ("npm_config_cache".into(), "/custom/cache".into()),
            ("TEST_SECRET".into(), "hidden".into()),
        ]);
        let runtime = NpmRuntime::resolve(node, &env, &["TEST_SECRET".into()]).unwrap();
        let command = runtime
            .npm(Path::new("/install"), Path::new("/temporary-cache"))
            .unwrap();
        assert_eq!(command.get_program(), runtime.node);
        assert_eq!(command.get_current_dir(), Some(Path::new("/install")));
        assert_eq!(runtime.env["PATH"], "/runtime/bin:/custom/bin");
        let env: BTreeMap<_, _> = command.get_envs().collect();
        assert_eq!(env[std::ffi::OsStr::new("TEST_SECRET")], None);
        assert_eq!(
            env[std::ffi::OsStr::new("npm_config_cache")],
            Some(std::ffi::OsStr::new("/custom/cache"))
        );
    }
}
