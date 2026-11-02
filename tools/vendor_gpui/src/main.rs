//! Reproduce the GPUI source override from a pristine, pinned Git tree.

use anyhow::{Context, Result, bail, ensure};
use std::{collections::BTreeMap, fs, path::Path, process::Command};
use toml::{Table, Value};

const PATCHES: &[&str] = &[
    "gpui-notify-lost-wakeup.patch",
    "gpui-held-key-keeps-modality.patch",
    "gpui-text-wrap-cache.patch",
];

fn run(command: &mut Command) -> Result<()> {
    let output = command.output().with_context(|| format!("{command:?}"))?;
    ensure!(
        output.status.success(),
        "{command:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

fn manifest(path: &Path) -> Result<Value> {
    Ok(toml::from_str(&fs::read_to_string(path)?)?)
}

fn pinned_source(root: &Value) -> Result<(&str, &str)> {
    let dependencies = root["workspace"]["dependencies"]
        .as_table()
        .context("workspace dependencies")?;
    let dependency = &dependencies["gpui"];
    let git = dependency["git"].as_str().context("gpui git URL")?;
    let rev = dependency["rev"].as_str().context("gpui pinned revision")?;
    for (name, dependency) in dependencies {
        if dependency.get("git").and_then(Value::as_str) == Some(git) {
            ensure!(
                dependency.get("rev").and_then(Value::as_str) == Some(rev),
                "{name} must use the same pinned Zed revision as gpui"
            );
        }
    }
    ensure!(
        root.get("patch")
            .and_then(|p| p.get(git))
            .and_then(|p| p.get("gpui"))
            .and_then(|p| p.get("path"))
            .and_then(Value::as_str)
            == Some("vendor/zed/crates/gpui"),
        "Cargo must select the checked-in GPUI override"
    );
    Ok((git, rev))
}

fn collect_dependencies(value: &Value, names: &mut Vec<String>) {
    if let Some(table) = value.as_table() {
        for (name, child) in table {
            if child.get("workspace").and_then(Value::as_bool) == Some(true) && name != "lints" {
                names.push(name.clone());
            } else {
                collect_dependencies(child, names);
            }
        }
    }
}

fn workspace(upstream: &Value, gpui: &Value, git: &str, rev: &str) -> Result<Value> {
    let mut names = Vec::new();
    for section in [
        "dependencies",
        "dev-dependencies",
        "build-dependencies",
        "target",
    ] {
        if let Some(value) = gpui.get(section) {
            collect_dependencies(value, &mut names);
        }
    }
    let mut dependencies = Table::new();
    for name in names {
        let mut dep = upstream["workspace"]["dependencies"]
            .get(&name)
            .with_context(|| format!("missing upstream dependency {name}"))?
            .clone();
        if let Some(table) = dep.as_table_mut()
            && table.remove("path").is_some()
        {
            table.insert("git".into(), git.into());
            table.insert("rev".into(), rev.into());
        }
        dependencies.insert(name, dep);
    }
    let mut workspace = Table::new();
    workspace.insert("members".into(), Value::Array(vec!["crates/gpui".into()]));
    workspace.insert("resolver".into(), "2".into());
    workspace.insert("package".into(), upstream["workspace"]["package"].clone());
    workspace.insert("lints".into(), upstream["workspace"]["lints"].clone());
    workspace.insert("dependencies".into(), Value::Table(dependencies));
    let mut root = Table::new();
    root.insert("workspace".into(), Value::Table(workspace));
    Ok(Value::Table(root))
}

fn files(
    root: &Path,
    dir: &Path,
    output: &mut BTreeMap<std::path::PathBuf, Vec<u8>>,
) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            files(root, &path, output)?;
        } else {
            output.insert(path.strip_prefix(root)?.to_owned(), fs::read(path)?);
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let mut args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() == 1 && args[0] == "--check" {
        let output = Command::new("cargo")
            .args(["metadata", "--locked", "--offline", "--format-version=1"])
            .output()?;
        ensure!(
            output.status.success(),
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        let package = metadata["packages"]
            .as_array()
            .context("Cargo packages")?
            .iter()
            .find(|p| p["name"] == "gpui_platform")
            .context("gpui_platform dependency")?;
        let path = Path::new(
            package["manifest_path"]
                .as_str()
                .context("upstream manifest path")?,
        );
        args.insert(
            0,
            path.parent()
                .and_then(Path::parent)
                .and_then(Path::parent)
                .context("Zed checkout root")?
                .as_os_str()
                .to_owned(),
        );
    }
    ensure!(
        args.len() == 2 || args.len() == 3,
        "usage: vendor_gpui <zed-checkout> --check | vendor_gpui <zed-checkout> --output <new-directory>"
    );
    let source = Path::new(&args[0]);
    let check = args[1] == "--check" && args.len() == 2;
    ensure!(
        check || (args[1] == "--output" && args.len() == 3),
        "invalid arguments"
    );
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let root = manifest(&repo.join("Cargo.toml"))?;
    let (git, rev) = pinned_source(&root)?;
    let temp = tempfile::tempdir()?;
    let archive = temp.path().join("source.tar");
    let stage = temp.path().join("zed");
    fs::create_dir(&stage)?;
    run(Command::new("git")
        .arg("-C")
        .arg(source)
        .args(["archive", "--format=tar", "--output"])
        .arg(&archive)
        .arg(rev)
        .args([
            "Cargo.toml",
            "LICENSE-APACHE",
            "crates/gpui",
            "assets/fonts/ibm-plex-sans",
            "assets/fonts/lilex",
            ":(exclude)crates/gpui/LICENSE-APACHE",
        ]))?;
    run(Command::new("tar")
        .arg("-xf")
        .arg(&archive)
        .arg("-C")
        .arg(&stage))?;
    fs::copy(
        stage.join("LICENSE-APACHE"),
        stage.join("crates/gpui/LICENSE-APACHE"),
    )?;
    let upstream = manifest(&stage.join("Cargo.toml"))?;
    let gpui = manifest(&stage.join("crates/gpui/Cargo.toml"))?;
    let manifest = workspace(&upstream, &gpui, git, rev)?;
    fs::write(stage.join("Cargo.toml"), toml::to_string_pretty(&manifest)?)?;
    for patch in PATCHES {
        run(Command::new("git")
            .current_dir(&stage)
            .env("GIT_CEILING_DIRECTORIES", temp.path())
            .args(["-c", "core.autocrlf=false"])
            .args(["apply", "--check"])
            .arg(repo.join("patches").join(patch)))?;
        run(Command::new("git")
            .current_dir(&stage)
            .env("GIT_CEILING_DIRECTORIES", temp.path())
            .args(["-c", "core.autocrlf=false"])
            .arg("apply")
            .arg(repo.join("patches").join(patch)))?;
    }
    fs::write(
        stage.join("UPSTREAM"),
        format!("{git}\n{rev}\n{}\n", PATCHES.join("\n")),
    )?;
    let mut generated = BTreeMap::new();
    files(&stage, &stage, &mut generated)?;
    if check {
        let checked_in = repo.join("vendor/zed");
        let mut existing = BTreeMap::new();
        files(&checked_in, &checked_in, &mut existing)?;
        if generated != existing {
            for path in generated.keys().chain(existing.keys()) {
                if generated.get(path) != existing.get(path) {
                    println!("GPUI vendor drift: {}", path.display());
                }
            }
            bail!("vendored GPUI differs from pinned source and patches");
        }
        println!("Vendored GPUI matches pinned source and patches.");
    } else {
        let destination = Path::new(&args[2]);
        ensure!(
            !destination.exists(),
            "output directory must not already exist"
        );
        for (path, bytes) in generated {
            let path = destination.join(path);
            fs::create_dir_all(path.parent().unwrap())?;
            fs::write(path, bytes)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
