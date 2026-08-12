# End-to-end scenario

Everything else about this crate is green against a scripted adapter and a
fake runner. These five flows are the part that only a real agent can answer,
so this is a checklist for a person, not a test suite.

```bash
cargo run -p daruda_flow --example run_flow -- \
  crates/daruda_flow/examples/flows/01-smoke.yaml <a scratch repo>
```

Run them **in a throwaway git repo**, not this one — an agent node writes to
the working tree, and step 3 asks one to create a file.

`DARUDA_FLOW_AGENT` overrides the adapter command; unset, it launches the
same one the app does.

**All five passed against a real agent on 2026-08-10**, along with the
cancel and cleanliness checks below. The list stays because it is what to
re-run when the runners change — a green suite says nothing about any of it.

> **`node_install_dir` must sit outside the repository.** The `.gitignore`
> the engine writes covers `flow-runs/` only, so a managed runtime unpacked
> next to it turns up in `git status`. The example learned this the hard way;
> the app already puts it under its own data directory.

---

## 1 — `01-smoke.yaml` · does a session open at all

One agent node, one file.

- [x] the run reaches `hello passed`
- [x] `run_dir/hello.md` exists and is not empty
- [ ] `run_dir/logs/` holds this attempt's artifacts — a transcript named
      like the command log beside it (`hello.attempt-1.evidence-N.md`),
      holding the prompt, what the agent said, and how the turn ended

**If it fails here**, nothing below will tell you anything new — the adapter,
the provisioning closure, or the file contract is the problem, and `run.md`
names which.

> This is also the first time `execute`'s real provisioning closure runs.
> Nothing in the suite covers that line.

## 2 — `02-mixed.yaml` · an agent and a gate in one run

- [x] both nodes run, in order
- [x] the gate passes — it reads the agent's output through a template

**Why this shape matters**: until `Runners` existed, each runner refused half
the grammar, so a mixed flow only ever worked with the fake. The gate also
exercises shell quoting: `run_dir` contains the repo path, and an unquoted
substitution splits on a space. **Run it from a directory whose path has a
space in it** and the check is real.

## 3 — `03-repair.yaml` · the design's flagship path

The gate fails, a fix session runs, the rerun set is re-derived.

- [x] the event stream reads:
      `verdict → gate(failed) → fixing for gate → fix done → gate re-derives: verdict → verdict → gate(passed)`
- [x] `run_dir/logs/` holds the **archived** first attempt —
      `verdict.attempt-1.evidence-*.md` — separate from the live `verdict.md`
- [x] `run.md` lists both of `verdict`'s attempts and says what the failure
      invalidated (`re-derived \`verdict\`, \`gate\``)
- [x] the fix session appears in `run.md` as `__fix__`, and the attempt
      counts add up to the session count

**What is being judged is the engine's sequence, not the agent's work.** The
prompt tells `verdict` to fail the first time and to look for the file the
fix creates the second, so the repair happens on demand.

## 4 — `04-prompt-file.yaml` · the record is self-contained

- [x] the node runs
- [x] **`run_dir/run.yaml` contains the prompt's text, not `prompt_file:`**
- [x] editing `prompts/note.md` afterwards does not change `run.yaml`
- [x] copying `run.yaml` elsewhere and running it produces the same flow

A path would be wrong twice: it resolves against the flow file's directory,
not the run directory the record lands in, and the file it names can change.

## 5 — `05-bad-model.yaml` · a setting that could not be applied

- [x] the node **does not run** — no `never.md`
- [x] `run.md` says the model was not on offer, and names what was
- [x] the run ends `Failed`, and `FAILED` is in the run directory

**Recording a model the session never used is worse than not running**, which
is why this fails instead of falling back.

---

## Then, once through, on any of them

- [x] **Ctrl-C mid-run**: the run stops, `CANCELED` is written, and the
      adapter the run started is gone. An agent that had written nothing yet
      has nothing to archive, which is what this one showed
- [x] **`git status`**: shows your own edits and nothing from the run —
      no `.lock`, no run directory
- [x] **`run.md` reads like something you would want to find** after a run
      you were not watching. This is the one item with no mechanical answer,
      and the reason this step exists.

## Known to be slow, not broken

A model or effort the adapter *rejects* (rather than not advertising) costs
the full settings budget before the node fails: `daruda_acp` downgrades a
rejection to an untyped `Notice`, so the runner has nothing to observe but
the budget expiring. The outcome is right; only the wait is wrong. Fixing it
needs a typed rejection event in `daruda_acp`.
