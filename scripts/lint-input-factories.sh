#!/usr/bin/env bash
# Keep raw field/action construction inside daruda_ui. Factories own geometry.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

SCANNER='
    my $raw = qr~
        \b(?:Input|Button|Select)\s*::\s*new\s*\(
        | \bgpui_component\s+as\b
        | \bgpui_component\s*::\s*(?:\{[^;]*?\b)?(?:input|button|select)\s*
          (?: ::\s*(?:(?:Input|Button|Select)\b|\{[^}]*\b(?:Input|Button|Select)\b) | as\b | [;,}] )
    ~xs;
    my $alias_constructor = 0;
    while (/\b(?:Input|Button|Select)\s+as\s+(\w+)/g) {
        my $name = $1;
        $alias_constructor ||= /\b\Q$name\E\s*::\s*new\s*\(/;
    }
    if (/$raw/ || $alias_constructor) {
        print "$ARGV: raw Input/Button/Select access; use daruda_ui factories\n";
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
        'crate::ui::Button::new(id)' \
        'use daruda_ui::Button as Action; Action::new(id)' \
        'use daruda_ui::select::Select as Choice; Choice::new(state)' \
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
        'gpui_component::input::InputState::new(window, cx)' \
        'use daruda_ui::Button as Action; struct Owner { action: Action }'; do
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
