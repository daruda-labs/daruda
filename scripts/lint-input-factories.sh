#!/usr/bin/env bash
# Keep raw text-input construction inside daruda_ui. The public factories
# own geometry; their opaque return types prevent per-screen padding overrides.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

SCANNER='
    my $raw = qr~
        \bInput\s*::\s*new\s*\(
        | \bgpui_component\s+as\b
        | \bgpui_component\s*::\s*(?:\{[^;]*?\b)?input\s*
          (?: ::\s*(?:Input\b|\{[^}]*\bInput\b) | as\b | [;,}] )
    ~xs;
    if (/$raw/) {
        print "$ARGV: raw Input access; use daruda_ui input factories\n";
        $found = 1;
    }
    END { exit($found ? 1 : 0); }
'

scan() {
    perl -0777 -ne "$SCANNER" "$@"
}

if [ "${1-}" = "--self-test" ]; then
    for sample in \
        'crate::ui::Input::new(state)' \
        'use gpui_component::input::Input as Field;' \
        'use gpui_component::{input::{Input as Field}};' \
        'use gpui_component::input as fields;' \
        'use gpui_component::{input as fields};' \
        'use gpui_component as widgets;' \
        $'use gpui_component::input::{\nInput as Field, InputState\n};' \
        $'Input\n::\nnew(state)'; do
        if printf '%s' "$sample" | scan >/dev/null; then
            echo "Input factory lint missed: $sample" >&2
            exit 1
        fi
    done
    for sample in \
        'crate::ui::form_input(state, cx, 0)' \
        'gpui_component::input::InputState::new(window, cx)'; do
        if ! printf '%s' "$sample" | scan >/dev/null; then
            echo "Input factory lint rejected: $sample" >&2
            exit 1
        fi
    done
    echo "Input factory lint self-test passed."
    exit 0
fi

find packages -type f -name '*.rs' ! -path 'packages/ui/*' -print0 \
    | xargs -0 perl -0777 -ne "$SCANNER"
