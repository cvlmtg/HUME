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
   go through `dispatch_native` in `commands/mod.rs`.  The lint
   `single_native_dispatch_funnel` in `lints.rs` enforces this: any second
   `match cmd` that binds a native variant's `fun` outside that file fails the
   build.

3. **Duplicate-match smell** — two identical `match cmd { Motion { fun } | … }`
   arms in different files is a SSOT violation.  Collapse to one funnel.

**Files:** `editor/src/editor/commands/mod.rs` (funnel),
`editor/src/editor/lints.rs` (lint), `editor/src/editor/tests/mod.rs`
(snapshot helper), `editor/src/editor/tests/sync_dispatch.rs` (parity tests).
