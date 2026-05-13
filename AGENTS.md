# Agent Workflow Notes

Supplements `CLAUDE.md` with workflow-level rules for automated agents.
Project conventions (crate layout, code style, error handling, pitfall
rules) live in `CLAUDE.md` — this file covers only how to move work
through the repo.

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

## Verification

- Provide explicit verification steps (commands + expected outcomes)
  for every non-trivial change.
- Do not claim to have executed a command unless the tool output for
  that execution is actually visible in the conversation.
- For UI-visible behavior (rendering, IME, window state), a test suite
  pass alone is not proof — call out what still needs manual
  verification.

## Documentation

- Follow the in-progress-docs rule in `CLAUDE.md`: session handoffs,
  progress reports, and other short-lived notes do not belong in git.
- Design notes, plans, ADRs, and session handoffs go to a personal
  document store outside the repository, not `docs/` or anywhere in the
  repo.
- Repo-level `.md` is reserved for files that ship with the code
  (README, CHANGELOG, in-tree architecture docs the project explicitly
  maintains, license files). When in doubt, use the vault.

## Language

- Code, identifiers, comments, and Markdown documents: English only
  (matches `CLAUDE.md`).
- Discussion with the user: whatever language the user uses.
