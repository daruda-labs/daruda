#!/usr/bin/env bash
# Authoring modals use form_input/form_select; picker queries use search_input.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
SCANNER='
    next unless $ARGV eq "-" || $ARGV =~ m{(?:^|/)[^/]*(?:modal|dialog)[^/]*(?:/|\.rs)};
    s{//[^\n]*}{}g;
    s{/\*.*?\*/}{}gs;
    my $form_alias = /\bform_input\s+as\s+input\b/;
    s/\bform_input\s+as\s+input\b//g;
    my $wrong = qr~
        \b(?:ui|daruda_ui)\s*::\s*input(?:_on)?\s*\(
        | \bselect\s*::\s*select\s*\(
        | \buse\s+(?:crate::ui|daruda_ui)\s*::\s*
          (?: input(?:_on)?\b(?!\s*::) | \{[^;]*?\binput(?:_on)?\b(?!\s*::) )
        | \buse\s+(?:crate::ui|daruda_ui)::select::(?:select\b|\{[^;]*?\bselect\b)
    ~xs;
    if (/$wrong/ || (!$form_alias && /(?<![.\w])input\s*\(/)) {
        print "$ARGV: authoring modal uses a standard field; use form_input/form_select\n";
        $found = 1;
    }
    END { exit($found ? 1 : 0); }
'
scan() { perl -0777 -ne "$SCANNER" "$@"; }
if [ "${1-}" = "--self-test" ]; then
    for sample in \
        'crate::ui::input(state, cx, 0)' \
        'use crate::ui::input as field;' \
        $'use crate::ui::{\nInputState, input\n};' \
        'select::select(state, cx, 0)' \
        'use crate::ui::select::select as choice;'; do
        if printf '%s' "$sample" | scan >/dev/null; then
            echo "Modal field lint missed: $sample" >&2
            exit 1
        fi
    done
    for sample in \
        'crate::ui::form_input(state, cx, 0)' \
        'crate::ui::search_input(state, cx, 0)' \
        'use crate::ui::{form_input as input}; input(state, cx, 0)' \
        'select::form_select(state, cx, 0)'; do
        if ! printf '%s' "$sample" | scan >/dev/null; then
            echo "Modal field lint rejected: $sample" >&2
            exit 1
        fi
    done
    echo "Modal field lint self-test passed."
    exit 0
fi
find packages/app/src -type f -name '*.rs' -print0 | xargs -0 perl -0777 -ne "$SCANNER"
