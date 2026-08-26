# HUME — Scriptable Completion Sources

Design document for the insert-mode completion menu driven by multiple Steel-registered sources (LSP, buffer words, custom plugins), mixed and prioritized by policy written in Steel.

The sibling fuzzy-finder (picker) shipped as `core:pickers` (roadmap for what's left: `docs/FUZZY-FINDERS.md`). The two share the "Rust store, Steel policy" architectural pattern and, as of Q-B6, the same `hume-editor/src/editor/fuzzy.rs` matcher (each with its own `FuzzyProfile`); see `hume-editor/src/editor/picker.rs`'s module doc for why they stay separate session types (item shape, query origin, accept semantics, lifetime, scale, and scroll model all differ).

**Status: A1 (the widget/render-layer rename pass) has landed; A2–A4 (multi-source merge, source plugin, buffer-words) are design only — not scheduled.** This document is the single place to resume from; it assumes the reader has *no* memory of the exploration that produced it.

**Headline conclusions** (the "do we need to lay foundations now?" answer):

1. **No foundation work is required now.** This design is additive. The completion architecture is already source-agnostic in the ways that matter. Nothing currently being built needs to change shape to keep it possible.
2. The one guardrail while other work proceeds: **don't deepen LSP coupling in the completion store**. `CompletionSession` today parses generic completion-item JSON and only touches LSP specifics inside the `text_edit` branch of `accept`. Keep it that way — new LSP-specific fields belong in the Steel plugin (which already receives the raw item), not in new Rust parsing.
3. Completion shares its "Rust store, Steel policy" split with the shipped picker (`core:pickers`), and its fuzzy matcher too (`hume-editor/src/editor/fuzzy.rs`, Q-B6) — but not a data structure: `CompletionSession` and `PickerSession` are siblings, not the same type. See `hume-editor/src/editor/picker.rs`'s module doc for the rationale.

## How to use this document

Same rules as `docs/LSP.md`:

1. **Verify before you write.** The codebase moves — `rg 'symbol_name'` before relying on anything named here. No line numbers anywhere in this doc — navigate by symbol search.
2. **If the doc contradicts the code, STOP** and report; don't silently adapt.
3. Open questions each carry a `Default:`. At implementation time, adopt the default unless evidence gathered during the task contradicts it — then record the decision in the Decisions table here.
4. Project-wide rules from `CLAUDE.md` apply (no `.unwrap()` outside tests, grapheme discipline, every command tested).

## Architecture constraints inherited from LSP work

These are settled project decisions (see `docs/LSP.md` Decisions table) and this design must respect them:

- **Frequency cut**: per-user-intent work (a trigger keypress, a response arriving, a selection made) may run in Steel; per-keystroke filtering, per-frame rendering, and unbounded-collection work must be Rust.
- **Bulk-data guardrail**: bulk item lists never cross the Rust↔Steel boundary on recurring paths. One-time ingest at user-intent frequency is the calibrated exception (measured: ~1ms for 1k completion items through the boundary — acceptable; do not assume this scales to 100k file paths).
- **Steel never on the render path**: Steel writes models/stores; Rust providers render from `Arc<RwLock<…>>` snapshots each frame.
- **Rust-rendered, Steel-fed widgets**: "LSP is their first client, not their owner."

---

## Current state — verified inventory

Everything below was read from source, not recalled. This is the substrate this design builds on.

### Insert-mode completion stack (the thing this design extends)

**Rust store — `hume-editor/src/editor/lsp/completion/` (`mod.rs`, `item/mod.rs`, `accept.rs`):**

- `StoredCompletionItem` — **typed via `lsp_types::CompletionItem`**, with a lenient JSON-field fallback (`from_json_lenient`) for items that fail strict deserialize (a real-world server population: spec drift concentrates in completion items and `$/progress`). Fields: `label: String`, `kind: Option<i64>` (raw LSP kind number read straight from JSON — display-only, no reader maps it to a name), `detail: Option<String>`, `sort_text`/`filter_text`/`insert_text` (each falling back to `label` when absent), `text_edit: Option<lsp_types::TextEdit>`, `additional_text_edits: Vec<lsp_types::TextEdit>` + `has_additional_text_edits: bool` (distinguishes "server sent no key at all" from "server sent an empty array" — only the former means `completionItem/resolve` might have more to offer), and `raw: serde_json::Value` — the **full unparsed item**, handed back to Steel on accept so Rust never grows readers for LSP fields it doesn't need. Snippet syntax (`insertTextFormat: Snippet`) is stripped from `insert_text`/`text_edit` at store ingress (`strip_snippet`); `raw` keeps the pristine text.
- `CompletionSession` — **singleton, one per editor** (field `LspState.completion: Option<CompletionSession>`, on the LSP-subsystem state so it dies with `:lsp-stop`), replaced wholesale by each `begin`. Tracks `bid`, `pane_id` (accept only proceeds while this pane is still focused), `anchor_at_begin` + `rope_at_begin` (the coordinate system the server's `textEdit` range was computed against) + `cs_since_begin: ChangeSet` (every edit observed since `begin`, composed — the position-mapping transform from that frozen snapshot to the live document), `items`, `filtered: Vec<u32>` (ranked indices), `rank_scratch` (reused per-keystroke, no allocation), `filter: String`, `matcher: FuzzyMatcher` (own instance, `FuzzyProfile::Autocomplete`), `incomplete: bool` (server's `isIncomplete`).
- Methods: `begin(state, bid, items_json, incomplete)`, `update_filter(state, text)`, `top(n)` (returns `{label, kind, detail}` JSON — `to_json` exposes only these three fields), `accept(state, lsp, idx)`.
- Filtering: `hume-editor/src/editor/fuzzy.rs`'s `FuzzyMatcher` (`nucleo-matcher` wrapper, shared with the picker — see `FuzzyProfile`); rank key is `(score desc, sort_text asc, index asc)`.
- **Accept path** (`accept.rs`) — the only LSP-coupled logic: applies the item's `text_edit` at every cursor via `replace_around_cursors` when present (safe everywhere, per the LSP containment guarantee on the server's own range); otherwise computes each cursor's own preceding-token span (`replace_span_around_cursors` / `word_start_before`) and synthesizes an edit from `insert_text` — no such containment guarantee exists for a synthesized range, so it can't reuse one uniform span across cursors. Rust applies the main edit, any `additional_text_edits`, and (when the item lacks `additionalTextEdits` entirely and the server advertises `resolveProvider`) a synchronous `completionItem/resolve` round trip, all atomically as one undo step. After it lands, queues `EditorEvent::OnCompletionAccept` with `(bid, raw-item)` — a plain extension point for anything the completion store doesn't itself parse (e.g. `command`).
- `CompletionMenuUi { selected }` — UI selection kept as a separate `LspState.completion_ui` field so session logic stays render-free.
- `clear_completion_menu(state, lsp)` — free function, not a method (called from `EditorHostImpl`, `set_mode` on any Insert exit, and `picker::open_picker`); clears session + UI + the shared `completion_menu_view` Arc.

**Steel-facing surface** (this is what makes it already-mostly-scriptable):

- Builtins in `hume-scripting/src/builtins/completion.rs`, registered in `builtins/mod.rs`, host-trait methods in `hume-scripting/src/host.rs`, implementations in `hume-editor/src/editor/host_impl.rs`:
  - `(completion-begin! bid items #:incomplete f)` — `items` is a list of completion-item hashmaps (LSP `CompletionItem` JSON shape). Replaces any open session.
  - `(completion-update-filter! text)`, `(completion-top n)`, `(completion-accept! idx)` (idx into the *ranked* order), `(completion-dismiss!)`.
  - `(register-trigger-chars! source language chars)` — writes `EditorState.trigger_chars: FxHashMap<(String, String), Vec<char>>`, keyed `(source, language)` so a second language attaching under the same source never clobbers the first's chars. **Already multi-source by design.**
- Hooks (`hume-scripting/src/hooks.rs`): `OnTriggerChar` `(bid ch source)` — fired from Insert mode after a registered char lands, once per source registered for that char under `bid`'s language; `OnCompletionAccept` `(bid raw-item)`; `OnCompletionRefilter` `(bid filter-text)` — fired per keystroke **only while `incomplete` is set**.

**The LSP feature plugin — `runtime/plugins/core/lsp/completion.scm`** (the model for what any source looks like):

- `lsp/request-and-begin-completions`: `lsp-request "textDocument/completion"` → decode (`CompletionItem[]` or `CompletionList`) → `completion-begin!`. Snippet stripping happens Rust-side at store ingress — items arriving here already have plain `insertText`/`textEdit.newText`.
- Entry points: `(define-command! "lsp-completion-trigger" …)` (Ctrl+Space is bound to that command name) and the `on-trigger-char` hook filtered by the server's registered trigger characters (populated on `on-lsp-attach` from `completionProvider.triggerCharacters`, cleared on detach).
- No `on-completion-accept` handler here, deliberately: Rust applies the main edit, `additionalTextEdits`, and `completionItem/resolve` atomically.
- `on-completion-refilter` handler: re-requests (isIncomplete flow).

**Insert-mode key handling — `hume-editor/src/editor/mappings/insert.rs`:**

- `handle_completion_key` — pre-guard while a session is open: Tab/Down next, BackTab/Up prev, Enter accept, Esc dismiss; Backspace dismisses only when it would cross the anchor, otherwise falls through; printable chars always fall through.
- `move_completion_selection` — clamped to the **displayed window** (`top(8)`, no scrolling past it).
- `accept_completion_selection` — same gen-checked path as `completion-accept!`; session ends on success or failure.
- `refilter_lsp_completion_after_edit` — after the edit lands, re-ranks against the buffer slice `anchor..head`, fires `OnCompletionRefilter` only if `incomplete`.

**Rendering:**

- `sync_completion_menu_view` (`hume-editor/src/editor/overlay_sync.rs`, runs in `prepare_frame`): `session.top(8)` → `StoredCompletionItem::menu_row_label` (`"label  detail"`, uniform style — per-part dimming would need segment-styled rows, which nothing requires yet) → `resolve_popup_geometry` → writes a `PopupState` into `EditorState.completion_menu_view: Arc<RwLock<Option<PopupState>>>`.
- Painted by the **generic** `PopupOverlay` (`hume-editor/src/ui/popup.rs`) — registered in `build_pane` (`hume-editor/src/ui/mod.rs`) as a third instance with its own `Arc`, scopes `ui.menu` / `ui.menu.selected` (same theme scopes as the selection menu). `PopupState { lines, x, y, selected }`; geometry (below-right preferred, flip above, clamp, max width `min(60, pane_width - 4)`, max height ⅓ pane) resolved once per frame on the write side.

### What is genuinely LSP-coupled vs. already generic

| Piece | Verdict |
|---|---|
| Item schema (`StoredCompletionItem::from_json`) | Generic — it's "LSP `CompletionItem` JSON shape as lingua franca"; any source can emit `{label, insertText, kind, detail, sortText, filterText}` hashmaps |
| Session store, filter, rank, top-N | Fully generic |
| `accept` with `text_edit` present | LSP wire positions — but isolated to one branch |
| `accept` fallback (no `text_edit`) | Generic; works with no server attached (UTF-16 default round-trips) |
| Trigger chars | Generic, already multi-source (`register-trigger-chars!` keyed `(source, language)`) |
| Trigger *ownership* (`lsp-completion-trigger` command, `on-trigger-char` subscription) | Lives in `core:lsp` plugin — needs relocation (task A3) |
| `on-completion-accept` post-processing | LSP-specific by content, but it's Steel — each source brings its own handler |
| Snippet stripping | Rust-side, at store ingress (`strip_snippet`) — a second source with its own snippet dialect would need its own stripping before handing items to the store |

### Adjacent infrastructure this design leans on

- **Async Rust→Steel callbacks**: `lsp-request` queues an `Effect::LspRequest(PendingLspRequest)`; `apply_script_effects` (`scripting_setup.rs`) applies queued effects in emission order, and `send_one_lsp_request` (`hume-editor/src/editor/lsp/bridge.rs`) registers a boxed callback keyed `(ServerId, RequestId)`; reader threads → mpsc → `drain_lsp` each frame → `dispatch_completed` → `Editor::queue_steel_call(callback, args)`. Staleness: response dropped if the buffer's `text_gen` moved, unless `#:allow-stale`. **This is the template for any "async work finishes → call Steel closure" need.**
- **Timers**: `(after ms thunk)` / `(cancel-timer! id)` builtins; `(debounce ms proc)` is pure Scheme over them (`builtins/bootstrap.scm`).
- **Generic widgets** (all in `host_impl.rs` + `ui/popup.rs` + `ui/drawer.rs`):
  - `(show-popup! text #:anchor 'cursor)` / `(close-popup!)` — `PopupModel`, hover-style text panel.
  - `(show-menu! items on-select)` / `(close-menu!)` — `MenuModel { items, selected, callback }`; callback fires exactly once (selection or dismissal); **blocked in Insert mode** (`show_menu` returns Err — deliberate, the completion menu owns that slot). Keys intercepted by `handle_menu_key` (`mappings/mod.rs`) ahead of keymap dispatch.
  - `(show-drawer-list! items on-select)` / `(close-drawer!)` — `DrawerModel { items, selected, scroll, callback }`, bottom chrome band, stays open across Enter (callback may fire repeatedly, `#f` on close), `handle_drawer_key`. Rows are pre-formatted display strings; "Rust never interprets row content."
  - `(prompt! label on-confirm #:prefill text)` — takes over the minibuffer (`MiniBuffer { prompt, input, cursor }` + `steel_prompt_callback`, one at a time), Mode::Command; confirm fires once with text or `#f`.
- **Buffer/introspection builtins available to sources**: `buffers`, `buffer-name`, `buffer-path`, `buffer-language`, `current-buffer`, `current-selections`, `symbol-under-cursor`, `diagnostics-for-buffer`, `lsp-capabilities`, … (full registry: `register_fn_with_ctx` calls in `hume-scripting/src/builtins/mod.rs`).

### The minibuffer completion system is a separate thing — leave it alone

`hume-editor/src/editor/completion/` (`Completer` trait: pure `complete(input, cursor, ctx) -> CompletionResult`; implementors `CommandCompleter`, `BufferNameCompleter`, `ThemeCompleter`, `PathCompleter`, `SetCompleter`; dispatched by a hardcoded match in `complete_minibuf`, `mappings/command_mode.rs`; prefix matching only; rendered by the bespoke statusline-anchored `CompletionOverlay` in `ui/completion_overlay.rs`). It shares no types with the insert-mode stack, and `ROADMAP.md` already records its own future direction ("Steel builtin to register custom completers … core does prefix matching only"). Neither this design nor the picker (`core:pickers`) builds on it, and neither changes it.

### Gaps (what does not exist today)

1. **No multi-source merge** — `completion-begin!` replaces; a slow source's arrival clobbers a fast source's session (or vice versa). No source tags, no priorities, no dedup.
2. **No way to read buffer text from Steel** (by design — bulk guardrail). A buffer-words completion source therefore needs a bounded Rust builtin (task A4), not a Steel scan.

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

One source class this contract can't serve yet: an **external-command-backed source** (dictionary/spell via `aspell`, a snippets CLI, shell history). `spawn-async!` now exists (`(spawn-async! cmd args cwd callback)`), but it's a **one-shot** builtin — `callback` fires exactly once with the command's complete output, not per-line batches. Its one built client (the git-modified picker) only ever needs the whole output at once — the bundled `core:git-diff` plugin's `git show` consumer has the same shape — so that's what got built. An external-command completion source wants the opposite shape — incremental line batches feeding `completion-add-items!` as they arrive, the same way the file picker's `picker-source-spawn!` streams today — which `spawn-async!` doesn't provide and isn't a drop-in fit for. That streaming variant is still deferred until a source that needs it is actually built.

**Incremental arrival — the one real Rust change.** Sources finish at different times (buffer-words: instant; LSP: 10–300ms). Two models considered:

- *Single-shot*: coordinator waits for all sources (with an `(after …)` timeout), concatenates, calls `completion-begin!` once. Works with zero Rust changes, but the menu's appearance is gated on the slowest source or a timeout constant — exactly the UX modern editors moved away from.
- *Incremental* (**chosen**): first `emit` calls `completion-begin!`; later `emit`s call a new `(completion-add-items! token items #:source name #:priority n #:incomplete flag)` that merges into the open session and re-ranks. Menu appears instantly with cheap sources, LSP items merge in when ready.

Rust work for incremental:

1. `completion-begin!` grows the same `#:source`/`#:priority` keywords (the first-arriving source is a tagged contributor like any other; it already has `#:incomplete`) and returns an opaque **session token** (monotonic `u64` on `EditorState`, bumped per begin). `completion-add-items!` takes the token and is a silent no-op if it doesn't match the current session — this kills the whole class of late-async-callback races (user dismissed and retriggered; source from the *previous* trigger finally answers). The existing edit-position-mapping guard (`rope_at_begin`/`cs_since_begin`) is orthogonal (it protects the *edit*, not session identity) and stays as-is.
2. **Merge is replace-per-source, not append**: an add first evicts any items already tagged with that source name, then inserts the new list. Same-source re-emission (the isIncomplete refilter flow below re-invokes a source on the *same* session) is therefore idempotent — no duplicates — while other sources' items are untouched.
3. `StoredCompletionItem` gains `source: Box<str>` (or an interned id) — used for the eviction in (2), for a rank tiebreaker (source priority, passed once at begin/add time), and available to `menu_row_label` for display. `to_json` grows a `"source"` field.
4. `update_filter`'s rank key becomes `(prefix_match, match_pos, source_priority, sort_text)`. Exact position of `source_priority` in the key: see Q-A3.
5. Merge must **preserve the user's current selection** if possible (re-rank moves rows under the cursor — v1: reset selection to 0 on merge, matching what `refilter_lsp_completion_after_edit` already does by clearing `completion_ui`; smarter selection-tracking is a polish item).

**Accept stays per-source via the existing hook.** `on-completion-accept` receives the raw item, which now carries `"source"` — the `core:lsp` plugin's handler guards on `(equal? (hash-ref item "source") "lsp")` before doing `additionalTextEdits`/resolve. Other sources register their own handlers or none. No Rust change.

**isIncomplete becomes per-source**: every `begin`/`add` carries `#:incomplete` for its source, and the session-level `incomplete` flag (which gates the `OnCompletionRefilter` hook fire) is recomputed as the OR across each source's *latest* flag — a slow source arriving incomplete via `completion-add-items!` must be able to flip a session that began complete. The coordinator tracks *which* sources were incomplete and re-invokes only those on refilter; their fresh results flow through the same `completion-add-items!`, where replace-per-source semantics (Rust work item 2) prevent duplication.

**Buffer-words needs one bounded builtin** (`(buffer-words bid prefix max-n)`): Rust scans the buffer with the existing word segmentation (`hume-editing/src/word.rs`), returns ≤ max-n distinct words matching prefix (case-insensitive subsequence or prefix — see Q-A5). Bounded output at user-intent frequency = guardrail-compliant. Steel wraps it into a source in ~10 lines.

**Store/module relocation belongs to A2, not a standalone rename.** The session stays on `LspState.completion`/`LspState.completion_ui`, and the module stays at `editor/lsp/completion/` — because the session deliberately lives on `LspState` so it dies with the LSP subsystem (`:lsp-stop` clears it via `lsp_stop_one`), and the store still parses `lsp_types::CompletionItem` directly. Relocating store + module onto `EditorState`/`editor/completion_session.rs` is real estate that only earns its keep once a second source exists, so it's **A2**'s job (session token + multi-source merge), not a rename done ahead of it. `CompletionSession`/`StoredCompletionItem` are already generic names.

**A2's rename surface also covers the dismiss-pending flag**: `EditorState.lsp_completion_dismiss_pending` and `Editor::take_pending_lsp_completion_dismiss` (`editor/mod.rs`). This is the deferred-dismiss flag `set_mode` sets on any Insert-mode exit — it can't clear the session directly because `set_mode` only has `&mut EditorState` and the session lives on the sibling `LspState`, so it defers to a flag consumed (with a full `&mut Editor` borrow) at the next `Editor::settle()` chokepoint. It's honestly LSP-scoped *today* for the same reason `LspState.completion` is (single source, single owner) — but once A2 moves the session off `LspState` and makes it source-agnostic, this flag stops being an LSP concept and should become `completion_dismiss_pending` / `take_pending_completion_dismiss` alongside that move.

### Task breakdown

| ID | Task | Depends | Size |
|----|------|---------|------|
| A1 | **DONE** — rename pass: de-LSP the widget/render-layer session/store/view names (see the inventory above). Store/module relocation deferred into A2 (see above). | — | S (mechanical, wide) |
| A2 | Session token + `completion-add-items!` (replace-per-source merge) + `source` tag + per-source `#:incomplete` (session flag = OR of latest per-source flags) + priority tiebreaker + per-source rank/display plumbing. Rust: `completion_session.rs`, host trait + `host_impl.rs`, builtin + bootstrap wrapper in `hume-scripting`. Also carries the store/module relocation off `LspState`/`editor/lsp/completion/` (deferred from A1, see above) and the `lsp_completion_dismiss_pending`/`take_pending_lsp_completion_dismiss` rename (see above). Tests: token mismatch no-op, merge re-rank, same-source re-add replaces (no duplicates), late add flips `incomplete`, selection reset, source tiebreak. | A1 | M |
| A3 | `core:completion` plugin: source registry, coordinator (begin/add orchestration, per-source incomplete tracking, trigger-char union, refilter fan-out), move `lsp-completion-trigger` + `on-trigger-char` + `on-completion-refilter` out of `core:lsp`; `core:lsp` re-shapes into a registered source (its `on-completion-accept` handler gains the source guard). Tests: two mock sources (fast sync + slow `after`-delayed), late-arrival merge, stale-token drop, accept-hook source filtering. | A2 | M |
| A4 | `buffer-words` builtin + the buffer-words source plugin (`core:buffer-words` or part of `core:completion` — Q-A6). Tests: dedup, bound, prefix vs subsequence per the Q-A5 decision, no-panic on huge buffer. | A3 | S–M |

No architectural risk; every remaining piece lands behind existing seams.

---

## What to do *now* (foundation checklist)

1. **Nothing structural.** Verified: no current abstraction blocks this design; no in-flight LSP work needs redirecting.
2. **Hold the line on store purity**: any new completion feature that wants Rust to parse another LSP-specific `CompletionItem` field should instead read it in Steel from the `raw` item (accept hook) — that's the existing design intent, keep honoring it.
3. ROADMAP points here at the "Scriptable insert-mode completion sources" line (`docs/ROADMAP.md`). Nothing left to groom.

## Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Foundation timing | **Nothing now; additive later** | Completion store already source-agnostic. Verified against source. |
| Item schema for completion sources | **LSP `CompletionItem` JSON shape as lingua franca** | Store already parses it with label-fallbacks making minimal items trivial; non-LSP sources omit `textEdit` and ride the generic anchor-span accept path (works serverless — UTF-16 default round-trips). |
| Multi-source merge model | **Incremental: `completion-begin!` returns token; `completion-add-items!` merges with replace-per-source semantics; both carry `#:source`/`#:priority`/`#:incomplete`** | Menu appears at fastest-source speed; token makes late async arrivals from stale triggers harmless; replace-per-source makes isIncomplete re-requests idempotent (no duplicates). Single-shot rejected: gates UX on slowest source/timeout. |
| Source registry location | **Pure Steel, in `core:completion`** | Registration and orchestration are user-intent frequency; no Rust registry earns its keep. Mirrors the `trigger_chars` precedent (Rust holds only the union it needs for the Insert-mode fire check). |
| Where mixing policy lives | **Steel (priority, caps, accept handlers); Rust (per-keystroke rank incl. priority tiebreak)** | Frequency cut. Priority crosses once at begin/add; ranking uses it per keystroke without re-crossing. |
| Completion vs picker core | **Siblings sharing the matcher, not a shared session type — see `hume-editor/src/editor/picker.rs`** | Six load-bearing axes differ (item shape, query origin, accept, lifetime, scale, scroll). Abstraction with two divergent call sites is premature; merging later is cheap if bodies converge. |

## Open questions

Each carries a default per the usage rules.

**Q-A1 — dedup across completion sources.** Buffer-words will echo identifiers LSP also returns. Dedup by what key — `label`? `(label, insertText)`? And who wins — higher priority source? *Default: no dedup in v1; priority ordering puts the richer (LSP) item first and the duplicate a few rows down. Revisit with real usage; if added, dedup belongs in Rust at `completion-add-items!` time (per-merge, not per-keystroke) keyed on `insert_text`, keeping the higher-priority item.*

**Q-A2 — token plumbing shape.** Return token from `completion-begin!` (builtin return value) vs. a separate `(completion-session-token)` getter. *Default: return it from `completion-begin!` — one fewer builtin, and the coordinator is the only caller anyway.*

**Q-A3 — where source priority sits in the rank key.** Before or after the fuzzy `score`? Before means a low-quality match from a high-priority source beats a perfect match from a low-priority one. *Default: `(score, source_priority, sort_text)` — priority as tiebreaker only; match quality stays king. Revisit if LSP items feel buried.*

**Q-A4 — per-source item caps.** Should the coordinator cap each source's contribution (e.g. buffer-words ≤ 50) in Steel, or should Rust enforce a per-add cap? *Default: Steel-side cap in the coordinator (policy), with `completion-add-items!` accepting whatever it's given; Rust store has no per-source limits.*

**Q-A5 — `buffer-words` matching semantics.** Prefix-only (cheap, classic vim `i_CTRL-N` feel) vs. subsequence (consistent with the session's own filter)? *Default: prefix at collection time — trivially cheap scan, vim-precedented feel. Honest cost: prefix collection is NOT a superset of what the session's subsequence filter can match (`flag_option` matches subsequence `fo` but not prefix `fo`), so subsequence-only candidates never reach the store; accepted for v1.*

**Q-A6 — buffer-words packaging.** Own plugin (`core:buffer-words`, lazy-loadable, deletable) vs. bundled into `core:completion`. *Default: own plugin — it's the reference example of a third-party-shaped source, and dogfooding the registration API from a *separate* plugin proves cross-plugin registration works.*

**Q-A7 — kind display.** `kind: i64` is currently display-unused (`menu_row_label` shows `label  detail` only). Map kind→short label/icon in Rust (`menu_row_label`) with a static table, themable? Non-LSP sources reuse LSP kind numbers? *Default: static Rust map (LSP kind numbers as the universal enum — sources pick the closest; 1=Text fits buffer-words), single-char column, no per-kind theming in v1. Note: per-part styling (dimmed detail, colored kind) needs segment-styled popup rows — a `PopupState` extension that's its own small task; don't smuggle it in.*

**Q-B6** (unifying completion's matcher with the picker's) shipped: both route through `hume-editor/src/editor/fuzzy.rs`'s `FuzzyMatcher`, distinguished by `FuzzyProfile`.
