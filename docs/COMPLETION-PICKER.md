# HUME — Scriptable Completion Sources

Design document for the insert-mode completion menu driven by multiple Steel-registered sources (LSP, buffer words, custom plugins), mixed and prioritized by policy written in Steel.

The sibling fuzzy-finder (picker) design lives in `docs/FUZZY-FINDERS.md`, in its own file. The two designs share the "Rust store, Steel policy" architectural pattern, and prospectively the fuzzy matcher if `Q-B6` there ever unifies them; see that doc's "Why not one shared session type" note for why they stay separate.

**Status: design only — not scheduled, no implementation started.** Written against the `lsp` branch (post Step 4 / F11). This document is the single place to resume from; it assumes the reader has *no* memory of the exploration that produced it.

**Headline conclusions** (the "do we need to lay foundations now?" answer):

1. **No foundation work is required now.** This design is additive. The lsp branch's completion architecture is already source-agnostic in the ways that matter. Nothing currently being built needs to change shape to keep it possible.
2. The one guardrail while other work proceeds: **don't deepen LSP coupling in the completion store**. `CompletionSession` today parses generic completion-item JSON and only touches LSP specifics inside the `text_edit` branch of `accept`. Keep it that way — new LSP-specific fields belong in the Steel plugin (which already receives the raw item), not in new Rust parsing.
3. Completion shares its "Rust store, Steel policy" split with the picker design (`docs/FUZZY-FINDERS.md`), and prospectively its fuzzy matcher too if `Q-B6` there ever unifies them (not in v1 — completion keeps `subsequence_match_pos`) — but not a data structure: `CompletionSession` and `PickerSession` are siblings, not the same type. See that doc for the rationale.

## How to use this document

Same rules as `docs/LSP.md`:

1. **Verify before you write.** The codebase moves — `rg 'symbol_name'` before relying on anything named here. No line numbers anywhere in this doc — navigate by symbol search.
2. **If the doc contradicts the code, STOP** and report; don't silently adapt.
3. Open questions each carry a `Default:`. At implementation time, adopt the default unless evidence gathered during the task contradicts it — then record the decision in the Decisions table here.
4. Project-wide rules from `CLAUDE.md` apply (no `.unwrap()` outside tests, grapheme discipline, every command tested).

## Architecture constraints inherited from LSP work

These are settled project decisions (see `docs/LSP.md` Decisions table) and this design must respect them:

- **Frequency cut**: per-user-intent work (a trigger keypress, a response arriving, a selection made) may run in Steel; per-keystroke filtering, per-frame rendering, and unbounded-collection work must be Rust.
- **Bulk-data guardrail**: bulk item lists never cross the Rust↔Steel boundary on recurring paths. One-time ingest at user-intent frequency is the calibrated exception (P8 spike: ~1ms for 1k completion items through the boundary — acceptable; do not assume this scales to 100k file paths).
- **Steel never on the render path**: Steel writes models/stores; Rust providers render from `Arc<RwLock<…>>` snapshots each frame.
- **Rust-rendered, Steel-fed widgets**: "LSP is their first client, not their owner."

---

## Current state — verified inventory (`lsp` branch)

Everything below was read from source, not recalled. This is the substrate this design builds on.

### Insert-mode completion stack (the thing this design extends)

**Rust store — `hume-editor/src/editor/lsp/completion.rs`:**

- `StoredCompletionItem` — decoded from raw `serde_json::Value` (deliberately not `lsp_types` structs): `label: String`, `kind: Option<i64>` (raw LSP kind number, display-only, no reader maps it), `detail: Option<String>`, `sort_text`/`filter_text`/`insert_text` (each falling back to `label` when absent — `string_or_label` helper), `text_edit: Option<WireEdit>` (via `text_edit_from_json`, handles both `Edit` and `InsertReplaceEdit` shapes), and `raw: serde_json::Value` — the **full unparsed item**, handed back to Steel on accept so Rust never grows readers for LSP fields it doesn't need.
- `CompletionSession` — **singleton, one per editor** (field `EditorState.lsp_completion: Option<CompletionSession>`), replaced wholesale by each `begin`. Fields: `bid`, `anchor` (char offset of token start = primary selection head at begin time), `items`, `filtered: Vec<u32>` (ranked indices), `rank_scratch` (reused per-keystroke, no allocation), `filter: String`, `incomplete: bool` (server's `isIncomplete`), `generation_at_begin: u64` (buffer `text_gen` stamp).
- Methods: `begin(state, bid, items_json, incomplete)`, `update_filter(state, text)` (re-ranks and **re-stamps the generation** — a legitimate keystroke must not look like buffer-changed-under-us), `top(n)` (returns `{label, kind, detail}` JSON — note `to_json` exposes only these three fields), `accept(state, lsp, idx)`.
- Filtering: `subsequence_match_pos` (case-insensitive ASCII subsequence, returns first-match char index) + `is_prefix_match`; rank key is `(prefix_match desc, match_pos asc, sort_text asc)`. Hand-rolled — **no fuzzy crate anywhere in the workspace** (verified: no nucleo / fuzzy-matcher / skim in any `Cargo.toml`).
- **Accept path** — the only LSP-coupled logic: uses the item's `text_edit` if present; otherwise **synthesizes** a `WireEdit` from `insert_text` spanning `anchor..(anchor + filter.chars().count())`, converting via `char_to_wire` with `introspect::encoding_for_buffer` (which defaults to `PositionEncoding::Utf16` when the buffer has **no attached server** — and since the same encoding converts back inside `edits::apply_text_edits`, the round-trip is self-consistent: **accept works today for buffers with no LSP server**, as long as the item has no `text_edit`). Applied gen-checked against `generation_at_begin` as one undo step. After the main edit lands, pushes `HookId::OnCompletionAccept` with `(bid, raw-item)` — Steel applies `additionalTextEdits` / does `completionItem/resolve` itself.
- `LspCompletionUi { selected }` — UI selection kept as a separate `EditorState.lsp_completion_ui` field so session logic stays render-free.
- `EditorState::clear_lsp_completion` — clears session + UI + view; called from `set_mode` (any Insert exit) and the Insert key paths.

**Steel-facing surface** (this is what makes it already-mostly-scriptable):

- Builtins in `hume-scripting/src/builtins/lsp.rs`, registered in `builtins/mod.rs`, host-trait methods in `hume-scripting/src/host.rs`, implementations in `hume-editor/src/editor/host_impl.rs`:
  - `(completion-begin! bid items #:incomplete f)` — Scheme wrapper over `%completion-begin!`; `items` is a list of completion-item hashmaps (LSP `CompletionItem` JSON shape). Replaces any open session. Host impl: `completion_begin`.
  - `(completion-update-filter! text)`, `(completion-top n)`, `(completion-accept! idx)` (idx into the *ranked* order), `(completion-dismiss!)`.
  - `(register-trigger-chars! source chars)` — writes `EditorState.trigger_chars: HashMap<String, Vec<char>>`; `EditorState::is_trigger_char` checks the union across sources. **Already multi-source by design.**
- Hooks (`hume-scripting/src/hooks.rs`): `OnTriggerChar` `(bid ch)` — fired from Insert mode after a registered char lands; `OnCompletionAccept` `(bid raw-item)`; `OnCompletionRefilter` `(bid filter-text)` — fired per keystroke **only while `incomplete` is set**.

**The LSP feature plugin — `runtime/plugins/core/lsp/completion.scm`** (the model for what any source looks like):

- `lsp/request-and-begin-completions`: `lsp-request "textDocument/completion"` → decode (`CompletionItem[]` or `CompletionList`) → strip snippets (`lsp/strip-snippet-item` rewrites `insertTextFormat == 2` items to plain text — v1 has no tabstop UI) → `completion-begin!`.
- Entry points: `(define-command! "lsp-completion-trigger" …)` (Ctrl+Space is bound to that command name) and the `on-trigger-char` hook filtered by `*completion-chars*` (populated on `on-lsp-attach` from `completionProvider.triggerCharacters`, cleared on detach).
- `on-completion-accept` handler: applies `additionalTextEdits`, or resolves via `completionItem/resolve` then applies.
- `on-completion-refilter` handler: re-requests (isIncomplete flow).

**Insert-mode key handling — `hume-editor/src/editor/mappings/insert.rs`:**

- `handle_completion_key` — pre-guard while a session is open: Tab/Down next, BackTab/Up prev, Enter accept, Esc dismiss; Backspace dismisses only when it would cross the anchor, otherwise falls through; printable chars always fall through.
- `move_completion_selection` — clamped to the **displayed window** (`top(8)`, no scrolling past it).
- `accept_completion_selection` — same gen-checked path as `completion-accept!`; session ends on success or failure.
- `refilter_lsp_completion_after_edit` — after the edit lands, re-ranks against the buffer slice `anchor..head`, fires `OnCompletionRefilter` only if `incomplete`.

**Rendering:**

- `Editor::sync_lsp_completion_view` (`hume-editor/src/editor/lifecycle.rs`, runs in `prepare_frame` step 9): `session.top(8)` → `completion_row_label` (`"label  detail"`, uniform style — per-part dimming would need segment-styled rows, which nothing requires yet) → `resolve_popup_geometry` → writes a `PopupState` into `EditorState.lsp_completion_view: Arc<RwLock<Option<PopupState>>>`.
- Painted by the **generic** `PopupOverlay` (`hume-editor/src/ui/popup.rs`) — registered in `build_pane` (`hume-editor/src/ui/mod.rs`) as a third instance with its own `Arc`, scopes `ui.menu` / `ui.menu.selected` (same theme scopes as the selection menu). `PopupState { lines, x, y, selected }`; geometry (below-right preferred, flip above, clamp, max width `min(60, pane_width - 4)`, max height ⅓ pane) resolved once per frame on the write side.

### What is genuinely LSP-coupled vs. already generic

| Piece | Verdict |
|---|---|
| Item schema (`StoredCompletionItem::from_json`) | Generic — it's "LSP `CompletionItem` JSON shape as lingua franca"; any source can emit `{label, insertText, kind, detail, sortText, filterText}` hashmaps |
| Session store, filter, rank, top-N | Fully generic |
| `accept` with `text_edit` present | LSP wire positions — but isolated to one branch |
| `accept` fallback (no `text_edit`) | Generic; works with no server attached (UTF-16 default round-trips) |
| Trigger chars | Generic, already multi-source (`register-trigger-chars!` keyed by source name) |
| Trigger *ownership* (`lsp-completion-trigger` command, `on-trigger-char` subscription) | Lives in `core:lsp` plugin — needs relocation (task A3) |
| `on-completion-accept` post-processing | LSP-specific by content, but it's Steel — each source brings its own handler |
| Naming (`lsp_completion*` fields, `editor/lsp/completion.rs` path, `LspCompletionUi`) | Cosmetic LSP residue — rename in A1 |
| Snippet stripping | Correctly lives in the LSP source plugin; stays there |

### Adjacent infrastructure this design leans on

- **Async Rust→Steel callbacks**: `lsp-request` queues `PendingLspRequest` on `SteelCtx.pending_lsp_requests`; after eval, `flush_pending_lsp_requests` / `send_one_lsp_request` (`hume-editor/src/editor/lsp/bridge.rs`) register a boxed callback keyed `(ServerId, RequestId)`; reader threads → mpsc → `drain_lsp` each frame → `dispatch_completed` → `Editor::queue_steel_call(callback, args)` (`scripting_setup.rs`). Staleness: response dropped if the buffer's `text_gen` moved, unless `#:allow-stale`. **This is the template for any "async work finishes → call Steel closure" need.**
- **Timers**: `(after ms thunk)` / `(cancel-timer! id)` builtins; `(debounce ms proc)` is pure Scheme over them (bootstrap in `builtins/mod.rs`).
- **Generic widgets** (all in `host_impl.rs` + `ui/popup.rs` + `ui/drawer.rs`):
  - `(show-popup! text #:anchor 'cursor)` / `(close-popup!)` — `PopupModel`, hover-style text panel.
  - `(show-menu! items on-select)` / `(close-menu!)` — `MenuModel { items, selected, callback }`; callback fires exactly once (selection or dismissal); **blocked in Insert mode** (`show_menu` returns Err — deliberate, the completion menu owns that slot). Keys intercepted by `handle_menu_key` (`mappings/mod.rs`) ahead of keymap dispatch.
  - `(show-drawer-list! items on-select)` / `(close-drawer!)` — `DrawerModel { items, selected, scroll, callback }`, bottom chrome band, stays open across Enter (callback may fire repeatedly, `#f` on close), `handle_drawer_key`. Rows are pre-formatted display strings; "Rust never interprets row content."
  - `(prompt! label on-confirm #:prefill text)` — takes over the minibuffer (`MiniBuffer { prompt, input, cursor }` + `steel_prompt_callback`, one at a time), Mode::Command; confirm fires once with text or `#f`.
- **Buffer/introspection builtins available to sources**: `buffers`, `buffer-name`, `buffer-path`, `buffer-language`, `current-buffer`, `current-selections`, `symbol-under-cursor`, `diagnostics-for-buffer`, `lsp-capabilities`, … (full registry: `register_fn_with_ctx` calls in `hume-scripting/src/builtins/mod.rs`).

### The minibuffer completion system is a separate thing — leave it alone

`hume-editor/src/editor/completion/` (`Completer` trait: pure `complete(input, cursor, ctx) -> CompletionResult`; implementors `CommandCompleter`, `BufferNameCompleter`, `ThemeCompleter`, `PathCompleter`, `SetCompleter`; dispatched by a hardcoded match in `complete_minibuf`, `mappings/command_mode.rs`; prefix matching only; rendered by the bespoke statusline-anchored `CompletionOverlay` in `ui/completion_overlay.rs`). It shares no types with the insert-mode stack, and `ROADMAP.md` already records its own future direction ("Steel builtin to register custom completers … core does prefix matching only"). Neither this design nor the picker design (`docs/FUZZY-FINDERS.md`) builds on it, and neither changes it.

### Gaps (what does not exist today)

1. **No fuzzy matcher** — hand-rolled subsequence only. (`docs/FUZZY-FINDERS.md`'s B1 adds one for the picker; Q-B6 there tracks whether completion ever adopts it.)
2. **No multi-source merge** — `completion-begin!` replaces; a slow source's arrival clobbers a fast source's session (or vice versa). No source tags, no priorities, no dedup.
3. **No way to read buffer text from Steel** (by design — bulk guardrail). A buffer-words completion source therefore needs a bounded Rust builtin (task A4), not a Steel scan.

---

## Scriptable completion sources

### Goal

A plugin author writes:

```scheme
(register-completion-source! "buffer-words"
  (lambda (bid prefix emit)
    (emit (map word->item (buffer-words bid prefix 50))))
  #:priority 10)
```

and their items appear in the same menu as LSP completions, ranked by the same Rust filter, accepted through the same gen-checked edit path. LSP becomes *a* source instead of *the* source. Mixing policy (ordering, per-source caps, dedup) is Steel; per-keystroke work stays Rust.

### Design

**A new `core:completion` plugin owns orchestration.** It is the only caller of `completion-begin!`/`completion-add-items!`. It owns:

- the `lsp-completion-trigger` command and its `(bind-key! 'insert "ctrl-space" …)` call (both move out of `core:lsp`'s `plugin.scm`; the binding targets the command *name*, so no other keymap changes are needed),
- the `on-trigger-char` subscription (each source declares its trigger chars; the coordinator unions them via the existing `register-trigger-chars!` mechanism — which is already keyed by source name),
- the `on-completion-refilter` subscription (re-invokes only sources that flagged themselves incomplete),
- a pure-Steel source registry: `(register-completion-source! name fn #:priority n #:trigger-chars lst)`. No Rust registry needed — this is per-user-intent frequency.

**Source contract**: `fn` receives `(bid prefix emit)` where `emit` is a closure the coordinator provides; the source calls `(emit items)` once, synchronously or from an async callback (e.g. inside an `lsp-request` callback). Items are completion-item hashmaps in the **LSP `CompletionItem` JSON shape** — that shape stays the lingua franca because `StoredCompletionItem::from_json` already parses it and the fallbacks (`filterText`→`label` etc.) make the minimal item just `{"label": "foo"}`. Non-LSP sources simply omit `textEdit` and get the generic anchor-span insert path.

One source class this contract can't serve yet: an **external-command-backed source** (dictionary/spell via `aspell`, a snippets CLI, shell history). Steel's process spawn is synchronous only — a spawn at trigger time freezes the editor for the command's duration. The fix is the generic `spawn-async` builtin designed and deferred in `docs/FUZZY-FINDERS.md` (B5's deferral note): command output batches to a Steel callback, which feeds `completion-add-items!` — the session token already makes late async arrivals harmless by construction. Such a source is the named candidate "second client" that would un-defer that builtin; no design change needed here when it does.

**Incremental arrival — the one real Rust change.** Sources finish at different times (buffer-words: instant; LSP: 10–300ms). Two models considered:

- *Single-shot*: coordinator waits for all sources (with an `(after …)` timeout), concatenates, calls `completion-begin!` once. Works with zero Rust changes, but the menu's appearance is gated on the slowest source or a timeout constant — exactly the UX modern editors moved away from.
- *Incremental* (**chosen**): first `emit` calls `completion-begin!`; later `emit`s call a new `(completion-add-items! token items #:source name #:priority n #:incomplete flag)` that merges into the open session and re-ranks. Menu appears instantly with cheap sources, LSP items merge in when ready.

Rust work for incremental:

1. `completion-begin!` grows the same `#:source`/`#:priority` keywords (the first-arriving source is a tagged contributor like any other; it already has `#:incomplete`) and returns an opaque **session token** (monotonic `u64` on `EditorState`, bumped per begin). `completion-add-items!` takes the token and is a silent no-op if it doesn't match the current session — this kills the whole class of late-async-callback races (user dismissed and retriggered; source from the *previous* trigger finally answers). The existing `generation_at_begin` guard is orthogonal (it protects the *edit*, not session identity) and stays as-is.
2. **Merge is replace-per-source, not append**: an add first evicts any items already tagged with that source name, then inserts the new list. Same-source re-emission (the isIncomplete refilter flow below re-invokes a source on the *same* session) is therefore idempotent — no duplicates — while other sources' items are untouched.
3. `StoredCompletionItem` gains `source: Box<str>` (or an interned id) — used for the eviction in (2), for a rank tiebreaker (source priority, passed once at begin/add time), and available to `completion_row_label` for display. `to_json` grows a `"source"` field.
4. `update_filter`'s rank key becomes `(prefix_match, match_pos, source_priority, sort_text)`. Exact position of `source_priority` in the key: see Q-A3.
5. Merge must **preserve the user's current selection** if possible (re-rank moves rows under the cursor — v1: reset selection to 0 on merge, matching what `refilter_lsp_completion_after_edit` already does by clearing `lsp_completion_ui`; smarter selection-tracking is a polish item).

**Accept stays per-source via the existing hook.** `on-completion-accept` receives the raw item, which now carries `"source"` — the `core:lsp` plugin's handler guards on `(equal? (hash-ref item "source") "lsp")` before doing `additionalTextEdits`/resolve. Other sources register their own handlers or none. No Rust change.

**isIncomplete becomes per-source**: every `begin`/`add` carries `#:incomplete` for its source, and the session-level `incomplete` flag (which gates the `OnCompletionRefilter` hook fire) is recomputed as the OR across each source's *latest* flag — a slow source arriving incomplete via `completion-add-items!` must be able to flip a session that began complete. The coordinator tracks *which* sources were incomplete and re-invokes only those on refilter; their fresh results flow through the same `completion-add-items!`, where replace-per-source semantics (Rust work item 2) prevent duplication.

**Buffer-words needs one bounded builtin** (`(buffer-words bid prefix max-n)`): Rust scans the buffer with the existing word segmentation (`hume-editing/src/word.rs`), returns ≤ max-n distinct words matching prefix (case-insensitive subsequence or prefix — see Q-A5). Bounded output at user-intent frequency = guardrail-compliant. Steel wraps it into a source in ~10 lines.

**Renames (fold into A1)**: `EditorState.lsp_completion` → `completion_session`, `lsp_completion_ui` → `completion_ui`, `lsp_completion_view` → `completion_view`, `LspCompletionUi` → `CompletionUi`, `clear_lsp_completion` → `clear_completion`, module `editor/lsp/completion.rs` → `editor/completion_session.rs` (NOT into `editor/completion/` — that's the minibuffer system; keep the two apart). `sync_lsp_completion_view` → `sync_completion_view`. Comment headers "LSP completion menu" updated. Mechanical; do it first so everything after reads honestly.

### Task breakdown

| ID | Task | Depends | Size |
|----|------|---------|------|
| A1 | Rename pass: de-LSP the session/store/view names (see list above). No behavior change. | — | S (mechanical, wide) |
| A2 | Session token + `completion-add-items!` (replace-per-source merge) + `source` tag + per-source `#:incomplete` (session flag = OR of latest per-source flags) + priority tiebreaker + per-source rank/display plumbing. Rust: `completion_session.rs`, host trait + `host_impl.rs`, builtin + bootstrap wrapper in `hume-scripting`. Tests: token mismatch no-op, merge re-rank, same-source re-add replaces (no duplicates), late add flips `incomplete`, selection reset, source tiebreak. | A1 | M |
| A3 | `core:completion` plugin: source registry, coordinator (begin/add orchestration, per-source incomplete tracking, trigger-char union, refilter fan-out), move `lsp-completion-trigger` + `on-trigger-char` + `on-completion-refilter` out of `core:lsp`; `core:lsp` re-shapes into a registered source (its `on-completion-accept` handler gains the source guard). Tests: two mock sources (fast sync + slow `after`-delayed), late-arrival merge, stale-token drop, accept-hook source filtering. | A2 | M |
| A4 | `buffer-words` builtin + the buffer-words source plugin (`core:buffer-words` or part of `core:completion` — Q-A6). Tests: dedup, bound, prefix vs subsequence per the Q-A5 decision, no-panic on huge buffer. | A3 | S–M |

Estimated total: comparable to one-and-a-half LSP Step 4 cards. No architectural risk; every piece lands behind existing seams.

---

## What to do *now* (foundation checklist)

1. **Nothing structural.** Verified: no current abstraction blocks this design; no in-flight lsp-branch work needs redirecting.
2. **Hold the line on store purity**: any new completion feature that wants Rust to parse another LSP-specific `CompletionItem` field should instead read it in Steel from the `raw` item (accept hook) — that's the existing design intent, keep honoring it.
3. **Optional, zero-risk, anytime**: the A1 rename pass can land independently whenever the lsp branch is quiet (it churns many lines; do it in a lull, not mid-feature).
4. ROADMAP already points here — the Future-section "Scriptable completion sources" line. Nothing left to groom.

## Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Foundation timing | **Nothing now; additive later** | Completion store already source-agnostic. Verified against source. |
| Item schema for completion sources | **LSP `CompletionItem` JSON shape as lingua franca** | Store already parses it with label-fallbacks making minimal items trivial; non-LSP sources omit `textEdit` and ride the generic anchor-span accept path (works serverless — UTF-16 default round-trips). |
| Multi-source merge model | **Incremental: `completion-begin!` returns token; `completion-add-items!` merges with replace-per-source semantics; both carry `#:source`/`#:priority`/`#:incomplete`** | Menu appears at fastest-source speed; token makes late async arrivals from stale triggers harmless; replace-per-source makes isIncomplete re-requests idempotent (no duplicates). Single-shot rejected: gates UX on slowest source/timeout. |
| Source registry location | **Pure Steel, in `core:completion`** | Registration and orchestration are user-intent frequency; no Rust registry earns its keep. Mirrors the `trigger_chars` precedent (Rust holds only the union it needs for the Insert-mode fire check). |
| Where mixing policy lives | **Steel (priority, caps, accept handlers); Rust (per-keystroke rank incl. priority tiebreak)** | Frequency cut. Priority crosses once at begin/add; ranking uses it per keystroke without re-crossing. |
| Completion vs picker core | **Siblings sharing the matcher, not a shared session type — see `docs/FUZZY-FINDERS.md`** | Six load-bearing axes differ (item shape, query origin, accept, lifetime, scale, scroll). Abstraction with two divergent call sites is premature; merging later is cheap if bodies converge. |

## Open questions

Each carries a default per the usage rules.

**Q-A1 — dedup across completion sources.** Buffer-words will echo identifiers LSP also returns. Dedup by what key — `label`? `(label, insertText)`? And who wins — higher priority source? *Default: no dedup in v1; priority ordering puts the richer (LSP) item first and the duplicate a few rows down. Revisit with real usage; if added, dedup belongs in Rust at `completion-add-items!` time (per-merge, not per-keystroke) keyed on `insert_text`, keeping the higher-priority item.*

**Q-A2 — token plumbing shape.** Return token from `completion-begin!` (builtin return value) vs. a separate `(completion-session-token)` getter. *Default: return it from `completion-begin!` — one fewer builtin, and the coordinator is the only caller anyway.*

**Q-A3 — where source priority sits in the rank key.** Before or after `match_pos`? Before means a low-quality prefix match from a high-priority source beats a perfect match from a low-priority one. *Default: `(prefix_match, match_pos, source_priority, sort_text)` — priority as tiebreaker only; match quality stays king. Revisit if LSP items feel buried.*

**Q-A4 — per-source item caps.** Should the coordinator cap each source's contribution (e.g. buffer-words ≤ 50) in Steel, or should Rust enforce a per-add cap? *Default: Steel-side cap in the coordinator (policy), with `completion-add-items!` accepting whatever it's given; Rust store has no per-source limits.*

**Q-A5 — `buffer-words` matching semantics.** Prefix-only (cheap, classic vim `i_CTRL-N` feel) vs. subsequence (consistent with the session's own filter)? *Default: prefix at collection time — trivially cheap scan, vim-precedented feel. Honest cost: prefix collection is NOT a superset of what the session's subsequence filter can match (`flag_option` matches subsequence `fo` but not prefix `fo`), so subsequence-only candidates never reach the store; accepted for v1.*

**Q-A6 — buffer-words packaging.** Own plugin (`core:buffer-words`, lazy-loadable, deletable) vs. bundled into `core:completion`. *Default: own plugin — it's the reference example of a third-party-shaped source, and dogfooding the registration API from a *separate* plugin proves cross-plugin registration works.*

**Q-A7 — kind display.** `kind: i64` is currently display-unused (`completion_row_label` shows `label  detail` only). Map kind→short label/icon in Rust (`completion_row_label`) with a static table, themable? Non-LSP sources reuse LSP kind numbers? *Default: static Rust map (LSP kind numbers as the universal enum — sources pick the closest; 1=Text fits buffer-words), single-char column, no per-kind theming in v1. Note: per-part styling (dimmed detail, colored kind) needs segment-styled popup rows — a `PopupState` extension that's its own small task; don't smuggle it in.*

Fuzzy-picker open questions (Q-B1–Q-B8) moved to `docs/FUZZY-FINDERS.md`. **Q-B6** there (unifying completion's matcher with the picker's) is the one that loops back to this document — noted in the Gaps section above.
