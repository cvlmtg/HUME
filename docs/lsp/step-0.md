# LSP Step 0 — Prerequisites (task cards)

Workspace groundwork with no LSP-visible behavior. Read `docs/LSP.md` (the hub) first — especially the *protocol primer* and the *orientation map*.

Ordering inside this step: **P1 → P8 → P2 → P3 → P7 → P4**, then C1 (step 1), then **P5 → P6**. P5/P6 live in `hume-lsp`, so they need C1's crate scaffold.

---

### P1 — serde + serde_json workspace deps

**Goal** — `serde` (with derive) and `serde_json` available as `[workspace.dependencies]`; the recorded toml 0.8 → 1.x upgrade trigger evaluated.

**Depends** — nothing. **Unlocks** — P8, C1.

**Files** — `Cargo.toml` (workspace root); possibly `hume-engine/Cargo.toml` (toml version).

**Read first** — root `Cargo.toml` `[workspace.dependencies]`; `hume-engine/Cargo.toml` (`toml = "0.8", default-features = false, features = ["parse"]` — used only for Helix theme parsing).

**Shape**
```toml
# [workspace.dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```
Crates opt in with `serde.workspace = true` — only where actually used (nothing uses them until P8/C1).

**Toml trigger** — serde entering the workspace is the recorded trigger to evaluate bumping `hume-engine` to toml 1.x (its `Value` moves behind a serde feature there). If the upgrade is a mechanical version bump with green tests, do it here; if it causes API churn in the theme loader, stay on 0.8 and record the rationale in `docs/ROADMAP.md`'s decisions table. Do not rabbit-hole — this is a side quest with a 30-minute budget.

**Tests** — `cargo build --workspace` + `cargo test --workspace`; `cargo tree -d` shows no duplicate serde versions.

**Done when** — deps present; toml decision recorded (either way); workspace green.

**Traps** — don't add `serde` to `hume-editing`/`hume-engine` dependency lists "for later"; each crate adds it in the task that first needs it.

**Size** — ~10 lines.

---

### P2 — Batch position mapping

**Goal** — a public batch API for mapping sorted position lists (and ranges) through a `ChangeSet` in one cursor pass. C9 and B5 remap stored diagnostics/decorations through every edit with this.

**Depends** — nothing. **Unlocks** — C9, B5.

**Files** — `hume-editing/src/changeset/mod.rs` (+ its test module).

**Read first** — `PosMapCursor` (struct + `map`), `map_pos` (the one-shot convenience), `Assoc` docs — all in that file. Note the existing invariant: `PosMapCursor.map` requires queries in non-decreasing position order.

**Mimic** — the existing `map_pos` doc/test style.

**Shape**
```rust
// Widen these two from pub(crate) to pub, with doc comments:
pub struct PosMapCursor<'a> { /* unchanged */ }
pub enum Assoc { Before, After }

impl ChangeSet {
    /// Map a sorted slice of positions in place, one cursor pass.
    /// debug_asserts the slice is sorted.
    pub fn map_positions(&self, positions: &mut [usize], assoc: Assoc);

    /// Map (start, end) ranges sorted by start, in place. Starts map with
    /// Assoc::After and ends with Assoc::Before, so a range shrinks (never
    /// swallows neighbouring text) when edits land at its edges; a range
    /// fully inside a deletion collapses to an empty range at the deletion
    /// point (caller may then drop it).
    pub fn map_ranges(&self, ranges: &mut [(usize, usize)]);
}
```
`map_ranges` needs *two* interleaved cursors or one pass over a flattened sorted list — start positions and end positions are each sorted, but interleaved. Simplest correct approach: one `PosMapCursor` pass over starts, a second pass over ends (two O(ops + n) passes beat one clever pass; keep it simple).

**Tests** — tier 1 (`cargo test -p hume-editing`). For each op kind (Retain/Delete/Insert): positions before / at / inside / after the op. Ranges: edit before, inside, spanning, at-start, at-end. **Oracle**: compare every element against the existing one-shot `map_pos` (independent implementation of the same semantics). Plus: `debug_assert` firing on unsorted input (`#[should_panic]` in a debug-assertions test), empty slice, `end < start` never produced (assert `start <= end` for all outputs).

**Done when** — API public + documented; tests green; no behavior change to existing `map` callers (`SelectionSet::translate_in_place` untouched).

**Traps**
- Do not change `map`'s existing semantics or visibility of anything else in the module.
- A position inside a deleted span maps to the deletion start — document it; C9 relies on it.
- Don't add a per-element `Assoc` parameter "for flexibility" — no caller needs it (premature abstraction).

**Size** — ~60 source + ~140 test lines.

---

### P3 — Generalized event-loop wake

**Goal** — the run loop's "is async work pending?" check composes over all async sources (parse worker now; LSP backend and timer wheel later) instead of hard-coding `parse_worker.has_in_flight()`; `prepare_frame` gets a named drain phase new sources plug into.

**Depends** — nothing. **Unlocks** — P7, C6/C9 drain, B4.

**Files** — new `hume-editor/src/editor/async_source.rs`; modify `hume-editor/src/editor/lifecycle.rs`, `hume-editor/src/editor/mod.rs` (module decl).

**Read first** — `lifecycle.rs`: the event section of the run loop (search `has_in_flight`) and `prepare_frame` (search `fn prepare_frame`); `editor/syntax/parse.rs` `reparse_stale_buffers` (the existing drain, called from the frame path).

**Mimic** — nothing directly; this is a small extraction refactor.

**Shape**
```rust
/// One source of asynchronous work the event loop must wake for.
/// Implemented by the parse worker (now), the LSP backend (C6) and the
/// timer wheel (P7).
pub(crate) trait AsyncSource {
    /// Work may complete soon — poll instead of blocking on input.
    fn has_pending(&self) -> bool;
    /// Absolute wake deadline, if this source schedules timed work (P7).
    fn next_deadline(&self) -> Option<std::time::Instant> { None }
}

impl Editor {
    /// One place to enumerate sources. Adding a source = one line here
    /// plus its AsyncSource impl.
    fn async_sources(&self) -> [&dyn AsyncSource; 1 /* grows */];

    /// Some(timeout) => poll with it; None => block on event::read().
    /// timeout = min(8ms if any source has_pending, nearest deadline - now).
    fn wake_timeout(&self) -> Option<std::time::Duration>;
}
```
The run loop's `if self.parse_worker.has_in_flight() && !event::poll(8ms)` becomes `if let Some(t) = self.wake_timeout() { if !event::poll(t)? { continue; } }` — same control flow, same 8 ms constant for the has-pending case. The parse worker's `AsyncSource` impl is a thin wrapper (implement on a newtype or directly on `Box<dyn ParseBackend>`'s holder — whatever borrows cleanly; don't fight the borrow checker for elegance points).

`prepare_frame` change: introduce a clearly-named `self.drain_async_sources(…)` step that currently only calls the existing `reparse_stale_buffers` path; C6/C9/B4 will add their drains inside it.

**Tests** — tier 2. Existing parse-worker integration tests must stay green unchanged (they are the behavior spec). Add: `wake_timeout()` returns `None` with no pending work (idle = blocking read — the no-busy-loop guarantee), `Some(8ms)` with an in-flight parse, `Some(<8ms)` when a P7 deadline is nearer (written now with a stub source or added in P7 — either is fine, but the min logic ships here).

**Done when** — loop behavior identical for the parse-only world; trait + enumeration point exist; `prepare_frame` has the named drain phase.

**Traps**
- Don't touch the resize-drain logic or the cursor-shape sequencing around the event section.
- Idle must remain a *blocking* read. A pending-but-distant timer deadline is not "pending work" — it bounds the timeout via `next_deadline`, it must not trigger the 8 ms spin.

**Size** — ~90 source + ~60 test lines.

---

### P4 — Position encoding conversion

**Goal** — convert rope char offsets ↔ wire `(line, character)` positions in both `utf-8` and `utf-16` encodings. Every protocol position in every later task goes through these two functions.

**Depends** — nothing. **Unlocks** — P6, C7, C9, B6.

**Files** — new `hume-editing/src/position_encoding.rs`; export from `hume-editing/src/lib.rs`.

**Read first** — `hume-editing` rope helpers (`lines.rs` neighbors) for module conventions; ropey docs for `char_to_line`, `line_to_char`, `char_to_byte`, `char_to_utf16_cu`, `utf16_cu_to_char` (all unconditional in ropey 1.6).

**Shape**
```rust
/// Wire-format position encoding negotiated with a server (hub: primer).
/// hume-editing must not depend on lsp-types, so this mirrors LSP Position
/// as a plain tuple; hume-lsp converts to/from lsp_types::Position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PositionEncoding { Utf8, Utf16 }

/// char offset -> (line, character) in `enc` code units.
pub fn char_to_wire(text: &Rope, char_idx: usize, enc: PositionEncoding) -> (usize, usize);

/// (line, character) -> char offset. Out-of-range input CLAMPS:
/// line past EOF -> last line; character past line end -> line end
/// (LSP spec behavior — servers send past-end positions routinely).
pub fn wire_to_char(text: &Rope, line: usize, character: usize, enc: PositionEncoding) -> usize;
```
Utf8: `character` = byte offset within the line (`char_to_byte(idx) - char_to_byte(line_start)`). Utf16: same with `char_to_utf16_cu`. Both directions derive the line via `char_to_line` / `line_to_char`.

**Tests** — tier 1. Cases: pure ASCII; `é` as one char (2 UTF-8 bytes / 1 UTF-16 unit); astral emoji `𐍈` or `😀` (4 bytes / 2 units); position at line start / line end / on the newline; EOF; the buffer-invariant trailing `\n` (a buffer is never empty — `"\n"` is the minimum); clamping: `character` past line end, `line` past EOF; round-trip property `wire_to_char(char_to_wire(i)) == i` for every char index of a mixed-content fixture, both encodings.

**Done when** — both functions total (no panics on any input), documented, exported; tests green.

**Traps**
- Never treat `character` as a char count or byte count without the encoding branch — that is the whole point of this task.
- Clamping must land on a **char boundary** (it does by construction if you clamp in code-unit space then convert — verify with the emoji tests: a `character` value that splits a surrogate pair clamps down, not mid-char).
- Grapheme helpers are wrong here (wire math is not motion math — hub: testing playbook).

**Size** — ~90 source + ~150 test lines.

---

### P5 — Path ↔ `file://` URI *(lives in `hume-lsp` — do after C1)*

**Goal** — lossless, canonical path↔URI conversion. Outgoing URIs always come from the canonical `Buffer.path` (the SSOT); incoming URIs are converted to paths here and canonicalized by the caller before buffer lookup.

**Depends** — C1. **Unlocks** — C5, C7, C9, B6.

**Files** — new `hume-lsp/src/uri.rs`.

**Read first** — `hume-platform`'s UNC handling (search `strip_unc_prefix`) — the display convention this must respect on Windows; `lsp_types`' `Uri` type for the version picked in C1 (0.96+ replaced the `url` crate with a lighter URI type — construct via `FromStr`).

**Shape**
```rust
pub enum UriError { NotFileScheme, NotAbsolute, Decode(String) }

/// Absolute path -> file:// URI. Percent-encodes everything but unreserved
/// chars and '/'. Windows: drive letters render as file:///C:/…, backslashes
/// become '/', and any \\?\ verbatim prefix is stripped first.
pub fn path_to_uri(path: &Path) -> Result<lsp_types::Uri, UriError>;

/// file:// URI -> PathBuf. Accepts empty and "localhost" authority; rejects
/// other schemes/authorities loudly (fail fast — no lossy fallback).
pub fn uri_to_path(uri: &lsp_types::Uri) -> Result<PathBuf, UriError>;
```

**Tests** — tier 1. Round-trip: plain ASCII path, path with spaces, non-ASCII (`/tmp/héllo/ö.rs`), symbols needing escapes (`#`, `?`, `%`). Inbound: `file:///C:/x` and `file:///c%3A/x` both parse (drive colon may arrive escaped); `file://localhost/x` accepted; `http://…` and relative paths rejected. Windows-shaped cases live behind `#[cfg(windows)]` **plus** string-level tests that run everywhere (build the expected URI string for a synthetic `C:\` path — don't gate all coverage on CI OS).

**Done when** — round-trip property holds for every produced URI; all error paths return `Err`, never a guessed path.

**Traps**
- Relative path in → hard error, not cwd-join (fail fast; the caller owns canonicalization).
- Percent-decode **before** building the `PathBuf`, and decode `%2F` defensively (reject if a decoded segment contains a path separator — path-traversal hygiene, see LESSONS).
- Don't return `\\?\`-prefixed paths to anything Steel-visible (existing convention).
- macOS tests: `TempDir` gives `/var/folders/…` but canonicalization yields `/private/var/…` — canonicalize expected values in tests (see LESSONS).

**Size** — ~120 source + ~120 test lines.

---

### P6 — `ChangeSet` → `TextDocumentContentChangeEvent[]` *(lives in `hume-lsp` — do after C1/P5)*

**Goal** — convert HUME's retain/delete/insert ChangeSet into the incremental range-edit list `textDocument/didChange` wants.

**Depends** — C1, P4. **Unlocks** — C7.

**Files** — new `hume-lsp/src/sync.rs`.

**Read first** — `hume-editing/src/changeset/mod.rs` `Operation` docs (ops are char counts, ordered left-to-right against the **old** document); hub primer *Document sync*; P4's API.

**Shape**
```rust
/// `before` is the pre-edit text. Events are emitted in document order and,
/// per the LSP spec, each event's range addresses the document state AFTER
/// all previous events in the list were applied.
pub fn changeset_to_content_changes(
    before: &Rope,
    cs: &ChangeSet,
    enc: PositionEncoding,
) -> Vec<lsp_types::TextDocumentContentChangeEvent>;
```
Algorithm: keep a working rope (start = `before.clone()`, O(1) structural sharing) and a char cursor. `Retain(n)` → advance cursor. `Delete(n)` → emit `{range: wire(cursor .. cursor+n), text: ""}` computed on the *working* rope, then remove from it. `Insert(s)` → emit `{range: wire(cursor .. cursor), text: s}`, insert into working rope, advance cursor by `s.chars().count()`. Optional (do only if trivially clean): fuse an adjacent Delete+Insert at the same cursor into one replace event.

**Tests** — tier 1. **Oracle** (hub: testing playbook): apply the emitted events to a plain `String` mirror using independent line/char math and assert it equals the post-edit rope's contents. Cases: single insert, single delete, replace, multiple disjoint ops in one set, insert containing `\n` (line-count shifts), delete spanning a line boundary, edits adjacent to multi-byte chars and emoji, both encodings, empty ChangeSet (→ empty vec).

**Done when** — oracle property holds for all cases; function documented with the after-previous-events semantics spelled out.

**Traps**
- **The #1 bug**: computing all ranges against `before`. Ranges after the first event address the *partially updated* document — that's why the working rope exists. The newline-in-insert test catches it.
- The didChange envelope (versions, URI) is C7's job — this function is pure text math.
- Ops count **chars**, wire wants **code units** — every range goes through P4; no arithmetic shortcuts even for the Utf8 case.

**Size** — ~80 source + ~160 test lines.

---

### P7 — Event-loop timer wheel

**Goal** — a nearest-deadline timer registry integrated with P3's wake logic. Rust-side machinery only; the Steel `after`/`debounce` surface is B4.

**Depends** — P3. **Unlocks** — B4 (and via it F7/F10 debouncing).

**Files** — new `hume-editor/src/editor/timers.rs`; wire into `lifecycle.rs` (`wake_timeout` min-logic + a fire step in the P3 drain phase); `editor/mod.rs` (field + module).

**Read first** — P3's `AsyncSource` trait and `wake_timeout`; `std::collections::BinaryHeap` with `Reverse` for a min-heap.

**Shape**
```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct TimerId(u64);

pub(crate) struct TimerWheel {
    heap: BinaryHeap<Reverse<(Instant, TimerId)>>,
    cancelled: HashSet<TimerId>,   // lazy deletion — skipped when popped
    next_id: u64,
}

impl TimerWheel {
    pub fn schedule(&mut self, after: Duration) -> TimerId;
    pub fn cancel(&mut self, id: TimerId);
    pub fn next_deadline(&self) -> Option<Instant>;   // AsyncSource::next_deadline
    /// Pop every timer with deadline <= now (skipping cancelled).
    pub fn take_due(&mut self, now: Instant) -> Vec<TimerId>;
}
```
The wheel is payload-agnostic — it hands back `TimerId`s; B4 owns the `TimerId → Steel thunk` side table (keeps Steel types out of the editor core). `AsyncSource::has_pending` for the wheel is **false** (a distant deadline must not cause 8 ms polling — the deadline bounds the poll timeout instead; due-now timers are caught by `take_due` in the drain phase).

**Tests** — tier 1/2. Deadline ordering (two timers, reversed insertion); cancellation (cancelled id never returned; heap entry skipped); `next_deadline` = min; `take_due` boundary (due exactly at `now` fires); idle wheel → `next_deadline() == None` → `wake_timeout() == None` (blocking read preserved — the no-busy-wheel guarantee); many-cancelled compaction isn't needed (document why: cancelled set is drained as entries pop).

**Done when** — wheel + loop integration merged behind P3's seams; all listed tests green; an end-to-end tick test (schedule 10 ms timer in a test editor, run one loop iteration with mocked time or a real sleep-free `take_due(now + 20ms)`) passes.

**Traps**
- `Instant`, never `SystemTime`.
- No threads, no `std::thread::sleep` — the event loop *is* the scheduler.
- Don't fire timers inside the input-handling path; fire in the P3 drain phase so hooks/commands see a consistent editor.

**Size** — ~90 source + ~100 test lines.

---

### P8 — Boundary-cost spike (measure, then decide)

**Goal** — real numbers for the Rust↔Steel boundary at LSP scale, recorded in the hub. This calibrates the two OQ defaults (diagnostics pull cap; completion scorer shape) and validates B8's one-time ingest exception.

**Depends** — P1. **Unlocks** — nothing structurally, but do it **before Step 2** commits to shapes.

**Files** — throwaway only: a `#[test] #[ignore]` bench module (e.g. `hume-scripting/tests/boundary_spike.rs`), run with `cargo test -p hume-scripting --test boundary_spike -- --ignored --nocapture`. **Deleted in the same session** after numbers are recorded — it does not ship.

**Read first** — `hume-scripting/src/context.rs` (`SteelCtxTestHarness`) for how to stand up an engine in tests; `steel` list/hashmap construction (see LESSONS: `Vec::<SteelVal>::new().into_steelval()`, never `SteelVal::ListV(steel::List::new())`).

**What to measure** (each at 100 / 1 000 / 5 000 items, timed over ≥100 iterations with `Instant`):
1. Build diagnostic-shaped `serde_json::Value`s (uri, range, severity, message) → convert to `SteelVal` (write a quick-and-dirty conversion inline — B1 will do it properly) → time the conversion.
2. Same for completion-shaped items (label, kind, detail, textEdit).
3. Steel-side iteration: eval a `(for-each … items)` / fold over the converted list; time end-to-end.
4. Per-call dispatch overhead: registered no-op builtin called 10 000× from a Steel loop; derive µs/call.

**Output** — a short paragraph per measurement added to the hub: update the two OQ **Defaults** (confirm or replace the 1 000 cap and re-rank-top-N=64), and note in the *Bulk-data guardrail* decision row whether B8's ingest-through-Steel holds at 1k items (if >2 ms, flip to the raw-response-routing flag design). Move what's now decided from Open Questions into Decisions.

**Done when** — hub updated with numbers + decisions; spike file deleted; workspace green.

**Traps**
- Numbers from a debug build are garbage — `cargo test --release … --ignored`.
- Don't ship "just the harness in case" — it rots; git history keeps it.
- Don't let the spike grow into B1: crude conversion is fine, the *relative* magnitudes are the answer.

**Size** — ~150 throwaway lines; zero shipped.
