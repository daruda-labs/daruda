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
