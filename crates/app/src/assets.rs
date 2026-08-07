//! Embedded application assets.
//!
//! `DarudaAssets` implements GPUI's `AssetSource` trait. All asset bytes are
//! baked into the binary at compile time via `include_bytes!` so the app
//! ships as a single self-contained executable.
//!
//! File icons are sourced from material-icon-theme (MIT).
//! Copyright (c) 2025 Material Extensions / Philipp Kief.
//! See LICENSES/material-icon-theme-MIT.txt for the full attribution.
//!
//! UI control icons under `icons/ui/` are sourced from Google Material
//! Symbols (Apache-2.0). See LICENSES/material-symbols-Apache-2.0.txt.

use std::borrow::Cow;

use gpui::{AssetSource, SharedString};

pub struct DarudaAssets;

impl AssetSource for DarudaAssets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        macro_rules! icon {
            ($file:literal) => {
                return Ok(Some(Cow::Borrowed(include_bytes!(concat!(
                    "../../../assets/icons/",
                    $file
                )) as &'static [u8])))
            };
        }
        match path {
            // ── Fallbacks ────────────────────────────────────────────────
            "icons/file.svg" => icon!("file.svg"),
            "icons/symlink.svg" => icon!("symlink.svg"),

            // ── ACP agents ───────────────────────────────────────────────
            "icons/agents/agoragentic-acp.svg" => icon!("agents/agoragentic-acp.svg"),
            "icons/agents/amp-acp.svg" => icon!("agents/amp-acp.svg"),
            "icons/agents/auggie.svg" => icon!("agents/auggie.svg"),
            "icons/agents/autohand.svg" => icon!("agents/autohand.svg"),
            "icons/agents/claude-acp.svg" => icon!("agents/claude-acp.svg"),
            "icons/agents/cline.svg" => icon!("agents/cline.svg"),
            "icons/agents/codebuddy-code.svg" => icon!("agents/codebuddy-code.svg"),
            "icons/agents/codex-acp.svg" => icon!("agents/codex-acp.svg"),
            "icons/agents/cortex-code.svg" => icon!("agents/cortex-code.svg"),
            "icons/agents/corust-agent.svg" => icon!("agents/corust-agent.svg"),
            "icons/agents/crow-cli.svg" => icon!("agents/crow-cli.svg"),
            "icons/agents/cursor.svg" => icon!("agents/cursor.svg"),
            "icons/agents/deepagents.svg" => icon!("agents/deepagents.svg"),
            "icons/agents/devin.svg" => icon!("agents/devin.svg"),
            "icons/agents/dimcode.svg" => icon!("agents/dimcode.svg"),
            "icons/agents/dirac.svg" => icon!("agents/dirac.svg"),
            "icons/agents/factory-droid.svg" => icon!("agents/factory-droid.svg"),
            "icons/agents/fast-agent.svg" => icon!("agents/fast-agent.svg"),
            "icons/agents/gemini.svg" => icon!("agents/gemini.svg"),
            "icons/agents/github-copilot-cli.svg" => icon!("agents/github-copilot-cli.svg"),
            "icons/agents/glm-acp-agent.svg" => icon!("agents/glm-acp-agent.svg"),
            "icons/agents/goose.svg" => icon!("agents/goose.svg"),
            "icons/agents/grok-build.svg" => icon!("agents/grok-build.svg"),
            "icons/agents/harn.svg" => icon!("agents/harn.svg"),
            "icons/agents/junie.svg" => icon!("agents/junie.svg"),
            "icons/agents/kilo.svg" => icon!("agents/kilo.svg"),
            "icons/agents/kimi.svg" => icon!("agents/kimi.svg"),
            "icons/agents/minion-code.svg" => icon!("agents/minion-code.svg"),
            "icons/agents/mistral-vibe.svg" => icon!("agents/mistral-vibe.svg"),
            "icons/agents/nova.svg" => icon!("agents/nova.svg"),
            "icons/agents/opencode.svg" => icon!("agents/opencode.svg"),
            "icons/agents/pi-acp.svg" => icon!("agents/pi-acp.svg"),
            "icons/agents/poolside.svg" => icon!("agents/poolside.svg"),
            "icons/agents/qoder.svg" => icon!("agents/qoder.svg"),
            "icons/agents/qwen-code.svg" => icon!("agents/qwen-code.svg"),
            "icons/agents/sigit.svg" => icon!("agents/sigit.svg"),
            "icons/agents/stakpak.svg" => icon!("agents/stakpak.svg"),
            "icons/agents/vtcode.svg" => icon!("agents/vtcode.svg"),

            // ── UI controls ───────────────────────────────────────────────
            "icons/ui/check.svg" => icon!("ui/check.svg"),
            "icons/ui/chrome-reader-mode.svg" => icon!("ui/chrome-reader-mode.svg"),
            "icons/ui/code.svg" => icon!("ui/code.svg"),
            "icons/ui/compress.svg" => icon!("ui/compress.svg"),
            "icons/ui/content-copy.svg" => icon!("ui/content-copy.svg"),
            "icons/ui/difference.svg" => icon!("ui/difference.svg"),
            "icons/ui/expand.svg" => icon!("ui/expand.svg"),
            "icons/ui/filter-alt.svg" => icon!("ui/filter-alt.svg"),
            "icons/ui/filter-alt-off.svg" => icon!("ui/filter-alt-off.svg"),
            "icons/ui/open-in-new.svg" => icon!("ui/open-in-new.svg"),
            "icons/ui/preview.svg" => icon!("ui/preview.svg"),
            "icons/ui/unfold-less.svg" => icon!("ui/unfold-less.svg"),
            "icons/ui/unfold-more.svg" => icon!("ui/unfold-more.svg"),
            "icons/ui/width-wide.svg" => icon!("ui/width-wide.svg"),

            // ── Folders ──────────────────────────────────────────────────
            "icons/folder.svg" => icon!("folder.svg"),
            "icons/folder-open.svg" => icon!("folder-open.svg"),
            "icons/folder-git.svg" => icon!("folder-git.svg"),
            "icons/folder-github.svg" => icon!("folder-github.svg"),
            "icons/folder-vscode.svg" => icon!("folder-vscode.svg"),
            "icons/folder-node.svg" => icon!("folder-node.svg"),
            "icons/folder-next.svg" => icon!("folder-next.svg"),
            "icons/folder-nuxt.svg" => icon!("folder-nuxt.svg"),
            "icons/folder-svelte.svg" => icon!("folder-svelte.svg"),
            "icons/folder-src.svg" => icon!("folder-src.svg"),
            "icons/folder-dist.svg" => icon!("folder-dist.svg"),
            "icons/folder-test.svg" => icon!("folder-test.svg"),
            "icons/folder-config.svg" => icon!("folder-config.svg"),
            "icons/folder-docs.svg" => icon!("folder-docs.svg"),
            "icons/folder-lib.svg" => icon!("folder-lib.svg"),
            "icons/folder-api.svg" => icon!("folder-api.svg"),
            "icons/folder-coverage.svg" => icon!("folder-coverage.svg"),

            // ── Systems / compiled ────────────────────────────────────────
            "icons/rust.svg" => icon!("rust.svg"),
            "icons/c.svg" => icon!("c.svg"),
            "icons/cpp.svg" => icon!("cpp.svg"),
            "icons/csharp.svg" => icon!("csharp.svg"),
            "icons/fsharp.svg" => icon!("fsharp.svg"),
            "icons/go.svg" => icon!("go.svg"),
            "icons/zig.svg" => icon!("zig.svg"),
            "icons/java.svg" => icon!("java.svg"),
            "icons/kotlin.svg" => icon!("kotlin.svg"),
            "icons/scala.svg" => icon!("scala.svg"),
            "icons/swift.svg" => icon!("swift.svg"),
            "icons/dart.svg" => icon!("dart.svg"),

            // ── Scripting ─────────────────────────────────────────────────
            "icons/python.svg" => icon!("python.svg"),
            "icons/ruby.svg" => icon!("ruby.svg"),
            "icons/lua.svg" => icon!("lua.svg"),
            "icons/elixir.svg" => icon!("elixir.svg"),
            "icons/erlang.svg" => icon!("erlang.svg"),
            "icons/haskell.svg" => icon!("haskell.svg"),
            "icons/nim.svg" => icon!("nim.svg"),
            "icons/julia.svg" => icon!("julia.svg"),
            "icons/r.svg" => icon!("r.svg"),
            "icons/elm.svg" => icon!("elm.svg"),
            "icons/ocaml.svg" => icon!("ocaml.svg"),
            "icons/clojure.svg" => icon!("clojure.svg"),
            "icons/shell.svg" => icon!("shell.svg"),
            "icons/powershell.svg" => icon!("powershell.svg"),

            // ── Web / JS ecosystem ────────────────────────────────────────
            "icons/javascript.svg" => icon!("javascript.svg"),
            "icons/typescript.svg" => icon!("typescript.svg"),
            "icons/react.svg" => icon!("react.svg"),
            "icons/vue.svg" => icon!("vue.svg"),
            "icons/svelte.svg" => icon!("svelte.svg"),
            "icons/astro.svg" => icon!("astro.svg"),
            "icons/html.svg" => icon!("html.svg"),
            "icons/css.svg" => icon!("css.svg"),

            // ── Data / config ─────────────────────────────────────────────
            "icons/json.svg" => icon!("json.svg"),
            "icons/toml.svg" => icon!("toml.svg"),
            "icons/yaml.svg" => icon!("yaml.svg"),
            "icons/xml.svg" => icon!("xml.svg"),
            "icons/markdown.svg" => icon!("markdown.svg"),
            "icons/sql.svg" => icon!("sql.svg"),
            "icons/table.svg" => icon!("table.svg"),
            "icons/jupyter.svg" => icon!("jupyter.svg"),
            "icons/webassembly.svg" => icon!("webassembly.svg"),

            // ── API / schema ──────────────────────────────────────────────
            "icons/graphql.svg" => icon!("graphql.svg"),
            "icons/proto.svg" => icon!("proto.svg"),
            "icons/prisma.svg" => icon!("prisma.svg"),

            // ── Infra ─────────────────────────────────────────────────────
            "icons/docker.svg" => icon!("docker.svg"),
            "icons/terraform.svg" => icon!("terraform.svg"),
            "icons/nix.svg" => icon!("nix.svg"),
            "icons/nginx.svg" => icon!("nginx.svg"),

            // ── Build tools / config ──────────────────────────────────────
            "icons/editorconfig.svg" => icon!("editorconfig.svg"),
            "icons/eslint.svg" => icon!("eslint.svg"),
            "icons/prettier.svg" => icon!("prettier.svg"),
            "icons/jest.svg" => icon!("jest.svg"),
            "icons/vite.svg" => icon!("vite.svg"),
            "icons/nodejs.svg" => icon!("nodejs.svg"),
            "icons/bun.svg" => icon!("bun.svg"),
            "icons/go-mod.svg" => icon!("go-mod.svg"),
            "icons/makefile.svg" => icon!("makefile.svg"),
            "icons/python-misc.svg" => icon!("python-misc.svg"),
            "icons/poetry.svg" => icon!("poetry.svg"),

            // ── Security / lock ───────────────────────────────────────────
            "icons/lock.svg" => icon!("lock.svg"),

            // ── Media ─────────────────────────────────────────────────────
            "icons/image.svg" => icon!("image.svg"),
            "icons/video.svg" => icon!("video.svg"),
            "icons/audio.svg" => icon!("audio.svg"),
            "icons/archive.svg" => icon!("archive.svg"),

            // Fallback: gpui_component's vendored icons (e.g. icons/check.svg).
            _ => gpui_component_assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        gpui_component_assets::Assets.list(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UI_ICON_PATHS: &[&str] = &[
        "icons/ui/check.svg",
        "icons/ui/chrome-reader-mode.svg",
        "icons/ui/code.svg",
        "icons/ui/compress.svg",
        "icons/ui/content-copy.svg",
        "icons/ui/difference.svg",
        "icons/ui/expand.svg",
        "icons/ui/filter-alt.svg",
        "icons/ui/filter-alt-off.svg",
        "icons/ui/open-in-new.svg",
        "icons/ui/preview.svg",
        "icons/ui/unfold-less.svg",
        "icons/ui/unfold-more.svg",
        "icons/ui/width-wide.svg",
    ];

    #[test]
    fn ui_control_icons_are_embedded_in_the_binary() {
        for path in UI_ICON_PATHS {
            let bytes = DarudaAssets
                .load(path)
                .unwrap_or_else(|e| panic!("loading {path} errored: {e}"))
                .unwrap_or_else(|| panic!("{path} is not registered in assets.rs"));
            assert!(
                bytes.windows(4).any(|w| w == b"<svg"),
                "{path} resolved to something that is not an SVG"
            );
        }
    }
}
