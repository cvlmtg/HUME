# HUME — Lessons Learned

Patterns that bit us; rules to prevent recurrence.

---

## L1 — Side-effect cluster regression (2026-06)

**Root cause:** A refactor added a *second* code path for an operation whose
correctness depends on a *cluster* of side effects, not just the primary effect.
The old path kept its inline bookkeeping; the new path was a bare copy of the
execution match without any of the surrounding bookkeeping.  Tests pinned the
primary effect (cursor moved, text changed) and stayed green on both paths.  The
entire cluster (jump list, `last_command`, dot-repeat, paste-session commit,
register routing) regressed silently on the new path.

**Concrete instance:** `b7a5af0` added `run_command_sync` for Steel `(call!)`.
Cursor/text assertions passed.  Nine bookkeeping regressions shipped.

**Prevention rules:**

1. **Path parity test** — whenever a refactor adds or forks a dispatch path,
   add a parity test asserting that both paths leave identical state (use a
   `BookkeepingSnapshot`-style helper that captures the whole cluster).  Never
   assert only the primary effect.

2. **Single-funnel lint** — all execution of native-command `fun` fields must
   go through `run_native_body` in `commands/mod.rs` (wrapped by
   `run_dispatch_pipeline` for bookkeeping).  The lint
   `single_native_dispatch_funnel` in `lints.rs` enforces this: any second
   `match cmd` that binds a native variant's `fun` outside that file fails the
   build.

3. **Duplicate-match smell** — two identical `match cmd { Motion { fun } | … }`
   arms in different files is a SSOT violation.  Collapse to one funnel.

**Files:** `hume-editor/src/editor/commands/mod.rs` (funnel),
`hume-editor/src/editor/lints.rs` (lint), `hume-editor/src/editor/tests/mod.rs`
(snapshot helper), `hume-editor/src/editor/tests/sync_dispatch.rs` (parity tests).

---

## L2 — "call A, then remember to call B" footgun (2026-07)

**Root cause:** Two operations were coupled by convention instead of by code:
every call site that called `ScopeRegistry::intern`/`intern_runtime` (A) had to
remember a matching `Theme::bake` (B) before the next render, or a newly
interned `ScopeId` would resolve to the *default* style — silently, since the
out-of-range guard was only a `debug_assert!` (no-op in release). Every call
site paired them correctly by hand, which meant the invariant held only as
long as nobody wrote a new call site — a bug waiting for the next commit that
interned a scope without knowing it needed a bake.

**Concrete instance:** flagged in code review as a "latent footgun" — not yet
triggered, since every existing intern site happened to already pair with a
bake. Fixed in `0b97c3f` before it could bite.

**Prevention rule — when A/B pairing can't be enforced by visibility or types,
make B self-triggering at the one place state is consumed, not at every place
it's produced.**

Concretely: don't chase every call site of A and insert a matching B (that's
the discipline this lesson is about *not* relying on). Instead:

1. Find the *single* place the paired state is actually read on the
   hot/production path (here: `prepare_frame`, the per-frame `&mut` chokepoint
   that runs before every `render`).
2. Give B a self-check that makes it a no-op when already-consistent, and call
   it unconditionally from that one place (here: `Theme::bake_if_stale`, which
   compares `baked.len()` vs `registry.len()` — cheap because `ScopeRegistry` is
   append-only, so the lengths alone detect staleness with no extra state).
3. Delete every hand-paired B at the individual A call sites. If B is only
   needed because of A, and B is now automatic, a manually-placed B is dead
   weight that can drift (e.g. call it twice, or call it and still forget it
   elsewhere) — one chokepoint, not N call sites each promising to behave.

This beats the compile-time-enforced alternative (make A `pub(crate)`, force
every caller through a wrapper that also calls B) when A has legitimate
callers outside the module that can't be rerouted without signature churn
(here: `hume-editor`'s runtime grammar/plugin code interns scope names that
`hume-engine` can't own). Self-healing at the read side gets the same
correctness guarantee — B can never be forgotten because nothing depends on it
being called promptly — without touching A's callers at all.

**Files:** `hume-engine/src/theme/mod.rs` (`Theme::bake_if_stale`),
`hume-editor/src/editor/lifecycle.rs` (`prepare_frame` call site).

---

## L3 — Plan said "ask user"; execution silently took the default (2026-07)

**Root cause:** A plan item was written as "behavior choice — ask user
(default: leave as-is)". During execution the default was applied without
ever asking; the question surfaced only as a passing remark in the final
summary ("say the word if you want…"), which is not asking.

**Concrete instance:** hume-editing review, finding F6 (`classify_char`
treats Unicode whitespace as `Punctuation`). Plan deferred the decision to
the user; implementation skipped the question entirely.

**Prevention rule:** If a plan marks an item "ask user", that is a blocking
action, not a soft note. Before declaring the task done, either ask the
question explicitly (AskUserQuestion) or state up front "did NOT do X —
needs your decision" as its own line item in the report — never bury it as
an aside inside an unrelated paragraph.

---

## L4 — Chokepoint invariant enforced only by a comment (2026-07)

**Root cause:** "While an edit group is open (an active Insert session),
every edit to that buffer must compose into it" is a real invariant — it
already had *some* recognition in the codebase (`run_native_body` branches
on `is_group_open_current`; buffer reload comments on the same hazard) — but
it was never enforced at the one chokepoint (`doc_ops::apply_doc_edit`)
every edit-applying caller goes through. A later LSP completion-accept path
called the *ungrouped* `apply_doc_edit` while an Insert-session edit group
was open. Nothing crashed at the accept itself — the buffer just changed
length out from under the open group's tracked state. The very next grouped
keystroke's `ChangeSet::compose` then panicked on a length mismatch,
deterministically, one keystroke removed from the actual mistake.

**Concrete instance:** type `DEFAULT_`, accept an LSP completion item
`DEFAULT_WIDTH` (grows the token by 5 chars), type any character — panic in
`hume-editing/src/changeset/mod.rs`'s `compose`. `lsp/edits.rs`'s
`commit_changeset` applied the accept through the ungrouped path; the
already-open insert-session group never saw it. A second, independent bug
found during the same trace: `refilter_lsp_completion_after_edit`
(`mappings/insert.rs`) sliced `anchor..head` with a comment claiming
`head < anchor` "can't happen" — an arrow key moving the cursor before the
anchor without dismissing the session proved it could, and did.

Root-cause note for *why several manual reviews missed this*: each side was
locally correct in isolation (the accept path was gen-checked; edit groups
worked correctly for every native command); the bug was only visible in
their interaction, and no test ever kept typing *after* an accept —
every completion test stopped at the terminal action (assert text, assert
session closed) and never drove another keystroke through it.

**Prevention rules:**

1. **Enforce invariants at the chokepoint, not by caller convention.** An
   invariant that lives only in a comment or in one caller's `if` branch is
   invisible to every other caller and every reviewer who didn't write that
   branch. `apply_doc_edit` now checks `edit_group.is_some()` itself and
   routes to the grouped path — no caller can bypass it again, by
   construction, not by discipline. Backed by `debug_assert!`s in
   `apply_doc_undo`/`apply_doc_redo` so any remaining bypass is loud instead
   of a silent corruption three calls later.
2. **Modal-flow tests must not stop at the terminal action.** After every
   accept/apply/dismiss in a stateful multi-keystroke flow (completion,
   paste cycling, pending register, etc.), keep interacting — type a char,
   move, undo, Esc — and assert the editor stays consistent. A test that
   only checks the terminal state systematically misses "what happens on
   the *next* keystroke" bugs, which is exactly where this class of bug lives.
3. When reviewing (or writing) a new caller of an existing chokepoint,
   explicitly check it against every piece of state that can be *live* when
   it runs — an open edit group, an open completion/paste session, a
   pending macro recording — not just against the chokepoint's own contract.

**Files:** `hume-editor/src/editor/doc_ops.rs` (`apply_doc_edit` routing,
undo/redo asserts), `hume-editor/src/editor/commands/mod.rs` (collapsed
caller-side branch), `hume-editor/src/editor/mappings/insert.rs` (refilter
guard), `hume-editor/src/editor/tests/lsp_completion_menu.rs` and
`lsp_completion_feature.rs` (type-after-accept / type-after-arrow-key
regression tests).
