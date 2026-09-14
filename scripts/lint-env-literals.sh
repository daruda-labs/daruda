#!/usr/bin/env bash
# Lint: daruda-owned environment names have one Rust declaration.
#
# The registry is `daruda_core::process_env`. Outside that file, any complete
# `DARUDA_*` token in a Rust string literal is a violation: readers, bootstrap
# writers, child-process injectors, examples, and tests must use a registered
# `Key`. Comments and rustdoc remain searchable documentation.
#
# `DARUDA_BIN` is shell-only and therefore is not a Rust exception. The only
# Rust exceptions are the exact PTY test-sentinel literals listed below.
#
# Usage:
#   scripts/lint-env-literals.sh
#   scripts/lint-env-literals.sh --self-test
#
# Exit codes:
#   0 — clean
#   1 — at least one forbidden literal
#   2 — discovery, registry, scanner, or argument failure

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

REGISTRY="crates/daruda_core/src/process_env.rs"

validate_registry() {
    local registry="${1-$REGISTRY}"
    if [ ! -r "$registry" ]; then
        echo "[lint-env-literals] cannot read registry: $registry" >&2
        return 2
    fi

    perl -ne '
        while (/Key::new\(([^)]*)\)/g) {
            $count++;
            my $argument = $1;
            if ($argument !~ /^\s*"(DARUDA_[A-Z0-9_]+)"\s*$/) {
                print STDERR "[lint-env-literals] invalid registry declaration: Key::new($argument)\n";
                $bad = 1;
                next;
            }
            $seen{$1}++;
        }
        END {
            if (!$count) {
                print STDERR "[lint-env-literals] registry contains no keys\n";
                exit 2;
            }
            for my $name (sort keys %seen) {
                if ($seen{$name} > 1) {
                    print STDERR "[lint-env-literals] duplicate registry value: $name\n";
                    $bad = 1;
                }
            }
            exit 2 if $bad;
        }
    ' "$registry"
}

scan_sources() {
    perl - "$@" <<'PERL'
use strict;
use warnings;

my $violations = 0;

sub is_pty_exception {
    my ($file, $content) = @_;
    return 0 unless $file =~ m{(?:^|/)crates/daruda_terminal/src/pty\.rs$};
    return $content eq 'DARUDA_TEST'
        || $content eq 'echo DARUDA_TEST\n'
        || $content eq 'Expected DARUDA_TEST in output, got: {output}';
}

sub report_literal {
    my ($file, $start_line, $content) = @_;
    while ($content =~ /\b(DARUDA_[A-Z0-9_]+)\b/g) {
        next if is_pty_exception($file, $content);
        my $line = $start_line + (substr($content, 0, $-[1]) =~ tr/\n//);
        print "$file:$line: $1 appears in a Rust string literal\n";
        $violations = 1;
    }
}

FILE:
for my $file (@ARGV) {
    next FILE if $file eq 'crates/daruda_core/src/process_env.rs';

    open my $fh, '<', $file or do {
        print STDERR "[lint-env-literals] cannot read $file: $!\n";
        exit 2;
    };
    local $/;
    my $source = <$fh>;
    close $fh;

    my $length = length $source;
    my $offset = 0;
    my $line = 1;
    my $advance = sub {
        my ($next) = @_;
        $line += (substr($source, $offset, $next - $offset) =~ tr/\n//);
        $offset = $next;
    };

    while ($offset < $length) {
        my $two = substr($source, $offset, 2);

        if ($two eq '//') {
            my $end = index($source, "\n", $offset + 2);
            $end = $length if $end < 0;
            $advance->($end);
            next;
        }

        if ($two eq '/*') {
            my $depth = 1;
            my $cursor = $offset + 2;
            while ($cursor < $length && $depth) {
                my $pair = substr($source, $cursor, 2);
                if ($pair eq '/*') {
                    $depth++;
                    $cursor += 2;
                } elsif ($pair eq '*/') {
                    $depth--;
                    $cursor += 2;
                } else {
                    $cursor++;
                }
            }
            if ($depth) {
                print STDERR "[lint-env-literals] unterminated block comment in $file:$line\n";
                exit 2;
            }
            $advance->($cursor);
            next;
        }

        # Skip a character literal, but leave lifetimes (`'a`) alone. Rust
        # character literals contain exactly one character or one escape.
        if (substr($source, $offset, 1) eq q{'}) {
            my $cursor = $offset + 1;
            if ($cursor < $length && substr($source, $cursor, 1) eq '\\') {
                $cursor += 2;
            } else {
                $cursor++;
            }
            if ($cursor < $length && substr($source, $cursor, 1) eq q{'}) {
                $advance->($cursor + 1);
                next;
            }
        }

        # Raw, raw-byte, and raw-C strings. A prefix must begin outside an
        # identifier, otherwise an `r` inside a name could be misread.
        my $rest = substr($source, $offset);
        my $previous = $offset ? substr($source, $offset - 1, 1) : '';
        if ($previous !~ /[A-Za-z0-9_]/ && $rest =~ /\A(?:br|cr|r)(\#*)"/) {
            my $hashes = $1;
            my $opening = length($&);
            my $content_start = $offset + $opening;
            my $closing = '"' . $hashes;
            my $content_end = index($source, $closing, $content_start);
            if ($content_end < 0) {
                print STDERR "[lint-env-literals] unterminated raw string in $file:$line\n";
                exit 2;
            }
            report_literal(
                $file,
                $line,
                substr($source, $content_start, $content_end - $content_start),
            );
            $advance->($content_end + length($closing));
            next;
        }

        # Ordinary, byte, and C strings. Escaped quotes do not close them.
        my $quote = -1;
        if (substr($source, $offset, 1) eq '"') {
            $quote = $offset;
        } elsif (
            $previous !~ /[A-Za-z0-9_]/
            && substr($source, $offset, 2) =~ /\A[bc]"/
        ) {
            $quote = $offset + 1;
        }
        if ($quote >= 0) {
            my $cursor = $quote + 1;
            while ($cursor < $length) {
                my $char = substr($source, $cursor, 1);
                if ($char eq '\\') {
                    $cursor += 2;
                    next;
                }
                last if $char eq '"';
                $cursor++;
            }
            if ($cursor >= $length) {
                print STDERR "[lint-env-literals] unterminated string in $file:$line\n";
                exit 2;
            }
            report_literal(
                $file,
                $line,
                substr($source, $quote + 1, $cursor - $quote - 1),
            );
            $advance->($cursor + 1);
            next;
        }

        $advance->($offset + 1);
    }
}

exit $violations;
PERL
}

expect_scan() {
    local expected="$1"
    local label="$2"
    local relative="$3"
    local source="$4"
    local fixture="$SELF_TEST_DIR/$relative"
    local output
    local actual

    mkdir -p "$(dirname "$fixture")"
    printf '%s' "$source" >"$fixture"
    if output="$(scan_sources "$fixture" 2>&1)"; then
        actual=0
    else
        actual=$?
    fi
    if [ "$actual" -ne "$expected" ]; then
        echo "[lint-env-literals] self-test failed: $label" >&2
        echo "expected $expected, got $actual" >&2
        echo "$output" >&2
        return 2
    fi
}

expect_registry() {
    local expected="$1"
    local label="$2"
    local source="$3"
    local fixture="$SELF_TEST_DIR/registry.rs"
    local output
    local actual

    printf '%s' "$source" >"$fixture"
    if output="$(validate_registry "$fixture" 2>&1)"; then
        actual=0
    else
        actual=$?
    fi
    if [ "$actual" -ne "$expected" ]; then
        echo "[lint-env-literals] registry self-test failed: $label" >&2
        echo "expected $expected, got $actual" >&2
        echo "$output" >&2
        return 2
    fi
}

self_test() {
    SELF_TEST_DIR="$(mktemp -d)"
    trap 'rm -rf "$SELF_TEST_DIR"' EXIT

    expect_registry 0 "valid declaration" \
        $'pub const A: Key = Key::new("DARUDA_A");\n'
    expect_registry 2 "duplicate value" \
        $'pub const A: Key = Key::new("DARUDA_A");\npub const B: Key = Key::new("DARUDA_A");\n'
    expect_registry 2 "malformed name" \
        $'pub const A: Key = Key::new("DARUDA-BAD");\n'
    expect_registry 2 "non-literal name" \
        $'pub const A: Key = Key::new(NAME);\n'
    expect_registry 2 "empty registry" $'pub struct Key;\n'

    expect_scan 0 "comments and external names" "clean.rs" \
        $'// "DARUDA_COMMENT_ONLY"\n/* r#"DARUDA_BLOCK_ONLY"# */\nlet _ = "CODEX_CONFIG";\n'
    expect_scan 1 "ordinary string" "ordinary.rs" \
        $'let _ = std::env::var("DARUDA_NEW_KNOB");\n'
    expect_scan 1 "raw shell string" "raw.rs" \
        $'let _ = r###"echo $DARUDA_RAW_KNOB"###;\n'
    expect_scan 1 "byte string" "bytes.rs" \
        $'let _ = b"DARUDA_BYTE_KNOB";\n'
    expect_scan 0 "exact PTY sentinels" "crates/daruda_terminal/src/pty.rs" \
        $'let _ = b"echo DARUDA_TEST\\n";\nlet _ = "DARUDA_TEST";\nlet _ = "Expected DARUDA_TEST in output, got: {output}";\n'
    expect_scan 1 "non-sentinel in PTY file" "crates/daruda_terminal/src/pty.rs" \
        $'let _ = "DARUDA_REAL_KNOB";\n'
    expect_scan 2 "scanner failure is not clean" "broken.rs" \
        $'let _ = "DARUDA_UNCLOSED;\n'

    echo "[lint-env-literals] self-test passed."
}

case "${1-}" in
    "") ;;
    --self-test)
        validate_registry
        self_test
        exit 0
        ;;
    *)
        echo "usage: scripts/lint-env-literals.sh [--self-test]" >&2
        exit 2
        ;;
esac

validate_registry

file_list="$(git ls-files --cached --others --exclude-standard -- \
    ':(glob)crates/**/*.rs' ':(glob)tools/**/*.rs')" || {
    echo "[lint-env-literals] source discovery failed" >&2
    exit 2
}

files=()
while IFS= read -r file; do
    [ -n "$file" ] || continue
    case "$file" in
        crates/gpui_component/*|crates/gpui_component_assets/*|\
        crates/gpui_component_macros/*|crates/ferrum_flow/*)
            continue
            ;;
    esac
    files+=("$file")
done <<<"$file_list"

if [ "${#files[@]}" -eq 0 ]; then
    echo "[lint-env-literals] no Rust sources discovered" >&2
    exit 2
fi

if scan_sources "${files[@]}"; then
    echo "[lint-env-literals] OK — daruda-owned Rust env names use process_env."
else
    status=$?
    if [ "$status" -eq 1 ]; then
        echo "Use daruda_core::process_env instead of a Rust string literal." >&2
    fi
    exit "$status"
fi
