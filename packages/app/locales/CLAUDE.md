# locales/ — i18n writing rules

## Overview

daruda uses [rust-i18n v3](https://github.com/longbridgeapp/rust-i18n).
`en.yml` is the canonical source of truth.  
`ko.yml` is the Korean translation. Both files must stay in sync.

## `common:` section — shared tokens

`common:` is the top-level section for strings reused across **two or more unrelated sections**.

### When to use `common.*`

- The exact same surface string appears in ≥ 2 sections (e.g. "Cancel" in `settings`, `task`, `mcp`, `skills`).
- Semantic independence: the sections can change their copy independently in the future — if they always change together, keep them in a shared domain section instead.

### Naming inside `common:`

| prefix | meaning |
|--------|---------|
| `btn_` | button label (`btn_cancel`, `btn_save`, `btn_delete` …) |
| `section_` | section/group heading (`section_project`, `section_personal`) |
| `field_` | form field label (`field_name`, `field_scope`) |
| `name_` | name-validation error (`name_required`) |
| _(bare)_ | UI state or navigation token (`loading`, `refresh`, `new_tab` …) |

### How to wire a new `common.*` key

1. Add the key to `common:` in both `en.yml` and `ko.yml`.
2. Call `s::common::<key>()`. There are no per-domain wrappers: one key is one
   function, so a shared string is reached by its `common.*` name.

### Do NOT put in `common:`

- Strings used in only one section (add them to that section).
- Strings that look identical today but belong to different UX concepts
  (`git.commit_btn: "Commit"` is a git action, not a generic button — keep it in `git:`).
- Strings with context-specific meaning that could diverge across locales.

---

## Key naming

```
<section>.<key>
```

- **section** = top-level YAML key (e.g. `settings`, `task`, `modal`)
- **key** = snake\_case identifier (e.g. `label_font_size`, `err_opacity`)

Use the section that owns the UI where the string appears:
`file_viewer.loading` → now `common.loading` (shared loading state).  
Do **not** add domain-prefixed duplicates like `ui.file_viewer_loading`.

### Prefixes within a section

| prefix | meaning |
|--------|---------|
| `label_` | form field label |
| `section_` | bold section header inside a settings page |
| `nav_` | sidebar navigation item |
| `err_` | validation error message |
| `btn_` / `button_` | button text |
| `field_` | modal field label |
| `action_` | context-menu / button action |
| `confirm_` | confirmation dialog title or button |
| `empty_` | empty-state placeholder |
| `placeholder_` | input placeholder (also used for "coming soon" pages) |

## Adding a new string

1. **Add to `en.yml`** under the appropriate section. Alphabetical order within
   a section is preferred but not required.

2. **Add the matching key to `ko.yml`** in the same position. Missing keys fall
   back to English at runtime (rust-i18n `fallback = "en"`);
   `locale_en_ko_key_parity` and `locale_en_ko_placeholder_parity` in
   `surface/strings/tests.rs` fail when the two drift.

3. **There is no step 3.** `packages/app/build.rs` turns every `en.yml` key into
   a function — `section.key` is `crate::surface::strings::section::key()`, and
   each `%{name}` placeholder becomes a `name: impl Display` parameter, in the
   order the English value first names them:

   ```yaml
   flow:
     # Said under a question when more are behind it.
     more_questions_waiting: "%{n} more waiting"
   ```
   ```rust
   s::flow::more_questions_waiting(queue.len())
   ```

   The `#` lines directly above a key (no blank line between) become its doc
   comment. A key name must be a Rust identifier — the build says which one
   is not. A misspelt key is a compile error, and a key nothing calls is a
   dead-code warning, so remove it from both locale files.

   **A string that needs logic** — a plural picked in code, an argument
   formatted first — goes in `packages/app/src/surface/strings/custom/<section>.rs`
   as a `pub(crate) fn`, declared in `custom/mod.rs`. Named like a key, it
   replaces the generated function for that key:

   ```rust
   // custom/flow.rs — replaces the generated `flow::pin_tooltip`
   pub(crate) fn pin_tooltip(nodes: &[daruda_flow::NodeId]) -> String {
       rust_i18n::t!("flow.pin_tooltip", nodes = node_list(nodes)).into_owned()
   }
   ```

4. **Call the function at the call site** (`s::section::key()`). Never embed
   raw string literals for user-visible text.

## YAML syntax rules

- Use double-quoted strings (`"…"`) for all values.
- Escape a literal ASCII double quote inside a value with `\"`.
- Use Unicode curly quotes (`"` U+201C, `"` U+201D) for decorative quotes that
  appear inside UI strings — do **not** use ASCII `"` inside a double-quoted
  YAML value; it closes the string and causes a parse error.
- After editing, validate **both** files:
  ```bash
  python3 -c "import yaml; yaml.safe_load(open('packages/app/locales/en.yml'))"
  python3 -c "import yaml; yaml.safe_load(open('packages/app/locales/ko.yml'))"
  ```

### ASCII quote trap (common authoring bug)

When an AI tool or text editor writes a file, curly quotes (`"` `"`) are sometimes
silently replaced with ASCII `"` (U+0022). Three adjacent ASCII double quotes
(`"""`) look like a value but parse as: open→close (empty string)→orphan `"` →
**parse error**.

**Detection**: `xxd` shows `e2 80 9c` / `e2 80 9d` for curly quotes; `22 22` in a
row signals the ASCII trap.

**Prevention**: after bulk-writing locale files, always run the Python validation
above before committing.

## Supported locales

| code | language |
|------|---------|
| `en` | English (fallback) |
| `ko` | Korean |

The `[general] language` config key (`"auto"` / `"en"` / `"ko"`) controls
which locale is active at runtime. `"auto"` follows the macOS system locale.
`daruda_config::general::SUPPORTED_LOCALES` lists the valid values.

## What NOT to put here

- Pixel sizes, color values → `packages/terminal/src/ux/theme.rs` or
  `packages/app/src/ui/theme.rs`
- Internal identifiers (action slugs, element ids, TOML keys) → inline `&str`
  literals are fine
- Strings that are never shown to users (log messages, debug output) → inline
  literals; log messages use English only

## Checklist when adding a feature with user-visible text

- [ ] String is **unique to this section** → add under the owning section; string is **already in `common:`** → reuse `common.*`; string is **reused across ≥ 2 sections** → add to `common:` first
- [ ] Key added to `en.yml`
- [ ] Matching key added to `ko.yml` with Korean translation
- [ ] Logic-bearing strings only: a `pub(crate) fn` in `surface/strings/custom/<section>.rs`
- [ ] Call sites use `s::section::key()`, not a string literal
- [ ] YAML validates (run from repo root):
  ```bash
  python3 -c "import yaml; yaml.safe_load(open('packages/app/locales/en.yml'))"
  python3 -c "import yaml; yaml.safe_load(open('packages/app/locales/ko.yml'))"
  ```
- [ ] `cargo build -p daruda` passes (proc-macro validates both locale files)
