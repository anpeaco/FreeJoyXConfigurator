---
description: Resume the FreeJoyX port — commit pending work, pick the next slice from Port.md, run it.
---

You are resuming autonomous work on the FreeJoyX Configurator Rust port.
Work one slice per invocation. `/loop /resume` calls this repeatedly so
the next invocation picks up where this one stopped.

## Step 1 — Orient

Run in parallel:

- Read the top of `SESSION_LOG.md` (latest entry — find "What's next").
- Read `Port.md` §5 (slice ordering) and §9 (locked decisions).
- `git status` + `git log --oneline -10`.
- `TaskList`.

The session log's most recent "What's next" section is the authoritative
pointer to the next slice. Port.md §5 is the tiebreaker if the log is
ambiguous.

## Step 2 — Commit any uncommitted work

If `git status` shows uncommitted changes:

1. Group them by logical scope (mirror the "suggested commit grouping"
   pattern at the bottom of the original SESSION_LOG bootstrap entry —
   one commit per concern: deps, codec, tests, UI, docs).
2. For each group: `git add <specific paths>` (never `-A` / `.`), then
   commit with a one-line subject describing the scope and a short body
   explaining why. Use the Co-Authored-By footer.
3. **Do NOT push.** The user pushes manually when they're ready.

If there's nothing to commit, skip.

## Step 3 — Identify the next slice

From the session log's "What's next" section, identify the next slice in
Port.md §5 order. Cross-check against TaskList — if open tasks exist
for the next slice, claim them; otherwise create tasks for the
sub-steps.

**Stop and ask the user** if:

- The next slice requires decisions outside Port.md §9's locked table.
- Hardware-at-bench verification is the next step (note it in the log
  and skip to a desk-side slice instead — don't block on hardware).
- Two slices look equally next and the log doesn't disambiguate.

## Step 4 — Work the slice

Execute the slice's sub-steps. Standard discipline:

- Mark each task `in_progress` on start, `completed` on done.
- Path B codec rules from Port.md §3 hold (no mirror struct, no unsafe).
- Run the five-command verification suite before declaring done:
  - `cargo fmt --all --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`
  - `cargo build --workspace --release`
  - `cargo run -p freejoyx-app -- list` (or the slice's specific
    end-to-end command)

If verification fails, fix it before continuing. Don't claim a slice
done with red tests.

## Step 5 — End-of-slice

1. Update `SESSION_LOG.md`:
   - Prepend a new dated entry summarising the slice (what landed,
     test count delta, notes for downstream slices, "what's next" hint).
   - Keep the prior "What's next" pointer accurate.
2. Commit the slice's work (Step 2 grouping rules).
3. Stop.

The next `/loop /resume` tick reads the updated log and starts the
slice after this one.

## Hard rules

- **Commit is autonomous; push is not.** Never `git push`.
- **Never skip the verification suite.** Five commands, every slice.
- **Never relitigate Port.md §9 locked decisions** without explicit user
  input. If a slice's design seems to require one, stop and ask.
- **No new workspace dependencies** without a clear reason
  (cross-reference Port.md's "no new deps without reason" rule).
- **Don't add comments that restate what code does** (per CLAUDE.md).
- **One slice per `/resume` invocation.** Don't chain slices in a
  single run — let the loop handle that.
