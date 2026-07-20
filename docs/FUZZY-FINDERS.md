# HUME — Fuzzy Finder (Picker) Foundation

Design document for a generic scriptable fuzzy-finder surface (files, buffers, symbols, anything a plugin feeds it), Helix/telescope-style.

**Status: design only — not scheduled, no implementation started.** This design lives in its own file, separate from `docs/COMPLETION-PICKER.md`'s scriptable-completion design, because the two share only the "Rust store, Steel policy" architectural pattern — and, prospectively, the B1 fuzzy-matcher module if `Q-B6` ever unifies them (v1 does not: completion keeps its own hand-rolled filter) — while everything else (item shape, query origin, accept semantics, lifetime, scale, task tables) is independent. See `docs/COMPLETION-PICKER.md` for the completion-menu design and the ["Why not one shared session type"](#why-not-one-shared-session-type) note below for the full rationale on why the two stay separate.

**Headline conclusion**: no foundation work is required now. The picker is a new widget + new builtins + one new crate dependency (`nucleo-matcher`). Nothing currently being built needs to change shape to keep it possible.

## How to use this document

Same rules as `docs/LSP.md` and `docs/COMPLETION-PICKER.md`:

1. **Verify before you write.** The codebase moves — `rg 'symbol_name'` before relying on anything named here. No line numbers anywhere in this doc — navigate by symbol search.
2. **If the doc contradicts the code, STOP** and report; don't silently adapt.
3. Open questions each carry a `Default:`. At implementation time, adopt the default unless evidence gathered during the task contradicts it — then record the decision in the Decisions table here.
4. Project-wide rules from `CLAUDE.md` apply (no `.unwrap()` outside tests, grapheme discipline, every command tested).

## Architecture constraints inherited from LSP work

These are settled project decisions (see `docs/LSP.md` Decisions table) and this design must respect them:

- **Frequency cut**: per-user-intent work (a trigger keypress, a response arriving, a selection made) may run in Steel; per-keystroke filtering, per-frame rendering, and unbounded-collection work must be Rust.
- **Bulk-data guardrail**: bulk item lists never cross the Rust↔Steel boundary on recurring paths. One-time ingest at user-intent frequency is the calibrated exception (see `docs/COMPLETION-PICKER.md`'s P8 spike note: ~1ms for 1k completion items through the boundary — acceptable; do not assume this scales to 100k file paths). The picker design honors this **strictly**: enumeration-scale source output streams into the store entirely Rust-side (B5) and never enters the Steel VM — only the single accepted item's line crosses, on select. See the *Spawned-source data path* decision row for why a Steel-callback streaming variant is rejected.
- **Steel never on the render path**: Steel writes models/stores; Rust providers render from `Arc<RwLock<…>>` snapshots each frame.
- **Rust-rendered, Steel-fed widgets**: "LSP is their first client, not their owner" (applies here as: "files are the picker's first client, not its owner").

## The three-layer model

The picker splits into three layers with different rules for where code lives:

1. **Widget, store, matcher, ranking (Layer 1)** — always Rust. Per-keystroke filtering and per-frame rendering are on the render path; the frequency-cut and Steel-never-on-render-path rules above apply unconditionally.
2. **Data sources (Layer 2)** — **external-first, split by scale** (see the *File enumeration* and *Spawned-source data path* decision rows for why a native Rust walker and a Steel-callback streaming variant are both rejected in favor of this). Small lists (open buffers, a plugin's own static data, output of a fast sync spawn like `git status`, ≤ a few thousand items) cross the boundary via `picker!` (open) and `picker-push!` (any later append, whether a synchronous second batch or an async callback's result — see B4). Enumeration-scale sources (file lists, future grep-like output) come from **external commands** (`git ls-files`, `fd`) spawned through `picker-source-spawn!` (B5): Steel supplies only cmd + argv; Rust reads the child's stdout on a reader thread, splits it into lines Rust-side, and appends them **directly into the picker store** — each line is its own display and payload, and no bulk data ever enters the Steel VM. Transformation happens at *accept* time (Steel parses the one selected line), not at ingest. A minimal native walker for directories that are neither git repos nor have `fd` installed is a deferred escalation (B7), and when built it feeds the store through the *same* reader/drain path.
3. **Picker definitions (Layer 3)** — **Steel only.** Which source feeds a picker, how each item displays, what `on-select` does — this is behavior, not performance-critical work, and runs at most twice per picker use (open, accept). There is no native picker definition path: default pickers (files, buffers) ship as the `core:pickers` Steel plugin (B6), built from the same public API a third-party plugin would use. Rust never hardcodes a picker.

This model was chosen over two rejected alternatives — see the **Picker scriptability** row in the Decisions table below for the full comparison.

## Relevant current-state inventory (`lsp` branch)

The picker design leans on infrastructure already built for LSP/completion work. Verify each symbol still exists before relying on it.

**Adjacent infrastructure** (all in `host_impl.rs` + `ui/popup.rs` + `ui/drawer.rs` unless noted):

- **Async Rust→Steel callbacks**: `lsp-request` queues `PendingLspRequest` on `SteelCtx.pending_lsp_requests`; after eval, `flush_pending_lsp_requests` / `send_one_lsp_request` (`hume-editor/src/editor/lsp/bridge.rs`) register a boxed callback keyed `(ServerId, RequestId)`; reader threads → mpsc → `drain_lsp` each frame → `dispatch_completed` → `Editor::queue_steel_call(callback, args)` (`scripting_setup.rs`). Staleness: response dropped if the buffer's `text_gen` moved, unless `#:allow-stale`. **This is the template for any "async work finishes → call Steel closure" need**, including a picker's async sources.
- **Timers**: `(after ms thunk)` / `(cancel-timer! id)` builtins; `(debounce ms proc)` is pure Scheme over them (bootstrap in `builtins/mod.rs`).
- **Generic widgets**:
  - `(show-popup! text #:anchor 'cursor)` / `(close-popup!)` — `PopupModel`, hover-style text panel.
  - `(show-menu! items on-select)` / `(close-menu!)` — `MenuModel { items, selected, callback }`; callback fires exactly once (selection or dismissal); **blocked in Insert mode** (`show_menu` returns Err — deliberate, the completion menu owns that slot). Keys intercepted by `handle_menu_key` (`mappings/mod.rs`) ahead of keymap dispatch.
  - `(show-drawer-list! items on-select)` / `(close-drawer!)` — `DrawerModel { items, selected, scroll, callback }`, bottom chrome band, stays open across Enter (callback may fire repeatedly, `#f` on close), `handle_drawer_key`. Rows are pre-formatted display strings; "Rust never interprets row content."
  - `(prompt! label on-confirm #:prefill text)` — takes over the minibuffer (`MiniBuffer { prompt, input, cursor }` + `steel_prompt_callback`, one at a time), Mode::Command; confirm fires once with text or `#f`.
- **Buffer/introspection builtins available to sources**: `buffers`, `buffer-name`, `buffer-path`, `buffer-language`, `current-buffer`, `current-selections`, `symbol-under-cursor`, `diagnostics-for-buffer`, `lsp-capabilities`, … (full registry: `register_fn_with_ctx` calls in `hume-scripting/src/builtins/mod.rs`).

**Gaps** (what does not exist today):

1. **No fuzzy matcher** — completion's filter is hand-rolled subsequence-only (see `docs/COMPLETION-PICKER.md`); no fuzzy crate anywhere in the workspace (no nucleo / fuzzy-matcher / skim in any `Cargo.toml`).
2. **No input-box-plus-filtered-list widget** — menu and drawer take static pre-filtered lists; `prompt!` has an input but no list. A picker needs both in one surface.
3. **No async process spawn** — plugins can spawn arbitrary processes via Steel's own `steel/process` (full-trust plugin model, see `docs/ROADMAP.md`'s plugin trust model decision — PLUM runs `git`/`curl`/`npm` straight through `command`/`spawn-process`, pattern in `runtime/plugins/core/plum/lib.scm`'s `plum/run!`), but only *synchronously*: `wait` blocks the whole editor, port reads block when the pipe is empty, and Steel has no non-blocking read — so a slow or large-output command freezes the UI for its duration, and streaming results across evals is impossible from Steel alone. That's the real gap B5 fills (`picker-source-spawn!` — picker-scoped, since the picker is the only current consumer of async command output; a *generic* async spawn is deferred until a second client exists, and would reuse the same reader-thread/drain infrastructure). Note: the bulk guardrail does not forbid a Steel-fed file source on "per keystroke" grounds — ingest crosses once per picker open (a burst); per-keystroke filtering runs in Rust over the store regardless of who fed it. The point is moot anyway: B5 streams source output directly into the store, so bulk never crosses at all.
4. **Completion menu shows a fixed top-8 window** with selection clamped to it — acceptable for completion; a picker needs real scrolling over the full ranked list.

**The minibuffer completion system is unrelated — leave it alone.** `hume-editor/src/editor/completion/` (`Completer` trait, `CommandCompleter`/`BufferNameCompleter`/`ThemeCompleter`/`PathCompleter`/`SetCompleter`, dispatched by a hardcoded match in `complete_minibuf`, prefix matching only, rendered by `CompletionOverlay`) shares no types with either the completion-menu stack or the picker design. Neither this document nor `docs/COMPLETION-PICKER.md` builds on it, and neither changes it.

---

## Goal

`(picker! items on-select #:prompt "buffer: ")` from any plugin opens a modal centered panel: an input line the user types a fuzzy query into, a scrolling ranked list below, Enter fires `on-select` with the chosen item's payload, Esc closes. Enumeration-scale sources (files) stream directly into the store via `picker-source-spawn!` while the picker is already open and usable — Steel never sees the item flood. Files/buffers/symbols pickers ship as thin Steel plugins.

## Design

**New Rust core: `PickerSession`** (new module, e.g. `hume-editor/src/editor/picker.rs`) — deliberately a sibling of completion's `CompletionSession`, not a generalization of it.

### Why not one shared session type

They rhyme (items, query, ranked indices, top-N, accept-by-index) but differ in every load-bearing detail: item shape (JSON completion items with edit semantics vs. display string + opaque payload), query origin (buffer text between anchor and cursor vs. a widget-owned input line), accept semantics (gen-checked buffer edit vs. fire a Steel callback), lifetime (bound to an Insert-mode token vs. modal), scale (≤ a few k items, replace-per-response vs. up to ~100k, streamed), and scroll model (top-8 window vs. full scroll). A shared abstract core would be parameterized over all six axes — premature abstraction with two very different call sites. **What they share instead: the architectural pattern, and prospectively the B1 fuzzy-matcher module.** Not in v1, though — B1 explicitly does not replace completion's `subsequence_match_pos` (Q-B6 tracks that unification, defaulted to "not yet"). If, after both exist, the bodies converge, merging is a cheap refactor; guessing the abstraction up front is not.

### The pieces

**B1 — fuzzy matcher.** Add `nucleo-matcher` (the matcher-only crate Helix's picker engine is built on: scoring + Unicode handling, no threading harness). Wrap it behind a small module (e.g. `hume-editor/src/editor/fuzzy.rs`) exposing `score(query, haystack) -> Option<(score, …)>` so neither session type names the crate directly. Completion's hand-rolled `subsequence_match_pos` is *not* replaced in this task (Q-B6 tracks later unification). Include a micro-benchmark-ish test at 100k synthetic paths to validate the single-threaded per-keystroke budget — run it in release mode or mark it `#[ignore]`-by-default (wall-clock asserts in debug/CI builds are flaky; the gate is an explicit `--ignored`/release run). If it blows the frame budget, the fallback is documented in Q-B1 (full `nucleo` with background matching).

**B2 — `PickerSession` store.**

```
PickerItem { display: String, payload: SteelVal }
PickerSession {
    items: Vec<PickerItem>,          // append-only while open
    query: String,
    filtered: Vec<u32>,              // ranked indices, rebuilt on query/items change
    rank_scratch: Vec<u32>,          // reused scoring buffer, no per-keystroke allocation (mirrors completion's rank_scratch)
    selected: usize,                 // index into filtered (full range, not a window)
    scroll: usize,                   // first visible row, clamped like DrawerModel
    on_select: SteelVal,             // fired with payload on Enter
    token: u64,                      // same stale-push-guard pattern as the session token designed in docs/COMPLETION-PICKER.md's A2 (also design-only)
}
```

`payload` is an arbitrary `SteelVal` (string path, hashmap, whatever the source chose) — Rust never interprets it, mirroring the drawer's "rows are pre-formatted display strings" contract. Query edits and item pushes re-rank via B1. Empty query = insertion order, i.e. source output order — `git ls-files` emits sorted paths (nice); `fd`'s parallel walk is nondeterministic (acceptable). (nucleo-matcher is expected to treat an empty query as all-match — external-crate claim, verify at impl time.) On any re-rank (query edit, streamed batch arriving, or session replace) `selected` resets to 0 — the top-ranked row — mirroring completion's reset-on-merge choice; anchoring the cursor to the same item across a re-rank is future polish, not v1.

**B3 — widget + interaction.** New centered overlay (e.g. `ui/picker_panel.rs`): bordered panel sized as a fraction of the *panes region* (the terminal minus chrome bands — see Q-B2) — say width `min(80%, 100 cols)`, height `min(60%, 30 rows)`; input line at top (rendered from `query` + a block cursor cell), ranked list below with `selected` highlighted and real scrolling. Theme scopes `ui.picker`, `ui.picker.selected`, `ui.picker.input`. **Theme fallback caveat**: `Theme::resolve_raw` falls back by prefix-trimming only (`ui.picker.selected` → `ui.picker` → `ui` → default) — it will *never* reach `ui.menu`. Graceful degradation on themes without the new scopes needs the picker's scope lookup to explicitly alias to the matching `ui.menu*` scope when `ui.picker*` is absent, plus `ui.picker*` entries added to the bundled themes (mind the theme `cursor.insert`-required precedent: document the new scopes). Rendering follows the universal write-side/read-side split: `sync_picker_view` in `prepare_frame` writes an `Arc<RwLock<Option<PickerViewState>>>`; a new `OverlayProvider` only paints (per-pane registration suffices — overlays receive the whole panes region, Q-B2).

Key routing: intercept in `dispatch_key` ahead of keymap dispatch while a picker is open — same pattern as `handle_menu_key`/`handle_drawer_key` in `mappings/mod.rs`, **not** a new `Mode` variant (mode machinery drags in statusline, cursor-shape, `OnModeChange` hooks, keymap tries — all noise here; the menu/drawer precedent is established and tested). All printable chars edit the query in Rust (no Steel per keystroke); Up/Down/Ctrl-p/Ctrl-n move selection; PageUp/PageDown scroll; Enter fires `on_select` with the payload via `queue_steel_call` and closes; Esc closes and fires `on_select` with `#f` (matching the menu/drawer dismissal convention).

**B4 — Steel surface.**

```scheme
(picker! items on-select #:prompt "…")      ; items: list of (display . payload) pairs or
                                            ; hashmaps {"display", "payload"} — decide at impl
(picker-push! token items)                  ; append (sync, or from an async callback), bounded chunks
(picker-close!)
```

`picker!` returns the token. Small/medium lists (buffers, recent files, custom plugin lists, ≤ a few k) go through `picker!` directly — one-time user-intent ingest, guardrail-compliant. Async Steel sources (LSP `workspace/symbol`, which is an explicit LSP-v1 non-goal but becomes reachable the moment this exists) use `picker-push!` from their `lsp-request` callbacks with the token guard eating staleness.

**Callback lifecycle**: `on_select` fires exactly once per session, matching the menu/drawer convention — payload on Enter, `#f` on every other way the session ends: Esc, an explicit `picker-close!`, or a second `picker!` call while one is already open (which replaces the session wholesale, killing any live B5 child per its kill-on-cancel contract, same as closing).

**B5 — `picker-source-spawn!` builtin.** Spawn an external command as a streaming picker source; its stdout lines flow **directly into the session store**, never through Steel.

```scheme
(picker-source-spawn! token cmd args #:cwd dir #:nul flag)
```

Steel supplies only cmd + argv (no shell involved — direct `argv` spawn, so no quoting/pipeline portability hazards on Windows). Rust spawns the child (stdin piped and closed immediately — same non-inherited-stdin contract as PLUM's `plum/run!`), a reader thread drains stdout, **splits into lines Rust-side** (delimiter `\n`, or NUL with `#:nul #t` for `git ls-files -z` / `fd -0`; a partial line at a read-chunk boundary is carried into the next batch — classic bug, gets a dedicated test), and sends batches of complete lines over an mpsc channel. Drained once per frame from `drain_async_sources` (`hume-editor/src/editor/async_source.rs`); each batch appends to the store as `PickerItem { display: line, payload: line }` and re-ranks. Spawn failure raises at the call site; a nonzero exit surfaces as a status message with captured stderr.

**Item semantics**: for a spawned source, the raw line is both display and payload — the realistic bulk commands (`git ls-files`, `fd`, `rg`) already emit human-readable lines. Any parsing (`path:line:text` → location) happens in Steel **at accept time, on the one selected line** — not at ingest time on 100k of them. Filtering-at-ingest is the command's own flags (`fd -e rs`, `git ls-files -m`, …), which Steel controls via argv. This is the deliberate trade that keeps bulk data out of the Steel VM entirely (guardrail holds strict, zero interpreter/GC cost at scale); a bulk source needing rich per-item display is out of scope until a real case appears.

**Kill-on-cancel is automatic**: the child handle is owned by the `PickerSession` — closing the picker (or replacing the session) kills the child and detaches the reader thread; the session-token check on the drain side makes any already-queued late batch harmless. No Steel-visible handle to leak or forget.

**Wake is reuse, not new machinery** (see `docs/ROADMAP.md`'s "Event-loop waker" row): `AsyncSource`/`wake_timeout` generalize the event-loop wake; arrival is not a poll cadence — background threads signal the loop's wait primitive directly via a `WakeCallback` the moment they post a result, and `AsyncSource` reports real deadlines only. A spawn source should follow the same pattern: thread a `WakeCallback` into the reader thread (call it after each batch send) rather than adding a poll-cadence `next_wake` impl. "Adding a source = one line here plus its `AsyncSource` impl" — where "a source" means "a real deadline," not "a poll cadence."

**Generic async spawn deferred**: a general-purpose `spawn-async` (batches to a Steel callback, for non-picker consumers) was designed and rejected for v1 — the picker is the only current client of async command output, and it doesn't need the Steel hop (see the *Spawned-source data path* decision row). If a second client ever materializes, the builtin is additive and reuses this task's reader-thread/split/drain infrastructure. The most plausible candidate is an external-command-backed **completion source** (dictionary/spell via `aspell`, a snippets CLI, shell history): a sync spawn at trigger time would freeze the editor, and `docs/COMPLETION-PICKER.md`'s A2 machinery (session token + `completion-add-items!`, itself unbuilt) is designed to absorb late async arrivals by construction — so the deferral is "until a client", not "never".

**B6 — shipped pickers plugin** (`core:pickers` or similar): **files** — Steel source over B5: in a git repo, `git ls-files -z --cached --others --exclude-standard` (index read, no filesystem walk — fast even on huge repos); in a bare directory, `fd -0` if installed, else a clear error message naming `fd` (external-tool posture matches PLUM, which already assumes `git`/`curl`/`npm`). `on_select` receives the path string and calls `(open-buffer! path)`. The composition — open empty, then attach the streaming source:

```scheme
(define token (picker! '() (lambda (path) (when path (open-buffer! path))) #:prompt "files: "))
(picker-source-spawn! token "git" '("ls-files" "-z" "--cached" "--others" "--exclude-standard") #:nul #t)
```

**Buffers** — pure Steel over the existing `buffers` builtin (display via `buffer-name`, payload `bid`, select via `switch-to-buffer!`), zero new Rust. Plus commands to bind (`picker-files`, `picker-buffers`). Built entirely from the B4/B5 public API — this is the dogfooding step proving the surface is complete for community plugins, and the reason no native picker definitions exist per the three-layer model above.

**B7 — minimal native walker (deferred, possibly forever).** Covers the one hole B6 leaves: bare directories without `fd`. When built: `ignore` crate walk on a std thread, emitting path batches over the **same** mpsc → drain → store path as B5 — behaves like an internal command, no second ingestion pipeline. Minimal Rust by construction. Not scheduled for v1 — build only if the fd-fallback posture proves inadequate in practice.

**Explicit non-goals for picker v1** (each is additive later): preview pane (needs scratch-buffer rendering inside the panel — a real project; see Q-B4), live-requery sources (live-grep style, where the *query* re-runs the source — needs an `on-query-change` callback option with debounce; the store/widget design above doesn't preclude it, and B5's automatic kill-on-cancel + a B2 `replace_items` op are most of the machinery, see Q-B5), multi-select, picker-specific keybinding customization, native walker (B7 above), generic `spawn-async` for non-picker consumers (see B5's deferral note).

## Task breakdown

| ID | Task | Depends | Size |
|----|------|---------|------|
| B1 | `nucleo-matcher` dep + `fuzzy.rs` wrapper + 100k-item budget test | — | S |
| B2 | `PickerSession` store: push/query/rank/select/scroll/token; unit tests with mock items | B1 | M |
| B3 | Panel widget + view sync + key interception; insta snapshot tests + interaction tests | B2 | M–L (largest single piece — new chrome surface) |
| B4 | Steel builtins `picker!`/`picker-push!`/`picker-close!` + host trait/impl + callback firing; tests incl. stale-token push | B3 | M |
| B5 | `picker-source-spawn!` builtin: reader thread, Rust-side line split (partial-line carry test), batch drain into the store, session-owned child (auto kill-on-close), fourth `AsyncSource` entry | B2, B4 | M |
| B6 | `core:pickers` plugin (files via git/fd over B5 + buffers pure Steel) + default bindings + user-manual page | B4, B5 | S–M |
| B7 | Minimal native walker for bare-dirs-without-fd (`ignore` crate, same mpsc→drain→store path as B5) | B5 | **Deferred** — build only if B6's fd posture proves inadequate |

Estimated total (B1–B6): comparable to one LSP macro-step (a Step-3-sized effort — new UI surface + new builtins + one worker). Independent of the scriptable-completion design in `docs/COMPLETION-PICKER.md` except for sharing B1's matcher module if that design later adopts it (Q-B6), and Q-B7's reuse of completion's session-clear entry point. ROADMAP M12 already points here: "File picker / fuzzy finder (Helix-style): full design + task breakdown in `docs/FUZZY-FINDERS.md`. Splits dependency satisfied by M10 T2; remaining gate is prioritization."

## What to do *now* (foundation checklist)

1. **Nothing structural.** Verified: no current abstraction blocks this design; no in-flight lsp-branch work needs redirecting.
2. ROADMAP already points here — M12's picker line and the Future-section "Scriptable completion sources + fuzzy pickers" line. Nothing left to groom.

## Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Completion vs picker core | **Siblings sharing the matcher, not a shared session type** | Six load-bearing axes differ (item shape, query origin, accept, lifetime, scale, scroll). Abstraction with two divergent call sites is premature; merging later is cheap if bodies converge. |
| Picker modality | **Key interception ahead of keymap (menu/drawer pattern), no new `Mode`** | Mode machinery (statusline, cursor shape, hooks, keymap tries) is all cost, no benefit; `handle_menu_key`/`handle_drawer_key` precedent is established and tested. |
| File enumeration | **External-first via `picker-source-spawn!` (B5): `git ls-files` in repos, `fd` in bare dirs, clear error otherwise; minimal native walker (B7) deferred as escalation** | Ingest happens once per picker open, not per keystroke — per-keystroke filtering is Rust over the store regardless of who fed it, so the bulk guardrail doesn't forbid an external source. External sources win on capability (`git ls-files` reads the index — no walk at all) and Rust surface (one spawn-source builtin instead of an embedded walker; note `fd`'s engine *is* the `ignore` crate, so spawning it loses nothing). Blocking-spawn UX is solved by B5's async streaming. |
| Spawned-source data path | **Direct-to-store: stdout lines flow Rust-side into `PickerSession`; Steel supplies only argv and parses the one accepted line** | A Steel hop buys nothing the real use cases need: ingest-time transformation is unnecessary (bulk commands emit human-readable lines; parse-on-accept handles structure for the single selected item; filtering is the command's own flags), and no *non-picker* consumer of async command output exists today — small-output finders use sync spawn, LSP/parsing/timers have dedicated paths. Direct-to-store keeps the bulk guardrail strict (no reinterpretation carve-out needed), avoids 100k transient Steel strings/GC pressure, makes kill-on-cancel automatic (session owns the child), and is *less* Rust (no callback/handle plumbing). Costs accepted: spawned-source items are raw lines (no rich display/payload for bulk sources until a real case appears); shell pipelines (`fd \| awk`) deliberately unsupported — direct argv spawn only, which also sidesteps Windows shell/quoting portability. A generic `spawn-async` stays additive later on the same infrastructure if a second client materializes. |
| `picker-source-spawn!` semantics | **Session-owned child, killed automatically on picker close/replace; Rust-side line splitting with partial-line carry; `#:nul` for NUL-delimited output; no shell — argv only; spawn failure raises, nonzero exit → status message + stderr** | An abandoned picker must not leave a child burning CPU invisibly, and session ownership makes cleanup unforgettable (no Steel-visible handle). Lines split in Rust so the interpreter never touches enumeration-scale output. Sync spawn (PLUM's `plum/run!` pattern) stays legitimate for small-output commands (`git status` finders) feeding `picker!` directly. |
| Minibuffer `Completer` system | **Untouched by this design** | Separate system, separate roadmap line; different interaction grammar. |
| Picker scriptability | **Steel-defined pickers only; no native picker definitions. Defaults ship as `core:pickers` plugin (B6); enumeration-scale sources are external commands via `picker-source-spawn!` (B5) per the File-enumeration row** | Per-keystroke performance is identical whether pickers are native or Steel-defined — Steel only ever touches picker *open* and *accept*, never the render/filter path (Layer 1 above is Rust regardless). A fixed native set fails the motivating examples (live grep, "find unstaged files") outright — every new finder would need a Rust PR. Native defaults alongside a Steel API for custom pickers means two code paths into one widget, with native defaults built against private hooks the public API lacks — leaving the Steel API unproven and likely to rot. Steel-only for both defaults and custom pickers means B6 dogfoods the public API by construction, matching the `core:lsp`/`core:completion` precedent where "built-in" features are core plugins, not Rust. |
| Plugin data access for finders (Q-B8) | **Community finders gather small inputs (git status, etc.) via Steel's own `steel/process` directly — no per-finder Rust builtin** | Full-trust plugin model (see `docs/ROADMAP.md`): plugins can already spawn `git status --porcelain`, parse it, and feed `picker!` — dozens of lines, user-intent frequency, no bulk-guardrail conflict. Caveat: **sync spawn blocks** — the whole editor freezes for the command's duration and the cooperative watchdog can't interrupt a Rust call blocked on a child process, so sync spawn is for fast/local, small-output commands only. Anything slow or enumeration-scale runs as a `picker-source-spawn!` streaming source (B5) instead. |

## Open questions

Each carries a `Default:` per the usage rules.

**Q-B1 — matcher crate: `nucleo-matcher` vs full `nucleo`.** Full nucleo brings the streaming injector + multithreaded incremental matching Helix uses; matcher-only means our store re-scores on one thread per keystroke. *Default: `nucleo-matcher` + the B1 budget test at 100k items. Escalate to full nucleo only if the test says so — its injector/snapshot model would replace much of B2's store, so the decision gate is cheap to hit early (that's why B1 is first).*

**Q-B2 — panel paint slot.** Per-pane `OverlayProvider`s are handed the whole panes region (`pane_area`) at render time, not their pane's rect — the pipeline comment says overlays "may span panes" (`hume-engine/src/pipeline/mod.rs`). So a picker centered in the *panes region* needs no engine change. What `pane_area` excludes is the chrome bands (tab bar, drawer, statusline rows); only a panel that must paint *over those rows* (true terminal-centered) needs a new top-level overlay slot. *Default: render as a pane-region overlay — zero engine change; add a top-level slot only if overlapping the chrome bands turns out to matter (it would also serve future command palettes/dialogs).*

**Q-B3 — item shape at the `picker!` boundary.** Pairs `(display . payload)` vs. hashmaps. *Default: pairs — cheaper to build in Steel, and payload opacity is the point; switch to hashmaps only if a third field (e.g. per-item kind/icon) earns it.*

**Q-B4 — preview pane.** Requires rendering a scratch view of an unopened file inside the panel (load, highlight, position) — touches buffer lifecycle and the render pipeline. *Default: defer entirely; design the panel width so a preview split can be added to its right without relayout of the list half.*

**Q-B5 — live-requery sources (live grep).** Query changes re-run the *source*, not just the filter — needs `#:on-query-change` callback + debounce + result-replacement semantics (vs append). Given the external-first design above, most of the machinery already exists in v1: a debounced query change is user-intent-ish frequency, so the coordinator can re-invoke `picker-source-spawn!` with a fresh `rg` argv (killing the previous session child) feeding a B2 `replace_items` op — no native grep source required; raw `rg` lines are the display, parse-on-accept extracts the location. What's genuinely missing: `#:on-query-change` on `picker!`, the debounce wiring, `replace_items`, and re-spawn-replaces-source semantics on B5. *Default: still defer from v1 — but as a small additive task now, not a design problem.*

**Q-B6 — unify completion filtering onto the B1 matcher.** Replace completion's `subsequence_match_pos`/`is_prefix_match` with nucleo scoring for consistency of feel? *Default: not during the scriptable-completion work (`docs/COMPLETION-PICKER.md`) or this design — completion's ranking (`prefix, pos, sortText`) is tuned to LSP conventions and tested; revisit once the picker's feel is validated, as its own small task with side-by-side comparison.*

**Q-B7 — should `show-menu!`'s Insert-mode block extend to the picker?** Menu is blocked in Insert because the completion popup owns that visual slot. The picker is full-modal and could open from Insert. *Default: allow from any mode but close any open completion session on open (one modal owner at a time); reuse completion's session-clear entry point (`clear_lsp_completion` today, renamed `clear_completion` by `docs/COMPLETION-PICKER.md`'s A1 — use whichever name is current at impl time).*

**Q-B8 (resolved — see Decisions table above)** — community finders gather small inputs (git status, etc.) via Steel's own `steel/process` directly; no per-finder Rust builtin needed for that class of data. See the "Plugin data access for finders" row above for the full rationale and caveats.
