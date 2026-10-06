#!/usr/bin/env bash
# Advisory only: review new numeric aliases, not the legacy backlog.
# Usage: bash scripts/report-ui-token-aliases.sh [base-ref | --self-test]
set -euo pipefail
cd "$(dirname "$0")/.."

exec perl - "$@" <<'PERL'
use strict;
use warnings;

sub alias_in {
    my ($line) = @_;
    return if $line =~ m{//\s*token-role:\s*\S};
    return unless $line =~ /^\s*pub(?:\([^)]*\))?\s+const\s+(\w+)\s*:
        \s*(?:f32|f64|usize|u8|u16|u32|u64|isize|i8|i16|i32|i64)
        \s*=\s*((?:\w+::)*[A-Z][A-Z0-9_]*)\s*;/x;
    return "$1 = $2";
}

if (@ARGV && $ARGV[0] eq '--self-test') {
    my @cases = (
        ['pub const ROW_GAP: f32 = GAP_SM;', 'ROW_GAP = GAP_SM'],
        ['pub(crate) const ROW_PAD: f32 = theme::PAD_XS;', 'ROW_PAD = theme::PAD_XS'],
        ['pub const STATE: Hsla = ERROR;', undef],
        ['pub const WIDTH: f32 = 24.0;', undef],
        ['pub const WIDTH: f32 = PAD_XS * 2.0;', undef],
        ['pub const MIN_W: f32 = CONTROL_TARGET_SIZE; // token-role: must fit a control', undef],
        ['pub const ROW_GAP: f32 = GAP_SM; // token-role:', 'ROW_GAP = GAP_SM'],
    );
    for my $case (@cases) {
        my $got = alias_in($case->[0]);
        die "Unexpected alias report: $case->[0]\n"
            unless ($got // '') eq ($case->[1] // '');
    }
    print "UI token alias report self-test passed.\n";
    exit 0;
}
die "Expected at most one base ref\n" if @ARGV > 1;
my $base = @ARGV ? $ARGV[0] : 'HEAD';
die "Invalid base ref\n" if $base =~ /^-/;
my $pathspec = ':(glob)packages/ui/src/**/*.rs';
open my $diff, '-|', 'git', 'diff', '--no-ext-diff', '--no-textconv',
    '--no-color', '--src-prefix=a/', '--dst-prefix=b/',
    '--unified=0', $base, '--', $pathspec or die "git diff: $!\n";
my ($path, $line, $count) = ('', 0, 0);
sub report {
    my ($path, $line, $text) = @_;
    if (my $alias = alias_in($text)) {
        print "$path:$line: review numeric alias $alias\n";
        $count++;
    }
}
while (<$diff>) {
    if (m{^\+\+\+ b/(.*)\n$}) { $path = $1; next; }
    if (/^@@ .* \+(\d+)/) { $line = $1; next; }
    if (/^\+(.*)/) { report($path, $line++, $1); }
}
close $diff or die "git diff failed\n";
open my $untracked, '-|', 'git', 'ls-files', '--others', '--exclude-standard',
    '-z', '--', $pathspec or die "git ls-files: $!\n";
{
    local $/ = "\0";
    while (my $path = <$untracked>) {
        chomp $path;
        open my $source, '<', $path or die "$path: $!\n";
        local $/ = "\n";
        my $line = 0;
        while (<$source>) { report($path, ++$line, $_); }
        close $source;
    }
}
close $untracked or die "git ls-files failed\n";
print "UI token aliases: $count new candidate(s); advisory only.\n";
PERL
