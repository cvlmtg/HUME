# HUME — Lessons Learned

Patterns that bit us; rules to prevent recurrence.

## Rule index

Scan this at session start; read a lesson body only when its rule fires.

Ordered by date, not by number: each `L<N>` is a stable ID assigned once, when
the lesson was written, and never reassigned — external docs cite these
numbers directly.

- **L1** — A forked dispatch path needs a parity test on the whole bookkeeping
  cluster, plus a compiler-enforced single funnel. Two identical `match cmd`
  arms is an SSOT bug.
- **L2** — When A/B pairing can't be enforced by types, make B idempotent and
  self-triggering at the one place the state is read — never at N write sites.
  Merge B into A instead when A has no external callers.
- **L3** — A plan item marked "ask user" is a blocking action, not a footnote.
- **L4** — Enforce invariants at the chokepoint, not by caller convention.
  Modal-flow tests must keep interacting past the terminal action.
- **L5** — Self-review may *flag* a claim; it may never *rewrite* one on
  inference. Read the implementing function first.
- **L15** — Platform-scoped tests belong in a platform-scoped module, gated once.
  Never a per-test `#[cfg(not(windows))]`.
- **L16** — Gate a terminal protocol on *decode* capability, not on terminal
  capability — enabling an output protocol changes the input encoding.
- **L6** — Bound every test that blocks on a real wait primitive. CI needs
  `timeout-minutes`. Sabotage runs must be disposable. A long-running unfamiliar
  process is a "read before you act" moment, not proof of a live bug.
- **L8** — A baseline diff only cancels state the baseline *shares*. Regenerate
  twice before trusting a generated file as deterministic.
- **L9** — The Nth call site is a design smell proportional to N; at N≥3 ask
  whether the repeated thing should be a first-class concept. A derived-join
  value needs an observation-point diff, not a setter hook.
- **L10** — Check a claim about a Unicode construct against how the pipeline
  actually segments text before stating it as fact.
- **L11** — A lock guarding process-global state binds implicit *readers* too;
  a subprocess spawned by unqualified name reads `PATH`.
- **L12** — Steel 0.8.2 miscompiled nested keyword-arg calls; fixed as of 0.8.3,
  verified with a real build. Don't trust a source-diff inference over an
  actual test run when re-checking a third-party interpreter bug.
- **L13** — A subagent's *placement* recommendation is a design decision, not
  research — re-derive it against the project's ownership rules.
- **L14** — Check a crate's latest release before writing, and especially before
  *extending*, a workaround for its bug.
- **L17** — Narrowing an enum's own visibility does not fence its variants'
  payloads: variants inherit the enum's visibility and cannot be narrowed
  individually. Fence a payload with a newtype, not a visibility change.
- **L18** — Cost is never a verdict on correctness. "One call site" / "not
  worth the churn" never license a known-wrong design; either fix it or
  measure the cost and report the number. N=1 is the template for N=2, not
  an exemption.
- **L19** — "Every path that names a buffer now takes it explicitly" doesn't
  cover a path that used to act on one *implicitly*, with no parameter to
  make explicit at all. Map the funnel that has no name, not just the ones
  that do — and re-derive downstream invariants (a versioned key's identity
  guarantee) rather than patching around a design that quietly violates them.
- **L20** — Count the categories the real commands need before adding a type
  per category; parallel enums kept in sync by `unreachable!()` are the tell.
  A value a function was handed must be the one it uses — re-reading ambient
  state (focus) inside a body or a teardown reintroduces the bug the
  parameter existed to remove.
- **L21** — A placeholder standing in for "not yet decided" needs its own
  explicit tag, not an incidental property (emptiness, `None`, a default) of
  a real variant it's borrowing — the incidental property will eventually
  also be true of the real thing.
- **L23** — A recorder that infers, after a dispatch, what the dispatch did
  (a counter diff, a length snapshot, a flag) grows one signal per bug.
  Record the input where it arrives; let the one piece of state that isn't
  re-derivable — input the user gave mid-command — be recorded by the code
  that consumes it, not declared by command authors.
- **L24** — End-user docs state a missing feature plainly, never as permanent.
  "Not yet" only for a gap the user has confirmed is planned.
- **L25** — "No in-tree callers" doesn't make a user-facing API dead. Read why it
  was added and why its last caller left before proposing removal.
- **L28** — Hiding one raw read does not make a model correct by construction
  while its constructors still take the raw form. Design the model first: type
  the values it is built from, pair it with the data it is valid for, and give
  it one repair point. Callers adapt to the model, whatever the blast radius.
- **L26** — A text primitive whose tests use only ASCII is untested for the
  Unicode claims the editor makes. Cover every class relevant to it from the
  shared corpus, and let the fuzzers draw from it.
- **L27** — An invariant about how a value is read or built is enforced by
  removing the raw accessor and providing the typed conversion, not by an
  audit or a comment. Propose the compile-time form in the plan.
- **L29** — A defect the model permitted is fixed where the model is built,
  and the caller patches that hid it are reverted. Before patching a caller,
  name the type or funnel that made the bug possible and put it in the plan's
  altitude line.
- **L30** — A redesign that derives policy from a class of text (word
  characters) must take that class from the source or server that owns the
  token, not from the editor. Ask what else could define it before fixing the
  rule in the model.
- **L31** — When an edge case forces a special value (an empty range, a tag,
  a deferred fix-up) at the point an operation is recorded, ask whether the
  invariant it protects can be checked once on the result instead. One
  result-level rule replaces the per-operation exceptions.
- **L32** — When an approved design turns out costlier than expected, the
  deviation report compares the options on correctness first: what each
  lets go wrong, when, and how it would show. Churn is a separate line, never
  the argument for the smaller option.
- **L35** — Moving a property to its owner means removing every other carrier
  of it in the same plan. Check each field and keyword of the existing shape for a
  second copy of what the new owner now holds.

---

## L1 — Side-effect cluster regression (2026-06)

**Root cause:** A refactor added a *second* code path for an operation whose
correctness depends on a *cluster* of side effects, not just the primary effect.
The old path kept its inline bookkeeping; the new path was a bare copy of the
execution match without any of the surrounding bookkeeping.  Tests pinned the
primary effect (cursor moved, text changed) and stayed green on both paths.  The
entire cluster (jump list, last-command tracking (since removed), dot-repeat,
paste-session commit, register routing) regressed silently on the new path.

**Concrete instance:** `b7a5af0` added `run_command_sync` for Steel `(call!)`.
Cursor/text assertions passed.  Nine bookkeeping regressions shipped.

**Prevention rules:**

1. **Path parity test** — whenever a refactor adds or forks a dispatch path,
   add a parity test asserting that both paths leave identical state (use a
   `BookkeepingSnapshot`-style helper that captures the whole cluster).  Never
   assert only the primary effect.

2. **Single funnel, compiler-enforced** — all execution of native-command
   `fun` fields must go through `run_body` in `commands/pipeline.rs`
   (wrapped by `commands::run` for bookkeeping). Every native
   variant's `fun` is wrapped in `NativeBody<F>` (`commands/pipeline.rs`), a
   newtype whose field is private to that file — destructuring still binds
   `fun` everywhere, but the value is opaque and uncallable outside
   `run_body`. A text-scanning lint tried this first and was too weak
   (see L17); the newtype closes the same gap the compiler, not a scan.

3. **Duplicate-match smell** — two identical `match cmd { Motion { fun } | … }`
   arms in different files is a SSOT violation.  Collapse to one funnel.

**Files:** `hume-editor/src/editor/commands/pipeline.rs` (funnel + `NativeBody`),
`hume-editor/src/editor/tests/mod.rs` (snapshot helper),
`hume-editor/src/editor/tests/sync_dispatch.rs` (parity tests).

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

**Concrete instance:** hume-editing review found `classify_char` treats
Unicode whitespace as `Punctuation`. Plan deferred the decision to
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
   construction, not by discipline. Backed by a `debug_assert!` in
   `apply_doc_history_walk` so any remaining bypass is loud instead
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

---

## L15 — Platform-gated tests: structure over attributes (2026-07-20)

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

---

## L16 — Terminal protocol enabling: gate on decode capability, not terminal capability (2026-07-20)

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
(`68a6226d` later renamed the first two symbols to `detects_signal` and
`wait_readable` respectively — named here as `sample` actually reported them
at the time.)

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

---

## L9 — Proposed an Nth call site instead of questioning the pattern (2026-08)

**Root cause:** Asked to fix "the disk-stale check doesn't run when a fuzzy
picker switches buffers", the investigation correctly found the mechanism (the
check rides a hand-rolled focus diff in `handle_event`, which picker accepts
bypass because the callback is queued and runs a frame later in
`prepare_frame`). The proposed fix was to add the diff at a second place.

That would have been the **fourth** site performing the same check: the
`handle_event` diff, `enter_buffer_with_jump`'s already-focused special case,
and two `check_all_disk_state` calls in `run`. Each of the three existing ones
had a reasoned doc comment justifying itself, which made adding a fourth feel
like following the established pattern rather than compounding a defect.

The user rejected the framing outright. The real defect was that "the focused
buffer changed" was not an event at all — every consumer hand-rolled its own
detection. Once it became one event with one raise site, the picker bug
disappeared as a consequence rather than as a fix, and a second, larger bug
surfaced on the way: hooks raised by async work (`drain_lsp`,
`drain_due_timers`, queued Steel callbacks) were never drained on the frame
path, so they waited for the next keystroke — forever, on an idle editor.

**Prevention rules:**

1. **Adding the Nth call site to a repeated pattern is a design smell,
   proportional to N.** At N≥3, stop and ask whether the thing being repeated
   should be a first-class concept. A fix that reads as "one more place that
   remembers to do X" is a fix that the next feature will also have to
   remember.
2. **Well-reasoned doc comments on each duplicate site are not evidence the
   duplication is sound.** They are evidence someone justified each step
   locally. Read them as a list of workarounds, and check whether they are all
   working around the same missing abstraction.
3. **When a value is a derived join of independently-written fields, it has no
   write-site chokepoint and never will.** `focused_buffer_id()` is
   `panes[focused_pane_id].buffer_id` — one field with 1 writer, another with
   5, and a focus move changes the result while writing neither. Notification
   for such a value must be a diff at an observation point, not a hook on a
   setter. Establish which kind of value it is *before* designing the
   notification.
4. **Two queues drained by two different rules will diverge.** The Steel-call
   queue drained once per frame in `prepare_frame`; the hook queue drained to
   fixpoint in `handle_event`. Nothing enforced that a producer in one phase
   had a consumer in the same phase. Prefer one queue with one rule; if two
   are genuinely needed, write the test that proves work queued by either is
   drained on every path.

**Files:** `hume-editor/src/editor/commands/typed_buffer.rs`
(`check_all_disk_state`), `hume-editor/src/editor/scripting_setup.rs`
(`check_all_disk_state`), `hume-editor/src/editor/tests/disk_change.rs`
(`enter_buffer_with_jump`), `hume-scripting/src/lib.rs` (`focused_buffer_id`).

---

## L10 — A "bug" claim about combining marks was never checked against grapheme segmentation (2026-08-22)

**Root cause:** Diagnosing a display-width bug (git-diff's tab-stop math
undercounting wide CJK graphemes), the plan asserted a second bug as fact:
that decomposed combining sequences (e.g. `e` + U+0301) also misrender in
`hume-engine`'s virtual-display-line renderer, because `segment_virtual_line`'s
`.clamp(1, 2)` differs from `push_insert_cells`'s `.min(255)` + skip-on-zero.
The claim sounded structurally plausible — two different width-clamping
policies in the same file *do* diverge somewhere — and was stated as
established fact in a plan file before being checked.

**Concrete instance:** `unicode_segmentation`'s `grapheme_indices(true)`
merges a base character and its following combining mark into *one*
grapheme cluster before either width-clamping policy ever sees it — `"e" +
U+0301` arrives at `.width()` as a single two-codepoint `&str` measuring 1
column, not two separate zero-width-adjacent codepoints. The two clamp
policies only actually diverge on a *degenerate* cluster with no base
character (a lone combining mark, a bare ZWJ) — a real but much narrower
case than "combining marks misalign," which the user corrected mid-review.

**Prevention rule:** A claim about how a specific Unicode construct (a
combining sequence, a ZWJ emoji, a regional-indicator flag pair) behaves
through a text pipeline must be checked against how that pipeline actually
segments text — here, tracing the value through `grapheme_indices(true)`
by hand — before it's written into a plan or a finding as fact. "Two code
paths compute width differently" is not itself evidence that a *specific*
input reaches the diverging branch; each path's actual input shape (one
cluster vs. one codepoint) has to be traced, not assumed from the
surrounding code's structure.

**Files:** `hume-engine/src/display_lines.rs` (`segment_virtual_line`, renamed
from `segment_virtual_row`) — `push_insert_cells` no longer resolves anywhere
in the tree; the width-clamp divergence this lesson describes needs
re-verification against current code.

---

## L11 — A process-global lock gated mutators but not readers (2026-08-23)

**Root cause:** `TestGlobals` (`hume-editor/src/editor/tests/mod.rs`) exists
so tests that redirect process-global `PATH`/`TMPDIR`/`HUME_RUNTIME`/cwd
don't race each other, and its own module doc already stated the real
hazard: mutating one of these "races every other test *reading or writing*
the same var." But its enforcement lints only
ever checked for *mutation* outside a claim-holding file. A test that spawns a subprocess by
unqualified name (`Command::new("tree-sitter")`, `Command::new("sh")`) is a
`PATH` reader — the OS resolves that name against the live process `PATH` at
the spawn instant — and no reader was required to hold a claim at all.

**Concrete instance:** `install_real_json_grammar_e2e`
(`hume-editor/src/editor/tests/unix/scripting_grammar.rs`) spawns `git`,
`curl`, and `tree-sitter` by name with no `TEST_GLOBALS` claim.
`scripting_lsp_install.rs` has four tests that claim `Global::Env` and then
narrow `PATH` to an empty (or shim-only) directory. A spawn from the e2e
test landing inside one of those windows resolved to nothing — `Os { code:
2, kind: NotFound }` — reproducing exactly under `--test-threads=16` at
~50%, and never under `--test-threads=1`. Four more tests across
`async_job.rs`, `picker_source.rs`, and their Steel twins spawned `sh`/
`sleep`/`kill` by name the same unguarded way, latent until the same window
opened under them.

**Prevention rule:** When a shared lock exists to serialize access to mutable
process-global state, every *implicit reader* of that state — not just every
explicit mutator — must take the same claim. A subprocess spawned by
unqualified name reads `PATH` (and, if it inherits cwd, the working
directory) exactly as much as a `std::env::var` call does; a lint (or
review pass) that greps for `set_var`/`remove_var` and stops there will
miss it. State the reader obligation on the lock type itself (`Global::Env`'s
doc now names it) so it doesn't need re-discovering at each new spawn site.

**Files:** `hume-editor/src/editor/tests/mod.rs` (`Global::Env` doc),
`hume-editor/src/editor/tests/unix/scripting_grammar.rs`,
`hume-editor/src/editor/tests/unix/async_job.rs`,
`hume-editor/src/editor/tests/unix/picker_source.rs`,
`hume-editor/src/editor/tests/unix/async_job_steel.rs`,
`hume-editor/src/editor/tests/unix/picker_source_steel.rs`.

---

## L12 — Steel 0.8.2 miscompiles a keyword-arg call nested inside another keyword-arg call (2026-09-02)

**Root cause:** Steel's `(define (f a #:kw [b default]) ...)` sugar desugars
each definition *and each call site that omits a keyword* through a
generated `####%list-argsN` dispatcher. When one such omitting call sits as
a sub-expression of another omitting call (e.g. `(outer-kw-fn (lambda ()
(inner-kw-fn positional-args-only)))`, both `outer-kw-fn` and `inner-kw-fn`
defined with `#:kw [x default]` sugar), the compiler emits the inner
dispatcher's reference before its own definition in the compiled unit,
raising `FreeIdentifier: Cannot reference an identifier before its
definition: ####%list-argsN` — a compile-time failure with no runtime
component. Confirmed with a minimal reproduction directly against
`steel_core::steel_vm::engine::Engine` (bypassing HUME entirely): two
top-level `#:kw`-sugared definitions, called from an unrelated top-level
`define`, compile and run fine; the same two definitions, with the second
called from inside a lambda passed as an argument to a third `#:kw`-sugared
function (mirroring `define-command!`), fail identically. Supplying every
keyword explicitly at the *inner* call site avoids the bug outright — the
miscompilation is specific to keyword *omission* at a nested call site, not
to keyword-arg sugar in general.

**Concrete instance:** `register-grammar!` (`runtime/scheme/prelude.scm`)
moved from a `define-syntax` macro (positional-only, expanded away before
Steel ever needed keyword dispatch) to a `#:kw`-sugared `define` when
`#:textobjects` joined `#:injections`. Two tests
(`register_grammar_command_mode_attaches_and_sweeps`,
`install_real_json_grammar_e2e` in
`hume-editor/src/editor/tests/unix/scripting_grammar.rs`) build an init.scm
containing `(define-command! "attach-json" ... (lambda () (register-grammar!
"json" ... )))` — `register-grammar!`'s call omits both keywords, nested
inside `define-command!`'s own `#:repeatable`/`#:inline-output`-omitting
call. Both failed to compile with the exact `####%list-args2` error, even
though every *production* call site (`runtime/scheme/grammars.scm`,
`runtime/plugins/core/plum/grammars.scm`) already supplies both keywords
explicitly and was unaffected.

**A second, independent trigger** surfaced while fixing the above: a plain
`define` with a `. rest` parameter (no `#:kw` sugar at all) that scans `rest`
at runtime for keyword markers *also* miscompiles — not on nesting, but when
two or more differently-shaped keyword calls to the same function appear in
one compiled program (`FreeIdentifier: ... ##restN`, confirmed the same way,
directly against `Engine`). `hume-scripting`'s `init_scripting`
(`hume-editor/src/editor/scripting_setup.rs`) compiles `prelude.scm`,
`languages.scm`, `grammars.scm`, and a user's `init.scm` as *separate*
programs, so this specific trigger is scoped to keyword calls within a
single file — but nothing stops a user's own `init.scm` from registering two
grammars with different keyword combinations in one file and hitting it with
no warning. This means the prevention rule below (spell out every keyword at
a nested call site) is not a complete safety net for `#:kw`-style call syntax
in general — it covers only the first trigger.

**Original resolution (2026-09-02):** `register-grammar!` was reverted to a
pure positional `define-syntax` macro (three arms, extending the
pre-`cafc071a` two-arm form by one more optional trailing argument for the
textobjects path) — no `#:kw` sugar and no bare `#:keyword val` tokens in
its call syntax at all, which is immune to both triggers simultaneously
rather than working around either one.

**Original prevention rule:** a `#:kw`-sugared Steel function's call site
should spell out every keyword explicitly when the call itself is nested
inside another keyword-arg call's argument position (a lambda passed to
`define-command!`, `debounce`, a picker callback, etc.) — omission is safe
at a top-level or otherwise unnested call site, *for that one trigger only*.
This is a workaround for a third-party interpreter limitation, not a HUME
style rule to apply blindly.

**Revised resolution (2026-09-26), superseding the above:** `register-grammar!`
is back to a `#:kw`-sugared `define` (`cafc071a`'s shape), verified for real
rather than by source-diff inference — reverted in the actual repo, then
built and tested against real steel-core 0.8.3, not a synthetic
reconstruction. Two things made this safe to re-check:

1. Trigger 2 (a plain `. rest` function, no `#:kw` sugar at all, that scans
   `rest` at runtime for keyword markers) was found in an *intermediate
   candidate* that was never shipped (`0a7bd62d`'s own message calls it
   "candidate B," tried and rejected before landing on the positional macro).
   Reverting to the `#:kw`-sugared form goes back to *candidate A* — which
   only ever hit trigger 1 — so trigger 2, as originally described, doesn't
   even apply to the shape being restored.
2. `register_grammar_command_mode_attaches_and_sweeps` and
   `install_real_json_grammar_e2e`
   (`hume-editor/src/editor/tests/unix/scripting_grammar.rs`) are trigger 1's
   exact real shape unchanged — nested inside `define-typed-command!`'s
   (itself `#:kw`-sugared) lambda, keywords omitted — and a new test,
   `register_grammar_two_differently_shaped_keyword_calls_in_one_file_compiles`,
   covers the closest real-code equivalent of trigger 2 for the restored
   shape (two `register-grammar!` calls with different keyword-omission
   shapes in one compiled `init.scm`). All three passed, along with the real
   production call sites in `runtime/scheme/grammars.scm` and
   `runtime/plugins/core/plum/grammars.scm` (the latter exercised end-to-end
   by `plum_install_grammar_recovers_from_stale_source_dir_on_first_try`,
   `hume-editor/src/editor/tests/unix/injections_editor.rs`).

The prevention rule above is retired for `register-grammar!` specifically —
it's `#:kw`-sugared again, call sites omit keywords freely, see
`runtime/scheme/prelude.scm` and `user-manual/docs/syntax-highlighting.md`.
The general caution (nested keyword omission was a real steel-core 0.8.2 bug
class) stays here as history; re-verify empirically, the way this revision
did, before relying on it being fixed in some future steel-core version too.

**Files:** `hume-editor/src/editor/tests/unix/scripting_grammar.rs`,
`hume-editor/src/editor/tests/unix/injections_editor.rs`,
`runtime/scheme/prelude.scm`, `runtime/scheme/grammars.scm`,
`runtime/plugins/core/plum/grammars.scm`.

---

## L13 — A subagent's *placement* proposal was carried into a plan unchecked (2026-09-05)

**Root cause:** Two `LineStore`s had to be merged into one, and the question
"where does the survivor live" was answered by a research subagent. It reasoned
from the borrow graph — the between-frame consumers can reach `EditorState` and
nothing else — and landed on `EditorState`. That is a correct answer to *which
struct is reachable*, which is not the same question as *which domain owns this
fact*. It was carried into a plan and put to the user as the recommended option
without ever being checked against the project's own ownership rule.

The rule it violated was not obscure: "check `hume-engine`'s `Pane` before
defaulting to an `Editor` field — `Pane` is a full-fat view object, and a
per-pane transient belongs there." It even had a near-identical precedent, a
`PaneViewportCache { HashMap<(PaneId, BufferId), _> }` on `Editor` that moved to
`Pane.viewport_memory` for exactly the reason that applied again here: a
`PaneId`-keyed map needs a prune-on-close sweep every future pane-close path has
to remember, while a field on `Pane` dies with the pane.

**Concrete instance:** the user caught it in one line — "why linestore on
editorstate? wasn't this refactoring limited to the engine?" Re-deriving the
answer put `PaneLineStore` on `Pane`, which additionally deleted `struct
LineStore`, `LineStore::retain_panes`, and both `prune_closed_pane_caches`
store sweeps. The rejected placement would have kept all four.

**Prevention rule:** A subagent's finding about *what the code does* can be
taken as research. A subagent's recommendation about *where state should live*
is a design decision and must be re-derived against the project's ownership
rules before it enters a plan — an agent optimising for the smallest diff will
propose whatever struct the current borrow graph already reaches, and the
borrow graph is a consequence of past decisions, not a source of authority over
new ones. Ask the three-part question explicitly (SSOT, separation of concerns,
would an engine-layer change be more elegant?) for any new field, including one
that is only *moving*. A move is a placement decision made fresh, not a
carry-over that inherits its old justification.

**Files:** `hume-editor/src/editor/frame.rs` (`prune_closed_pane_caches`),
`hume-editor/src/editor/scroll/`, `cursor/`, `mouse/` (`PaneLineStore`
consumers).

---

## L14 — Extended a crate workaround instead of checking for a newer release (2026-09-06)

**Root cause:** `hume-platform`'s terminator carried a ~700-line subsystem
(`Trigger::Hangup`, `hangup_status`, `confirm_hangup`, `is_tty_gone`,
`fionread_outcome`, `quit_acknowledged` ack-gating, the `Terminator` RAII
handle) built entirely to work around termina 0.3.3's `UnixEventSource::
try_read` mapping a controlling terminal's tty EOF to `Ok(None)` instead of
an error. `f14a65cf` went further and *patched a bug inside that workaround
itself* (the tty watch stopped during the quit-grace window, letting a
hangup racing a signal go undetected) — real effort spent hardening
mitigation code, with nobody checking whether termina had already fixed the
underlying bug upstream. It had: termina 0.4.0's `try_read` now returns
`Err(io::ErrorKind::UnexpectedEof)` on that same read, at the source. The
entire subsystem, including the fix just built on top of it, was deletable
the moment the dependency was bumped.

**Concrete instance:** `f14a65cf` ("cut quit grace short on a stalled tty
hangup") extended the pre-existing tty-hangup watcher rather than pausing to
check `termina`'s releases. The next session's very next task was "termina
0.4.0 has been released, check if it fixes the bugs we worked around" — it
did, and the fix plus the entire workaround it extended were squashed into
one commit (`68a6226d`) that nets *negative* lines against the pre-workaround
baseline (`unix.rs` 1472 → 598 lines, `rustix` dependency dropped entirely).

**Prevention rule:** Before writing code to work around a third-party crate's
bug or shortcoming — and *especially* before extending or hardening an
existing workaround — check the crate's latest released version and
changelog for a fix first. If one exists, upgrade and delete the workaround
instead of building on it. This applies with extra force the second time: a
workaround already in the tree is a standing invitation to keep patching it
locally rather than to ask whether it's still needed, and every hour spent
hardening one that's since been fixed upstream is doubly wasted. Cf. L12,
which is this same rule applied correctly on a later pass — the register-grammar!
workaround got re-checked against 0.8.3 with a real build rather than
assumed still necessary, and turned out to be removable.

**Files:** `hume-platform/src/unix.rs`, `hume-platform/src/lib.rs`,
`hume-editor/src/lib.rs`, `hume-platform/Cargo.toml` (and the other three
crates' `termina` deps).

---

## L17 — A visibility narrowing was mistaken for closing an enforcement gap it didn't touch (2026-09-10)

**Root cause:** L1's `single_native_dispatch_funnel` lint (a line-based grep
for `Motion { fun` and its three siblings outside `commands/pipeline.rs`) was
deleted twice, on two different justifications. The first replaced it with
`NativeBody<F>`, a newtype wrapping each native variant's body with a field
private to `pipeline.rs` — a real compiler-enforced fence, but on a branch
that was later abandoned and never reached `main`. The second, on `main`,
narrowed `MappableCommand` (and friends) from `pub(crate)` to
`pub(in crate::editor)` and deleted the lint on the claim that the narrowing
"closes off every module that was previously exposed." It doesn't: enum
variants inherit their enum's visibility and cannot be narrowed individually,
so every one of the ~279 files already under `crate::editor` could still
write `MappableCommand::Motion { fun, .. }` and call `fun` directly, skipping
`run_dispatch_pipeline`'s entire bookkeeping cluster — the exact regression
L1 exists to catch. The enforcement delta from the visibility change, checked
against the pre-change state, was zero: `editor/mod.rs` already declared
`mod registry;` private, so the type was already unnameable from outside
`crate::editor` before the narrowing.

**Concrete instance:** `45f07532` (`NativeBody`, branch `old`, abandoned) →
`7875cf1c` (visibility narrowing, `main`, wrongly framed as replacing the
same lint) → `f6686a3d` (lint restored, correctly, but without noticing that
a real fence for the same problem already existed on the abandoned branch) →
this lesson (`NativeBody` finally ported to `main`, lint deleted for good).
Three deletions/restorations of the same guard because each session re-judged
the trade-off without a durable record of what had already been tried and why
it did or didn't work.

**Prevention rule:** When a scanning-style lint (L1, and the ones
`docs/LESSONS.md`'s siblings-in-spirit replaced: line-count, grapheme-stepping,
column-naming, statusline-writes, text-writer) is proposed for deletion, the
replacement must be checked against what the lint actually caught, not
against what the refactor intended to catch. A visibility change enforces
reachability of a *type name*; it cannot enforce anything about a value
already reachable through a variant already in scope. Enum variant payloads
need a newtype with a private field (the pattern already used for
`CharOffset`, `RopeyLine`/`ContentLine`, the column types, `ResyncKey`) —
never a visibility annotation on the enum itself. When a lint is deleted in
favor of a type-level fence, verify the fence with the lint's own fail oracle
*and* against a surface the lint never scanned (its `tests/` blind spot, in
this case) — the delta between the two is exactly what the newtype has to
prove it closes.
`EditorState::focused_pane_id` (`pub(crate)`, raw-written from `mouse.rs`,
`commands/jump.rs`, `commands/pane.rs`) was this same class of gap; it is
now closed by `focus::Focus`, whose private field makes `focus::focus_pane`
the only writer.

**Files:** `hume-editor/src/editor/commands/pipeline.rs` (`NativeBody`),
`hume-editor/src/editor/registry/command.rs` (`MappableCommand`'s `fun`
fields).

---

## L18 — A known-wrong design was accepted on an unmeasured cost argument (2026-09-15)

**Root cause:** Adding `#:actions` to `picker!`/`live-picker!` needed the same
invariant the keymap trie already had — "every stored key is canonical" — but
instead of a newtype, `canonical()` was widened from module-private to
`pub(in crate::editor)` so `picker.rs` could call it directly. That is
precisely the substitution L17's own prevention rule names as wrong: a
visibility change was used where a payload needed fencing with a private
field. The resulting gap (nothing stopped a raw `KeyEvent` from landing in
`PickerSession.actions` outside its one construction path) was correctly
identified in review, then dismissed on a cost estimate — "a real but
non-trivial API change for one call site that already gets it right; not
worth it here" — that was never measured. Worse, nothing about the surrendered
invariant reached the tree: the "not worth it" reasoning lived only in an
ephemeral review transcript, and the doc comment that *did* ship on `actions`
instead framed the weaker design as a deliberate win — canonicalizing on
lookup "keeps the comparison in one place (`canonical`) rather than needing a
`Hash`/`Eq` impl to agree with it separately." That framing was also false on
the merits: a `CanonicalKey` newtype supplies exactly that agreeing
`Hash`/`Eq`. A future reader had no signal anything had been given up — they
had a comment telling them the weaker design was intended.

**Concrete instance:** `3514a4b9` (`feat(picker): add #:actions key→proc
bindings`) widens `canonical()` and ships the rationalizing comment. A review
the next day flags the gap, proposes the newtype fix, and is told to do it —
rejecting "one call site" and "not worth the churn" as reasons on their own:
a small wrong pattern is still wrong, the one call site is what a future
implementation copies from or grows into, and this is a learning project
where the proper solution is the deliverable, not an acceptable-for-now one.
`a26dadf7` (`refactor(keymap): restore compiler-enforced canonicalization for
binding keys`) is the fix, and it measures what the declined estimate never
did: 5 files, +205/−159, of which 110 insertions are `canonical.rs` moved
verbatim and 76 are its relocated tests — net new logic is one hand-written
`PartialEq` and one new test. Churn at `PickerOpts`/`LivePickerOpts`'s ~15
construction sites in `editor/tests/`: zero, because those types live in
`hume-scripting` and the FFI seam already converts at the crate boundary. One
grep would have settled the "non-trivial API change" claim before it was
made.

**Prevention rules:**

1. **Cost is not a verdict on correctness.** Wrongness is not subject to a
   cost/benefit vote. Either fix it, or measure the cost and report the
   number — never assert it.
2. **Measure before declining.** Count the call sites. Check whether the type
   crosses a crate boundary (that decides whether an FFI seam absorbs the
   change). Check whether the "API change" is a rename of an item with one
   construction path. State findings, not adjectives.
3. **N=1 is the template, not an exemption.** Read against L9: the smell of a
   repeated pattern scales with N, but the converse is *not* "N=1 is fine."
   The first site is what the second is copied from, and it may itself be
   refactored into something larger later — fixing it is cheapest exactly
   while N=1.
4. **A deferral must leave a trace that names the invariant surrendered** —
   never a comment that reframes the weaker design as the intended one. If
   the trace being written is a rationalization, that is L4/the Constraint
   Relaxation Check's lint-tell smell saying the fix was wrong; say so out
   loud instead of writing the comment.
5. **This is a learning project: the proper solution is the deliverable.**
   "Small" and "works today" are not the acceptance criteria.

**Files:** `hume-editor/src/editor/keymap/canonical.rs` (the newtype that
should have shipped in `3514a4b9`), `hume-editor/src/editor/keymap/mod.rs`,
`hume-editor/src/editor/picker.rs` (`actions` field and its formerly
rationalizing doc comment).

---

## L19 — An "every explicit-bid path" refactor missed the one path with no parameter to make explicit (2026-09-24)

**Root cause:** The explicit-buffer-targeting refactor (9cebd29e..1804caf2)
correctly threaded `bid` through every Steel *entry point* — keymap dispatch,
typed-command dispatch, `lsp-request` callbacks, hooks — each of which had a
leading-parameter slot the injection could land in. `(call! "native-cmd")`
has no such slot: a native command's Rust body has never taken a buffer
parameter at all, so there was nothing to make explicit, and the seam was
missed entirely. `run_command_sync` kept reading live focus, meaning a
`call!` to a native command from a hook or async callback silently acted on
whatever buffer was focused when it ran — the exact bug class the refactor
existed to remove, surviving in the one place the "add a parameter" pattern
didn't apply.

A second, independent gap in the same refactor: `close_buffer`'s
last-buffer branch reused the closed buffer's `BufferId` in place for a
fresh scratch buffer, rather than freeing the slot. This wasn't a missed
seam so much as an unexamined downstream consequence — every other part of
the codebase relies on a versioned slotmap key being unable to alias a
different buffer's content once closed (that's the whole *point* of using
one), and this one code path quietly violated it. `Buffer::replace_stamp`
existed only to patch around the violation for `:reload-config`'s own
snapshot/resync, rather than removing it.

**Concrete instance:** user asked for an audit of the refactor's own
back-and-forth, suspecting an unresolved problem. Three parallel Explore
agents found: native `call!` has no bid parameter to inject into at all
(`run_command_sync` had none); `ResponseAnchor` was checked once at LSP
drain time but the Steel callback only runs later, after arbitrary other
queued work; `lsp-position->offset`/`lsp-range->offsets` took a tagged
`JsonHandle` and discarded the tag, re-resolving encoding from the buffer's
*current* attachment (the exact bug `a548a117` fixed everywhere else,
missed on these two); and the last-buffer scratch replacement. All four
were fixed at the funnel each one's own class of bug lived in, not patched
locally.

**Prevention rule:** "Every path that does X now takes Y explicitly" is a
claim about paths that already had a parameter list for Y to join. It says
nothing about a path that acted on Y *implicitly*, with no parameter at
all — `call!` to a native command reads focus not because a bid parameter
was left unfilled, but because there was never a parameter to fill. When
auditing or writing this class of refactor, explicitly enumerate every way
the *old* implicit behavior was reached, not just every explicit parameter
list added — a grep for "takes bid" finds every site that already has one;
it structurally cannot find the site that doesn't.

The second half generalizes further: an explicit-identity refactor is only
as strong as the identity invariant it depends on. Before trusting a
versioned key as sufficient proof of "not aliased," check every path that
mutates the underlying store for one that swaps content in place under a
surviving key — the exact class of shortcut a versioned-key design is
supposed to make impossible to take.

**Files:** `hume-editor/src/editor/host_impl/commands.rs`
(`run_command_sync`'s focused-buffer check), `hume-scripting/src/host/commands.rs`,
`hume-editor/src/editor/buffer/lifecycle.rs` (`close_buffer`'s last-buffer
branch), `hume-editor/src/editor/lsp/mod.rs` (`ResponseAnchor`),
`hume-editor/src/editor/scripting_setup.rs` (`run_pending_batch`'s
per-call anchor re-check), `hume-scripting/src/builtins/lsp.rs`
(`lsp-position->offset`/`lsp-range->offsets`).

---

## L20 — Four target categories for two real kinds; handed a pane, read focus anyway (2026-09-25)

**Root cause:** The explicit-target refactor (17ad8600 and five follow-ups)
gave native commands four target categories — `Pane`, `FocusedPane`,
`Buffer`, `Global` — each with its own fn type, builder, resolved-target
variant, and resolver arm, plus a separate `Scope` enum saying whether the
target was focus. Four parallel enums (`TargetCategory`, `EditorCmdBody`,
`ResolvedTarget`, `Scope`) were kept in agreement by `unreachable!()` arms
and five resolver functions. Nobody counted what the commands actually
needed: two `Buffer` commands and four `Global` ones, and every one of the
six turned out to act through a pane (a per-pane search cursor) or on focus
(a mode layer, the Extend flag, a tab switch). Separately, 31 `FocusedPane`
bodies received the focused pane as a parameter and most ignored it,
re-minting `FocusedPane::current` inside; Insert teardown read focus instead
of the session that recorded its own owner, which made "end sessions before
the focus write" an ordering rule every caller had to respect.

**Concrete instance:** a blank-sheet review (2026-09-25) collapsed the
categories to `Pane`/`FocusedPane`, replaced `Scope` with
`Target::focused()`, replaced five resolvers with `CommandPane::resolve`/
`FocusedPane::resolve`/`Target::resolve`, made teardown read
`EditSession.pane`/`.buffer`, and gave typed `:` commands the focused pane
at invocation. A test that moves focus raw before the Insert layer pops
panicked on the old teardown's consistency assert — the ordering rule had
been the only thing keeping it correct. A same-day follow-up cleanup pass
folded the keypress-only `Target::at_focus` into `Target::resolve` itself
(a `Pane`-category handle that names the focused pane now comes back
`Focused` from `resolve` too), so every mint site left is `Target::resolve`
or a bare `Target::Focused(fp)`.

**Prevention rules:**

1. Before adding a category type, list the commands in each category. A
   category with two or four members is a question, not a design: check
   whether each member really lacks what the other categories have.
2. Parallel enums whose variants must agree, held together by
   `unreachable!()`, mean one of them is redundant. Derive the others from
   one source, or merge them.
3. A function handed a resolved value (a pane, a session owner) must use it.
   Re-reading the ambient equivalent (`FocusedPane::current`,
   `state.focus.id()`) inside the body is the implicit-target bug L19
   describes, reintroduced one layer down. Mint from ambient state only at
   the entry point where the input arrives.
4. State that records its own owner (an `EditSession`) is the owner's
   source of truth for teardown. Reading it from elsewhere turns a data
   fact into a call-ordering convention.

**Files:** `hume-editor/src/editor/commands/pipeline.rs` (`Target`,
`CommandPane::resolve`, `FocusedPane::resolve`, `run`, `run_body`),
`hume-editor/src/editor/registry/command.rs` (`TargetCategory`),
`hume-editor/src/editor/commands/insert_session.rs` (`tear_down_insert`),
`hume-editor/src/editor/doc_ops.rs` (`commit_edit_group`),
`hume-editor/src/editor/tests/insert_session_buffer_switch.rs`
(`insert_teardown_commits_on_the_sessions_own_pane_not_current_focus`).

---

## L21 — One session-kind variant meant two things, gated by an incidental property (2026-09-25)

**Root cause:** `Editor::replay_dot` pre-opens a placeholder `EditSession`
before dispatching the replayed command, to fold a recipe replay plus the
main edit into one undo revision and to signal `begin_insert_session` that
keystroke recording should be suppressed. The placeholder reused
`EditSessionKind::Insert` — the same variant a real, live Insert session
uses — and `open_or_retarget`'s retarget rule told the two apart only by
`is_empty()` (no edits composed yet). A genuinely real Insert session, right
after `i`/`a`/`o` and before the first keystroke, is *also* empty. A
`(call! "paste-after" pane)` from a hook or timer landing in that one-
keystroke window passed the emptiness check and silently retargeted the
live session to `Paste`, corrupting `active_session` while the
`InsertLayer` stayed on the mode stack — the next keystroke would then
panic on `apply_doc_edit_grouped`'s `is_insert_at` filter.

**Concrete instance:** caught by the altitude review of the Insert/paste-
session-ownership refactor (`aa5049c5..HEAD`, `3791a746`'s `/simplify`
pass) before release — no shipped regression. `EditSessionKind::Replay`
now gives the placeholder its own tag; only that kind is eligible for
`open_or_retarget`'s retarget branch, so a real, even-empty `Insert`/`Paste`
session correctly refuses instead. `begin_insert_session_preserving_
register`'s single "is a group already open" check had to split into two —
whether to open/retarget the group (needed unless a real Insert session is
already here) and whether to suppress fresh keystroke recording (needed
whenever *either* a real Insert session or the Replay placeholder is
already here) — since the two questions only had the same answer by
coincidence, back when both cases shared one variant.

**Prevention rules:**

1. A placeholder standing in for "not yet decided" needs its own explicit
   tag, not an incidental property (`is_empty()`, a `None`, a default) of a
   real variant it's borrowing. The incidental property will eventually
   also be true of the real thing.
2. When two states are told apart only by a derived condition, ask what the
   condition would have to mean for *every* variant it's checked against —
   not just the one it was written for.
3. When a design collapses two different questions into one check because
   they happen to have the same answer under the current representation,
   changing that representation is the signal to re-split them — don't
   assume the coincidence was structural.

**Files:** `hume-editor/src/editor/edit_session.rs` (`EditSessionKind::
Replay`, `is_replay_at`, `blocks_open`), `hume-editor/src/editor/replay.rs`
(`replay_dot`'s pre-open, `finish_replay_session`),
`hume-editor/src/editor/commands/insert_session.rs`
(`begin_insert_session_preserving_register`),
`hume-editor/src/editor/tests/paste.rs`
(`paste_during_an_empty_open_insert_session_is_refused_and_leaves_the_session_intact`).

---

## L22 — Unreleased changelog entries narrated internals and pre-empted future history (2026-09-25)

**Root cause:** `## Unreleased` entries were drafted the way a PR description
or a design doc would be written — explaining *how* a change works
internally (JSON handles crossing the FFI boundary, a completion source
answering "sync or from a later callback", `#:resolve` "licensing
`completionItem/resolve` on accept") and, for features with no prior
release, narrating the limitation of an approximation nobody ever shipped or
saw (`core:buffer-words` "no longer merges the words around it into one
unreachable candidate", as if replacing its own past behavior). Both defects
share one cause: the changelog's audience was never checked against the
target audience for end-user docs stated elsewhere in this file — a
changelog line is a release note, not a design rationale or a diff summary,
and an *unreleased* entry has no "previously"/"no longer" to narrate because
nothing it describes has ever been in a user's hands.

**Prevention rules:**

1. A changelog entry states the current behavior in one line — what changed,
   for the person using the editor. Configuration and capability detail
   belongs in the user manual; internal mechanism (FFI encoding, callback
   timing, why a flag is licensed) belongs in a source comment, never here.
2. An entry under `## Unreleased` describes a feature as it stands today,
   never as an improvement over an earlier draft of itself — there is no
   released baseline to be "no longer" replacing.
3. Before rewriting or reviewing a changelog section, diff its claims
   against the last release tag (`git tag`/`git grep <symbol> <tag>`) to
   tell "this is a real behavior change since release" from "this is an
   unreleased feature being second-guessed against its own history."

**Files:** `CHANGELOG.md` (`## Unreleased`).

---

## L23 — Dot-repeat inferred a dispatch's effect from side signals, and replayed an interactive one by re-running it (2026-09-26)

**Root cause:** Dot-repeat recorded an Insert session by watching each
keymap-bound dispatch and inferring, afterward, what it had done: a
`text_gen` diff decided "did the binding edit?", `in_insert_key_dispatch`
decided "is a binding in flight?", a branch in `handle_insert` decided "did
the binding close the session?", `tear_down_insert` moved keystrokes into
the action with `extend` so a re-entering binding kept them, and the
`Replay` edit-session kind doubled as "suppress recording" — each signal
patched over a bug the previous one couldn't see. An accepted completion
was still never recorded this way, and arrow keys were dropped from replay
because a pure motion doesn't move `text_gen`. Recording every input at the
seam it arrives (`Key`, `Paste`, `Binding`, unconditionally before its
dispatch) fixed that half, but replay still re-ran every `Binding`,
including one that had gone *interactive* — whose outcome depends on input
the user gave while it ran (accepting a completion, picking a picker item).
Only the one value a re-run couldn't re-derive (a completion's pick) had
anywhere to go, handed back through a per-binding answer slot for
`completion-accept!` alone to consume. Every other question the binding
could ask about that session (`completion-top`) saw nothing on replay and
could take a different branch than it did live; a binding that opened a
picker had no slot to record a pick into at all, so `.` reopened the picker
instead of applying it; and a failed or unregistered binding stopped the
whole replay, dropping everything typed after it.

**Concrete instance:** a code review surfaced the completion-top and
picker-reopening gaps. The user's own framing named the actual fix: a
picker is an interactive command like autocomplete, so it must not be run
again — dot-repeat inserts its result.

**Resolution:** Record the input at the seam it arrives, never a conclusion
drawn from side effects afterward. An interactive input is never re-run:
`completion-accept!`/`picker!`/`live-picker!` mark the dispatch that called
them (`EditorState::mark_dot_interactive`); a checkpoint right after it
returns (`Editor::resolve_or_arm_dot_capture`) either finalizes on the spot
or, for a picker whose pick resolves later via a queued `on_select`, arms
`EditorState::dot_capture` for `resolve_dot_capture_if_ready` to finalize
once it closes. Finalizing diffs the Insert session's own composed
`ChangeSet` (before vs. after, via `ChangeSet::invert`/`compose`) into an
`InsertInput::Result` — the whole dispatch's net edit, replayed by applying
it directly rather than asking any one builtin to describe its own effect.
A `Binding` that fails or is unregistered on replay is reported and
skipped, not fatal to the rest — the live session kept going past it too.

`invert(X).compose(X)` does not reduce to `is_identity()` even when
nothing happened between two snapshots of the same `ChangeSet`:
`compose`'s own doc says a *self*-Insert consumed by an *other*-Delete
cancels, but a *self*-Delete followed by an *other*-Insert of the same
text — exactly what an inverted `before` composed with an equal `after`
produces — does not; both are emitted verbatim. A structural
`is_identity()` check isn't enough; the diff is confirmed against the
actual text instead.

**Prevention rules:**

1. Record the input at the seam where it arrives, never a conclusion drawn
   from side effects after the fact. A counter diff or a length snapshot
   answers "did something change?", which is never quite the question, and
   it breaks the first time two things change or the thing it measures is
   replaced mid-dispatch.
2. When a replay mechanism re-executes commands, find the inputs a re-run
   can't re-derive (a user's pick, a prompt answer). If any part of an
   input's outcome can't be re-derived, don't re-run any of it: record its
   whole net effect and replay that instead. Handing back just the one
   un-derivable value to an otherwise-live re-run still leaves every other
   side of that command's behavior — a session query, a further edit —
   exposed to running against a different world than it did live.
3. The Nth signal added to disambiguate the (N-1)th is L9's smell applied
   to state instead of call sites. At the third, redesign.

**Addendum (2026-09-26):** the resolution above still inferred, one level
up: it diffed the Insert session's whole composed `ChangeSet` before vs.
after the capture window (`invert`/`compose` against the session-open
snapshot) to recover what happened *during* the window, rather than
recording what happened as it happened. That diff conflated any edit typed
earlier in the session with the capture's own edit — `compose`'s own doc
warns a self-Delete/other-Insert pair of the same text doesn't collapse
back to identity — so a session with more than one typed run before the
capture, or a multi-cursor accept, produced a delta with more than one
edited region and had nowhere to put the extra one but an error. A code
review surfaced this, plus a completion accept immediately followed by
`exit-insert` in the same dispatch: the checkpoint that would have
finalized the capture never ran, because the session it belonged to had
already been torn down by the time control returned to it.

**Rule 4 (addendum):** diffing a *before* and *after* snapshot of an
accumulator to recover what happened in between is the same inference L23's
main text warns about, one layer removed — it answers "what changed since
the snapshot," not "what did the operation I'm capturing actually do."
Record each contributing edit directly, at the one funnel it must already
pass through (here, `doc_ops::apply_doc_edit_grouped`), and compose just
those. This also fixes the "session torn down before the checkpoint" case
for free: tying the capture's lifetime to the session that owns the funnel
(rather than to a separate global flag) means the session's own teardown
can resolve a still-armed capture as its last act, instead of leaving it to
a checkpoint that may never run.

**Files:** `hume-editor/src/editor/edit_session.rs` (`DotCapture`, now a
field on `EditSession`, and its `arm_dot_capture`/`dot_capture_mut`/
`take_dot_capture`), `hume-editor/src/editor/doc_ops.rs`
(`apply_doc_edit_grouped`'s own funnel push),
`hume-editor/src/editor/replay.rs` (`InsertInput`, `CursorReplacement`,
`record_insert_input`, `mark_dot_interactive`, `arm_dot_capture`,
`resolve_dot_capture`, `finalize_dot_capture`, `cursor_replacement_at`),
`hume-editor/src/editor/input_stack/insert.rs` (`handle_insert`),
`hume-editor/src/editor/input_stack/completion.rs`
(`accept_completion_selection`),
`hume-editor/src/editor/commands/insert_session.rs` (`tear_down_insert`'s
own backstop),
`hume-editor/src/editor/completion/session/accept.rs`,
`hume-editor/src/editor/commands/mode.rs` (`cmd_completion_trigger`),
`hume-editor/src/editor/host_impl/ui.rs`,
`hume-editor/src/editor/commands/pipeline.rs` (`repeat_slot_owned`),
`hume-editor/src/editor/scripting_setup.rs` (`drain_pending_work`).

**Addendum 2 (2026-09-26):** `resolve_dot_capture` finalized a still-armed
capture whenever `input.picker().is_some()` came back false — a third
signal, checked from three separate call sites in `drain_pending_work`,
standing in for "has the thing this capture was waiting on resolved yet?"
Rule 3 above already named the fix at N=3: stop inferring readiness from an
unrelated poll and give the capture to whatever is actually responsible for
resolving it. `DotCapture` now moves with the cause: `picker::open_picker`
takes it off the `EditSession` and attaches it to the `PickerSession`;
closing the picker hands it to the queued `on_select` call
(`PendingWork::Call::dot_capture`); `Editor::run_pending_batch` re-arms it
on whatever session is current only for that one call. A capture with
nothing armed on it (dropped by a cascade cap, or a picker torn down by
`reset_config_state`) is simply never finalized — the same outcome the poll
produced, without a poll. This also closed two bugs the poll's own blind
spot let through: an edit landing on the same buffer through a foreign
`Call` while a picker sat open used to feed the still-armed-but-parked
capture (nothing detached it), corrupting the composed result;
`accept_completion_selection`'s own early `Err` return skipped
`resolve_dot_capture` entirely, leaking the capture until the next
drain-empty poll happened to catch it — both closed by wrapping the
operation in a closure (`Editor::with_dot_capture`) that always finalizes
once the closure returns, success or error alike.

**Files (addendum 2):** `hume-editor/src/editor/edit_session.rs`
(`DotCapture::pane`/`buffer`, `fallback` replacing `has_placeholder`),
`hume-editor/src/editor/replay.rs` (`with_dot_capture`/`run_dot_captured`
replacing `arm_dot_capture`/`resolve_dot_capture`),
`hume-editor/src/editor/input_stack/picker/{mod,session}.rs`
(`PickerSession::dot_capture`, `open_picker`'s hand-off),
`hume-editor/src/editor/event.rs` (`PendingWork::Call::dot_capture`),
`hume-editor/src/editor/mod.rs` (`queue_steel_call_with_capture`),
`hume-editor/src/editor/scripting_setup.rs` (`run_pending_batch`, the three
removed poll sites).

---

## L24 — One "(not yet)" request became a blanket rule (2026-09-27)

**Root cause:** Asked to add "(not yet)" to undo persistence, plus a
general note not to describe missing features as never coming, the
migration guides got "yet"/"for now" added to about twenty gaps. The
general note asked for neutral wording, not a promise: not every missing
feature will be implemented.

**Prevention rule:** State a missing feature plainly ("HUME has no marks",
"*(none)*"), and rewrite only phrases that imply permanence ("the closest
HUME gets", "fixed rather than user-declarable"). Use "not yet" only for a
gap the user has confirmed is planned. When one instruction is specific and
the next is general, don't apply the specific fix everywhere; ask if unsure.

---

## L25 — A user-facing convenience was deleted as "unused" (2026-09-28)

**Root cause:** An API-consistency review listed `split-words` twice. The
plan resolved it by deleting `core:stdlib`'s `stdlib/split-words` because
no shipped plugin called it. The command was a convenience for plugin
authors: it spares them reading `word-chars` themselves. Its one in-tree
caller had been moved off it for a performance reason, and nobody asked
why the command was kept.

**Prevention rule:** "No callers in the repo" is not evidence that a
user-facing API (builtin, stdlib command, option) is dead. Before
proposing to remove one, read the commit that added it and the commit
that removed its last caller, and state both reasons in the plan. If an
in-tree caller left for a fixable reason (cost, shape), fix that and keep
the caller rather than dropping the API.

---

## L28 — A model hid one accessor and kept its raw constructors (2026-09-29)

**Root cause:** L27's fix made `Selection::end()` crate-private and added
typed reads, but `Selection::new`, `collapsed` and `directed` stayed public and
took raw char offsets. Cluster alignment was still restored at runtime by five
separate snap funnels, commands still assembled ranges and char counts from
seven accessors, the engine kept its own selection type, and stored selections
were translated by scattered code. A review found nine defects hiding behind
the snaps. The first repair plan narrowed constructors but kept the second type,
the count-based builder calls and an unenforced tie between a selection and its
text, weighing each against its blast radius. The user rejected it.

**Prevention rule:** When a model must be correct by construction, list every
way to build, read and store it. Type the values it is built from so only the
right primitives produce them, pair it with the data it is valid for, and give
it exactly one repair point. One type per concept across crates. Blast radius
is not a reason to keep an old shape; state the gaps no type can close instead.

---

## L26 — A combining-mark bug hid behind ASCII-only tests (2026-09-29)

**Root cause:** A refactor of the word primitives fixed a bug where a
cursor on a trailing combining mark was mishandled. Nothing had caught it
because the tests around word motions, text objects, edits and the rope
primitives used ASCII text, or one recycled `cafe\u{301}` literal, while the
editor claims to handle grapheme clusters. The generators behind the
proptests drew from `a-z`, space and newline, and the selection generator
picked arbitrary char offsets, so no test could produce a selection that
split a cluster or notice one.

**Prevention rule:** A test of a text-sensitive primitive or command that
uses only ASCII is a coverage gap. Draw its inputs from
`test_fixtures::unicode` for the classes that primitive can mishandle
(precomposed and combining letters, ZWJ and flag sequences, wide and
astral chars, no-break and ideographic spaces, a chunk-straddling rope for
anything that walks clusters). Generators for property tests draw from the
same corpus, and their invariant checker asserts cluster alignment.

---

## L27 — An invariant kept by convention was audited, not enforced (2026-09-29)

**Root cause:** The audit found selection ends sitting on a cluster's last
codepoint in some commands and on its first in others, because
`Selection::end()` was a `CharOffset` that could be used as a range bound,
a position or a line lookup. The first plan fixed each producer and
proposed an audit of the raw reads. The user asked for the compiler to
enforce it instead.

**Prevention rule:** When a value must be read or built in one way, make the
raw form unreachable (`pub(crate)` accessor) and add the typed conversion
(`span`, `from_span`, `end_exclusive`). Then the compiler lists every
outside site, and a new command cannot reintroduce the bug. Offer that
form in the plan's altitude line before proposing a grep-based audit.

---

## L29 — Caller patches worked around what the model permitted (2026-09-30)

**Root cause:** A review of the cluster-position selection model found
defects the model itself allowed: an edit builder whose result depended on the
order operations were recorded, a selection set that was silently re-fitted
when paired with another text, registers that lost the shape of what was
yanked, and stored positions (jump entries, the last insertion, the completion
session) carried by hand at each write site. The first repairs patched each
caller. The user rejected them: working around a model weakness in the caller
is the wrong altitude and breaks separation of concerns. The same shape had
already been documented for two-step calls (L2).

**Prevention rule:** When a defect comes from what the model permits, change
the model and revert the caller patches: make operations order-independent,
refuse the wrong pairing loudly, record a fact where it is known (a register
piece's shape), and carry every stored position from one chokepoint
(`PositionStores`). Before patching a caller, ask which type or funnel made the
bug possible, and state it in the plan's altitude line.

---

## L30 — The completion token was modeled as "all word characters" (2026-09-30)

**Root cause:** The proposed completion redesign derived "did the cursor leave
the token" from the buffer's word characters, the rule the old code already
hardcoded for every source. A source or language server can define a token
with other characters (`foo-bar`, `$var`, a path, a dotted member). The user
asked what happens when one does.

**Prevention rule:** When a redesign turns an existing hardcoded rule into
derived policy, list who owns the definition of the thing the rule classifies.
Here the source declares extra token characters (`#:token-chars`) and a server's
own edit range says where an item's token starts; the editor supplies only the
default.

---

## L31 — Three structural-newline exceptions hid one result-level rule (2026-09-30)

**Root cause:** Typing the edit builder's inputs as `ClusterRange` stalled on
`d` over an empty last line, which needed an empty range tagged with its
lines so a finish-time pass could move the deletion back one break. The
proposed fix kept that shape (`delete_lines(lines)`). It protected "the text
ends with `\n`" through three per-operation rules: every deletion clamped
short of the structural `\n`, whole-line runs to the last line moved back,
and a replacement reaching the break dropping its own final `\n`. The user
asked why not delete what the selections cover and fix the final `\n` once.

**Prevention rule:** When an invariant is defended at each operation and an
edge case needs a special value to survive until the end, test the invariant
on the result instead: here, when the result would not end with `\n`, every
deletion stops before the structural one. State it so the edit stays minimal
(stop the deletion, don't delete and re-insert), and delete the per-operation
rules it replaces.

## L32 — A deviation report argued for the smaller option on cost (2026-09-30)

**Root cause:** Routing LSP diagnostics and decorations through
`PositionStores::carry` had been approved. Mid-implementation it turned out
to need moving `DiagnosticsStore` out of `LspState` and changing borrows at
fifteen sites. The deviation report recommended documenting the two
exceptions instead, arguing from that cost and describing the flush-time
remap as "paired with the didChange stream", with no word on what the
exceptions let go wrong. The user asked for the options' pros and cons from
a correctness point of view, stating churn was not a concern. That analysis
found a reachable bug in the recommended option: a decoration set right
after an edit was shifted a second time by the next flush. A red test then
confirmed it.

**Prevention rule:** A deviation report weighs the options on what each
lets go wrong: stale windows, reachable misplacements, invariants left to
convention. Name the failure each permits before naming its cost. If the
smaller option has a correctness gap, it is not the recommendation, whatever
the churn. Write the test for the gap before choosing.

## L33 — A deviation report recommended skipping on an unmeasured churn count (2026-10-01)

**Root cause:** Threading the acting pane through `Buffer::install` and the
`apply_edit*` family had been approved. The deviation report recommended
skipping it, citing "about 130 call sites". The count came from one `grep -c`
that matched 119 calls to a test helper's own `apply_edit` method. The real
change was about ten signatures and call sites, and the third option on
offer (a mutable "acting" flag on `PositionStores`) was a convention-kept
invariant. The user asked for the correct option and told us to ignore
churn.

**Prevention rule:** Count what a change touches by reading the call sites
(group by caller, separate wrappers from real call sites) before putting a
number in a report. Even a true number is not a reason to recommend the
option that leaves wasted work or a convention-kept invariant in place.

## L34 — A review finding was relayed before it was checked against the code (2026-10-02)

**Root cause:** A code-review run reported that splitting `core:lsp-install`
left `:lsp-install` undefined. The summary passed that finding to the user
as stated. The default declaration goes through the manifest, which declares
the commands entry, so the command was defined and completed. The finding
only held for `load-plugin!` and for an explicit `#:typed-commands` declare.

**Prevention rule:** Open the code a review finding names and trace the
default path before presenting it. State which setups the finding applies to,
and mark any finding not yet traced as unverified.

## L35 — A plan moved root markers onto the language and kept them on the server (2026-10-06)

**Root cause:** The plan made root markers a language property
(`define-language! #:roots`) and still kept `register-lsp-server!`'s
`#:root-markers`, merged in front of the language's. Two carriers of one
property survived, which is the duplicate-source shape the redesign existed
to remove. The user asked why the keyword was still there.

**Prevention rule:** When a plan moves a property to its owner, list every
field, keyword and merge step that carried it before, and remove each one in
the same plan. A consumer that needs the property (`core:steel-server`'s
`cog.scm`) then supplies it through the owner.
