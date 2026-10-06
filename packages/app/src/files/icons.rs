//! Extension-to-icon-path mapping for the Files dock.
//!
//! `icon_path` returns an asset path key understood by `DarudaAssets::load`.
//! Callers pass the returned string directly to `gpui::svg().path(...)` (monochrome)
//! or `gpui::img(...)` (color).

use crate::files::tree::EntryKind;
use crate::path_ext::PathExt;

/// Return the asset path for the icon best representing a file-tree entry.
///
/// Directory icons are chosen first by special folder name (e.g. `node_modules`,
/// `.github`), then by expanded state (`folder-open` / `folder`).
/// Symlinks always get `"icons/symlink.svg"`.
/// Files are matched first by exact name (e.g. `Dockerfile`), then by
/// lower-cased extension. Unknown extensions fall back to `"icons/file.svg"`.
pub fn icon_path(kind: EntryKind, is_symlink: bool, is_expanded: bool, name: &str) -> &'static str {
    if is_symlink {
        return "icons/symlink.svg";
    }
    if kind.is_dir() {
        return dir_icon(name, is_expanded);
    }
    file_icon_path(name)
}

fn dir_icon(name: &str, is_expanded: bool) -> &'static str {
    let special = match name {
        // VCS
        ".git" => Some("icons/folder-git.svg"),
        ".github" | ".github/workflows" => Some("icons/folder-github.svg"),
        ".vscode" => Some("icons/folder-vscode.svg"),
        // JS/TS ecosystem
        "node_modules" => Some("icons/folder-node.svg"),
        ".next" => Some("icons/folder-next.svg"),
        ".nuxt" => Some("icons/folder-nuxt.svg"),
        ".svelte-kit" => Some("icons/folder-svelte.svg"),
        // Common project folders
        "src" => Some("icons/folder-src.svg"),
        "dist" | "build" | "bin" | "out" => Some("icons/folder-dist.svg"),
        "test" | "tests" | "__tests__" | "spec" | "specs" => Some("icons/folder-test.svg"),
        "config" | ".config" => Some("icons/folder-config.svg"),
        "docs" | "doc" | "documentation" => Some("icons/folder-docs.svg"),
        "lib" | "vendor" | "third_party" => Some("icons/folder-lib.svg"),
        "api" => Some("icons/folder-api.svg"),
        "coverage" | ".coverage" => Some("icons/folder-coverage.svg"),
        _ => None,
    };
    if let Some(path) = special {
        return path;
    }
    if is_expanded {
        "icons/folder-open.svg"
    } else {
        "icons/folder.svg"
    }
}

fn file_icon_path(name: &str) -> &'static str {
    // Exact filename matches
    match name {
        "Dockerfile" | "Containerfile" => return "icons/docker.svg",
        "docker-compose.yml"
        | "docker-compose.yaml"
        | "docker-compose.override.yml"
        | "docker-compose.override.yaml" => return "icons/docker.svg",
        ".dockerignore" => return "icons/docker.svg",
        ".gitignore" | ".gitattributes" | ".gitmodules" => return "icons/lock.svg",
        ".editorconfig" => return "icons/editorconfig.svg",
        ".eslintrc" | ".eslintrc.js" | ".eslintrc.cjs" | ".eslintrc.json" | ".eslintrc.yml"
        | ".eslintrc.yaml" | ".eslintignore" => return "icons/eslint.svg",
        "eslint.config.js" | "eslint.config.mjs" | "eslint.config.cjs" => return "icons/eslint.svg",
        ".prettierrc" | ".prettierrc.js" | ".prettierrc.cjs" | ".prettierrc.json"
        | ".prettierrc.yml" | ".prettierrc.yaml" | ".prettierignore" => return "icons/prettier.svg",
        "prettier.config.js" | "prettier.config.mjs" | "prettier.config.cjs" => {
            return "icons/prettier.svg";
        }
        "jest.config.js" | "jest.config.ts" | "jest.config.mjs" | "jest.config.cjs"
        | "jest.config.json" => return "icons/jest.svg",
        "vite.config.js" | "vite.config.ts" | "vite.config.mjs" | "vite.config.cjs" => {
            return "icons/vite.svg";
        }
        "webpack.config.js" | "webpack.config.ts" | "webpack.config.mjs" => return "icons/vite.svg",
        "package.json" | "package-lock.json" | ".nvmrc" | ".node-version" => {
            return "icons/nodejs.svg";
        }
        "bun.lock" | "bun.lockb" => return "icons/bun.svg",
        "go.mod" | "go.sum" => return "icons/go-mod.svg",
        "Makefile" | "makefile" | "GNUmakefile" => return "icons/makefile.svg",
        "Rakefile" => return "icons/ruby.svg",
        "Gemfile" | "Gemfile.lock" => return "icons/ruby.svg",
        "Podfile" | "Podfile.lock" => return "icons/swift.svg",
        "pom.xml" => return "icons/java.svg",
        "requirements.txt" | "requirements-dev.txt" | "requirements-test.txt" => {
            return "icons/python-misc.svg";
        }
        "pyproject.toml" => return "icons/python-misc.svg",
        "poetry.lock" => return "icons/poetry.svg",
        "nginx.conf" => return "icons/nginx.svg",
        ".env" | ".env.local" | ".env.development" | ".env.production" | ".env.test" => {
            return "icons/lock.svg";
        }
        _ => {}
    }

    // Extension-based matches
    let ext = std::path::Path::new(name).extension_lower();
    match ext.as_deref() {
        // Rust
        Some("rs") => "icons/rust.svg",
        // Python
        Some("py" | "pyi" | "pyx") => "icons/python.svg",
        // JavaScript
        Some("js" | "mjs" | "cjs") => "icons/javascript.svg",
        // TypeScript
        Some("ts" | "mts" | "cts") => "icons/typescript.svg",
        // JSX/TSX (React)
        Some("jsx" | "tsx") => "icons/react.svg",
        // Web
        Some("html" | "htm") => "icons/html.svg",
        Some("css") => "icons/css.svg",
        Some("scss" | "sass" | "less") => "icons/css.svg",
        // Data/config
        Some("json" | "jsonc" | "json5") => "icons/json.svg",
        Some("toml") => "icons/toml.svg",
        Some("ini" | "cfg" | "conf") => "icons/toml.svg",
        Some("yaml" | "yml") => "icons/yaml.svg",
        // Docs
        Some("md" | "mdx" | "markdown") => "icons/markdown.svg",
        Some("xml" | "xsd" | "xsl" | "xslt" | "plist" | "iml" | "resx") => "icons/xml.svg",
        // Shell
        Some("sh" | "zsh" | "bash" | "fish" | "nu") => "icons/shell.svg",
        Some("ps1" | "psm1" | "psd1" | "ps1xml" | "pssc") => "icons/powershell.svg",
        // Systems / compiled
        Some("go") => "icons/go.svg",
        Some("zig") => "icons/zig.svg",
        Some("c" | "h") => "icons/c.svg",
        Some("cpp" | "cc" | "cxx" | "hpp" | "hxx") => "icons/cpp.svg",
        Some("cs") => "icons/csharp.svg",
        Some("fs" | "fsi" | "fsx" | "fsproj") => "icons/fsharp.svg",
        Some("java" | "jsp") => "icons/java.svg",
        Some("kt" | "kts") => "icons/kotlin.svg",
        Some("scala" | "sc") => "icons/scala.svg",
        Some("swift") => "icons/swift.svg",
        Some("dart") => "icons/dart.svg",
        Some("rb" | "rbs" | "erb") => "icons/ruby.svg",
        Some("lua") => "icons/lua.svg",
        Some("ex" | "exs" | "heex" | "leex" | "eex") => "icons/elixir.svg",
        Some("erl" | "hrl") => "icons/erlang.svg",
        Some("hs" | "lhs") => "icons/haskell.svg",
        Some("nim" | "nimble") => "icons/nim.svg",
        Some("jl") => "icons/julia.svg",
        Some("r" | "rmd") => "icons/r.svg",
        Some("elm") => "icons/elm.svg",
        Some("ml" | "mli" | "cmx") => "icons/ocaml.svg",
        Some("clj" | "cljc" | "cljs" | "edn") => "icons/clojure.svg",
        // Web frameworks
        Some("vue") => "icons/vue.svg",
        Some("svelte") => "icons/svelte.svg",
        Some("astro") => "icons/astro.svg",
        // API / schema
        Some("gql" | "graphql") => "icons/graphql.svg",
        Some("proto") => "icons/proto.svg",
        // Infra
        Some("tf" | "tfvars" | "tfstate") => "icons/terraform.svg",
        Some("nix") => "icons/nix.svg",
        // Data
        Some("sql") => "icons/sql.svg",
        Some("csv" | "tsv" | "tab") => "icons/table.svg",
        Some("ipynb") => "icons/jupyter.svg",
        Some("wasm") => "icons/webassembly.svg",
        // Tooling
        Some("prisma") => "icons/prisma.svg",
        // Lock / security
        Some("lock") => "icons/lock.svg",
        Some("pem" | "key" | "crt" | "cer" | "p12" | "pfx") => "icons/lock.svg",
        // Media
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "ico" | "bmp" | "tiff" | "avif" | "svg") => {
            "icons/image.svg"
        }
        Some("mp4" | "mov" | "mkv" | "avi" | "webm" | "flv" | "m4v") => "icons/video.svg",
        Some("mp3" | "wav" | "flac" | "ogg" | "m4a" | "aac" | "opus") => "icons/audio.svg",
        // Archives
        Some("zip" | "tar" | "gz" | "bz2" | "xz" | "7z" | "rar" | "zst") => "icons/archive.svg",
        _ => "icons/file.svg",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::tree::EntryKind;

    fn file(name: &str) -> &'static str {
        icon_path(EntryKind::File, false, false, name)
    }
    fn dir(name: &str, expanded: bool) -> &'static str {
        icon_path(EntryKind::Dir, false, expanded, name)
    }

    #[test]
    fn language_extensions() {
        assert_eq!(file("main.rs"), "icons/rust.svg");
        assert_eq!(file("app.py"), "icons/python.svg");
        assert_eq!(file("index.js"), "icons/javascript.svg");
        assert_eq!(file("index.ts"), "icons/typescript.svg");
        assert_eq!(file("App.tsx"), "icons/react.svg");
        assert_eq!(file("page.html"), "icons/html.svg");
        assert_eq!(file("style.css"), "icons/css.svg");
        assert_eq!(file("data.json"), "icons/json.svg");
        assert_eq!(file("Cargo.toml"), "icons/toml.svg");
        assert_eq!(file("config.yaml"), "icons/yaml.svg");
        assert_eq!(file("README.md"), "icons/markdown.svg");
        assert_eq!(file("build.sh"), "icons/shell.svg");
        assert_eq!(file("main.go"), "icons/go.svg");
        assert_eq!(file("main.zig"), "icons/zig.svg");
        assert_eq!(file("lib.c"), "icons/c.svg");
        assert_eq!(file("app.cpp"), "icons/cpp.svg");
        assert_eq!(file("schema.sql"), "icons/sql.svg");
    }

    #[test]
    fn new_language_extensions() {
        assert_eq!(file("Main.java"), "icons/java.svg");
        assert_eq!(file("app.kt"), "icons/kotlin.svg");
        assert_eq!(file("App.vue"), "icons/vue.svg");
        assert_eq!(file("App.svelte"), "icons/svelte.svg");
        assert_eq!(file("View.swift"), "icons/swift.svg");
        assert_eq!(file("app.rb"), "icons/ruby.svg");
        assert_eq!(file("Program.cs"), "icons/csharp.svg");
        assert_eq!(file("main.scala"), "icons/scala.svg");
        assert_eq!(file("init.lua"), "icons/lua.svg");
        assert_eq!(file("app.dart"), "icons/dart.svg");
        assert_eq!(file("app.ex"), "icons/elixir.svg");
        assert_eq!(file("server.erl"), "icons/erlang.svg");
        assert_eq!(file("main.hs"), "icons/haskell.svg");
        assert_eq!(file("app.nim"), "icons/nim.svg");
        assert_eq!(file("main.jl"), "icons/julia.svg");
        assert_eq!(file("query.gql"), "icons/graphql.svg");
        assert_eq!(file("api.proto"), "icons/proto.svg");
        assert_eq!(file("index.astro"), "icons/astro.svg");
        assert_eq!(file("schema.prisma"), "icons/prisma.svg");
        assert_eq!(file("config.xml"), "icons/xml.svg");
        assert_eq!(file("data.tf"), "icons/terraform.svg");
        assert_eq!(file("config.nix"), "icons/nix.svg");
        assert_eq!(file("script.ps1"), "icons/powershell.svg");
        assert_eq!(file("analysis.r"), "icons/r.svg");
        assert_eq!(file("main.elm"), "icons/elm.svg");
        assert_eq!(file("lib.ml"), "icons/ocaml.svg");
        assert_eq!(file("core.clj"), "icons/clojure.svg");
        assert_eq!(file("notebook.ipynb"), "icons/jupyter.svg");
        assert_eq!(file("data.csv"), "icons/table.svg");
        assert_eq!(file("module.wasm"), "icons/webassembly.svg");
    }

    #[test]
    fn tooling_filenames() {
        assert_eq!(file(".editorconfig"), "icons/editorconfig.svg");
        assert_eq!(file(".eslintrc.json"), "icons/eslint.svg");
        assert_eq!(file(".prettierrc"), "icons/prettier.svg");
        assert_eq!(file("jest.config.ts"), "icons/jest.svg");
        assert_eq!(file("vite.config.ts"), "icons/vite.svg");
        assert_eq!(file("package.json"), "icons/nodejs.svg");
        assert_eq!(file("bun.lock"), "icons/bun.svg");
        assert_eq!(file("go.mod"), "icons/go-mod.svg");
        assert_eq!(file("Makefile"), "icons/makefile.svg");
        assert_eq!(file("docker-compose.yml"), "icons/docker.svg");
        assert_eq!(file("requirements.txt"), "icons/python-misc.svg");
    }

    #[test]
    fn media_and_archives() {
        assert_eq!(file("photo.png"), "icons/image.svg");
        assert_eq!(file("clip.mp4"), "icons/video.svg");
        assert_eq!(file("song.mp3"), "icons/audio.svg");
        assert_eq!(file("dist.zip"), "icons/archive.svg");
        assert_eq!(file("Cargo.lock"), "icons/lock.svg");
    }

    #[test]
    fn special_names() {
        assert_eq!(file("Dockerfile"), "icons/docker.svg");
        assert_eq!(file(".gitignore"), "icons/lock.svg");
        assert_eq!(file(".env"), "icons/lock.svg");
        assert_eq!(file(".env.production"), "icons/lock.svg");
    }

    #[test]
    fn default_fallback() {
        assert_eq!(file("binary"), "icons/file.svg");
        assert_eq!(file("unknown.xyz"), "icons/file.svg");
    }

    #[test]
    fn folder_basic() {
        assert_eq!(dir("myproject", false), "icons/folder.svg");
        assert_eq!(dir("myproject", true), "icons/folder-open.svg");
    }

    #[test]
    fn folder_special() {
        assert_eq!(dir(".git", false), "icons/folder-git.svg");
        assert_eq!(dir(".git", true), "icons/folder-git.svg");
        assert_eq!(dir(".github", false), "icons/folder-github.svg");
        assert_eq!(dir("node_modules", false), "icons/folder-node.svg");
        assert_eq!(dir(".next", false), "icons/folder-next.svg");
        assert_eq!(dir(".nuxt", false), "icons/folder-nuxt.svg");
        assert_eq!(dir(".svelte-kit", false), "icons/folder-svelte.svg");
        assert_eq!(dir("src", false), "icons/folder-src.svg");
        assert_eq!(dir("dist", false), "icons/folder-dist.svg");
        assert_eq!(dir("build", false), "icons/folder-dist.svg");
        assert_eq!(dir("test", false), "icons/folder-test.svg");
        assert_eq!(dir("tests", false), "icons/folder-test.svg");
        assert_eq!(dir("__tests__", false), "icons/folder-test.svg");
        assert_eq!(dir("config", false), "icons/folder-config.svg");
        assert_eq!(dir("docs", false), "icons/folder-docs.svg");
        assert_eq!(dir("lib", false), "icons/folder-lib.svg");
        assert_eq!(dir("api", false), "icons/folder-api.svg");
        assert_eq!(dir("coverage", false), "icons/folder-coverage.svg");
        assert_eq!(dir(".vscode", false), "icons/folder-vscode.svg");
    }

    #[test]
    fn symlink() {
        assert_eq!(
            icon_path(EntryKind::File, true, false, "link"),
            "icons/symlink.svg"
        );
        assert_eq!(
            icon_path(EntryKind::Dir, true, false, "src"),
            "icons/symlink.svg"
        );
    }

    #[test]
    fn pending_and_unloaded_dir() {
        use crate::files::tree::EntryKind;
        assert_eq!(
            icon_path(EntryKind::UnloadedDir, false, false, "somelib"),
            "icons/folder.svg"
        );
        assert_eq!(
            icon_path(EntryKind::PendingDir, false, true, "somelib"),
            "icons/folder-open.svg"
        );
    }
}
