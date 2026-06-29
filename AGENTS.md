# Agent Workflow Notes

Workflow-level rules for automated agents. Project conventions (crate
layout, code style, error handling, pitfall rules) live in the project
guide — this file covers only how to move work through the repo.

## Branch & commit

- Work on the `main` branch unless the user asks for a feature branch.
- Keep commits small and reviewable. Prefer one logical change per
  commit.
- Only commit when the user asks. Never push to `origin` without an
  explicit request from the user.

## Pre-commit checks

Run these locally and make them pass before committing:

```sh
cargo fmt --all -- --check
cargo clippy -p ghostty_vt -p ghostty_vt_sys -p daruda_terminal -p daruda \
  --all-targets -- -D warnings
cargo test
```

If a hook fails, fix the underlying issue rather than bypassing it
(`--no-verify`, `-c commit.gpgsign=false`, etc. are off-limits unless
the user explicitly asks).

## Scope discipline

- Implement only what the current task explicitly requires. No
  speculative refactors or abstractions.
- When a refactor is needed to keep a change reviewable, do it in a
  separate commit with a clear reason.
- Don't leave half-finished code paths in the repo; if a change cannot
  land end-to-end, pause and surface the blocker instead.

## Change-impact discipline

Before editing a shared or public type, decide the *extension point* — not
just the change — and pick the one with the smallest blast radius.

- Before changing any shared/public type, function signature, or enum,
  grep all usages and call sites first. Don't assume isolated impact;
  state the blast radius (how many sites this forces you to touch).
- Prefer additive, backward-compatible changes (a new optional field /
  parameter, a new opt-in hook) over invasive ones (a new enum variant, a
  changed signature). A host-specific behavior belongs in an opt-in
  callback installed where it's needed — not in a shared enum that every
  exhaustive `match` must now grow an arm for.
- If a single logical change forces the *same mechanical edit* across many
  unrelated files (Shotgun Surgery — e.g. an identical no-op `match` arm
  added in N places), stop: the seam is wrong, redesign rather than push
  the churn through. Copy-pasting the same edit a second time is the tell.
- Look for an existing precedent in the same module before inventing a new
  mechanism, and mirror it only after checking *why* it's shaped that way
  (an existing enum variant is cheap to match; a *new* one is a breaking
  change to every consumer).
- When two designs are equally correct, the one that touches fewer sites on
  the next change wins (Correctness > Maintainability).

## Verification

- Provide explicit verification steps (commands + expected outcomes)
  for every non-trivial change.
- Do not claim to have executed a command unless the tool output for
  that execution is actually visible in the conversation.
- For UI-visible behavior (rendering, IME, window state), a test suite
  pass alone is not proof — call out what still needs manual
  verification.

## Documentation

- In-progress docs stay out of the repo: session handoffs, progress
  reports, and other short-lived notes do not belong in git.
- Design notes, plans, ADRs, and session handoffs go to a personal
  document store outside the repository, not `docs/` or anywhere in the
  repo.
- Repo-level `.md` is reserved for files that ship with the code
  (README, CHANGELOG, in-tree architecture docs the project explicitly
  maintains, license files). When in doubt, use the vault.

## Language

- Code, identifiers, comments, and Markdown documents: English only.
- Discussion with the user: whatever language the user uses.
