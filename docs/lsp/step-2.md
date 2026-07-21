# LSP Step 2 — Steel platform (task cards)

The bridge and primitives that make Steel the feature layer. After this step a plugin can reach any LSP method and any editor surface. Read `docs/LSP.md` (hub) first — the *Steel API index* is the contract these cards implement; if a card and the index disagree, that's a doc bug: stop and report.

Shared context for all B-cards:
- Builtins register in `hume-scripting/src/builtins/mod.rs` via `steel.register_fn_with_ctx(HUME_CTX, "name", module::fn)`; new LSP builtins go in `hume-scripting/src/builtins/lsp.rs` (started by C8), UI-ish ones can share it until Step 3 adds widgets.
- Builtins that need editor state go through `SteelCtx<'a>` (`hume-scripting/src/context.rs`) — borrow-based, no TLS (see LESSONS). Keyword-arg surfaces are Scheme wrappers in the bootstrap string calling positional `%primitives` (the `declare-plugin` pattern).
- Steel error discipline: `steel::stop!` with a clear message, never `panic!` (LESSONS).
- Steel-visible tests follow tier 3 (hub: testing playbook): `SteelCtxTestHarness` for pure builtin logic, full editor + `InlineLspBackend` for flows.

---

### B1 — JSON↔SteelVal codec

**Goal** — total, documented conversion both ways: `serde_json::Value ↔ SteelVal`. Foundation for B2 params/results, C8's option blobs, everything.

**Depends** — C1 (serde_json in tree; the code itself lives editor-side of the fence). **Unlocks** — B2, C8 upgrade (init-options as real data).

**Files** — `hume-scripting/src/json.rs` (new; hume-scripting already sees steel types; add `serde_json.workspace = true` to its Cargo.toml).

**Read first** — LESSONS entries on Steel list construction (`Vec::<SteelVal>::new().into_steelval()`) and `equal?` on opaque types; `steel::rvals::SteelVal` variants (`NumV`, `IntV`, `StringV`, `BoolV`, `Void`, `ListV`, `HashMapV`).

**Shape**
```rust
/// json -> steel:  null -> void        (NOT #f — false round-trips!)
///                 bool -> BoolV
///                 number -> IntV when i64-representable, else NumV (f64)
///                 string -> StringV
///                 array -> ListV
///                 object -> HashMapV with STRING keys (not symbols —
///                           JSON keys are arbitrary strings like "rust-analyzer.cargo")
pub fn json_to_steel(v: &serde_json::Value) -> SteelVal;

/// steel -> json: inverse mapping; void/'() -> null; symbols -> strings;
/// IntV/NumV -> number; unrepresentable inputs (closures, opaque customs)
/// -> Err with the offending type name (fail fast, no silent null).
pub fn steel_to_json(v: &SteelVal) -> Result<serde_json::Value, String>;
```

**Tests** — tier 1 (harness): round-trip property `steel_to_json(json_to_steel(v)) == v` over a fixture set: nested objects/arrays, null vs false vs 0 vs "" (all distinct after round-trip), i64 max, floats, unicode strings, empty object/array. Reverse-direction: symbol keys accepted, closure → Err naming the type.

**Done when** — mapping table in the module docs matches the code and this card; P8's crude spike conversion deleted in favor of nothing (spike is already gone) — this is the only conversion in the tree.

**Traps**
- `null → #f` is the classic bug (JSON `false` becomes indistinguishable) — hub decided void; the round-trip test enforces it.
- JSON object keys with dots/spaces are normal (`"rust-analyzer.checkOnSave"`) — string keys, not symbols.
- Don't special-case LSP shapes here — this is generic JSON, B2 keeps it that way.

**Size** — ~110 source + ~130 test lines.

---

### B2 — Generic LSP bridge

**Goal** — any protocol method reachable from Steel: `(lsp-request server method params callback)`, `(lsp-notify …)`, `(on-lsp-notification method handler)`. The dogfooding surface every F-card and every community plugin uses.

**Depends** — B1, C6. **Unlocks** — every F-card.

**Files** — `hume-scripting/src/builtins/lsp.rs`, editor glue in `hume-editor/src/editor/lsp/mod.rs` (Steel-callback arm of the C6 dispatch table, notification fan-out).

**Read first** — C6's `CallbackToken` design (`hume-lsp/src/client.rs` + editor callback table); how hooks store and invoke Steel handlers (`hume-scripting/src/hooks.rs` registry + the eval inside `drain_hooks`, `editor/scripting_setup.rs`) — callback invocation reuses that delivery discipline (queued, evaluated at the drain boundary, never re-entrant).

**Shape**
```scheme
;; callback signature: (lambda (err result) …) — exactly one is non-#f.
;; err is a hashmap {"code" "message"} for protocol errors, or the strings
;; "timeout" / "server-crashed".
(lsp-request server method params callback)            ; server = name or #f (focused buffer's)
(lsp-request server method params callback #:allow-stale #t)
(lsp-notify server method params)
(on-lsp-notification method handler)                   ; handler: (lambda (server params) …)
```
Rust side: `lsp-request` = `steel_to_json(params)` → C6 `send_request` with a fresh `CallbackToken`; the editor's token table stores the Steel closure. At drain, completed entries with a Steel token enqueue `(closure err result)` through the same queued-delivery path hooks use (never eval inside the drain loop's borrow). Auto gen-tagging: if `params` contains a `textDocument.uri` matching an open buffer, tag with its `text_gen` (C6 staleness); `#:allow-stale` opts out of the drop. Notification handlers: `HashMap<String, Vec<SteelVal>>` on the editor; C6's "everything else" arm fans out here (still logs Trace when no handler).

**Tests** — tier 3 (editor + double): request → scripted response → callback receives decoded result; protocol error → `err` hashmap; timeout → `err = "timeout"`; stale drop (edit between send and drain) → callback never runs; `#:allow-stale` → runs; `on-lsp-notification` receives a pushed notification; callback that itself calls `lsp-request` doesn't re-enter (delivery is queued); errors raised *inside* a callback land in the message log, not a crash.

**Done when** — Step 2 milestone hover round-trip works from `init.scm` against rust-analyzer (manual), and the same flow is a green tier-3 test against the double.

**Traps**
- Callbacks are one-shot: remove the token entry before eval (a second response for the same id is a protocol violation — log, don't call twice).
- `server = #f` resolution (focused buffer) must happen at call time, not drain time.
- Never hold the callback-table borrow across the Steel eval — queue, then eval (the hook path shows the discipline; see also the LESSONS NLL patterns).

**Size** — ~180 source + ~200 test lines.

---

### B3 — Introspection builtins

**Goal** — Steel can see what servers can do and where buffers stand — features must capability-check before firing (hub primer).

**Depends** — C5, B1. **Unlocks** — F-cards' capability guards.

**Files** — `hume-scripting/src/builtins/lsp.rs`.

**Shape**
```scheme
(lsp-capabilities server)      ; decoded ServerCapabilities hashmap (B1), or #f if not Running
(lsp-server-status)            ; list of hashmaps: {"language" "root" "state" "pending"}
(lsp-server-for-buffer bid)    ; server name (language) or #f
(buffer-generation bid)        ; int — Steel-side staleness checks
(lsp-position-params bid)      ; ready-made TextDocumentPositionParams hashmap:
                               ;   {"textDocument" {"uri" …} "position" {"line" … "character" …}}
                               ;   built from the primary cursor head, converted to the owning
                               ;   server's negotiated encoding (P4) — Steel never does wire math
(lsp-range-params bid)         ; same but {"textDocument" … "range" …} from the primary selection
```
Capabilities decode once at C5 handshake time (store the decoded SteelVal alongside the typed caps — conversion is per-server-startup, not per-call). The two params helpers exist because every F-card needs them and position-encoding conversion must stay Rust-side; without them each feature would hand-build positions and get the encoding wrong.

**Tests** — tier 3: caps reflect the double's handshake fixture; `#f` before Running and after crash; generation increments across an edit.

**Done when** — all four callable from `init.scm`; `(lsp-capabilities …)` output for rust-analyzer eyeballed once (manual).

**Traps** — `buffer-generation` takes the opaque bid value (`SteelBufferId`); prefer comparing the int it returns over comparing bids directly for staleness checks (`equal?`/hash-keying on bids themselves work by value, but a generation counter is the more direct staleness signal here).

**Size** — ~80 source + ~80 test lines.

---

### B4 — Steel timers

**Goal** — `(after ms thunk)` / `(cancel-timer! id)` builtins on P7's wheel, plus a `debounce` wrapper — the throttling primitive F7/F10 and B7's viewport hook depend on.

**Depends** — P7. **Unlocks** — B7 (debounced hooks), F7, F10.

**Files** — `hume-scripting/src/builtins/lsp.rs` (or a `timers.rs` sibling — builtin modules are by domain; timers aren't LSP-specific, so `builtins/timers.rs` is cleaner), bootstrap string for `debounce`, editor glue: `TimerId → thunk` table + fire step in the P3 drain phase.

**Read first** — P7's `TimerWheel` API; the queued hook-delivery path (same reason as B2: thunks fire at the drain boundary).

**Shape**
```scheme
(after ms thunk)        ; -> timer id (int)
(cancel-timer! id)      ; ok if already fired/cancelled (idempotent)
;; bootstrap wrapper:
(define (debounce ms proc)
  ;; returns a proc that (re)schedules `proc` ms in the future,
  ;; cancelling the previous pending call — trailing-edge semantics.
  …)
```
Editor side: `timer_thunks: HashMap<TimerId, SteelVal>`; the P3 drain phase calls `wheel.take_due(now)` and enqueues each thunk through the hook-delivery queue. `after` from inside a timer callback works (schedules onto the live wheel).

**Tests** — tier 3: thunk fires once after deadline (drive the loop, or call the drain directly with a synthetic `now`); cancel before fire → never runs; debounce: three rapid calls → one trailing invocation; a thunk raising an error lands in the message log and doesn't kill the wheel.

**Done when** — `(after 200 (lambda () (log! "tick")))` observably fires in a manual run.

**Traps**
- `debounce` state lives in a closure — pure bootstrap Scheme, zero Rust (don't build a Rust debouncer).
- Thunk errors must not leave the `TimerId → thunk` entry leaked — remove before eval.

**Size** — ~100 source + ~100 test lines (+ ~10 bootstrap lines).

---

### B5 — Decoration stores + setters + diagnostics pull

**Goal** — the Steel-writable stores Step 3 renders from, and the bounded diagnostics pull over C9.

**Depends** — C9, B1, P2. **Unlocks** — U1/U2/U8/U9, F4, F10.

**Files** — `hume-editor/src/editor/lsp/stores.rs` (or `editor/decorations.rs` — they're not LSP-specific; pick the neutral name), `hume-scripting/src/builtins/lsp.rs` (setters + pulls).

**Read first** — the `SharedHighlighter` feeding pattern (`hume-editor/src/ui/highlight_providers.rs` module docs) — stores here are the write side of that split; C9's `for_range`/`counts`; hub OQ default for the pull cap.

**Shape**
```rust
/// One store per decoration kind, all following the same design:
/// keyed by (source: String, BufferId); positions are char offsets,
/// remapped through edits with P2 next to C9's remap (same chokepoint).
pub(crate) struct DecorationStores {
    pub inlay_hints: …,      // Vec<(char_pos, text, before/after)>
    pub signs: …,            // Vec<(line, text, scope_name, priority)>
    pub virtual_lines: …,    // Vec<(line, text)>
    pub extra_highlights: …, // Vec<(start, end, scope_name)>
    pub generation: u64,
}
```
```scheme
(set-inlay-hints! bid hints)          ; hints: list of (position text 'before|'after) where
                                      ;   position is the WIRE {"line" "character"} hashmap from
                                      ;   the inlayHint response — the setter converts to char
                                      ;   offsets at set time (bid's server encoding, P4); the
                                      ;   store keeps char offsets, remapped by P2 thereafter
(set-signs! source bid signs)         ; signs: list of (line text scope priority) — line indices,
(set-virtual-lines! source bid lines) ; lines: list of (line text)      encoding-independent
(set-extra-highlights! source bid spans) ; spans: list of (start end scope) — CHAR offsets
                                      ;   (HUME-side producers; not an LSP response consumer)
(diagnostics-for-buffer bid #:severity floor #:range (start . end))
   ; -> list of hashmaps {"start" "end"        — char offsets (range math)
   ;                      "line" "col"         — char-indexed start, ready for
   ;                                             goto-location! shape 2 (F4)
   ;                      "severity" "message" "code" "source"},
   ;    capped at 1000 after filtering (hub OQ default)
(diagnostic-counts bid)               ; -> (errors . warnings)
```
Setters **replace** the (source, buffer) slice wholesale (swap a Vec — cheap; matches the "write once, render many" pattern); `source` namespacing lets `core:lsp` and other plugins coexist. Scope names are interned lazily at first render (`intern_runtime` — see the theme baking chokepoint in `prepare_frame`).

**Tests** — tier 3: set → readable back through the store (Rust-side assert); replace semantics (second set drops the first batch); positions remap through an edit (inlay hint glued to its char after an insert above); pull respects floor/range/cap; counts match C9.

**Done when** — stores are populated from `init.scm` snippets and visible to Rust (Step 3 renders them; until then, assert via store accessors in tests).

**Traps**
- Setter inputs are per-user-intent sized (visible-range hints, one buffer's signs) — do NOT add incremental/diff APIs (premature; replace-wholesale is the design).
- Remap on edit or decorations drift — same hook point as C9's `remap_through`, one call site for all stores.
- `diagnostics-for-buffer` allocates per call — that's fine (user-intent frequency); don't cache SteelVals in the store (staleness trap).

**Size** — ~200 source + ~180 test lines.

---

### B6 — Edit + navigation primitives

**Goal** — the correctness-critical shared machinery: `apply-text-edits!` (one undoable transaction), `apply-workspace-edit!` (multi-file engine), `goto-location!` (jump-list-correct navigation). One Rust implementation; every feature composes it.

**Depends** — B1, P4, C7 (version checks), P5. **Unlocks** — F2, F4, F5, F8, F9; completes C6's `workspace/applyEdit` arm.

**Files** — `hume-editor/src/editor/lsp/edits.rs`, builtins in `hume-scripting/src/builtins/lsp.rs`.

**Read first** — how an editing command builds and applies a ChangeSet + selection update + undo grouping (trace one op in `hume-editor/src/ops/edit/` to its apply site); jump-list discipline: `current_jump_entry` (`editor/commands/mod.rs`) and the **push-after-commit-point** rule (LESSONS: pushing before a possible abort truncates forward history); `:e`'s buffer-open path (`editor/buffer/file_open.rs`) for opening files by path.

**Shape**
```scheme
(apply-text-edits! bid edits)      ; edits: list of ((start-line . start-col) (end-line . end-col) text)
                                   ; wire positions, converted via P4 at the boundary
(apply-workspace-edit! wsedit)     ; wsedit: the decoded WorkspaceEdit hashmap (B1 output)
(goto-location! loc)               ; loc is ONE of two shapes (Rust detects which):
                                   ;  1. a raw decoded Location/LocationLink hashmap straight
                                   ;     from an LSP response — uri + WIRE range, converted
                                   ;     Rust-side (P5 + P4, encoding of the focused buffer's
                                   ;     server — correct because the caller is that server's
                                   ;     response callback; document this assumption)
                                   ;  2. (list target line col) with CHAR-indexed line/col —
                                   ;     target = path string, file:// URI string, or bid
                                   ; opens the buffer if needed, pushes a jump entry (after
                                   ; the commit point), moves the cursor, centers the view
```
```rust
/// Sort edits descending by start, verify non-overlap, build ONE ChangeSet,
/// apply as ONE undo step. Rejects (Err, no partial application) when:
/// overlapping edits, or expected version mismatch (caller passes the
/// text_gen the edits were computed against — B2's staleness tag).
fn apply_text_edits(ed: &mut Editor, bid: BufferId, edits: Vec<WireEdit>, expect_gen: Option<u64>) -> Result<(), String>;

/// WorkspaceEdit: handle BOTH `changes` (uri -> edits map) and
/// `documentChanges` (array of TextDocumentEdit with versions; honor
/// versions when present). Unopened files: open-as-buffer (OQ default),
/// apply, report "N buffers modified — :wa writes all". Per-file failure
/// aborts the whole edit BEFORE any file is touched (validate all, then
/// apply all — atomicity per hub decision "correctness-critical").
fn apply_workspace_edit(ed: &mut Editor, we: lsp_types::WorkspaceEdit) -> Result<Summary, String>;
```
`goto-location!`: resolve target → existing buffer or open; **jump push after the commit point** (after open succeeds, before cursor move — mirror the existing goto commands' ordering). The two accepted shapes exist because locations come from two worlds: raw LSP responses (wire positions — shape 1 converts them Rust-side with the right encoding, so F2/F6 pass response hashmaps straight through) and HUME-side data like the C9 store (char positions — shape 2, used by F4). Steel never converts between the two.

One more small read builtin lives here (F8's range-format gate; no existing builtin exposes selection extents — verified against the registered-builtin list at spec time):
```scheme
(selection-spans-full-line? bid)   ; #t iff the primary selection covers at least one
                                   ; complete line (start at col 0 or before first char,
                                   ; end at/past line end) — Rust-side line math
```

**Tests** — tier 3, the densest suite in Step 2:
- text-edits: single edit; multiple edits same line (descending application); adjacent-not-overlapping accepted; overlapping rejected; whole-thing is ONE undo step (`u` restores exactly); version mismatch rejected; edits at buffer end (trailing-`\n` invariant preserved).
- workspace-edit: `changes` shape; `documentChanges` shape; mixed open/unopened files (unopened opens dirty); one invalid file → nothing applied anywhere; message-log summary text.
- goto: same buffer (no reopen), other open buffer, unopened path, nonexistent path → error + **no jump-list entry** (the commit-point test), jump-back (`Ctrl+o`-equivalent binding per keymap) returns to origin.

**Done when** — C6's `workspace/applyEdit` arm swapped from the stub to this engine (and its C6 test updated from `applied: false` to a real application).

**Traps**
- Descending-order application is what makes multiple edits against one version composable — ascending with offset-fixups is the classic bug; the same-line-multi-edit test catches it.
- One ChangeSet ⇒ one undo step ⇒ one `lsp_did_change` — if you see N didChanges for one `apply-text-edits!`, the transaction leaked through the chokepoint N times.
- Selections after the edit follow the existing op conventions (cursor lands per HUME edit semantics — mimic what a native edit does, don't invent placement).
- Never `head += 1` -style arithmetic on the结果 selections — grapheme rules apply the moment you touch selection placement (`next_grapheme_boundary`).

**Size** — ~280 source + ~300 test lines. The biggest B-card; budget accordingly.

---

### B7 — New hooks

**Goal** — four events Steel can react to: `on-lsp-attach`, `on-diagnostics-changed`, `on-viewport-change` (debounced), `on-trigger-char` (+ `register-trigger-chars!`).

**Depends** — B4 (debounce), C5/C9 (fire sites). **Unlocks** — F3, F7, F10.

**Files** — `hume-scripting/src/hooks.rs` (`HookId` + `HOOKS` — the SSOT), fire sites in `hume-editor` (see below), `hume-scripting/src/builtins/lsp.rs` (`register-trigger-chars!`).

**Read first** — `hume-scripting/src/hooks.rs` top-to-bottom (the SSOT table pattern + the lazy-activation note in `OnLanguageSet`'s doc); `fire_hook_silent`/`drain_hooks` (`editor/scripting_setup.rs`); where scroll/resize resolve (`editor/commands/scroll.rs` / the resize arm in `lifecycle.rs`); the insert-mode char-insertion path (`editor/mappings/` insert handling).

**Shape**
```rust
// HookId additions (+ HOOKS entries — compiler + from_symbol tests keep them honest):
OnLspAttach,          // args: (bid server-name)         — fires when C5 reaches Running for an attached buffer, and on later attaches
OnDiagnosticsChanged, // args: (bid)                      — once per drain batch that touched bid (C9), payload-free by design
OnViewportChange,     // args: (bid first-line last-line) — fires AFTER scroll/resize resolves; DEBOUNCED (see below)
OnTriggerChar,        // args: (bid char-string)          — Insert mode, typed char ∈ registered set, fires after the char is inserted
```
```scheme
(register-trigger-chars! source chars)   ; chars: list of 1-char strings; union across sources
```
Debounce for `OnViewportChange`: fire sites call a tiny Rust-side coalescer (schedule-or-reset a P7 timer, 150 ms default knob `lsp.viewport-debounce-ms`) rather than firing raw — scroll storms must not queue hundreds of hook evals. (B4's Scheme `debounce` is for user procs; this one guards a built-in fire site, so it's Rust.)

**Tests** — tier 3: attach fires with server name after handshake; diagnostics signal once per batch (two publishes, one drain → one fire); viewport hook fires once after a burst of scrolls (drive the debounce timer); trigger char fires for registered chars only, in Insert mode only, after the char is in the buffer (assert buffer content inside the handler); `HOOKS`/`HookId` round-trip (`from_symbol`) covers the new names.

**Done when** — all four registerable from `init.scm` and firing observably (log lines).

**Traps**
- `OnTriggerChar` must fire **after** the insertion is applied — F7 reads the buffer inside the handler.
- Don't fire `OnViewportChange` from inside `prepare_frame`'s render math — fire from the scroll/resize *commands* (user intent), debounced; the render path stays Steel-free.
- New `HookId` variants: check `activate_lazy_event_plugins` (`#:events` activation) picks them up for free — the SSOT table should make it so; verify, don't assume.

**Size** — ~150 source + ~160 test lines.

---

### B8 — Completion orchestration API

**Goal** — completion items live in a Rust store with a Rust per-keystroke filter; Steel drives the session. The one calibrated bulk crossing (ingest at `completion-begin!`) per the hub guardrail.

**Depends** — B1, B6 (accept applies edits), P8's verdict. **Unlocks** — U7, F3.

**Files** — `hume-editor/src/editor/lsp/completion.rs` (session + store + filter), builtins in `hume-scripting/src/builtins/lsp.rs`.

**Read first** — hub decision *Bulk-data guardrail* (the P8-calibrated exception, and the fallback design if P8 said no); `editor/completion/mod.rs` (`CompletionState` — minibuffer selection-state conventions worth mirroring, not reusing directly).

**Shape**
```rust
pub(crate) struct CompletionSession {
    bid: BufferId,
    anchor: usize,            // char offset where the completed token starts
    items: Vec<StoredCompletionItem>,   // label, kind, detail, sort/filter text, the TextEdit
    filtered: Vec<u32>,       // indices into items, ranked; rebuilt per filter update
    filter: String,
    incomplete: bool,         // server said isIncomplete — F3 re-requests on narrowing
    generation_at_begin: u64,
}
/// Rust filter: case-insensitive subsequence match, rank = (prefix-match,
/// match-position, sort_text) — deliberately simple; the Steel scorer hook
/// (OQ default: re-rank top-64) can refine on top.
fn update_filter(&mut self, text: &str);
```
```scheme
(completion-begin! bid items #:incomplete f)  ; items: list of hashmaps (decoded CompletionItem)
(completion-update-filter! text)              ; per-keystroke — Rust-side work only
(completion-top n)                            ; -> list of hashmaps for display (bounded)
(completion-accept! idx)                      ; applies the item's textEdit via B6 (falls back
                                              ;   to insertText/label at the anchor); ends session
(completion-dismiss!)
```
If P8 ruled the Steel ingest out at 1k items: `lsp-request` grows `#:into-completion-store bid` routing the raw response straight into the session (Steel gets item count, not items) — implement whichever branch P8 recorded in the hub; the Steel API above stays identical either way.

**Tests** — tier 2/3: begin → top returns ranked items; filter narrows and re-ranks (prefix beats infix); accept applies the textEdit exactly (server-provided range wins over the anchor guess) as one undo step; accept with no textEdit inserts insertText at anchor; dismiss clears; buffer edit during session invalidates it (generation check) — accept after invalidation errors instead of applying against the wrong text.

**Done when** — a scripted session (begin with 1k synthetic items → filter → top → accept) runs under the P8-recorded budget in a release-mode timing test (loose bound, e.g. <5 ms — this is the guardrail's regression test).

**Traps**
- The filter runs per keystroke — no allocation churn (retain a scratch Vec, rank in place; see the flatten_overlaps scratch-reuse precedent in engine-opts).
- `textEdit` ranges are wire positions against the version at request time — convert via P4 with the owning server's encoding and reject on version drift (B6's `expect_gen`).
- Session is a singleton (one per editor, not per buffer) — starting a new one dismisses the old.

**Size** — ~250 source + ~220 test lines.

---

### B9 — Steel minibuffer prompt

**Goal** — `(prompt! label #:prefill text on-confirm)`: a one-shot minibuffer prompt whose confirmation calls a Steel callback. No prompt primitive exists today (the minibuffer only knows `:` command and `/`-`?` search) — F5 rename needs this.

**Depends** — B2's callback-delivery plumbing (reuses the queued eval path). **Unlocks** — F5.

**Files** — `hume-editor/src/editor/minibuf/` (a prompt context variant), `editor/mappings/command_mode.rs` (or a sibling — wherever Command-mode keys dispatch), builtin in `hume-scripting/src/builtins/lsp.rs` (name it in `builtins/mod.rs` next to the others; it's not LSP-specific — `ui.rs` module is fine too, Step 3 will add one anyway).

**Read first** — `editor/minibuf/mod.rs` (`MiniBuffer`, `MiniBufferEvent`) and `editor/mappings/command_mode.rs` end-to-end: how `:` enters Command mode, feeds keys, and dispatches on `Confirm`. The typed-command dispatch memory (LESSONS): a `:`-context handler assigns state directly, never calls `switch_focused_pane`.

**Shape**
```rust
/// Why the prompt reuses Command mode's input machinery with a routing tag
/// instead of a new EditorMode: the mode set is Steel-visible via
/// on-mode-change, and a new variant leaks a Rust implementation detail to
/// every mode-keyed keymap/theme. The tag only changes where Confirm routes.
enum MinibufPurpose { Command, Search { … }, SteelPrompt { label: String, callback: SteelVal } }
```
```scheme
(prompt! "New name: " #:prefill "old_name" (lambda (input) …))  ; input = #f on cancel
(symbol-under-cursor bid)   ; word at the primary cursor head as a string, "" when on
                            ; whitespace/punctuation — F5's prefill; implemented in Rust
                            ; over the existing word-boundary helpers (hume-editing
                            ; word.rs) because grapheme/word logic never lives in Scheme
```
Entering: set up the minibuffer with `prompt` display = label (the existing `prompt: char` field needs to grow to a small string — survey the render site in `ui/statusline.rs` `MiniBuf` element for the display change), prefill `input` + cursor at end. `Confirm(text)` → queue `(callback text)`; `Cancel` → queue `(callback #f)`. History: none for v1 (prompts are one-shot; don't wire the `:`-history).

**Tests** — tier 3: prompt → type → Enter → callback gets text; Esc → callback gets `#f` (exactly one call either way); prefill visible and editable; a second `prompt!` while one is open errors (`steel::stop!`) rather than stacking; mode round-trips (Normal → prompt → Normal, `on-mode-change` fires like Command mode does).

**Done when** — `(prompt! "test: " (lambda (s) (log! s)))` works end-to-end manually.

**Traps**
- `prompt: char` → `String` touches the statusline renderer — check the width math for multi-char prompts (grapheme width via the existing unicode-width usage in `minibuf/mod.rs`).
- Callback delivery is queued like every Steel eval (never inside key handling) — B2's discipline.
- Don't grow this into a generic input framework (`#:validate`, `#:complete` …) — F5 needs label + prefill + confirm; stop there.

**Size** — ~130 source + ~120 test lines.

---

### B10 — Platform addendum (Step 4 prerequisite)

**Goal** — four small platform gaps surfaced while verifying the Step 4 (F1–F11) cards against the tree: no Step 4 feature could be written zero-Rust without them. Landed as one Rust pre-task, its own commit, before F1.

**Depends** — B4 (hooks fire through the same queued-delivery path), B7 (extends its `HookId`/`HOOKS` table), B8 (extends `StoredCompletionItem`/`CompletionSession`). **Unlocks** — F3, F7, F8, F10.

**Files** — `hume-scripting/src/builtins/lsp.rs` (register-trigger-chars! gate), `hume-scripting/src/host.rs` + `null_host.rs` (`OptionValue`, `EditorHost::get_option`), `hume-scripting/src/builtins/settings.rs` (`get-option` builtin), `hume-scripting/src/hooks.rs` (`OnCompletionAccept`/`OnCompletionRefilter`), `hume-editor/src/settings.rs` (`setting_value` generic accessor + `lsp.inlay-hints` default), `hume-editor/src/editor/host_impl.rs` + `testing/mock_host.rs` (`get_option` impls), `hume-editor/src/editor/lsp/completion.rs` (raw-item retention, accept fires the hook), `hume-editor/src/editor/mappings/insert.rs` (refilter fires the hook), `hume-engine/src/builtins/line_number.rs` + `pane.rs` (`Display` for `LineNumberStyle`/`WrapMode`, needed by the generic settings-read accessor).

**Shape**
```rust
// (a) register-trigger-chars! (lsp.rs): delete the `!ctx.is_init && ctx.plugin_stack.is_empty()`
// gate entirely — callable from any context, including on-lsp-attach hook handlers.

// (b) get-option (host.rs / settings.rs / builtins/settings.rs):
pub enum OptionValue { Bool(bool), Int(i64), Str(String) }   // host.rs
fn get_option(&self, key: &str, bid: BufferId) -> Result<OptionValue, String>;  // EditorHost
pub fn setting_value(key: &str, settings: &EditorSettings, overrides: Option<&BufferOverrides>)
    -> Option<OptionValue>;   // hume-editor/settings.rs, generated by define_settings! alongside
                              // apply_setting — covers every global/buffer macro key; manual_keys
                              // (whitespace-*, statusline) return None (no reader needed yet)

// (c) completion accept/refilter hooks (hooks.rs + completion.rs + mappings/insert.rs):
OnCompletionAccept,     // args (bid item) — item = accepted item's raw JSON; fires after the
                        // main edit in CompletionSession::accept
OnCompletionRefilter,   // args (bid filter-text) — fires from the Insert-mode refilter path,
                        // ONLY while the session's isIncomplete flag is set
```

**Tests** — tier 1 (`hume-editor`/`hume-engine` unit): `setting_value` type dispatch (bool/usize/from_str keys) + buffer-override-wins + unknown-key `None`; `Display` round-trips for `LineNumberStyle`/`WrapMode`. Tier 2 (`hume-scripting` guard tests via `SteelCtxTestHarness`): `get-option` blocked during init, reaches the host in command mode. Tier 3 (`hume-editor` editor + double, and `hume-editor/tests/scripting.rs` + `MockHost`): `register-trigger-chars!` called from inside an `on-lsp-attach` handler takes effect (oracle: compare against a parallel plain editor, not a bare before/after — typing the trigger char changes state either way); `(get-option "tab-width")`/`"tab-style"`/`"lsp.inlay-hints"` read back correctly through a real `ScriptingHost` + `MockHost`, unknown key errors; `completion-accept!` fires `on-completion-accept` with the raw item after the edit lands; the Insert-mode refilter path fires `on-completion-refilter` only when `isIncomplete`, never for a complete session.

**Done when** — all four sub-items compile and the tests above are green; `cargo test --workspace` green; hub *Steel API index* and *Decisions* updated for the four resolutions; `docs/lsp/step-4.md`'s sketches corrected to match (see hub Decisions rows).

**Traps**
- The generic `setting_value` accessor is macro-generated (same table as `apply_setting`) — every `from_str`-parsed setting type must have a `Display` that round-trips through its own `FromStr`; two types (`LineNumberStyle`, `WrapMode`) didn't and needed one added (mirroring `TabStyle`'s existing pair). A new `from_str` setting without a `Display` impl fails to compile here, not silently misreads.
- `on-completion-refilter`'s incomplete-only gate is load-bearing, not incidental — an unconditional per-keystroke hook would violate the hub's frequency-cut rule (recurring paths must be Rust) even though the payload is cheap.
- `CompletionSession::accept` pushes onto `state.pending_hooks` directly (it only has `&mut EditorState`, not `&mut Editor`) — the same `Vec<(HookId, Vec<SteelVal>)>` the `fire_hook_*` convenience methods on `Editor` push onto; there is no second hook-delivery mechanism.

**Size** — ~120 source + ~180 test lines (across four crates).
