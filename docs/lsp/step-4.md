# LSP Step 4 — Features in Steel, `core:lsp` (task cards)

Each feature is Steel code composing Step 2–3 primitives. **Rust changes in this step must be zero** — a needed Rust change means a missing platform primitive: stop and report (hub rule 4). Read `docs/LSP.md` (hub) first, especially the *Steel API index* and the primer's *v1 method map*.

Shared context for all F-cards:
- Code lives in `runtime/plugins/core/lsp/` — `plugin.scm` entry + one file per feature area via relative `require` (mimic `runtime/plugins/core/plum/`'s multi-file layout). Until F11 packages it, develop features as `init.scm` snippets or directly in the plugin with an eager `(load-plugin "core:lsp")` in your test init.
- Every feature **capability-checks** before firing: `(lsp-capabilities server)` (B3) — no `completionProvider` ⇒ the command reports "not supported by <server>" instead of sending the request. One shared helper `(lsp/supports? bid cap-key)` in the plugin's lib file.
- Request params: build with `(lsp-position-params bid)` / `(lsp-range-params bid)` (B3) — never hand-build positions in Scheme (encoding lives Rust-side).
- Callbacks follow B2's `(err result)` convention; on `err`, report via the message log and stop — no silent failures, no retries.
- Response shapes vary per spec (single | array | null) — every card lists its cases; `#f` (null) is always "nothing there", report politely.
- Tests are tier 3 (hub playbook): editor + `InlineLspBackend` with scripted responses; drive keys with `key()`/`key_enter()`; **verify key sequences against `keymap/defaults.rs` before writing them into tests** (LESSONS).
- Default keybindings are *suggested* here, **chosen and bound only at F11** after a keymap audit. Known-free today under the `g` goto trie: everything except `g`, `e`, `h`, `l`, `s`. `[`/`]` are Normal-mode paste-ring leaves (`keymap/defaults.rs`) — never bind sequences there; a `]d`-style binding would silently replace paste-ring cycling.

A worked fixture, referenced by the other cards (F1's test):
```rust
let mut lsp = InlineLspBackend::with_default_handshake();
lsp.respond_to("textDocument/hover", serde_json::json!({
    "contents": { "kind": "plaintext", "value": "fn main()" },
    "range": { "start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 7} }
}));
// … build editor with the double, open a file, run the hover key,
// drain, assert popup content.
```

---

### F1 — Hover

**Goal** — keybinding → `textDocument/hover` at the cursor → popup; long content overflows to the drawer (hub OQ default).

**Composes** — B2, B3, U4, U6.

**Steel sketch**
```scheme
(define-command! "lsp-hover" "Show hover info for the symbol under the cursor."
  (lambda ()
    (guard-capability "hoverProvider"
      (lsp-request #f "textDocument/hover" (lsp-position-params (focused-buffer))
        (lambda (err res)
          (cond (err  (report-lsp-error "hover" err))
                ((not res) (log! "No hover info"))
                (else (show-hover (hover-contents->text res)))))))))
```
`hover-contents->text`: handle the three `contents` shapes — `MarkupContent {kind, value}`, legacy `MarkedString` (string or `{language, value}`), and arrays of the latter. v1 renders markdown **as plain text** (strip nothing, show raw — code fences read fine in a monospace popup). `show-hover`: line count ≤ popup max (U4's ⅓-height rule is enforced by the widget; count before calling) → `show-popup!`; else `show-drawer-list!`-free path — the drawer *text* mode is just a single-column list of lines with a no-op on-select. Dismiss: close on any cursor motion or mode change (register transient `on-mode-change` + motion… simplest v1: close-popup! at the top of the *next* `lsp-hover` and on `Esc` via mode-change hook — pick, document, test).

**Response cases** — null; MarkupContent; MarkedString; MarkedString[]; with/without `range` (ignored v1).

**Tests** — fixture above: popup shows text; null → message-log line, no popup; error → error report; tall content → drawer path. Suggested default key: `gk` (verify at F11).

**Done when** — manual rust-analyzer hover on a stdlib call shows docs.

**Traps** — hover is the canonical `#:allow-stale` user (harmless if the buffer moved) — pass it; that's also its test.

**Size** — ~80 Steel + ~120 test lines.

---

### F2 — Goto definition family

**Goal** — four commands: `lsp-goto-definition` / `-declaration` / `-type-definition` / `-implementation`. One result → jump; many → drawer list.

**Composes** — B2, B3, B6, U6.

**Steel sketch** — one generic worker, four thin wrappers:
```scheme
(define (goto-request method cap)
  (guard-capability cap
    (lsp-request #f method (lsp-position-params (focused-buffer))
      (lambda (err res) …))))
```
Response cases (all four methods share them): null → "No definition found"; single `Location {uri, range}` → `goto-location!`; `Location[]` → length 1 jumps, else drawer rows `(uri->display-path, line, col, "")`; `LocationLink[]` → use `targetUri` + `targetSelectionRange.start` (prefer over `targetRange` — it's the identifier, not the whole body).

**Tests** — each response case scripted; the drawer path selects row 2 and lands there; jump-list: after a jump, the jump-back binding returns (B6 discipline). Suggested keys: `gd` / `gD` / `gy` / `gi` (all free in the goto trie today; F11 confirms).

**Done when** — manual `gd` into another file of this repo works, jump-back returns.

**Traps** — cross-file jumps open buffers — assert the no-jump-entry-on-failed-open case (nonexistent target in a fixture).

**Size** — ~70 Steel + ~140 test lines.

---

### F3 — Completions

**Goal** — the full insert-mode completion flow: trigger (U7 manual key + B7 trigger chars from server caps) → `textDocument/completion` → `completion-begin!` → U7 UI; `completionItem/resolve` on selection; re-request when `isIncomplete`.

**Composes** — B2, B3, B7, B8, U7.

**Steel sketch**
```scheme
;; at attach (on-lsp-attach): register the server's trigger characters
(register-trigger-chars! "lsp" (caps-ref caps "completionProvider" "triggerCharacters"))
;; on trigger (U7's named command + on-trigger-char hook):
(lsp-request #f "textDocument/completion" (lsp-position-params bid)
  (lambda (err res)
    (unless err
      (let-values (((items incomplete) (completion-response->items res)))
        (completion-begin! bid items #:incomplete incomplete)))))
```
`completion-response->items`: `CompletionItem[]` (incomplete = #f) | `CompletionList {isIncomplete, items}` | null. Strip snippet syntax when `insertTextFormat == 2`: v1 default = drop `${n:…}`/`$n` placeholders to plain text (a small regexish scheme helper; confirm acceptability here — this is the snippet OQ gate). `isIncomplete` + narrowing (U7 signals filter changes… simplest v1: re-run the request when the filter *shrinks* below the last requested prefix only if `incomplete`). Resolve: on selection change (U7 exposes the selected index via `completion-top`'s ordering — v1: resolve on **accept only** if `resolveProvider`, merging `additionalTextEdits`; per-selection resolve is polish, skip).

**Response cases** — null; bare array; CompletionList; items with `textEdit` vs `insertText` vs label-only; `additionalTextEdits` (auto-import!) applied via B6 after the main edit.

**Tests** — scripted: trigger char fires the request; accept applies textEdit + additionalTextEdits as **one visible undo step each per B6/B8 semantics** (assert exact buffer text then undo behavior); isIncomplete re-request happens; snippet-format item lands as stripped plain text; capability-gated (no completionProvider → no request on trigger).

**Done when** — manual: rust-analyzer completes `Vec::` members with auto-import edits applied; the snippet OQ row moves to Decisions with the verdict.

**Traps**
- Stale completion responses are auto-cancelled/dropped (C6) — do *not* pass `#:allow-stale`.
- `additionalTextEdits` can edit *above* the cursor — order of application matters (B6 sorts descending; hand it everything in one call).

**Size** — ~130 Steel + ~200 test lines. The biggest F-card.

---

### F4 — Diagnostics navigation

**Goal** — next/prev-diagnostic motions + `:diagnostics` drawer list. No LSP request — reads the C9 store via B5.

**Composes** — B5, B6, U6.

**Steel sketch**
```scheme
(define-command! "goto-next-diagnostic" "Jump to the next diagnostic." (lambda () (diag-jump +1)))
(define-command! "goto-prev-diagnostic" "…" (lambda () (diag-jump -1)))
(define-command! "diagnostics" ":diagnostics — list buffer diagnostics." (lambda () …drawer…))
```
`diag-jump`: `(diagnostics-for-buffer bid)` (already sorted) → compare the cursor's char offset (`(call! "stdlib/cursor-char-index" (current-selections))` — `core:stdlib`, loaded first) against each entry's `"start"` → first entry strictly after / last strictly before (wrap around, report "no diagnostics" when empty) → `(goto-location! (list bid line col))` using the entry's `"line"`/`"col"` fields (shape 2 — char-indexed, no encoding involved). Drawer rows: severity glyph + first message line.

**Tests** — next from before/inside/after a diagnostic; wraparound both directions; empty buffer message; drawer Enter jumps. Keys: `gn` / `gp` under the goto trie (free today — hub F4 decision; **not** `]d`/`[d`, paste-ring owns those leaves); confirm at F11.

**Done when** — manual: `gn` cycles through rust-analyzer errors.

**Traps** — cursor position vs diagnostic *start*: "next" compares starts; a cursor inside diagnostic A must still find A's *end*? No — next means next start after the cursor head; inside-A jumps to B (document in the command help string).

**Size** — ~70 Steel + ~110 test lines.

---

### F5 — Rename

**Goal** — prompt (prefilled with the symbol under the cursor) → `textDocument/rename` → `apply-workspace-edit!`. Tree-sitter fallback stays a separate command until Future (the hub decision's fallback is *not* built here — v1 reports "no LSP server" when unattached; wire the fallback when a tree-sitter rename op exists).

**Composes** — B2, B3, B6, B9.

**Steel sketch**
```scheme
(define-command! "lsp-rename" "Rename the symbol under the cursor."
  (lambda ()
    (guard-capability "renameProvider"
      (prompt! "Rename: " #:prefill (symbol-under-cursor)
        (lambda (new-name)
          (when new-name
            (lsp-request #f "textDocument/rename"
              (hash-insert (lsp-position-params (focused-buffer)) "newName" new-name)
              (lambda (err res)
                (cond (err (report-lsp-error "rename" err))
                      ((not res) (log! "Nothing to rename"))
                      (else (apply-workspace-edit! res)))))))))))
```
`symbol-under-cursor` is B9's builtin (word-boundary logic stays in Rust — never implement it in Scheme). Empty string → prefill empty, prompt still opens.

**Response cases** — null; `WorkspaceEdit` with `changes`; with `documentChanges` (B6 handles both — the *test* here just proves the wiring passes each through).

**Tests** — prompt prefill shows the symbol; cancel sends nothing (assert `sent` has no rename); multi-file WorkspaceEdit fixture applies + message-log summary; null response message. Suggested key: `gr`… conflicts with references' conventional `gr` — suggest `gR` rename / `gr` references; F11 arbitrates.

**Done when** — manual: rename a local across two files in a scratch cargo project; `:wa` writes both.

**Size** — ~60 Steel + ~130 test lines.

---

### F6 — References

**Goal** — `textDocument/references` → drawer list.

**Composes** — B2, B3, U6.

**Steel sketch** — F2's worker with `context.includeDeclaration = #t` added to the params and always-drawer presentation (even a single reference lists — you asked "where is it used", the answer is a list). Response: `Location[]` or null.

**Tests** — 3-location fixture lists 3 rows, Enter jumps, drawer stays open (U6 browse behavior); null → "No references".

**Done when** — manual on a symbol in this repo.

**Size** — ~30 Steel + ~60 test lines.

---

### F7 — Signature help

**Goal** — trigger chars (`(`, `,` from caps) fire a **debounced** `textDocument/signatureHelp`; popup with the active parameter emphasized; dismissed on `)`, mode exit, or cursor leaving the call.

**Composes** — B2, B3, B4, B7, U4.

**Steel sketch**
```scheme
(define sighelp-request
  (debounce 150
    (lambda (bid)
      (lsp-request #f "textDocument/signatureHelp" (lsp-position-params bid)
        (lambda (err res) (if (and (not err) res) (show-sighelp res) (close-popup!)))))))
;; on-trigger-char: char in sigHelp triggerCharacters -> (sighelp-request bid)
;;                  char == ")" -> (close-popup!)
;; on-mode-change: leaving Insert -> (close-popup!)
```
`show-sighelp`: `signatures[activeSignature ?? 0]`, label line + the active parameter (by `activeParameter` index into `parameters[].label` — label is a string or `[start, end)` offsets into the signature label; handle both) rendered on its own line/marker (plain text; no styling API in `show-popup!` v1 — mark with `⟨…⟩` around the active param).

**Response cases** — null (close); no signatures (close); activeParameter out of range (clamp); label-offset vs string parameter labels.

**Tests** — trigger char → (debounce elapses) → popup with marked param; `,` re-triggers and advances the marked param per fixture; `)` closes; Esc closes; debounce coalesces rapid trigger chars into one request (assert `sent` count).

**Done when** — manual: typing a call in rust-analyzer shows the signature and tracks parameters.

**Traps** — register `)` via `register-trigger-chars!` too (it's a dismiss trigger, not just a request trigger) — the handler branches on the char.

**Size** — ~90 Steel + ~140 test lines.

---

### F8 — Formatting

**Goal** — `:fmt` → whole-buffer `textDocument/formatting`, or `rangeFormatting` when a selection spans at least one full line (hub F8 decision; HUME selections are never empty, so bare "has selection" would always match). One undo step. **Never** format-on-save by default — ship the hook recipe commented out.

**Composes** — B2, B3, B6.

**Steel sketch**
```scheme
(define-command! "fmt" ":fmt — format the buffer (or the selected lines) via LSP."
  (lambda ()
    (let ((range? (selection-spans-full-line? (focused-buffer))))
      (guard-capability (if range? "documentRangeFormattingProvider" "documentFormattingProvider")
        (lsp-request #f (if range? "textDocument/rangeFormatting" "textDocument/formatting")
          (format-params range?)   ; adds FormattingOptions {tabSize, insertSpaces} from settings
          (lambda (err res)
            (cond (err (report-lsp-error "fmt" err))
                  ((not res) (log! "Already formatted"))
                  (else (apply-text-edits! (focused-buffer) res)))))))))
```
`selection-spans-full-line?` is B6's builtin (the gate decision lives in Rust line math). `FormattingOptions`: `tabSize` / `insertSpaces` read from the existing indent settings (find the setting keys via `:set` docs / `settings.rs`).

**Tests** — whole-buffer edits applied as one undo; sub-line selection formats the whole buffer (the decision's test!); full-line selection sends rangeFormatting with the right range; null → message; commented-out on-buffer-save recipe exists in the plugin source and stays inert (grep-test that loading the plugin registers no save hook).

**Done when** — manual `:fmt` on a scrambled rust file matches `cargo fmt` output.

**Size** — ~70 Steel + ~120 test lines.

---

### F9 — Code actions

**Goal** — keybinding → `textDocument/codeAction` for the cursor/selection with diagnostic context → U5 menu → apply (`edit` and/or `command`).

**Composes** — B2, B3, B5, B6, U5.

**Steel sketch**
```scheme
(lsp-request #f "textDocument/codeAction"
  (let ((p (lsp-range-params (focused-buffer))))
    (hash-insert p "context"
      (hash "diagnostics" (diagnostics-overlapping-range …)  ; from B5 pull
            "triggerKind" 1)))
  (lambda (err res)
    (when (and (not err) res (not (null? res)))
      (show-menu! (map action-title res)
        (lambda (idx)
          (when idx (run-action (list-ref res idx))))))))
```
`run-action` on a `CodeAction`: apply `edit` via `apply-workspace-edit!` if present, **then** send `command` via `workspace/executeCommand` if present (spec order: edit first, then command). A bare `Command` (legacy shape, has `command` string at top level) → executeCommand only. `executeCommand` responses are `null`; real effects arrive as a server→client `workspace/applyEdit` (C6+B6 already handle it — nothing to do here, but the test proves the loop).

**Response cases** — null/empty (report "No code actions"); CodeAction with edit only / command only / both; legacy Command; `disabled` actions (filter out).

**Tests** — menu lists titles; selecting an edit-action applies it; selecting a command-action sends executeCommand and a scripted follow-up `workspace/applyEdit` from the double lands in the buffer (the full loop test); empty → message, no menu. Suggested key: `ga` (free; F11 confirms).

**Done when** — manual: rust-analyzer quick-fix (e.g. "add missing match arms") applies.

**Traps** — don't pre-filter by `kind` v1 (no only-quickfix UX yet); do filter `disabled`.

**Size** — ~90 Steel + ~160 test lines.

---

### F10 — Inlay hints

**Goal** — visible-range `textDocument/inlayHint` on viewport change (debounced) + on edit (via diagnostics-changed? no — on didChange there's no hook; use `on-viewport-change` + a B4-debounced wrapper fired from `on-diagnostics-changed` as the “document settled” proxy — servers refresh hints roughly when diagnostics refresh) → `set-inlay-hints!`; U9 renders. Toggle setting `lsp.inlay-hints` (default off v1 — opt-in).

**Composes** — B2, B3, B4, B5, B7, U9.

**Steel sketch**
```scheme
(define refresh-hints
  (debounce 200
    (lambda (bid first last)
      (when (and (setting "lsp.inlay-hints") (lsp/supports? bid "inlayHintProvider"))
        (lsp-request #f "textDocument/inlayHint"
          (range-params-for-lines bid first last)
          (lambda (err res)
            (when (and (not err) res)
              (set-inlay-hints! bid (map hint->store-entry res)))))))))
;; on-viewport-change -> refresh-hints ; on-diagnostics-changed -> refresh for current viewport
```
`hint->store-entry`: `InlayHint {position, label (string | InlayHintLabelPart[]), paddingLeft/Right}` → `(pos text 'before)` with label parts concatenated and padding as literal spaces.

**Tests** — viewport change triggers one debounced request with the visible range; hints land in the store (assert store content — rendering is U9's pinned snapshots); toggle off → no requests; label-parts fixture concatenates.

**Done when** — manual: rust-analyzer type hints appear in a rust file with the setting on, scroll updates them.

**Traps** — hint positions are wire positions at response time — they go into the store through the same conversion path as everything else (the setter converts; you pass raw response positions — see B5's setter contract) and then live remapped by P2.

**Size** — ~80 Steel + ~120 test lines.

---

### F11 — `core:lsp` packaging + docs

**Goal** — the plugin ships: manifest, final keybindings, user-facing docs, plugin-author guide.

**Composes** — everything.

**Deliverables**
1. **Manifest** — `runtime/plugins/core/lsp/plugin.scm`: activation via `declare-plugin` in the user's `init.scm` with `#:events '(on-lsp-attach)` (activates on the first server attach — languages aren't known statically) **plus** `#:commands` for every `:`-command (`diagnostics`, `fmt`, `lsp-hover`, … — invoking any command also activates). Verify this dual activation against `lazy.rs` semantics. Multi-file layout mimicking `plum` (`plugin.scm` + `lib.scm` + per-feature files via relative `require`).
2. **Keybindings** — the final audit: every suggested default from F1–F10 checked against `keymap/defaults.rs` **at this moment** (things move); bind via `bind-key!` under the goto trie where suggested (`gd gD gy gi gr gR gk ga gn gp` were free at spec time). Collisions: prefer the goto-trie letter closest to convention, document the loser in the plugin README.
3. **User docs** — `user-manual/docs/lsp.md`: how to install a server, `register-lsp-server!` in `init.scm` (copy-paste rust-analyzer + one non-rust example), the commands/keys, the settings knobs. **Audience rules apply** (project `CLAUDE.md`): no Rust type names, no builtin internals beyond the documented Steel API, no babysitting.
4. **Plugin-author guide** — a section in the user manual or `docs/`: calling `lsp-request` for custom server extensions, worked example: rust-analyzer's `rust-analyzer/expandMacro` → popup. This is the community-offload payoff — write it like the reader never saw this repo.
5. **`init.scm.example`** — add a commented LSP block.

**Tests** — tier 3: plugin loads lazily (declared → `:lsp-hover` activates it → command works); `:plugin-status` shows it; every default binding dispatches (one smoke test iterating the binding table); the example `init.scm` block parses (eval it in a test).

**Done when** — a fresh checkout + the documented `init.scm` block + `cargo run src/main.rs` gives hover/goto/diagnostics with zero extra steps; every hub checkbox in Steps 0–4 is ticked; remaining OQ rows are either moved to Decisions or explicitly re-filed as Future items.

**Size** — ~120 Steel + docs + ~100 test lines.
