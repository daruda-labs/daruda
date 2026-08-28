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

## `gpui-held-key-keeps-modality.patch`

Targets the **cargo git checkout** of GPUI, on the same terms as the
CJK patch above: auto-applied by `scripts/apply-gpui-patch.sh`,
idempotent on the marker `PlatformInput::KeyDown(ev) if !ev.is_held`.

One line in `Window::dispatch_event`'s input-modality decision: an
**auto-repeat no longer counts as new keyboard input**.

`last_input_modality` exists for focus-visible styling and hover
suppression, and every change to it calls `Window::refresh` — which
sets `refreshing` and so **bypasses every `AnyView::cached` for that
frame** (the one global invalidation daruda otherwise bans on a hot
path; see the root `CLAUDE.md`). Only `KeyDown` and
`MouseMove`/`MouseDown` set it; `MouseUp` leaves it alone.

So a key *held* through a mouse drag — daruda's space-to-pan on the
flow canvas — put the window in a loop: each OS auto-repeat took the
modality back to Keyboard, each of the drag's own moves handed it to
Mouse, and each flip refreshed the whole tree. At macOS's repeat rate
that is tens of full, cache-defeating repaints a second for as long as
the drag lasts. Measured on a minimal workspace, a refreshed frame
costs **2.0×** a plain one (1.11 ms → 2.26 ms, release); the real
figure is higher, since what `refresh` throws away is exactly the
cached subtrees a minimal workspace does not have.

The same flip-flop is why the pointer kept vanishing: the hide fires
inside a `key_char` KeyDown, so every repeat re-armed it.

Semantics: a repeat says nothing a first press has not already said.
The first press still claims the modality. What changes is that a key
held while the mouse is used no longer takes it back — and having moved
the mouse, Mouse is the truthful answer. Upstream has not addressed
this, and zed never reaches it: its app code uses `is_held` nowhere,
its drag-modifying gestures are all real modifiers (which arrive as
`ModifiersChanged` and do not touch the modality), and its one
hand-cursor pan — the image viewer — needs no key at all. daruda is on
this path because it wanted the canvas convention (space to pan), so
the risk here is daruda's own.

Guarded by `an_auto_repeat_does_not_flip_the_input_modality`
(`workspace/tests/flow/graph.rs`), which fails with the patch reverted.

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

## `crates/gpui_component/src/highlighter/` — vendored, **compiled-query cache**

Applied in place, like `gpui-component-root.patch` above: daruda is the
source of truth for the vendored copy, and this section is the record of
what diverges from `longbridge/gpui-component` v0.5.1.

### Why

`SyntaxHighlighter::new(lang)` compiled the language's tree-sitter queries
from scratch on every call, and `text::node::CodeBlock::new` calls it **once
per fenced code block**. Compiling is the expensive part —
`ts_query__perform_analysis` over the whole grammar — and the result is
immutable and depends only on the language, so every call after the first was
waste.

It surfaced as a stall when switching to an agent-chat pane. A pane in an
inactive tab is not rendered; gpui drops every element state a frame did not
touch (`Frame::finish`), including the `TextViewState` holding a markdown
body's parse; and `TextView::request_layout` re-parses **synchronously on the
main thread**. So one tab switch recompiled the queries for every code block
on screen. Measured in release, 1920×1080: an ordinary repaint is ~1 ms, a
switch back was ~23 ms with one visible fence and ~66 ms with three — linear
in visible fences, independent of conversation length. After the cache, ~1.3 ms
in all cases.

zed has the same work but never pays it twice: compiled queries live in
`language_core::Grammar`, built once per language and shared through
`Arc<Language>` in the `LanguageRegistry`, and the build runs on the
background executor rather than in a layout pass.

### Patch contents

1. **`highlighter/highlighter.rs`** — split `SyntaxHighlighter` into the
   per-language half (`LanguageQueries`: the compiled `Query`, the injection
   queries, the pattern indices and capture indices) and the per-instance half
   (`text` / `parser` / `tree`). `LanguageQueries` is built by
   `build_combined_injections_query` — unchanged apart from taking a
   `&LanguageConfig` instead of resolving the name itself — and handed out as
   an `Arc` from a process-wide `QUERY_CACHE` keyed by `LanguageConfig::name`.
   Resolving to the config before keying is load-bearing, not just alias dedup:
   callers include markdown fence tokens, which are arbitrary user text, and
   `LanguageRegistry::language` falls back to a built-in config rather than
   returning `None` — so keying by the caller's spelling would let a
   conversation mint unbounded entries, each paying its own compile.
   `SyntaxHighlighter::new` keeps its signature; the registry lookup and parser
   setup move to `for_language`.
2. **`highlighter/registry.rs`** — `LanguageRegistry::register` evicts the
   cache entry it replaces, so a language re-registered with different query
   sources is recompiled instead of serving a stale compilation.
3. **Observability for the regression test** — `query_compilations(language)`
   reports how many times that language has actually been compiled, so the
   guard asserts the defect (a recompile happened) rather than a wall-clock
   budget. Re-exported through `crate::ui::highlighter`; the guard is
   `crates/app/src/workspace/tests/agent_switch_cost.rs`, which registers its
   own private language so the process-global counter cannot be moved by
   another test running in parallel.

Thread-safety holds: `tree_sitter::Query` is `Send + Sync`, and nothing mutates
a query after construction (upstream's `disable_pattern` block is commented
out). A concurrent cache miss may build twice — identical result, last write
wins — which is deliberate: holding the lock across a multi-millisecond compile
would serialise every caller behind it.

### Re-vendor procedure

Copy the fresh upstream `highlighter/` in, then re-apply the split by hand.
`agent_switch_cost.rs` fails loudly if you forget.

---

## `crates/gpui_component/src/checkbox.rs` / `radio.rs` - vendored, **mixed-state checkbox**

Adds `Checkbox::indeterminate(bool)` for partial selections. The checkbox draws
`IconName::Minus`, treats checked and indeterminate as visible marks for
border/fill/animation, and resolves a mixed-state click to checked.
`radio.rs` passes `false` to the new internal helper argument.

Re-vendor by copying upstream `checkbox.rs` and `radio.rs`, then re-applying
the indeterminate flag plus the `mark_shown`, `next_checked`, and helper
argument changes. Inline tests cover the click and mark behavior.

---

## `crates/ferrum_flow/` — vendored, **six source patches**

Provenance for a vendored crate, plus the source deltas it now carries.
Re-vendoring stays a file copy followed by re-applying those.

| | |
|---|---|
| Upstream | [tu6ge/ferrum-flow](https://github.com/tu6ge/ferrum-flow) — Apache-2.0 |
| Pinned at | `43b762ced6f61313bbc2b388871bdc93f64893d0` on `master` (2026-06-05) |
| Version string | `0.3.1` — the last tag, 127 commits behind this pin. Upstream has been silent since 2026-06-07, so `master` is effectively its final state; the tag would have forgone level-of-detail node rendering and lazy paint-order traversal, both of which matter as a flow grows. |
| Copied | `crates/core/src/` → `crates/ferrum_flow/src/`, plus `crates/core/README.md` and the repository-root `LICENSE` |
| Not copied | `crates/core/examples/` — they call `Application::new()`, which no longer exists at daruda's pinned GPUI rev. Also the sibling `crates/sync_plugin` (Yrs CRDT collaboration), which daruda does not use. |

### Source patches

| Patch | File | What |
|-------|------|------|
| `FlowCanvas::viewport` accessor | `src/canvas.rs` | read-only `pub fn viewport(&self) -> &Viewport`, mirroring the existing `graph()`. `Viewport` is already exported; only the getter was missing, so a host could see *what* the graph is but not *where the canvas put it* — which is decided by the host's own plugin choice plus a drawable size only layout knows. Same role as `gpui_component`'s `visible_rows` / `code_editor_language` accessors: it is what lets a test assert the result instead of the intent. Backs `opening_a_graph_brings_every_node_into_view` (`workspace/tests/flow.rs`), which fails if the pane stops framing the graph into the drawable. |
| Unmeasured drawable fails open | `src/viewport.rs` (`is_world_bounds_visible` early return, plus a daruda-authored `mod tests` guarding it) | `Viewport::window_bounds` is `None` until GPUI layout measures the canvas in `on_children_prepainted`, and every node's and edge's visibility is decided against it. Returning `false` there culled the entire graph on the first frame — and nothing recovered it: gpui's `WindowInvalidator::invalidate_view` (`window.rs`) records the entity but **skips `dirty = true`** when `draw_phase != DrawPhase::None`, so the `cx.notify()` the canvas raises from that same prepaint cannot schedule a second frame. The canvas stayed blank until an unrelated event dirtied the window (a mouse move over it — which is why upstream's own examples look fine, and why `default_plugins()` gets away with omitting `FitAllGraphPlugin`, the only handler of `FlowEvent::DrawableBoundsReady`). Culling is an optimization, so an unknown drawable now fails open: one frame of overdraw instead of an empty canvas. Repro: `--screenshot-scenario flow-graph`; with the early return back at `false` the pane's card region collapses to the single app-canvas colour. |
| Dragged wire says yes as well as no | `src/plugins/port/interaction.rs` (`preview_tint` + its use in `PortConnecting::render`, plus a daruda-authored `mod tests` guarding it) | Upstream coloured only the **refusal**: `target_highlight` was computed solely when `validation_error` was set, and the line and dot only branched on the same flag. So a port that *would* take the drop rendered identically to empty space — the one thing a person needs to see while dragging was the one thing nothing said. The state was already there (`hovered_port` is set for every hovered candidate, valid or not), so the patch is the branch that reads it: over a valid port the wire takes `theme.success` and the port is ringed, over a refusal `theme.error` as before, over nothing upstream's two-tone neutral. Extracted into a pure `preview_tint` so the decision is testable without a window, which is also what keeps the edit inside `render` to one call. No new theme field — `FlowTheme::success` already existed and daruda maps it to the same green a passed card uses (`flow_theme`). Not reachable from outside: the active `InteractionState` is `pub(crate)` and `RenderContext` does not expose it, and a host plugin cannot even see the drag begin, since this plugin's priority (125) claims the port's `MouseDown` with `EventResult::Stop`. |
| Dangling links are opt-out | `src/plugins/port/interaction.rs` (`PortInteractionPlugin::dangling_links` + a `dangling` field threaded into `PortConnecting`, gating the `PendingLinkCommitted` emit in `on_mouse_up`) | Releasing a wire over empty space left a line to nowhere, drawn until the *next* click — and clicking its endpoint built a blank node plus an edge to it. Both are upstream features for a canvas whose nodes are created on it; daruda's come from a file, so the node has nowhere to be recorded and the line says something untrue until it is dismissed. Off, a release that landed on no port simply ends the drag. Shaped as a builder flag defaulting to `true` — the same extension point as the neighbouring `validator`, so upstream behaviour is what a caller gets without asking, and the re-vendor diff stays one field and one `if`. A drop on a port that is *refused* already ended immediately (it emits `FlowEvent::error` and returns), so only the empty-space case needed this. The consequence is separately guarded on daruda's side — `reconcile_edges` removes any node the flow file cannot name (`a_blank_node_the_file_never_named_is_taken_off_the_canvas`) — but that fires after the fact; this removes the cause. |
| A release lands where the wire said it would | `src/plugins/port/interaction.rs` (`PortConnecting::on_mouse_up` reads `hovered_port` / `validation_error` instead of hit-testing again) | The move pass decides which port a drag is over by `port_screen_big_bounds` — a 30×30 box — and the wire's colour reports that decision. The release then decided *again*, against `port_screen_bounds`, which is the port's own 12×12. So a ring roughly nine pixels wide around every port showed green and refused the drop. Now the release uses what the move pass already concluded, which is React Flow's structure rather than merely its numbers: `XYHandle` computes `closestHandle` and `isValid` on pointer-move and its `onPointerUp` reuses both, so the preview and the drop cannot disagree. Starting a drag still needs the port itself (`bounds`, on mouse-down) — strict to start and forgiving to land is the same asymmetry React Flow has, where a drag begins on the handle element but ends anywhere inside `connectionRadius`. One behaviour is dropped with the second hit-test: a refused release no longer emits `FlowEvent::error`. Nothing consumed it — the message has no reader in the vendor or in daruda — and the wire had already turned red under the cursor. |
| A drag hears its own release | `src/canvas.rs` (`on_mouse_up` steps aside for a live interaction; a `canvas()` child registers an ungated window listener that serves one) | The canvas starts an [`Interaction`] on mouse-down and ends it on mouse-up, but `div`'s mouse-up listeners are gated on `Hitbox::is_hovered` — which is false once the pointer has left (dragging a view is dragging it *away*), and, less obviously, false whenever `Window::last_input_was_keyboard` (gpui `window.rs`, in `is_hovered`). Only `KeyDown` and `MouseMove`/`MouseDown` set that modality; `MouseUp` leaves it alone. So a key held through a drag — daruda's space-to-pan, and any modifier-style hold — auto-repeats, and a release landing between two repeats was dropped: `on_mouse_up` never ran, the interaction stayed installed with nothing to end it, and the next mouse move carried on dragging with no button down. Intermittent by construction, since it turns on whether the last repeat beat the last move. The two listeners are mutually exclusive on `interaction.handler.is_some()`, so exactly one forwards any release and no plugin sees it twice. Upstream's own node and port drags have the same hole; this fixes it for all of them rather than for the one daruda noticed. |

**The other daruda-authored file is `Cargo.toml`.** It differs from
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
# list by hand, re-apply the source patches above (the viewport one's
# `mod tests` fails loudly if you forget), then:
cargo build -p ferrum_flow && cargo clippy -p ferrum_flow && cargo test -p ferrum_flow
```

### Framing is daruda's, not the vendored fit's

`FitAllGraphPlugin` is not registered. It was, briefly, and it needed a patch
to stop it magnifying a small graph to 3× — which was the signal that the
policy belonged on this side: how far out it is worth zooming is a question
about what a card still says at that size, and daruda's cards drop rows as
they shrink. `flow_graph_pane/frame.rs` holds the whole rule (fit to shrink,
never magnify, floor at 0.2) for both entry points, opening and ⌘0, and
reaches the vendor only through public API — a `Plugin` for the two events and
a `Command` for the write, since `PluginContext` exposes no viewport setter.
That patch is therefore gone rather than kept.

### Unused modules are kept, on purpose

The crate ships plugins daruda never registers — clipboard, context menu,
minimap, zoom controls, snap guides, align, select-all, focus-selection,
node interaction, and the ⌘Z history plugin. Deleting them was tried and
reverted; the measurement is here so it does not get re-tried on the
assumption that it helps.

| | |
|---|---|
| Removed | 2,576 of 15,460 lines (17%) |
| Crate rebuild | 0.60s → 0.58s. The whole crate compiles in under a second either way, so the "less to compile on a gpui bump" argument has nothing to buy |
| Inline tests lost | 4 of 66 — and those tests are the cheapest signal that a gpui bump broke vendored code |
| Re-vendor cost | copy, then re-delete 10 paths, then re-edit three *retained* files: `plugins/mod.rs`, `plugins/node/mod.rs`, and `canvas.rs` — whose `default_plugins()` would silently stop matching upstream's, which is a trap for whoever reads it next |

The risk pruning was meant to buy down — a bump breaking code we do not
use — is also weaker than it looks: the unused plugins reach for the same
gpui surface (`div`, `Element`, mouse events) as the ones daruda does
register, so a bump that breaks them very likely breaks the rest too. If a
bump ever does break only an unused module, deleting it *then* is the same
work, paid only if it is ever needed.
