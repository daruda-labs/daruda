#!/usr/bin/env bash
# Lint: a raw mouse listener must say which button it answers.
#
# GPUI has two tiers of mouse input, with opposite defaults:
#
#   * `div().on_click()` / `.on_mouse_down(MouseButton::X, ..)` — the button
#     is part of the API. `on_click` is left-only by construction; every
#     other button is routed to `on_aux_click` (gpui `elements/div.rs`).
#   * `window.on_mouse_event::<MouseDownEvent>()` — the raw firehose. It
#     hears every button, and the filter is a field you have to remember to
#     read.
#
# A hand-rolled `Element` has no `div` to hang interactivity on, so it can
# only use the raw tier — which is why every such listener re-derives the
# filter, and why forgetting it is silent. Three shipped bugs came from
# exactly that: a right click opened an agent-chat link, jumped any
# scrollbar (and swallowed the host context menu), and dropped a live
# flow-editor wire.
#
# This lint flags a `MouseDownEvent` / `MouseUpEvent` listener whose body
# never mentions `MouseButton` or `.button`. A listener that genuinely
# answers every button says so:
#
#     // ANY-BUTTON: any press outside the popover dismisses it.
#
# Vendored crates are scanned deliberately — all three bugs lived there,
# and a re-vendor drops the markers, which makes this lint the re-vendor
# checklist for this class.
#
# Comments and string literals are stripped before both the search and the
# button test, so a `// TODO: check MouseButton?` cannot pass a listener and
# a brace inside a string cannot derail the body scan. `--self-test` drives
# the known-hard shapes through the detector.
#
# Usage:
#   scripts/lint-raw-mouse-button.sh
#   scripts/lint-raw-mouse-button.sh --self-test
#
# Exit codes:
#   0 — every raw down/up listener filters or is marked
#   1 — at least one unfiltered, unmarked listener (printed)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

SCANNER="$(mktemp)"
trap 'rm -f "$SCANNER"' EXIT

cat > "$SCANNER" <<'PY'
import re
import sys
from pathlib import Path

# The raw tier is `window.on_mouse_event(..)` and nothing else: the safe
# tier (`.on_mouse_down(MouseButton::X, ..)`) names the button in the call,
# so its closure looks identical and must not be flagged. Anchoring on the
# call rather than on the closure signature is what keeps the two apart.
EVENT = r"(?:[A-Za-z_][A-Za-z0-9_]*::)*Mouse(?:Down|Up)Event"
CALL = re.compile(rf"\bon_mouse_event\s*(?:::<\s*(?P<turbofish>{EVENT})\s*>\s*)?\(")
# `move` is optional and the type may be path-qualified.
PARAM = re.compile(rf"\|\s*[_A-Za-z0-9]+\s*:\s*&{EVENT}\b")
MARKER = re.compile(r"ANY-BUTTON:\s*\S")
FILTER = re.compile(r"MouseButton|\.button\b")


def blank_noise(src: str) -> str:
    """Replace comments and string/char literals with spaces, keeping every
    byte offset and newline — so offsets still map to the original text."""
    out = list(src)
    i, n = 0, len(src)
    while i < n:
        c = src[i]
        if c == "/" and i + 1 < n and src[i + 1] == "/":
            while i < n and src[i] != "\n":
                out[i] = " "
                i += 1
        elif c == "/" and i + 1 < n and src[i + 1] == "*":
            depth, start = 1, i
            out[i] = out[i + 1] = " "
            i += 2
            while i < n and depth:
                if src.startswith("/*", i):
                    depth += 1
                    out[i] = out[i + 1] = " "
                    i += 2
                elif src.startswith("*/", i):
                    depth -= 1
                    out[i] = out[i + 1] = " "
                    i += 2
                else:
                    if src[i] != "\n":
                        out[i] = " "
                    i += 1
            if depth:
                i = start + 2
        elif c == 'r' and (m := re.match(r'r(#*)"', src[i:])):
            close = '"' + m.group(1)
            end = src.find(close, i + len(m.group(0)))
            end = n if end < 0 else end + len(close)
            for j in range(i, end):
                if src[j] != "\n":
                    out[j] = " "
            i = end
        elif c in '"\'':
            # A lifetime (`&'a T`) is not a char literal; it has no closer.
            if c == "'" and re.match(r"'[A-Za-z_][A-Za-z0-9_]*[^']", src[i:]):
                i += 1
                continue
            j = i + 1
            while j < n and src[j] != c:
                j += 2 if src[j] == "\\" else 1
            for k in range(i, min(j + 1, n)):
                if src[k] != "\n":
                    out[k] = " "
            i = j + 1
        else:
            i += 1
    return "".join(out)


def call_region(code: str, open_paren: int) -> str:
    """The call's argument list: from its `(` to the matching `)`. Balanced,
    so a nested call or closure inside it is included whole."""
    depth, i, n = 0, open_paren, len(code)
    while i < n:
        c = code[i]
        if c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
            if depth == 0:
                return code[open_paren : i + 1]
        i += 1
    return code[open_paren:]


def comment_block_above(lines: list[str], line_no: int) -> str:
    """The unbroken run of `//` comments and blank lines directly above
    `line_no` (1-indexed). Stops at the first line of code."""
    block = []
    i = line_no - 2
    while i >= 0:
        stripped = lines[i].strip()
        if stripped and not stripped.startswith("//"):
            break
        block.append(lines[i])
        i -= 1
    return "\n".join(block)


def scan(path: Path) -> list[str]:
    src = path.read_text(encoding="utf-8", errors="replace")
    code = blank_noise(src)
    lines = src.splitlines()
    hits = []
    for call in CALL.finditer(code):
        region = call_region(code, call.end() - 1)
        # Which event this listener is for: the turbofish if it has one,
        # otherwise the closure's own annotation. Neither means it is some
        # other event (move, scroll) and not this lint's business.
        if not call.group("turbofish") and not PARAM.search(region):
            continue
        if FILTER.search(region):
            continue
        line_no = code.count("\n", 0, call.start()) + 1
        # The marker may sit inside the listener, or in the comment block
        # written directly above it. "Directly" is the point: a marker
        # separated by a line of code belongs to that code, not to this
        # listener, so one marker cannot cover its neighbour.
        within = src[call.start() : call.start() + len(region)]
        if MARKER.search(within) or MARKER.search(comment_block_above(lines, line_no)):
            continue
        hits.append(f"{path}:{line_no}: {lines[line_no - 1].strip()}")
    return hits


def scan_tree(root: Path) -> list[str]:
    hits = []
    for path in sorted(root.rglob("*.rs")):
        parts = path.parts
        if "tests" in parts or path.name == "tests.rs" or path.name.endswith("_tests.rs"):
            continue
        hits.extend(scan(path))
    return hits


SELF_TEST = {
    # Every shape that must be caught. The awk detector this replaced missed
    # all of these; they are the regression suite for the detector itself.
    "turbofish.rs": ("window.on_mouse_event::<MouseDownEvent>(move |ev, p, _, cx| { go(ev); });", 1),
    "qualified.rs": ("window.on_mouse_event(move |ev: &gpui::MouseDownEvent, p, _, cx| { go(ev); });", 1),
    "no_move.rs": ("window.on_mouse_event(|ev: &MouseUpEvent, p, _, cx| { go(ev); });", 1),
    "comment_only.rs": (
        "window.on_mouse_event(move |ev: &MouseDownEvent, p, _, cx| {\n"
        "    // TODO: should this check MouseButton?\n    go(ev);\n});",
        1,
    ),
    "brace_in_string.rs": (
        'window.on_mouse_event(move |ev: &MouseDownEvent, p, _, cx| {\n'
        '    log("}}}} unbalanced");\n    go(ev);\n});',
        1,
    ),
    # A braceless listener must not blind the scan that follows it.
    "braceless_then_real.rs": (
        "// ANY-BUTTON: ends a drag, any release will do.\n"
        "window.on_mouse_event(move |ev: &MouseUpEvent, _, _, cx| end_drag(ev, cx));\n"
        "window.on_mouse_event(move |ev: &MouseDownEvent, p, _, cx| { go(ev); });",
        1,
    ),
    # Must stay silent.
    "gated.rs": (
        "window.on_mouse_event(move |ev: &MouseDownEvent, p, _, cx| {\n"
        "    if ev.button != MouseButton::Left { return; }\n    go(ev);\n});",
        0,
    ),
    "marked.rs": (
        "// ANY-BUTTON: any press outside dismisses it.\n"
        "window.on_mouse_event(move |ev: &MouseDownEvent, p, _, cx| { go(ev); });",
        0,
    ),
    "not_a_listener.rs": ('let lifetime: &\'a str = "MouseDownEvent";', 0),
    # The safe tier names the button in the call; its closure is identical.
    "safe_tier.rs": (
        ".on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, window, cx| { go(cx); });",
        0,
    ),
    # A move listener is a different event and none of this lint's business.
    "other_event.rs": ("window.on_mouse_event(move |ev: &MouseMoveEvent, p, _, cx| { go(ev); });", 0),
}


def self_test() -> int:
    import tempfile

    failures = []
    with tempfile.TemporaryDirectory() as tmp:
        for name, (body, expected) in SELF_TEST.items():
            path = Path(tmp) / name
            path.write_text(f"fn probe() {{\n{body}\n}}\n", encoding="utf-8")
            found = len(scan(path))
            if found != expected:
                failures.append(f"  {name}: expected {expected} hit(s), found {found}")
            path.unlink()
    if failures:
        print("lint-raw-mouse-button --self-test FAILED", file=sys.stderr)
        print("\n".join(failures), file=sys.stderr)
        return 1
    print(f"✓ self-test: {len(SELF_TEST)} detector cases behave as specified.")
    return 0


if __name__ == "__main__":
    if sys.argv[1:2] == ["--self-test"]:
        sys.exit(self_test())
    found = scan_tree(Path("crates"))
    if found:
        print("\n".join(found))
        sys.exit(1)
PY

if [[ "${1-}" == "--self-test" ]]; then
    exec python3 "$SCANNER" --self-test
fi

if [[ ! -d crates ]]; then
    echo "lint-raw-mouse-button: crates/ not found — run from the repo root." >&2
    exit 2
fi

if ! HITS=$(python3 "$SCANNER"); then
    echo "lint-raw-mouse-button: raw mouse listeners that answer every button" >&2
    echo "" >&2
    echo "$HITS" >&2
    echo "" >&2
    echo "Filter on event.button (gpui's own on_click is left-only), or mark the" >&2
    echo "listener with '// ANY-BUTTON: <reason>' if every button is intended." >&2
    exit 1
fi

echo "✓ Every raw MouseDown/MouseUp listener filters on a button or is marked."
