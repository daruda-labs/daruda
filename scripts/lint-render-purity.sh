#!/usr/bin/env bash
# Keep the workspace frame split: everything a frame changes belongs in
# `prepare_frame`, and `build_frame` takes `&self` so the compiler enforces
# the rest. This guards the two things the compiler cannot:
#
#   1. `build_frame` keeping its shared borrow — flip it to `&mut self` and
#      every mutation the split moved out can quietly come back.
#   2. `Render::render` staying a two-line dispatch — a statement added there
#      runs with `&mut self` and is outside both guards.
#
# Why it matters: the audit the split forced found three mutations hidden in
# 1,100 lines, and one of them (staging a snapshot into a dock Settings had
# covered) re-registered an off-screen entity with gpui and cost a full
# workspace render four times a second. See AGENTS.md pitfall 10.
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

echo "✓ Workspace frame stays split: build_frame is &self, render only dispatches."
