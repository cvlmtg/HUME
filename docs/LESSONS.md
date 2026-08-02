# HUME — Lessons Learned

Patterns that bit us; rules to prevent recurrence.

---

## L1 — Side-effect cluster regression (2026-06)

**Root cause:** A refactor added a *second* code path for an operation whose
correctness depends on a *cluster* of side effects, not just the primary effect.
The old path kept its inline bookkeeping; the new path was a bare copy of the
execution match without any of the surrounding bookkeeping.  Tests pinned the
primary effect (cursor moved, text changed) and stayed green on both paths.  The
entire cluster (jump list, last-command tracking — since removed, dot-repeat,
paste-session commit, register routing) regressed silently on the new path.

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

**Files:** `hume-editor/src/editor/commands/pipeline.rs` (funnel),
`hume-editor/src/editor/lints/dispatch_funnel.rs` (lint), `hume-editor/src/editor/tests/mod.rs`
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

**Second instance, stronger remedy:** `Buffer::set_path`/`set_display_path`
had the same convention-plus-`debug_assert!` shape (every path-setting call
site had to remember a matching display-path derivation). Here A's only
caller was `Buffer::set_path` itself — no external callers to reroute — so
the fix skipped self-healing-at-consumption entirely and merged B into A:
`set_path` now derives `display_path` directly, making the pairing
structural instead of convention-enforced. Prefer this merge when A has no
legitimate external callers; reach for self-heal-at-consumption only when it
does (as in the `ScopeRegistry` case above). Fixed alongside the `04591455`/
`45ed2c51`/`0ab787a4` review. **Files:**
`hume-editor/src/editor/buffer/mod.rs` (`Buffer::set_path`).

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

---

## L5 — "Fixed" a correct doc claim by reasoning instead of reading (2026-07)

**Root cause:** During a self-review pass, a *correct* documentation claim
was changed to an incorrect one. The trigger was a plausible-sounding chain
of inference — "`x` selects the line including its trailing `\n`, so `c` on
that selection must delete the newline and join the lines" — built from a
comment skimmed in a different file. The actual implementation was never
opened.

**Concrete instance:** `user-manual/docs/from-vim.md`, Vim `S` row. The
original "`x` then `c`" was right. It was "corrected" to `m i l` then `c`
on the assumption that `xc` joins lines. `change_span`
(`hume-ops/src/edit/delete.rs`) explicitly excludes a trailing `\n` —
"`c` clears line content but keeps the line" — and its doc comment names
`select-line` / `x` as the very case it exists to handle. The user caught
it with "`xc` does NOT join lines".

**Prevention rule:** Self-review may only *flag* a claim as suspect; it may
never *rewrite* one on inference alone. Any edit to a behavioral claim
requires reading the function that implements the behavior first — for an
editing command that means the command body plus every span/range helper it
calls, not a neighboring comment. Doubt about a claim is a signal to open
the file, never a licence to swap in a different claim. This applies with
extra force when editing something already verified earlier in the session:
changing a previously-checked line needs *more* evidence than writing it
did, not less.

## Platform-gated tests: structure over attributes (2026-07-20)

**Mistake pattern:** Unix-only tests accumulated as per-test
`#[cfg(not(windows))]` attributes inside otherwise-portable test files.
Every such file's module-level imports then only compile as "used" on
unix — each new gated test risks a fresh crop of Windows-only
unused-import warnings, invisible until Windows CI runs. Fixing them by
gating individual imports treats the symptom and multiplies attributes.

**Prevention rule:** Platform-scoped tests belong in a platform-scoped
module: `hume-editor/src/editor/tests/unix/` is gated once via
`#[cfg(unix)] mod unix;` in `tests/mod.rs`, and files inside carry no
cfg attributes at all. A wholly unix-only test file goes in `unix/`; a
file mixing portable and unix-only tests is split into a same-named pair
(portable half stays, unix half moves). Never add a new
`#[cfg(not(windows))]` to a test or import in the editor test tree —
put the test in `unix/` instead. The same gate-once shape applies to
inline `mod tests` blocks in library crates, just nested one level
deeper: a wholly unix-only test module takes `#[cfg(all(test, unix))]`
on the whole `mod tests` (`hume-lsp/src/backend.rs`); a `mod tests` that
mixes portable and unix-only tests gets a nested `#[cfg(unix)] mod unix`
holding the unix-only tests, itself gated once, with no per-test
attributes (`hume-lsp/src/transport.rs`).

## Terminal protocol enabling: gate on decode capability, not terminal capability (2026-07-20)

**Mistake pattern:** The kitty keyboard probe asked the *terminal* "do you
support kitty?" and enabled the protocol on a yes — on every platform. On
Windows the answer travels through ConPTY, whose passthrough (bundled
ConPTY ≥ 1.22, and ConPTY itself answers the kitty query from Windows
Terminal 1.25) happily says yes, while the *input* side still delivers
ConPTY-translated `INPUT_RECORD`s that crossterm's Windows event source
cannot map back from CSI-u. Result: every keypress leaked into the buffer
as literal text (`[105;1:3u]`) and the editor was undriveable. The bug was
latent for as long as older bundled conhosts silently ate the probe
queries; a wezterm nightly ConPTY bump unmasked it.

**Prevention rule:** Enabling a terminal *output* protocol changes the
*input* encoding — so the gate must be "can our input path decode the
resulting encoding", not "does the terminal support it". A capability
probe of the terminal is only half the handshake; when an OS layer
(ConPTY) re-translates input independently of the terminal, the probe can
say yes while decode is impossible. If the decode capability is statically
absent on a platform, hardwire the feature off there (`probe_kitty_support`
returns `false` on Windows) instead of probing.

**Resolution (2026-07-20, same day):** HUME migrated from crossterm to
termina for terminal I/O. Termina's Windows backend sets
`ENABLE_VIRTUAL_TERMINAL_INPUT` and decodes kitty CSI-u on Windows the same
way it does on Unix — the decode capability that was statically absent is
now present, so the rule above applies in the other direction: Windows
probes for real again (`probe_via_events` in `hume-platform/src/lib.rs`),
using the *same* `EventReader` real input goes through, which is what makes
the probe's answer trustworthy this time.

---

## L6 — A sabotage run outlived its own build and was mistaken for a live bug (2026-07-28)

**Root cause:** A deliberate-mutation ("sabotage") verification run of an
*unbounded* blocking test — one that calls a real wait primitive
(`select(2)`) directly on the test thread with no upper bound — spun at
100% CPU exactly as the mutation intended, but the process was never killed.
Three days and a rebuild later, the still-running process was found and
read as live evidence that shipped signal-handling code was broken, despite
"multiple code reviews." It wasn't: the binary underneath the running
process had already been replaced (`txt` vnode size didn't match the file at
that path), and the process had started *before* the terminator code it was
supposedly testing was even committed. Its stdout/stderr pointed at a
`/private/tmp/...` log that was itself already unlinked, destroying the one
artifact that would have made the run's provenance obvious immediately.

**Concrete instance:** `hume_platform-13eec481c3fad2cd terminator_tests
--test-threads=1`, started `Sat Jul 25 02:09:05`, still spinning
`2026-07-28`. `sample` showed it stuck inside
`unix::terminator_tests::detects_signal_with_tty_idle` →
`run_terminator_blocking` → `wait_readable_pair` → `select` returning
instantly without draining — precisely the failure mode
`terminator_exits_instead_of_spinning_when_the_pipe_closes`
(`hume-platform/src/unix.rs`) exists to catch. The terminator module itself
landed in `92c96c07` on `2026-07-28`, three days after the process started.

**Prevention rules:**

1. **Bound every test that blocks on a real wait primitive.** A regression
   in code under test must turn into a fast test *failure*, not an
   unkillable 100%-CPU hang. `hume-platform/src/unix.rs`'s
   `terminator_tests::run_bounded` helper is the pattern: run the call on
   its own thread, poll `is_finished()` against a generous (not
   latency-sensitive) deadline, and `panic!` if it's blown. Latency
   assertions stay separate — the bound is a hang detector, not an
   assertion on how fast success should be.
2. **CI must have a job-level `timeout-minutes`.** Without one, a hang runs
   to GitHub's 6-hour default instead of failing visibly and fast.
   `.github/workflows/ci.yml`'s `test` job now sets one.
3. **A sabotage/mutation-testing run must be disposable.** Run it in the
   foreground (or under `timeout`), and route its output somewhere that
   survives inspection — not `/tmp`, where an unlinked file after the
   process outlives its intended lifetime erases the evidence of what the
   process actually is.
4. **A long-running, unfamiliar process is a "read before you act" moment,
   not a "trust the symptom" one.** Before treating a stuck process as proof
   of a live bug, check what it actually is: `ps -o lstart=`, `lsof` for its
   binary and open fds (a stale `txt` vnode size vs. the on-disk file is
   definitive proof of a stale binary), and whether the code path it's
   allegedly exercising even existed when it started.

**Files:** `hume-platform/src/unix.rs` (`run_bounded` + `SPIN_BOUND`),
`.github/workflows/ci.yml` (`timeout-minutes`).

---

## L7 — Non-reentrant test mutex held across a helper that re-acquires it (2026-07-30)

**Root cause:** A test held `HUME_RUNTIME_MUTEX` (a plain `std::sync::Mutex`,
not reentrant) for its *entire* body via `let _lock = MUTEX.lock()...` bound
at the top of the function — the standard pattern for guarding a process-global
env var (`HUME_RUNTIME`) for as long as anything might read it. Later in the
same function, a call to the shared `safe_tempdir()` helper tried to acquire
the *same* mutex again, on the *same* thread. `std::sync::Mutex` doesn't
detect same-thread re-entrancy — it just blocks forever, since the lock is
already held by the very thread trying to acquire it.

**Concrete instance:** `steel_server_plugin_registers_scheme_with_generated_globals_env`
(`hume-editor/src/editor/tests/scripting_host_globals.rs`) held `_lock` for the
whole test, then called `safe_tempdir()` near the end — `cargo test` reported
the test as "running for over 60 seconds" instead of failing fast.

**Prevention rules:**

1. **Scope a mutex guard to the critical section, not the whole test.**
   `HUME_RUNTIME` only needs the lock held from `set_var` to `remove_var`
   around `ScriptingHost::new()` (the one place that reads it) — wrap just
   that in a block expression (`let host = { let _lock = ...; ...; host };`)
   so the guard drops before any later helper in the same function can touch
   the same mutex.
2. **Before adding `safe_tempdir()` (or anything else documented as taking
   `HUME_RUNTIME_MUTEX`) inside a test, check whether that test already holds
   the lock from an earlier step.** A same-thread self-deadlock on a
   `std::sync::Mutex` doesn't panic or error — it hangs silently, and the only
   symptom is the test runner's own generic "running for over Ns" notice.
3. **When a test hangs with no assertion failure, suspect a lock, not the
   logic under test first.** Instrument with `eprintln!`s bracketing each
   step (`--nocapture`) to find the last one that printed — the gap between
   the last print and the next is where the hang is, and a mutex acquire is
   the first thing to check there before doubting the code under test.

**Files:** `hume-editor/src/editor/tests/scripting_host_globals.rs`
(`host_and_editor_after_runtime_layers`'s scoped-lock block).

---

## L8 — Diffing a live `Engine` against a fresh baseline still leaked non-deterministic internals (2026-07-30)

**Root cause:** Generating a list of "every Steel identifier HUME adds" by
diffing a fully-built `ScriptingHost`'s engine against a bare `Engine::new()`
baseline assumed the diff would cleanly separate "ours" from "upstream's".
It didn't: steel-core mints anonymous wrapper names (`###ctx-funcN`) for each
context-aware builtin registration, drawn from a `thread_local!` `AtomicUsize`
counter (`GENSYM` in `steel_vm/builtin.rs`) shared by *every* `Engine`
constructed on that OS thread — not reset per `Engine::new()`. Since
`cargo test` reuses worker threads across many tests, the baseline engine
(constructed *after* the real one, later in the same test) drew a
*different, non-overlapping* range of counter values than the real one — so
the diff never cancelled them out, and the generated file's `###ctx-func*`
entries changed on every run depending on test scheduling.

**Concrete instance:** the first generated
`runtime/plugins/core/steel-server/lsp-home/hume-globals.scm` differed
between two consecutive `HUME_WRITE_STEEL_GLOBALS=1` runs purely from
`###ctx-func0`..`###ctx-func106`-style entries; a regenerate-then-immediately-
recheck cycle failed the drift test it was meant to satisfy.

**Prevention rules:**

1. **A baseline diff only cancels state the baseline shares with the
   subject.** Per-instance, monotonically-numbered internal names (gensyms,
   arena/generation counters, anything seeded from process- or thread-global
   mutable state) are never shared across two separately-constructed
   instances, however "fresh" both are — a diff must filter these by pattern,
   not rely on the baseline to absorb them.
2. **Before trusting a generated/snapshotted list as deterministic, regenerate
   it twice in a row (not just once) and diff the two outputs.** One
   successful generation proves the mechanism runs; it proves nothing about
   run-to-run stability.
3. **Also run the drift test as part of (not isolated from) the full suite**
   — the nondeterminism here only showed up because other tests on the same
   worker thread had already advanced the shared counter; running the new
   test alone in isolation looked stable.

**Files:** `hume-scripting/src/lib.rs` (`ScriptingHost::host_global_names`'s
`!n.starts_with('#')` filter).
