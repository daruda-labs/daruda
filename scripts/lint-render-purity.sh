#!/usr/bin/env bash
# Keep the workspace frame split: everything a frame changes belongs in
# `prepare_frame`, and `build_frame` takes `&self` so the compiler keeps it
# from writing `Workspace`'s own fields. That borrow says nothing about other
# entities — `&mut Context` still reaches them — so this guards what the
# compiler cannot:
#
#   1. `build_frame` keeping its shared borrow — flip it to `&mut self` and
#      every mutation the split moved out can quietly come back.
#   2. `Render::render` staying a two-line dispatch — a statement added there
#      runs with `&mut self` and is outside both guards.
#   3. No dock staging on the build side. Staging writes into a dock entity
#      through `cx`, which `&self` allows, and it belongs in `prepare_frame`
#      behind the check for whether the docks are on screen at all.
#
# None of these stops a frame *reading* an entity it does not draw, which
# gpui counts as displaying it (AGENTS.md pitfall 10). Behind Settings that is
# kept out by structure — `prepare_frame` picks the frame body once and the
# settings frame builds nothing it covers — and pinned by the render-count
# tests in `crates/app/src/workspace/tests/settings_view.rs`.
set -euo pipefail

FILE="crates/app/src/workspace/render/mod.rs"

if [ ! -f "$FILE" ]; then
    echo "lint-render-purity: $FILE is gone — update or delete this guard." >&2
    exit 1
fi

# 1. The shared borrow.
if ! rg -qU 'fn build_frame\(\s*\n\s*&self,' "$FILE"; then
    echo "build_frame must take &self — that borrow is what keeps a frame from" >&2
    echo "mutating. Put the change in prepare_frame instead." >&2
    exit 1
fi

# 2. The dispatch. Statement lines only: comments, attributes, the signature
#    and the closing brace are not what we are counting.
body=$(awk '
    /fn render\(&mut self, window: &mut Window, cx: &mut Context<Self>\)/ { inside = 1; next }
    inside && /^    \}$/ { exit }
    inside { print }
' "$FILE" | rg -v '^\s*(//|#\[|$)')

expected='        WORKSPACE_RENDERS.with(|n| n.set(n.get() + 1));
        let prep = self.prepare_frame(window, cx);
        self.build_frame(prep, window, cx)'

if [ "$body" != "$expected" ]; then
    echo "Render::render must stay a dispatch to prepare_frame + build_frame." >&2
    echo "Anything added there runs with &mut self, outside both guards." >&2
    echo "--- found ---" >&2
    echo "$body" >&2
    echo "--- expected ---" >&2
    echo "$expected" >&2
    exit 1
fi

# 3. Staging. The build side runs from `fn build_frame(` to the free function
#    that follows the impl block.
build=$(awk '
    /fn build_frame\(/ { inside = 1 }
    inside && /^fn root_key_context\(/ { exit }
    inside { print }
' "$FILE")
if [ -z "$build" ]; then
    echo "lint-render-purity: could not find the build side of $FILE —" >&2
    echo "update the region markers in this guard." >&2
    exit 1
fi
if echo "$build" | rg -n '\.stage\(|stage_docks\(' >&2; then
    echo "Dock staging belongs in prepare_frame, not the build side of the frame." >&2
    exit 1
fi

echo "✓ Workspace frame stays split: build_frame is &self, render only dispatches, build stages nothing."
