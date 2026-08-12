# patches/

Tracked diffs against external sources. Two distinct flavours live
here — they serve different goals and have different application
flows.

## `gpui-ime-cjk-path-a.patch`

Targets the **cargo git checkout** of GPUI (Zed's `gpui` crate). The
checkout sits under `~/.cargo/git/checkouts/zed-*/<short-rev>/` and
is recreated by `cargo fetch`, so this patch must be re-applied each
time the cache is cleared or the GPUI rev bumps.

- Auto-applied by `scripts/apply-gpui-patch.sh`.
- Run order: `cargo fetch` → `./scripts/apply-gpui-patch.sh` → build.
- Idempotent — bails out early if the marker (`has_non_ascii_key_char`)
  is already present.

Patch contents: route non-ASCII `key_char` (Korean jamo, Japanese
kana, …) through macOS IME-first dispatch (PATH A) so composition
works during IMK Mach Port initialization delays.

## `gpui-component-input-state-ime-selection.patch`

Targets the **vendored `crates/gpui_component/src/input/state.rs`**
inside this repository. This patch is **already applied** to the file
on disk and records daruda's IME selection-range fix for re-vendoring.
After replacing `state.rs` from upstream, re-apply it from the repo
root:

```bash
git apply patches/gpui-component-input-state-ime-selection.patch
```

Patch contents: treat AppKit's `selectedRange` for `setMarkedText` as
relative to the marked string, convert those UTF-16 offsets inside the
new marked text to UTF-8 byte offsets, then store the resulting
document-relative selection. This keeps Korean/CJK IME composition
inside the document at non-zero offsets instead of feeding relative
UTF-16 offsets through the document-wide converter.

## `gpui-component-root.patch`

Targets the **vendored `crates/gpui_component/src/root.rs`** inside
this repository. Unlike the cargo-cache patch above, this one is
**already applied** to the file on disk — daruda is the source of
truth for the vendored copy. The patch file exists for two purposes:

1. **Provenance** — record exactly what daruda changed relative to
   `longbridge/gpui-component` v0.5.1 so re-vendoring is auditable.
2. **Re-application** — when bumping the vendored snapshot to a new
   upstream release, copy the fresh upstream file in, then
   `git apply patches/gpui-component-root.patch` to reinstate the
   daruda divergences in one shot.

### Re-vendor procedure

```bash
# 1. Pull fresh upstream root.rs into the vendor tree
curl -sL \
  https://raw.githubusercontent.com/longbridge/gpui-component/<tag>/crates/ui/src/root.rs \
  -o crates/gpui_component/src/root.rs

# 2. Re-apply daruda patches (from repo root)
git apply patches/gpui-component-root.patch

# 3. Rebuild
cargo build -p daruda
```

If `git apply` rejects, the upstream file has drifted in the same
hunks daruda patches. Regenerate the patch by editing `root.rs`
manually, then:

```bash
# (after fixing the file)
curl -sL <upstream-url> -o /tmp/upstream-root.rs
diff -u /tmp/upstream-root.rs crates/gpui_component/src/root.rs \
  > patches/gpui-component-root.patch
# fix the patch header paths to a/... b/... form
```

### Patch contents

Three groups of changes captured in a single diff:

1. **GPUI API signature update** — the daruda fork of `zed-industries/zed`
   takes a `cx: &mut App` parameter on `focus`, `focus_next`,
   `focus_prev`, `Window::focus`. Six call sites in `root.rs` and the
   matching function signatures (`focus_back`, `on_action_tab`,
   `on_action_tab_prev`) updated.
2. **Modal Tab containment** — `on_action_tab` /
   `on_action_tab_prev` now wrap focus inside the topmost active
   `Dialog` when one is open, so Shift+Tab off the first input no
   longer leaks into terminal panes / sidebar behind the modal
   overlay. Adds a `focus_within(handle, window, cx)` helper.
3. **Drop theme overrides + window_border** in `Render::render` —
   daruda owns its own theme (`daruda_terminal::ux::theme`) and the
   window has native macOS chrome. Removes `ActiveTheme` and
   `window_border` imports along with the unused
   `set_rem_size` / `font_family` / `bg` / `text_color` /
   `window_border()` calls.

---

## `crates/ferrum_flow/` — vendored, **no source patch**

Not a patch: a record of provenance for a vendored crate that carries
zero source delta, so re-vendoring stays a plain file copy.

| | |
|---|---|
| Upstream | [tu6ge/ferrum-flow](https://github.com/tu6ge/ferrum-flow) — Apache-2.0 |
| Pinned at | `43b762ced6f61313bbc2b388871bdc93f64893d0` on `master` (2026-06-05) |
| Version string | `0.3.1` — the last tag, 127 commits behind this pin. Upstream has been silent since 2026-06-07, so `master` is effectively its final state; the tag would have forgone level-of-detail node rendering and lazy paint-order traversal, both of which matter as a flow grows. |
| Copied | `crates/core/src/` → `crates/ferrum_flow/src/`, plus `crates/core/README.md` and the repository-root `LICENSE` |
| Not copied | `crates/core/examples/` — they call `Application::new()`, which no longer exists at daruda's pinned GPUI rev. Also the sibling `crates/sync_plugin` (Yrs CRDT collaboration), which daruda does not use. |

**The only daruda-authored file is `Cargo.toml`.** It differs from
upstream's in four ways, none of which touch source:

1. `gpui = { workspace = true }` — the reason this crate is vendored at
   all. Upstream takes `gpui` from crates.io, which resolves to a
   *different* crate instance than daruda's pinned zed git rev, so the
   types would not unify.
2. `image` / `futures` / `uuid` declared locally at the versions already
   in `Cargo.lock`, since they are not in `workspace.dependencies`. Same
   shape as `gpui_component`'s own local `uuid`.
3. `[lints]` — every clippy and rustc group set to `allow` with
   `priority = -1`, matching the "vendored upstream, do not lint" stance
   used for `gpui_component`. The `priority` is load-bearing: without it
   the one lint deliberately left armed (below) is buried by the groups.
4. `rust-version = "1.95"` plus `incompatible_msrv = "deny"` — unlike
   `gpui_component`, this crate declares the floor. The manifest is
   daruda's own file, so declaring it costs nothing when re-vendoring,
   and it lets `cargo clippy -p ferrum_flow` catch a std API newer than
   CI's pinned toolchain without installing that toolchain locally.
   Note the check only sees std API stabilization, not language
   features; CI's 1.95 pin remains the final gate.

### Re-vendor procedure

```bash
git clone https://github.com/tu6ge/ferrum-flow /tmp/ff && cd /tmp/ff
git checkout <new-commit>
rm -rf <daruda>/crates/ferrum_flow/src
cp -R crates/core/src <daruda>/crates/ferrum_flow/src
cp crates/core/README.md LICENSE <daruda>/crates/ferrum_flow/
# Cargo.toml is daruda's — reconcile it against upstream's dependency
# list by hand, then:
cargo build -p ferrum_flow && cargo clippy -p ferrum_flow && cargo test -p ferrum_flow
```
