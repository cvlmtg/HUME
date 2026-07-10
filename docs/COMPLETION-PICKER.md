# HUME — Scriptable Completion Sources & Fuzzy Picker Foundation

Design document for two related future features:

- **Part A — scriptable completion**: the insert-mode completion menu driven by multiple Steel-registered sources (LSP, buffer words, custom plugins), mixed and prioritized by policy written in Steel.
- **Part B — fuzzy pickers**: a generic scriptable fuzzy-finder surface (files, buffers, symbols, anything a plugin feeds it), Helix/telescope-style.

**Status: design only — not scheduled, no implementation started.** Written 2026-07-10 against the `lsp` branch (post Step 4 / F11). This document is the single place to resume from; it assumes the reader has *no* memory of the exploration that produced it.

**Headline conclusions** (the "do we need to lay foundations now?" answer):

1. **No foundation work is required now.** Both parts are additive. The lsp branch's completion architecture is already source-agnostic in the ways that matter; the picker is a new widget + new builtins + one new crate dependency. Nothing currently being built needs to change shape to keep these possible.
2. The one guardrail while other work proceeds: **don't deepen LSP coupling in the completion store**. `CompletionSession` today parses generic completion-item JSON and only touches LSP specifics inside the `text_edit` branch of `accept`. Keep it that way — new LSP-specific fields belong in the Steel plugin (which already receives the raw item), not in new Rust parsing.
3. Completion and picker should **share the fuzzy matcher and the "Rust store, Steel policy" split**, but not a data structure. `CompletionSession` and the future `PickerSession` are siblings, not the same type — see [Why not one shared session type](#why-not-one-shared-session-type).

## How to use this document

Same rules as `docs/LSP.md`:

1. **Verify before you write.** Symbols named here were verified on 2026-07-10, but the codebase moves. `rg 'symbol_name'` before relying on anything. No line numbers anywhere in this doc — navigate by symbol search.
2. **If the doc contradicts the code, STOP** and report; don't silently adapt.
3. Open questions each carry a `Default:`. At implementation time, adopt the default unless evidence gathered during the task contradicts it — then record the decision in the Decisions table here.
4. Project-wide rules from `CLAUDE.md` apply (no `.unwrap()` outside tests, grapheme discipline, every command tested).

## Architecture constraints inherited from LSP work

These are settled project decisions (see `docs/LSP.md` Decisions table) and both parts must respect them:

- **Frequency cut**: per-user-intent work (a trigger keypress, a response arriving, a selection made) may run in Steel; per-keystroke filtering, per-frame rendering, and unbounded-collection work must be Rust.
- **Bulk-data guardrail**: bulk item lists never cross the Rust↔Steel boundary on recurring paths. One-time ingest at user-intent frequency is the calibrated exception (P8 spike: ~1ms for 1k completion items through the boundary — acceptable; do not assume this scales to 100k file paths).
- **Steel never on the render path**: Steel writes models/stores; Rust providers render from `Arc<RwLock<…>>` snapshots each frame.
- **Rust-rendered, Steel-fed widgets**: "LSP is their first client, not their owner."

---

## Current state — verified inventory (lsp branch, 2026-07-10)

Everything below was read from source, not recalled. This is the substrate both parts build on.

### Insert-mode completion stack (the thing Part A extends)

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
- Entry points: `(define-command! "completion-trigger" …)` (Ctrl+Space is bound to that command name) and the `on-trigger-char` hook filtered by `*completion-chars*` (populated on `on-lsp-attach` from `completionProvider.triggerCharacters`, cleared on detach).
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
| Trigger *ownership* (`completion-trigger` command, `on-trigger-char` subscription) | Lives in `core:lsp` plugin — needs relocation (task A3) |
| `on-completion-accept` post-processing | LSP-specific by content, but it's Steel — each source brings its own handler |
| Naming (`lsp_completion*` fields, `editor/lsp/completion.rs` path, `LspCompletionUi`) | Cosmetic LSP residue — rename in A1 |
| Snippet stripping | Correctly lives in the LSP source plugin; stays there |

### Adjacent infrastructure the designs lean on

- **Async Rust→Steel callbacks**: `lsp-request` queues `PendingLspRequest` on `SteelCtx.pending_lsp_requests`; after eval, `flush_pending_lsp_requests` / `send_one_lsp_request` (`hume-editor/src/editor/lsp/bridge.rs`) register a boxed callback keyed `(ServerId, RequestId)`; reader threads → mpsc → `drain_lsp` each frame → `dispatch_completed` → `Editor::queue_steel_call(callback, args)` (`scripting_setup.rs`). Staleness: response dropped if the buffer's `text_gen` moved, unless `#:allow-stale`. **This is the template for any "async work finishes → call Steel closure" need.**
- **Timers**: `(after ms thunk)` / `(cancel-timer! id)` builtins; `(debounce ms proc)` is pure Scheme over them (bootstrap in `builtins/mod.rs`).
- **Generic widgets** (all in `host_impl.rs` + `ui/popup.rs` + `ui/drawer.rs`):
  - `(show-popup! text #:anchor 'cursor)` / `(close-popup!)` — `PopupModel`, hover-style text panel.
  - `(show-menu! items on-select)` / `(close-menu!)` — `MenuModel { items, selected, callback }`; callback fires exactly once (selection or dismissal); **blocked in Insert mode** (`show_menu` returns Err — deliberate, the completion menu owns that slot). Keys intercepted by `handle_menu_key` (`mappings/mod.rs`) ahead of keymap dispatch.
  - `(show-drawer-list! items on-select)` / `(close-drawer!)` — `DrawerModel { items, selected, scroll, callback }`, bottom chrome band, stays open across Enter (callback may fire repeatedly, `#f` on close), `handle_drawer_key`. Rows are pre-formatted display strings; "Rust never interprets row content."
  - `(prompt! label on-confirm #:prefill text)` — takes over the minibuffer (`MiniBuffer { prompt, input, cursor }` + `steel_prompt_callback`, one at a time), Mode::Command; confirm fires once with text or `#f`.
- **Buffer/introspection builtins available to sources**: `buffers`, `buffer-name`, `buffer-path`, `buffer-language`, `current-buffer`, `current-selections`, `symbol-under-cursor`, `diagnostics-for-buffer`, `lsp-capabilities`, … (full registry: `register_fn_with_ctx` calls in `hume-scripting/src/builtins/mod.rs`).

### The minibuffer completion system is a separate thing — leave it alone

`hume-editor/src/editor/completion/` (`Completer` trait: pure `complete(input, cursor, ctx) -> CompletionResult`; implementors `CommandCompleter`, `BufferNameCompleter`, `ThemeCompleter`, `PathCompleter`, `SetCompleter`; dispatched by a hardcoded match in `complete_minibuf`, `mappings/command_mode.rs`; prefix matching only; rendered by the bespoke statusline-anchored `CompletionOverlay` in `ui/completion_overlay.rs`). It shares no types with the insert-mode stack, and `ROADMAP.md` already records its own future direction ("Steel builtin to register custom completers … core does prefix matching only"). Neither Part A nor Part B builds on it, and neither changes it. The only touchpoint: if the picker ever subsumes `:b`-style buffer switching, the `:b` completer stays anyway (different interaction grammar — typed command vs. modal picker).

### Gaps (what does not exist today)

1. **No fuzzy matcher** — hand-rolled subsequence only.
2. **No multi-source merge** — `completion-begin!` replaces; a slow source's arrival clobbers a fast source's session (or vice versa). No source tags, no priorities, no dedup.
3. **No input-box-plus-filtered-list widget** — menu and drawer take static pre-filtered lists; `prompt!` has an input but no list. A picker needs both in one surface.
4. **No way to enumerate project files from Steel** — the builtin registry has `curl-fetch` / `git-clone` / `git-clone-rev` / `git-pull` (PLUM) and a `list-dir` (`builtins/fs.rs` — flat, non-recursive, sandboxed to the plugin directories), but **no shell/spawn builtin and no general-purpose walk**. A file picker's source *cannot* be written in Steel today (the sandbox and non-recursion make `list-dir` useless for it) — and per the bulk guardrail it *shouldn't* be (100k paths through the boundary per keystroke is exactly the forbidden pattern). File enumeration must be a native Rust source.
5. **No way to read buffer text from Steel** (by design — bulk guardrail). A buffer-words completion source therefore needs a bounded Rust builtin (task A4), not a Steel scan.
6. **Completion menu shows a fixed top-8 window** with selection clamped to it — acceptable for completion; a picker needs real scrolling over the full ranked list.

---

## Part A — Scriptable completion sources

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

- the `completion-trigger` command (moves out of `core:lsp`; the Ctrl+Space binding already targets the command *name*, so the keymap doesn't change),
- the `on-trigger-char` subscription (each source declares its trigger chars; the coordinator unions them via the existing `register-trigger-chars!` mechanism — which is already keyed by source name),
- the `on-completion-refilter` subscription (re-invokes only sources that flagged themselves incomplete),
- a pure-Steel source registry: `(register-completion-source! name fn #:priority n #:trigger-chars lst)`. No Rust registry needed — this is per-user-intent frequency.

**Source contract**: `fn` receives `(bid prefix emit)` where `emit` is a closure the coordinator provides; the source calls `(emit items)` once, synchronously or from an async callback (e.g. inside an `lsp-request` callback). Items are completion-item hashmaps in the **LSP `CompletionItem` JSON shape** — that shape stays the lingua franca because `StoredCompletionItem::from_json` already parses it and the fallbacks (`filterText`→`label` etc.) make the minimal item just `{"label": "foo"}`. Non-LSP sources simply omit `textEdit` and get the generic anchor-span insert path.

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

### Part A task breakdown

| ID | Task | Depends | Size |
|----|------|---------|------|
| A1 | Rename pass: de-LSP the session/store/view names (see list above). No behavior change. | — | S (mechanical, wide) |
| A2 | Session token + `completion-add-items!` (replace-per-source merge) + `source` tag + per-source `#:incomplete` (session flag = OR of latest per-source flags) + priority tiebreaker + per-source rank/display plumbing. Rust: `completion_session.rs`, host trait + `host_impl.rs`, builtin + bootstrap wrapper in `hume-scripting`. Tests: token mismatch no-op, merge re-rank, same-source re-add replaces (no duplicates), late add flips `incomplete`, selection reset, source tiebreak. | A1 | M |
| A3 | `core:completion` plugin: source registry, coordinator (begin/add orchestration, per-source incomplete tracking, trigger-char union, refilter fan-out), move `completion-trigger` + `on-trigger-char` + `on-completion-refilter` out of `core:lsp`; `core:lsp` re-shapes into a registered source (its `on-completion-accept` handler gains the source guard). Tests: two mock sources (fast sync + slow `after`-delayed), late-arrival merge, stale-token drop, accept-hook source filtering. | A2 | M |
| A4 | `buffer-words` builtin + the buffer-words source plugin (`core:buffer-words` or part of `core:completion` — Q-A6). Tests: dedup, bound, prefix vs subsequence per the Q-A5 decision, no-panic on huge buffer. | A3 | S–M |

Estimated total: comparable to one-and-a-half LSP Step 4 cards. No architectural risk; every piece lands behind existing seams.

---

## Part B — Fuzzy picker foundation

### Goal

`(picker! items on-select #:prompt "buffer: ")` from any plugin opens a modal centered panel: an input line the user types a fuzzy query into, a scrolling ranked list below, Enter fires `on-select` with the chosen item's payload, Esc closes. Native bulk sources (files) stream into the same surface without Steel ever seeing the item flood. Files/buffers/symbols pickers then ship as thin Steel plugins.

### Design

**New Rust core: `PickerSession`** (new module, e.g. `hume-editor/src/editor/picker.rs`) — deliberately a sibling of `CompletionSession`, not a generalization of it.

#### Why not one shared session type

They rhyme (items, query, ranked indices, top-N, accept-by-index) but differ in every load-bearing detail: item shape (JSON completion items with edit semantics vs. display string + opaque payload), query origin (buffer text between anchor and cursor vs. a widget-owned input line), accept semantics (gen-checked buffer edit vs. fire a Steel callback), lifetime (bound to an Insert-mode token vs. modal), scale (≤ a few k items, replace-per-response vs. up to ~100k, streamed), and scroll model (top-8 window vs. full scroll). A shared abstract core would be parameterized over all six axes — premature abstraction with two very different call sites. **What they share instead: the fuzzy matcher (B1) and the architectural pattern.** If, after both exist, the bodies converge, merging is a cheap refactor; guessing the abstraction up front is not.

#### The pieces

**B1 — fuzzy matcher.** Add `nucleo-matcher` (the matcher-only crate Helix's picker engine is built on: scoring + Unicode handling, no threading harness). Wrap it behind a small module (e.g. `hume-editor/src/editor/fuzzy.rs`) exposing `score(query, haystack) -> Option<(score, …)>` so neither session type names the crate directly. Completion's hand-rolled `subsequence_match_pos` is *not* replaced in this task (Q-B6 tracks later unification). Include a micro-benchmark-ish test at 100k synthetic paths to validate the single-threaded per-keystroke budget — run it in release mode or mark it `#[ignore]`-by-default (wall-clock asserts in debug/CI builds are flaky; the gate is an explicit `--ignored`/release run). If it blows the frame budget, the fallback is documented in Q-B1 (full `nucleo` with background matching).

**B2 — `PickerSession` store.**

```
PickerItem { display: String, payload: SteelVal, source_idx: u32 }
PickerSession {
    items: Vec<PickerItem>,          // append-only while open
    query: String,
    filtered: Vec<u32>,              // ranked indices, rebuilt on query/items change
    selected: usize,                 // index into filtered (full range, not a window)
    scroll: usize,                   // first visible row, clamped like DrawerModel
    on_select: SteelVal,             // fired with payload on Enter
    token: u64,                      // same stale-push guard as completion's A2 token
}
```

`payload` is an arbitrary `SteelVal` (string path, hashmap, whatever the source chose) — Rust never interprets it, mirroring the drawer's "rows are pre-formatted display strings" contract. Query edits and item pushes re-rank via B1. Empty query = insertion order — note B5's *parallel* walker delivers in nondeterministic interleaved order, not walk order; accept the arbitrary order for v1 or use the serial walker if it reads badly. (nucleo-matcher is expected to treat an empty query as all-match — external-crate claim, verify at impl time.)

**B3 — widget + interaction.** New centered overlay (e.g. `ui/picker_panel.rs`): bordered panel sized as a fraction of the *panes region* (the terminal minus chrome bands — see Q-B2) — say width `min(80%, 100 cols)`, height `min(60%, 30 rows)`; input line at top (rendered from `query` + a block cursor cell), ranked list below with `selected` highlighted and real scrolling. Theme scopes `ui.picker`, `ui.picker.selected`, `ui.picker.input`. **Theme fallback caveat**: `Theme::resolve_raw` falls back by prefix-trimming only (`ui.picker.selected` → `ui.picker` → `ui` → default) — it will *never* reach `ui.menu`. Graceful degradation on themes without the new scopes needs the picker's scope lookup to explicitly alias to the matching `ui.menu*` scope when `ui.picker*` is absent, plus `ui.picker*` entries added to the bundled themes (mind the `feedback_theme_cursor_insert_required` precedent: document the new scopes). Rendering follows the universal write-side/read-side split: `sync_picker_view` in `prepare_frame` writes an `Arc<RwLock<Option<PickerViewState>>>`; a new `OverlayProvider` only paints (per-pane registration suffices — overlays receive the whole panes region, Q-B2).

Key routing: intercept in `dispatch_key` ahead of keymap dispatch while a picker is open — same pattern as `handle_menu_key`/`handle_drawer_key` in `mappings/mod.rs`, **not** a new `Mode` variant (mode machinery drags in statusline, cursor-shape, `OnModeChange` hooks, keymap tries — all noise here; the menu/drawer precedent is established and tested). All printable chars edit the query in Rust (no Steel per keystroke); Up/Down/Ctrl-p/Ctrl-n move selection; PageUp/PageDown scroll; Enter fires `on_select` with the payload via `queue_steel_call` and closes; Esc closes and fires `on_select` with `#f` (matching the menu/drawer dismissal convention).

**B4 — Steel surface.**

```scheme
(picker! items on-select #:prompt "…")      ; items: list of (display . payload) pairs or
                                            ; hashmaps {"display", "payload"} — decide at impl
(picker-push! token items)                  ; append from async callbacks (bounded chunks)
(picker-close!)
```

`picker!` returns the token. Small/medium lists (buffers, recent files, custom plugin lists, ≤ a few k) go through `picker!` directly — one-time user-intent ingest, guardrail-compliant. Async Steel sources (LSP `workspace/symbol`, which is an explicit LSP-v1 non-goal but becomes reachable the moment this exists) use `picker-push!` from their `lsp-request` callbacks with the token guard eating staleness.

**B5 — native file source.** File enumeration never touches Steel: `(picker-files! dir on-select #:prompt "file: ")` (or `picker!` with a `#:native 'files` source selector — pick whichever reads better at impl time) starts a Rust background walker — `ignore` crate (ripgrep's walker: .gitignore-aware, hidden-file rules, parallel) on a std thread, streaming batches over an mpsc channel drained in `prepare_frame` (mirror `drain_lsp` / the parse-worker drain in shape — note both drain *everything available* per frame, not a bounded count; bound the walker drain only if item floods prove a problem). **The wake is new machinery, not reuse**: timers wake the loop only by bounding the `event::poll` timeout from a known future deadline (`AsyncSource::next_wake` → `wake_timeout()`); when nothing is pending the loop blocks in `event::read()`, which a walker thread finishing at an unpredictable moment cannot interrupt. B5 therefore includes its own wake sub-task — either a waker that injects an event into the crossterm stream, or an `AsyncSource` whose `next_wake` returns a short polling deadline while the walker is live. Each batch appends to the session store and re-ranks. `on_select` receives the path string; the shipped files picker plugin calls `(open-buffer! path)`. Walker thread is cancelled/detached on picker close (token check on the drain side makes late batches harmless — same pattern as everything else).

**B6 — shipped pickers plugin** (`core:pickers` or similar): files (B5), buffers (pure Steel over the `buffers` builtin — display via `buffer-name`, payload `bid`, select via `switch-to-buffer!`), plus commands to bind (`picker-files`, `picker-buffers`). This is the dogfooding step proving the surface is complete for community plugins.

**Explicit non-goals for picker v1** (each is additive later): preview pane (needs scratch-buffer rendering inside the panel — a real project; see Q-B4), live-requery sources (live-grep style, where the *query* re-runs the source — needs an `on-query-change` callback option with debounce; the store/widget design above doesn't preclude it, see Q-B5), multi-select, picker-specific keybinding customization.

### Part B task breakdown

| ID | Task | Depends | Size |
|----|------|---------|------|
| B1 | `nucleo-matcher` dep + `fuzzy.rs` wrapper + 100k-item budget test | — | S |
| B2 | `PickerSession` store: push/query/rank/select/scroll/token; unit tests with mock items | B1 | M |
| B3 | Panel widget + view sync + key interception; insta snapshot tests + interaction tests | B2 | M–L (largest single piece — new chrome surface) |
| B4 | Steel builtins `picker!`/`picker-push!`/`picker-close!` + host trait/impl + callback firing; tests incl. stale-token push | B3 | M |
| B5 | Native file walker source (`ignore` crate, drain, cancellation) + event-loop wake for walker batches (new machinery — see design note) | B4 | M |
| B6 | `core:pickers` plugin (files + buffers) + default bindings + user-manual page | B5 | S |

Estimated total: comparable to one LSP macro-step (a Step-3-sized effort — new UI surface + new builtins + one worker). Independent of Part A except for sharing B1's matcher module if A later adopts it. ROADMAP M12 already points here: "File picker / fuzzy finder (Helix-style): full design + task breakdown in `docs/COMPLETION-PICKER.md` (Part B). Splits dependency satisfied by M10 T2; remaining gate is prioritization."

---

## What to do *now* (foundation checklist)

1. **Nothing structural.** Verified: no current abstraction blocks either part; no in-flight lsp-branch work needs redirecting.
2. **Hold the line on store purity**: any new completion feature that wants Rust to parse another LSP-specific `CompletionItem` field should instead read it in Steel from the `raw` item (accept hook) — that's the existing design intent, keep honoring it.
3. **Optional, zero-risk, anytime**: the A1 rename pass can land independently whenever the lsp branch is quiet (it churns many lines; do it in a lull, not mid-feature).
4. ROADMAP already points here — M12's picker line (Part B) and the Future-section "Scriptable completion sources + fuzzy pickers" line (Part A + B). Nothing left to groom.

## Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Foundation timing | **Nothing now; both parts additive later** | Completion store already source-agnostic; picker is net-new surface. Verified against source 2026-07-10. |
| Item schema for completion sources | **LSP `CompletionItem` JSON shape as lingua franca** | Store already parses it with label-fallbacks making minimal items trivial; non-LSP sources omit `textEdit` and ride the generic anchor-span accept path (works serverless — UTF-16 default round-trips). |
| Multi-source merge model | **Incremental: `completion-begin!` returns token; `completion-add-items!` merges with replace-per-source semantics; both carry `#:source`/`#:priority`/`#:incomplete`** | Menu appears at fastest-source speed; token makes late async arrivals from stale triggers harmless; replace-per-source makes isIncomplete re-requests idempotent (no duplicates). Single-shot rejected: gates UX on slowest source/timeout. |
| Source registry location | **Pure Steel, in `core:completion`** | Registration and orchestration are user-intent frequency; no Rust registry earns its keep. Mirrors the `trigger_chars` precedent (Rust holds only the union it needs for the Insert-mode fire check). |
| Where mixing policy lives | **Steel (priority, caps, accept handlers); Rust (per-keystroke rank incl. priority tiebreak)** | Frequency cut. Priority crosses once at begin/add; ranking uses it per keystroke without re-crossing. |
| Completion vs picker core | **Siblings sharing the matcher, not a shared session type** | Six load-bearing axes differ (item shape, query origin, accept, lifetime, scale, scroll). Abstraction with two divergent call sites is premature; merging later is cheap if bodies converge. |
| Picker modality | **Key interception ahead of keymap (menu/drawer pattern), no new `Mode`** | Mode machinery (statusline, cursor shape, hooks, keymap tries) is all cost, no benefit; `handle_menu_key`/`handle_drawer_key` precedent is established and tested. |
| File enumeration | **Native Rust walker (`ignore` crate) streaming into the store; never through Steel** | Bulk guardrail: 10k–100k paths per keystroke through the boundary is the exact forbidden pattern; also no Steel fs/shell builtin exists, and adding one for this would be the wrong fix. |
| Minibuffer `Completer` system | **Untouched by both parts** | Separate system, separate roadmap line; different interaction grammar. |

## Open questions

Each carries a default per the usage rules.

**Q-A1 — dedup across completion sources.** Buffer-words will echo identifiers LSP also returns. Dedup by what key — `label`? `(label, insertText)`? And who wins — higher priority source? *Default: no dedup in v1; priority ordering puts the richer (LSP) item first and the duplicate a few rows down. Revisit with real usage; if added, dedup belongs in Rust at `completion-add-items!` time (per-merge, not per-keystroke) keyed on `insert_text`, keeping the higher-priority item.*

**Q-A2 — token plumbing shape.** Return token from `completion-begin!` (builtin return value) vs. a separate `(completion-session-token)` getter. *Default: return it from `completion-begin!` — one fewer builtin, and the coordinator is the only caller anyway.*

**Q-A3 — where source priority sits in the rank key.** Before or after `match_pos`? Before means a low-quality prefix match from a high-priority source beats a perfect match from a low-priority one. *Default: `(prefix_match, match_pos, source_priority, sort_text)` — priority as tiebreaker only; match quality stays king. Revisit if LSP items feel buried.*

**Q-A4 — per-source item caps.** Should the coordinator cap each source's contribution (e.g. buffer-words ≤ 50) in Steel, or should Rust enforce a per-add cap? *Default: Steel-side cap in the coordinator (policy), with `completion-add-items!` accepting whatever it's given; Rust store has no per-source limits.*

**Q-A5 — `buffer-words` matching semantics.** Prefix-only (cheap, classic vim `i_CTRL-N` feel) vs. subsequence (consistent with the session's own filter)? *Default: prefix at collection time — trivially cheap scan, vim-precedented feel. Honest cost: prefix collection is NOT a superset of what the session's subsequence filter can match (`flag_option` matches subsequence `fo` but not prefix `fo`), so subsequence-only candidates never reach the store; accepted for v1.*

**Q-A6 — buffer-words packaging.** Own plugin (`core:buffer-words`, lazy-loadable, deletable) vs. bundled into `core:completion`. *Default: own plugin — it's the reference example of a third-party-shaped source, and dogfooding the registration API from a *separate* plugin proves cross-plugin registration works.*

**Q-A7 — kind display.** `kind: i64` is currently display-unused (`completion_row_label` shows `label  detail` only). Map kind→short label/icon in Rust (`completion_row_label`) with a static table, themable? Non-LSP sources reuse LSP kind numbers? *Default: static Rust map (LSP kind numbers as the universal enum — sources pick the closest; 1=Text fits buffer-words), single-char column, no per-kind theming in v1. Note: per-part styling (dimmed detail, colored kind) needs segment-styled popup rows — a `PopupState` extension that's its own small task; don't smuggle it in.*

**Q-B1 — matcher crate: `nucleo-matcher` vs full `nucleo`.** Full nucleo brings the streaming injector + multithreaded incremental matching Helix uses; matcher-only means our store re-scores on one thread per keystroke. *Default: `nucleo-matcher` + the B1 budget test at 100k items. Escalate to full nucleo only if the test says so — its injector/snapshot model would replace much of B2's store, so the decision gate is cheap to hit early (that's why B1 is first).*

**Q-B2 — panel paint slot.** Per-pane `OverlayProvider`s are handed the whole panes region (`pane_area`) at render time, not their pane's rect — the pipeline comment says overlays "may span panes" (`hume-engine/src/pipeline/mod.rs`). So a picker centered in the *panes region* needs no engine change. What `pane_area` excludes is the chrome bands (tab bar, drawer, statusline rows); only a panel that must paint *over those rows* (true terminal-centered) needs a new top-level overlay slot. *Default: render as a pane-region overlay — zero engine change; add a top-level slot only if overlapping the chrome bands turns out to matter (it would also serve future command palettes/dialogs).*

**Q-B3 — item shape at the `picker!` boundary.** Pairs `(display . payload)` vs. hashmaps. *Default: pairs — cheaper to build in Steel, and payload opacity is the point; switch to hashmaps only if a third field (e.g. per-item kind/icon) earns it.*

**Q-B4 — preview pane.** Requires rendering a scratch view of an unopened file inside the panel (load, highlight, position) — touches buffer lifecycle and the render pipeline. *Default: defer entirely; design the panel width so a preview split can be added to its right without relayout of the list half.*

**Q-B5 — live-requery sources (live grep).** Query changes re-run the *source*, not just the filter — needs `#:on-query-change` callback + debounce + result-replacement semantics (vs append). *Default: defer; note that B2's store needs a `replace_items` operation anyway for it, which is trivial to add later; nothing in v1 precludes it.*

**Q-B6 — unify completion filtering onto the B1 matcher.** Replace `subsequence_match_pos`/`is_prefix_match` with nucleo scoring for consistency of feel? *Default: not during Part A or B — completion's ranking (`prefix, pos, sortText`) is tuned to LSP conventions and tested; revisit once the picker's feel is validated, as its own small task with side-by-side comparison.*

**Q-B7 — should `show-menu!`'s Insert-mode block extend to the picker?** Menu is blocked in Insert because the completion popup owns that visual slot. The picker is full-modal and could open from Insert. *Default: allow from any mode but close any open completion session on open (one modal owner at a time); reuse `clear_completion` (post-A1 name).*
