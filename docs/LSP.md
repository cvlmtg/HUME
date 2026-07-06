# HUME — LSP Support

Design and task breakdown for Language Server Protocol support. All LSP decisions live here (moved out of `ROADMAP.md`, which keeps a pointer).

This file is the **hub**: architecture, decisions, open questions, shared reference (protocol primer, codebase orientation, Steel API index, implementation order, testing playbook), and the progress tracker. Per-task implementation cards live in the step files:

| File | Contents |
|------|----------|
| `docs/lsp/step-0.md` | P1–P8 — prerequisites (workspace groundwork) |
| `docs/lsp/step-1.md` | C1–C10 — LSP core (Rust data plane) |
| `docs/lsp/step-2.md` | B1–B9 — Steel platform (bridge + primitives) |
| `docs/lsp/step-3.md` | U1–U9 — UI surfaces (widgets + render wiring) |
| `docs/lsp/step-4.md` | F1–F11 — features (Steel, `core:lsp` plugin) |

## How to use this document

Rules for the implementing session. Follow them exactly.

1. **One task per session.** Pick the first task in the [Implementation order](#implementation-order) whose checkbox (in the step sections below) is still unticked. Read: this hub's Overview + the sections your task card references, the task's card in its step file, and every file in the card's *Read first* list. Do not start coding before that.
2. **Verify before you write.** Cards name real files, types, and functions, but the codebase moves. Before relying on any named symbol, confirm it exists (`rg 'symbol_name'`). The doc deliberately avoids line numbers — navigate by symbol search.
3. **If the doc contradicts the code, STOP.** Do not improvise a workaround, do not silently adapt the design. Report the contradiction and wait for guidance. Same rule if a card's approach turns out to be impossible as written.
4. **Stay inside the card.** No drive-by refactors, no extra features, no "while I'm here". If a Step 4 feature needs a Rust change, that is a missing platform primitive: stop and report (fix belongs in a Step 1–3 task, not inline).
5. **A task is done when** every item in its card's *Done when* list is true, `cargo test --workspace` is green, and the task's checkbox in this hub is ticked. If the card was a decision gate (see Open Questions), also move the resolved question into the Decisions table with its rationale.
6. **Defaults are decisions.** Every open question has a `Default:`. At the gate, adopt the default unless the gate's own evidence (e.g. P8 numbers) contradicts it. Never leave a gate undecided.
7. Project-wide rules from `CLAUDE.md` apply everywhere: no `.unwrap()` outside tests, grapheme discipline in motion/selection code, every command tested, idiomatic Rust. Read `docs/LESSONS.md` at session start.

**Card format** (used by all step files):

- **Goal** — what exists after the task.
- **Depends / Unlocks** — hard ordering edges.
- **Files** — create/modify list (paths are indicative; verify parent modules).
- **Read first** — files/symbols to read before planning.
- **Mimic** — the existing code whose *pattern* you copy.
- **Shape** — signature sketches. These are design contracts, not literal code: names and argument shapes should survive; adjust internals to fit reality.
- **Tests** — what to test and with which harness.
- **Done when** — acceptance checklist.
- **Traps** — task-specific mistakes to avoid.
- **Size** — rough effort calibration (source + test lines, excluding churn).

## Overview

**Goals**, in order:
1. **Community-extensible**: LSP features live in Steel so plugins can add features, server integrations, and custom UX without touching Rust.
2. **Snappy on big-realistic workloads**: rust-analyzer on a large crate, 100k-line files, 5k-diagnostic bursts, 1k-item completion lists, large terminals.

**Architecture: Steel on the control plane, Rust on the data plane.** The split is a *frequency cut*, not a per-feature assignment:

- Runs **per user intent** (a keypress-triggered command, a server response arriving, a menu selection) → Steel. A Steel dispatch costs tens of microseconds against a 16 ms frame budget.
- Runs **per frame, per scroll row, or over unbounded collections** (rendering, diagnostic bursts, completion filtering, position math over edits) → Rust.

Two structural guardrails keep the experience snappy *by construction*, not by discipline:
- **Bulk data never crosses the Rust↔Steel boundary on recurring paths** (per-frame, per-keystroke, per-scroll). Diagnostics and completion items live in Rust-side stores; Steel sees signals, bounded subsets, and policy knobs. A one-time ingest at user-intent frequency (e.g. a completion response passing through Steel into the store, B8) is the calibrated exception — P8 measures whether it holds at 1k items.
- **Steel never runs on the render path.** Steel writes data stores; Rust providers render from them every frame.

Rust therefore ships a *platform* — transport, document sync, stores, render providers, a generic LSP bridge, scriptable UI widgets — and the features themselves are Steel code in a `core:lsp` plugin: send request → transform response → call a UI/store builtin. A community plugin author writes the exact same three-line shape.

**v1 feature set**: diagnostics, completions, hover, go-to-definition (+ declaration / type-definition / implementation), rename, references, signature help, formatting, code actions, inlay hints.

**Non-goals for v1**: semantic tokens, snippet placeholder navigation, `workspace/symbol`, document symbols, code lens, DAP, multiple servers per language or buffer.

**Implementation shape** (five macro-steps):
1. **Step 0 — Prerequisites**: workspace groundwork (serde, position mapping, event-loop wake + timers, encodings, URIs, the boundary-cost spike).
2. **Step 1 — LSP core (Rust data plane)**: the `hume-lsp` crate (transport + client) and editor glue (document sync, diagnostics store, registration).
3. **Step 2 — Steel platform**: the generic bridge, codec, stores, edit/navigation primitives, and hooks that let Steel drive everything.
4. **Step 3 — UI surfaces**: scriptable widgets (popup, menu, drawer) and store-fed render wiring (underlines, signs, virtual lines).
5. **Step 4 — Features (Steel)**: the `core:lsp` plugin composing Steps 2–3 into user-facing features.

Steps 1–3 are ordered by dependency but Step 4 tasks unlock incrementally — each feature can land as soon as the primitives it composes exist.

## Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| LSP architecture | **Hybrid: Steel control plane, Rust data plane** | Rust owns transport, JSON-RPC, document sync, bulk stores, and rendering. Steel owns behavior: every user-facing feature is Steel code composing Rust primitives. The boundary is frequency: per-user-intent work may be Steel; per-frame / per-scroll / unbounded-collection work must be Rust. Maximizes community extensibility without sacrificing latency. |
| Feature delivery | **Generic LSP bridge + `core:lsp` plugin, not hardcoded Rust features** | `(lsp-request server method params callback)` / `(lsp-notify …)` / `(on-lsp-notification method handler)` make any protocol method reachable from Steel. v1 features ship as the `core:lsp` plugin using the same public surface plugins get — dogfooding guarantees the bridge is complete. Rust volume is similar to hardcoding v1 (generic infra costs more up front), but marginal feature cost afterward is a Steel file anyone can write. |
| Bulk-data guardrail | **Bulk never crosses the boundary on recurring paths** | Diagnostics are ingested, stored, and remapped in Rust; Steel receives change *signals* and pulls bounded subsets (visible lines, top-N). Completion items live in a Rust store with a Rust per-keystroke filter; Steel orchestrates and sees the top-N. The one crossing allowed is a per-user-intent ingest (a completion response flowing through the B2 callback into `completion-begin!`) — if the P8 spike shows that's too slow at 1k items, `lsp-request` grows a flag routing the raw response straight into the store, handing Steel a handle instead of items. |
| Render decoupling | **Data-driven decoration stores; Steel never on the render path** | Steel setters (`set-inlay-hints!`, `set-signs!`, `set-virtual-lines!`, …) write Rust-side stores; the existing engine providers (`HighlightSource`, `SignSource`, `VirtualLineSource`, `InlineDecoration`) render from them each frame with zero Steel involvement. |
| UI widgets | **Rust-rendered, Steel-fed** | Cursor popup, selection menu, drawer list are generic Rust widgets taking content + callbacks from Steel (`show-popup!`, `show-menu!`, `show-drawer-list!`). LSP is their first client, not their owner — any plugin can use them. |
| Symbol rename | **LSP-first, tree-sitter fallback** | `textDocument/rename` when LSP active; falls back to tree-sitter local rename via `locals.scm` — file-local only but scope-correct. Same keybinding in both cases. |
| Position remapping through edits | **Batch `PosMapCursor`, never per-position** | Diagnostic ranges, bookmarks, and any stored positions must be remapped through edits with the batch primitive: widen `ChangeSet`'s `PosMapCursor` (`hume-editing/src/changeset/mod.rs`) from `pub(crate)` to `pub` and map sorted position lists through one cursor. Per-position one-shot mapping reintroduces the O(positions × ops) cost that `translate_in_place` was migrated off. |
| Protocol types | **`lsp-types` crate** | De-facto standard (Helix, rust-analyzer ecosystem), spec-tracking, serde-based. Hand-transcribing the LSP spec is weeks of error-prone tedium. Brings `serde`/`serde_json` into the workspace (which also fires the toml 0.8 → 1.x upgrade trigger). |
| Transport concurrency | **std threads + mpsc; `LspBackend` trait mirroring `ParseBackend`** | Per server: reader thread (blocks on server stdout), writer thread (stdin), stderr logger. Messages cross to the main loop via mpsc; `prepare_frame` drains; `has_pending()` switches the event loop to `poll(timeout)` — byte-for-byte the `ThreadedParseBackend` pattern. No tokio: transport concurrency is never the bottleneck (heavy JSON parse lands on reader threads off the main loop; incremental `didChange` is proportional to the edit, not the file), and a runtime would split a fully-sync codebase into two concurrency idioms. The trait boundary is the insurance policy. Cost accepted: request timeouts are hand-rolled (deadline check at drain time). |
| Crate boundary | **New `hume-lsp/` crate** | Follows the `hume-treesitter` precedent: transport, JSON-RPC codec, and client state have zero `Editor` dependency. Depends on `hume-editing` (ChangeSet → didChange conversion) + `lsp-types`; acyclic. Editor glue (drain, stores, builtins, hooks) stays in `hume-editor`. |
| Server instance scope | **One per (language, workspace root)** | First buffer open whose (language, resolved root) pair has no running server spawns it (C3 + C5 handshake); later buffers with the same pair attach. Correct `rootUri` when two projects share a session; the diagnostics store already keys by server. |
| Position encoding | **Negotiate `utf-8`, fall back to UTF-16 conversion** | LSP defaults to UTF-16 code-unit columns; 3.17's `positionEncoding` capability lets client and server agree on `utf-8`. HUME is char/grapheme-based, so both paths need conversion helpers; ropey 1.6 provides `char ↔ utf16_cu` primitives (`char_to_utf16_cu` / `utf16_cu_to_char`, unconditional — no feature flag in 1.x) for the fallback. Never assume byte == column. |
| Timers | **Timer wheel in the event loop; `(after ms thunk)` + debounced hooks in Steel** | The loop already degrades to `poll(timeout)` when async work is pending; a nearest-deadline timer wheel bounds that timeout naturally. Required for debounced inlay-hint refresh, signature-help triggering, and completion debounce — without it, scroll- and keystroke-driven Steel work would fire unthrottled. |
| Server→client requests | **Answered in Rust, never surfaced to Steel in v1** | JSON-RPC requires a response to every request, including server-initiated ones. C6 dispatches them: `workspace/configuration` answered from C8 settings, `workspace/applyEdit` applied via the B6 engine, `client/registerCapability` / `window/workDoneProgress/create` acknowledged and ignored, anything else gets a `MethodNotFound` error response. Steel handles notifications only (`on-lsp-notification`) — response obligations stay where timeouts can't be caused by a plugin. |

## Open Questions

Every row has a **Default** — at the gate, adopt it unless the gate's evidence contradicts it (rule 6 above). When a gate resolves, move the row into Decisions.

| Question | Context |
|----------|---------|
| Diagnostics exposure granularity | Rust ingests/stores/remaps (decided). Open: exactly what Steel pulls — per-line visible subset, per-buffer summaries, full list under a size cap? The P8 spike (JSON↔SteelVal throughput at 100/1k/5k items) decides where the cap sits. **Default:** `diagnostics-for-buffer` returns at most 1 000 items, filtered by the optional `#:severity` / `#:range` arguments before the cap applies. |
| Steel completion scorer budget | Rust does prefix/fuzzy filtering (decided; matches the existing "fuzzy scoring is a plugin concern" ROADMAP note for minibuffer). Open: the optional Steel scorer hook — score all candidates (cost unknown until P8) or re-rank Rust's top-N only? **Default:** re-rank Rust's top-N only (N = 64). |
| Hover surface | Cursor popup vs Class B bottom drawer. Decide when U4/U6 land. **Default:** popup primary; content taller than the popup's max height (⅓ of pane height) overflows to the drawer. |
| Snippet completions | v1 strips `${1:...}` placeholders to plain text. Confirm acceptable UX for rust-analyzer (which snippet-ifies aggressively) at F3. **Default:** strip. When full support lands (Future): placeholder *parsing* could be Steel; the insert-mode tabstop state machine with multi-cursor placeholder selections is likely Rust. |
| Server crash policy | Manual `:lsp-restart` only, or bounded auto-restart (e.g. 3 attempts with backoff)? Revisit with real usage. Policy knob belongs to Steel either way. **Default:** manual-only. |
| WorkspaceEdit on unopened files | Rename/code actions can touch files with no open buffer. Open-as-buffer (undoable, dirty, user saves) vs direct fs write (atomic but bypasses undo). Decide at B6. **Default:** open-as-buffer; after applying, report `"N buffers modified — :wa writes all"` to the message log. |
| `workspace/configuration` flow | How Steel user config reaches servers. Decide at C8. **Default:** the `register-lsp-server!` `#:settings` blob is answered verbatim to `workspace/configuration` requests and sent once as `didChangeConfiguration` after `initialized`. |
| `$/progress` | Servers report indexing progress. Statusline spinner element vs message-log only. Decide at U3. **Default:** message-log only in v1 (begin/end messages, no per-report spam); spinner element is Future. |
| `workspace/didChangeWatchedFiles` | Ties to the existing "File watcher" Future item in ROADMAP — LSP may be the trigger that promotes it. Not required for v1. **Default:** do not implement; do not advertise the capability. |
| Multiple servers per language | v1: exactly one server per language. The diagnostics store keys by (server, buffer) so the door stays open. **Default:** reject a second `register-lsp-server!` for the same language with a loud error. |
| `set-extra-highlights!` tier | Generic plugin spans need a `HighlightTier`: new variant vs reusing an existing one (Syntax / SearchMatch / Diagnostic / BracketMatch). Decide at U1 when wiring the provider. **Default:** new variant `Extra` between `Syntax` and `SearchMatch` (plugin spans beat syntax, lose to search/diagnostics/brackets); renumber discriminants and the `TIER_COUNT` arrays in `hume-engine/src/style/highlight.rs`. |

## LSP protocol primer

Compact model of the protocol as HUME uses it. Full spec: <https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/>. The `lsp-types` crate mirrors the spec's type names — when a card names a protocol type, that's the `lsp_types::` type.

**Wire format.** Each message is `Content-Length: N\r\n\r\n` followed by exactly N bytes of a JSON-RPC 2.0 body (other headers may appear before the blank line; ignore all but `Content-Length`). Three body kinds:
- *Request*: `{jsonrpc, id, method, params}` — expects exactly one response with the same `id`. Both sides may send requests: the client (hover, completion…) **and the server** (`workspace/configuration`, `workspace/applyEdit`). Every received request must be answered — with a `MethodNotFound` error if unhandled. Ignoring a server request can hang the server.
- *Response*: `{jsonrpc, id, result}` or `{jsonrpc, id, error: {code, message}}`. Correlated to its request purely by `id` (HUME allocates monotonically increasing integers).
- *Notification*: `{jsonrpc, method, params}` — no id, no response, may be sent freely in both directions.

**Lifecycle.** `initialize` request (client capabilities, workspace root) → server replies with `ServerCapabilities` → client sends `initialized` notification. Everything else only after that. Shutdown: `shutdown` request, then `exit` notification, then reap the process. Capabilities gate features: e.g. no `completionProvider` in the reply → server can't complete; features must check (B3) instead of firing blind.

**Document sync.** The client owns document truth. `textDocument/didOpen` (full text + `languageId` + integer `version`) starts server-side tracking; every edit sends `textDocument/didChange` with the new version and a list of `TextDocumentContentChangeEvent` range-edits (we negotiate incremental sync); `didSave` / `didClose` bracket the rest. Versions must be strictly increasing — HUME uses the buffer's `text_gen`. A server response computed against version K is stale if the buffer has moved past K (C6 handles this).

**Positions.** A protocol `Position` is `{line, character}` where `character` counts *code units in the negotiated encoding* — UTF-16 by default, UTF-8 when both sides agree via `positionEncoding` (3.17). It is never a byte offset, never a grapheme count, and never a Rust `char` count. All conversions go through P4's helpers. A `Range` is half-open: `end` is exclusive.

**URIs.** Documents are identified by URI (`file:///path/with/%20escapes`). `Buffer.path` (canonical) is the SSOT for outgoing URIs; incoming URIs are canonicalized before buffer lookup (P5).

**v1 method map.**

| Feature | Client→server | Direction / notes |
|---------|---------------|-------------------|
| Sync (C7) | `textDocument/didOpen`, `didChange`, `didSave`, `didClose` | notifications |
| Diagnostics (C9) | — | server pushes `textDocument/publishDiagnostics` notification |
| Hover (F1) | `textDocument/hover` | `Hover { contents: MarkupContent \| … }` or null |
| Goto (F2) | `textDocument/definition`, `declaration`, `typeDefinition`, `implementation` | result: `Location`, `Location[]`, `LocationLink[]`, or null |
| Completion (F3) | `textDocument/completion`, then `completionItem/resolve` | result: `CompletionItem[]` or `CompletionList { isIncomplete, items }` |
| Diag nav (F4) | — | reads C9 store, no request |
| Rename (F5) | `textDocument/rename` | result: `WorkspaceEdit` (`changes` **or** `documentChanges` — handle both) |
| References (F6) | `textDocument/references` | params include `context.includeDeclaration` |
| Signature help (F7) | `textDocument/signatureHelp` | `SignatureHelp { signatures, activeSignature, activeParameter }` |
| Formatting (F8) | `textDocument/formatting` / `rangeFormatting` | result: `TextEdit[]` — apply as one transaction (B6) |
| Code actions (F9) | `textDocument/codeAction`, `workspace/executeCommand` | result: `(Command \| CodeAction)[]`; actions carry `edit` and/or `command` |
| Inlay hints (F10) | `textDocument/inlayHint` | params: visible range; result: `InlayHint[]` |
| Cancel (C6) | `$/cancelRequest` | notification, best-effort |
| Server→client (C6) | — | `workspace/configuration`, `workspace/applyEdit` (requests — must answer); `window/showMessage`, `window/logMessage`, `$/progress` (notifications → message log) |

## HUME orientation map

The subsystems LSP touches, where they live, and what each step does with them. Read the row's file before the task that names it.

| Subsystem | Where | LSP relationship |
|-----------|-------|------------------|
| Event loop | `hume-editor/src/editor/lifecycle.rs` — `run` loop: blocking `event::read()` degrades to `event::poll(8ms)` while `parse_worker.has_in_flight()`; `prepare_frame` does per-frame work (`reparse_stale_buffers` drains parse results) | P3 generalizes the wake predicate over parse worker + LSP + timers; LSP messages drain in `prepare_frame` |
| Async-backend precedent | `hume-treesitter/src/parse_worker.rs` — `ParseBackend` trait (`post` / `drain_done` / `is_in_flight` / `remove_in_flight`), `ThreadedParseBackend` (worker thread + two mpsc channels + kill-on-drop), `InlineParseBackend` (synchronous test double); editor holds `parse_worker: Box<dyn ParseBackend>` (`editor/mod.rs`) | C3/C4 mirror this wholesale: `LspBackend` trait, threaded impl, inline scripted double |
| Buffer & versions | `hume-editor/src/editor/buffer/mod.rs` — `Buffer { text_gen: u64, path, language, … }`; every text mutation bumps `text_gen` | `text_gen` is the LSP document version and the staleness token (C6/C7) |
| ChangeSet | `hume-editing/src/changeset/mod.rs` — `Operation::{Retain(n), Delete(n), Insert(String)}` (char counts, old-document order); `PosMapCursor` (batch position mapping, currently `pub(crate)`); `map_pos` (one-shot, the test oracle) | P2 widens `PosMapCursor`; P6 converts ChangeSets to LSP edits; C9/B5 remap stored ranges through edits |
| Hooks | `hume-scripting/src/hooks.rs` — `HookId` enum + `HOOKS` const table (the SSOT: variant ↔ Steel name); editor side `editor/scripting_setup.rs` — `fire_hook_silent` enqueues into `state.pending_hooks`, `drain_hooks` evals with a cascade cap | B7 adds four hooks: extend `HookId` + `HOOKS`, add fire sites, done |
| Steel builtins | `hume-scripting/src/builtins/mod.rs` — every builtin registered via `steel.register_fn_with_ctx(HUME_CTX, "name", module::fn)`; one module per domain (`buffers.rs`, `syntax.rs`, `statusline.rs`, …); the Scheme bootstrap string in the same file defines wrappers like `declare-plugin` | Steps 2–3 add an `lsp.rs` (and `ui.rs`) builtin module following the same shape |
| Init-only registration queue | `hume-scripting/src/builtins/syntax.rs` — `%define-language!` pushes `PendingLanguageReg` onto `ScriptingHost.pending_language_regs`; editor drains via `flush_pending_language_regs` (`editor/syntax/mod.rs`) after each init eval | C8's `register-lsp-server!` clones this queueing pattern |
| Typed commands | Handlers `editor/commands/typed_*.rs` with signature `fn(ed: &mut Editor, arg: Option<&str>, force: bool) -> Result<(), CommandError>`; registered via `typed_cmd!` in `editor/registry/defaults.rs`; dispatcher tries typed first, then falls back to named commands (`editor/mappings/command_mode.rs`) — so Steel `define-command!` commands are `:`-invocable too | C10 adds `:lsp-status` / `:lsp-stop` / `:lsp-restart`; F4/F8 add `:diagnostics` / `:fmt` in Steel |
| Render providers | `hume-engine/src/providers.rs` — `HighlightSource` (line-relative byte spans, tier-layered: `HighlightTier::{Syntax, SearchMatch, Diagnostic, BracketMatch}`), `VirtualLineSource`, `InlineDecoration` (per-line `InlineInsert`s), `OverlayProvider` (raw ratatui cell access), `ProviderSet::{add_*, remove}`; signs in `hume-engine/src/builtins/sign_column.rs` (`Sign`, `SignSource`, `SignColumn`) | Step 3 implements one provider per decoration kind over the Step 1–2 stores |
| Provider feeding pattern | `hume-editor/src/ui/highlight_providers.rs` — `SharedHighlighter { scope, tier, data: Arc<RwLock<Vec<…>>> }`: editor writes the Arc once per frame, provider reads per line; registered per-pane in `build_pane` (`hume-editor/src/ui/mod.rs`) | U1/U2/U8/U9 all follow this write-side/read-side split |
| Statusline | `hume-editor/src/ui/statusline.rs` — `StatusElement` enum rendered in Rust; Steel configures element lists via `configure-statusline!` (`hume-scripting/src/builtins/statusline.rs`) | U3 adds a `Diagnostics` element variant (Rust reads the C9 store directly — Steel never renders) |
| Minibuffer | `hume-editor/src/editor/minibuf/mod.rs` — `MiniBuffer { prompt: char, input, cursor }` + `MiniBufferEvent` (`Confirm`/`Cancel`/…); drives Command and Search modes | B9 adds a Steel-callback prompt mode on the same machinery (F5 rename needs it) |
| Completion (minibuffer) | `hume-editor/src/editor/completion/` — `Completer` trait, `CompletionState`; overlay `hume-editor/src/ui/completion_overlay.rs` (statusline-anchored `OverlayProvider`) | U7 reuses the selection-state idea; U4 reuses the cell-painting approach; geometry is new |
| Plugins | `runtime/plugins/core/<name>/plugin.scm`; lazy manifests via `declare-plugin` (`#:commands` / `#:events` / `#:languages` — wrapper in the bootstrap string, `hume-scripting/src/builtins/mod.rs`; state machine in `hume-scripting/src/lazy.rs`); `plum` is the biggest example (multi-file via relative `require`) | Step 4 ships `runtime/plugins/core/lsp/` the same way |
| Jump list | `editor/commands/mod.rs` — `current_jump_entry(state, view)`; push only after the "commit point" (see LESSONS/memory: pushing earlier truncates forward history on abort) | B6's `goto-location!` pushes a jump entry |
| Message log | `Editor::report(Severity, String)`; `:messages` opens a read-only view (`open_read_only_view`, `editor/buffer/file_open.rs`) | C10 stderr/log routing; several defaults report here |
| Read-only views | `open_read_only_view(label, content, cursor_line)` (`editor/buffer/file_open.rs`) — synthetic read-only buffer | Not the drawer (U6 is real chrome), but the fallback pattern if a list surface is needed before U6 lands |

## Steel API index

Every Steel-visible surface Steps 1–3 introduce. Cards define the semantics; this index is the lookup table (and the completeness check for F11's docs). Conventions: `bid` = buffer id value (opaque; compare via `(buffer-generation …)` ints, not `equal?` — see LESSONS on opaque types); positions produced by HUME builtins (B5 pulls, params helpers' inputs) are 0-based and **char-indexed** — raw wire positions appear only inside undecoded LSP response hashmaps, and only `goto-location!` (which accepts a raw `Location` hashmap) or the B5/B8 setters (which take response-shaped data) may consume them; Steel never converts encodings itself. Callbacks take `(err result)` — exactly one is non-`#f`.

| Surface | Kind | Task |
|---------|------|------|
| `(register-lsp-server! lang #:command cmd #:args '(…) #:root-markers '(…) #:init-options blob #:settings blob)` | builtin, init-only | C8 |
| `(lsp-request server method params callback #:allow-stale bool)` | builtin | B2 |
| `(lsp-notify server method params)` | builtin | B2 |
| `(on-lsp-notification method handler)` | builtin | B2 |
| `(lsp-capabilities server)` → decoded caps or `#f` | builtin | B3 |
| `(lsp-server-status)` → list of status records | builtin | B3 |
| `(lsp-server-for-buffer bid)` → server name or `#f` | builtin | B3 |
| `(buffer-generation bid)` → int | builtin | B3 |
| `(lsp-position-params bid)` / `(lsp-range-params bid)` → ready-made params hashmaps (encoding-correct) | builtin | B3 |
| `(after ms thunk)` → timer id; `(cancel-timer! id)` | builtin | B4 |
| `(debounce ms proc)` → debounced proc | builtin (bootstrap wrapper over `after`) | B4 |
| `(set-inlay-hints! bid hints)` | builtin | B5 |
| `(set-signs! source bid signs)` | builtin | B5 |
| `(set-virtual-lines! source bid lines)` | builtin | B5 |
| `(set-extra-highlights! source bid spans)` | builtin | B5 |
| `(diagnostics-for-buffer bid #:severity floor #:range (start . end))` | builtin | B5 |
| `(diagnostic-counts bid)` → `(errors . warnings)` | builtin | B5 |
| `(apply-text-edits! bid edits)` | builtin | B6 |
| `(apply-workspace-edit! wsedit)` | builtin | B6 |
| `(goto-location! loc)` | builtin | B6 |
| `(selection-spans-full-line? bid)` → bool (F8's range-format gate) | builtin | B6 |
| `on-lsp-attach` (server ready for buffer) | hook | B7 |
| `on-diagnostics-changed` (signal only; pull via B5) | hook | B7 |
| `on-viewport-change` (debounced) | hook | B7 |
| `on-trigger-char` (typed char ∈ registered set, Insert mode) + `(register-trigger-chars! source chars)` | hook + builtin | B7 |
| `(completion-begin! bid items #:incomplete f)` / `(completion-update-filter! text)` / `(completion-top n)` / `(completion-accept! idx)` / `(completion-dismiss!)` | builtins | B8 |
| `(prompt! label #:prefill text on-confirm)` | builtin | B9 |
| `(symbol-under-cursor bid)` → string (word at primary cursor; Rust grapheme/word logic) | builtin | B9 |
| `(show-popup! text #:anchor 'cursor)` / `(close-popup!)` | builtin | U4 |
| `(show-menu! items on-select)` / `(close-menu!)` | builtin | U5 |
| `(show-drawer-list! items on-select)` / `(close-drawer!)` | builtin | U6 |
| `:lsp-status`, `:lsp-stop`, `:lsp-restart` | typed commands | C10 |
| Settings knobs (`lsp.request-timeout-ms`, `lsp.diagnostics-severity-floor`, `lsp.inlay-hints`, …) | via existing `:set` / `set-option!` | owning cards |

## Implementation order

Canonical linearization — implement top to bottom. Progress is tracked by the checkboxes in the step sections below (the single tracker — tick there, not here). The only intentional deviations from numeric order: P8 needs serde (P1); P5/P6 live in `hume-lsp` so they follow C1.

1. **Step 0 + Step 1 core:** P1 → P8 → P2 → P3 → P7 → P4 → C1 → P5 → P6 → C2 → C3 → C4 → C5 → C6 → C8 → C7 → C9 → C10
2. **Step 2 (Steel platform):** B1 → B2 → B3 → B4 → B7 → B5 → B6 → B9 → B8
3. **Step 3 (UI surfaces):** U1 → U2 → U3 → U4 → U5 → U6 → U9 → U7 → U8
4. **Step 4 (features; each unlocks when its composition exists):** F1 → F2 → F6 → F4 → F5 → F8 → F9 → F7 → F3 → F10 → F11

### Milestone checkpoints

Observable proof that a step landed. Run the check before starting the next step.

- **After Step 0**: `cargo test --workspace` green; editor behaves identically in normal use (the wake/timer refactor is invisible); P8's numbers are recorded in this hub (Decisions/OQ updated).
- **After Step 1**: with rust-analyzer registered in `init.scm` and a `.rs` file open — `:lsp-status` shows the server `Running` with the correct root; typing produces no protocol errors in `:messages`; quitting the editor leaves no orphan `rust-analyzer` process (`pgrep`).
- **After Step 2**: from `init.scm`: an `(lsp-request … "textDocument/hover" … (lambda (err res) …))` round-trip logs a result; `(after 200 thunk)` fires once; `(apply-text-edits! …)` performs an undoable edit. All against the C4 double in tests, against rust-analyzer manually.
- **After Step 3**: introduce an error into a `.rs` file → underline + gutter sign + statusline count appear (no `core:lsp` plugin yet — wired via a test snippet in `init.scm`); `(show-popup! "hi")` renders at the cursor and `(close-popup!)` removes it.
- **After Step 4**: each F-card's *Done when* manual check; `:plugin-status` shows `core:lsp` lazy-loading on first matching buffer.

## Testing playbook

Four tiers, cheapest first. Every card's *Tests* section names its tier(s).

1. **`hume-lsp` unit tests** (`cargo test -p hume-lsp`): codec framing round-trips, P5 URI round-trips, P6 converter against a string-mirror oracle, request-id correlation. No editor, no process spawns.
2. **Editor integration with the C4 inline double** (`cargo test -p hume-editor`): the `InlineLspBackend` answers scripted `(method → response)` fixtures synchronously — the LSP analog of `InlineParseBackend`, and the workhorse for everything from C5 to F11. Editor test helpers: `key()` / `key_enter()` (`editor/tests/mod.rs`); remember key sequences must match `keymap/defaults.rs` (e.g. goto-last-line is `ge`, not `G`).
3. **Steel-level tests** (`hume-editor/tests/scripting.rs` pattern): eval plugin code against `SteelCtxTestHarness` (`hume-scripting/src/context.rs`) or a full editor with the double; this is how Step 4 features are tested — scripted server responses in, editor state assertions out.
4. **Manual smoke** (record what you did in the PR/commit message): `cargo run -- src/main.rs` with an `init.scm` containing `(register-lsp-server! "rust" #:command "rust-analyzer" #:root-markers '("Cargo.toml"))`; exercise the feature; check `:messages` for protocol errors.

Test-writing rules (from `CLAUDE.md`, restated because they bite here):
- **Independent oracle**: P6's oracle applies the emitted LSP events to a plain `String` mirror and compares with the post-edit rope — never re-derive expectations through the converter itself.
- **Flip check**: after writing a test, break the code (flip a condition) and confirm the test fails.
- **Grapheme rules don't apply to wire math**: protocol positions are char/code-unit based — use P4 helpers, not grapheme helpers, for codec/conversion code. Grapheme discipline still governs anything selection-like (e.g. "symbol under cursor" in F5). The `no_raw_char_stepping_in_motion_code` lint only scans `ops/` + `lines.rs`/`word.rs`; don't move protocol math there.
- Shared-state test mutexes: recover from poison with `unwrap_or_else(|e| e.into_inner())`.

## Step 0 — Prerequisites

Workspace groundwork with no LSP-visible behavior. Cards: `docs/lsp/step-0.md`. Note P5/P6 live in `hume-lsp` and therefore follow C1 (see Implementation order).

- [ ] **P1** — serde + serde_json workspace deps (fires the toml 0.8→1.x evaluation)
- [ ] **P2** — batch position mapping (`PosMapCursor` public API)
- [ ] **P3** — generalized event-loop wake (compose async sources behind one predicate)
- [ ] **P4** — position encoding conversion (rope char offset ↔ LSP line/character, utf-8 + utf-16)
- [ ] **P5** — path ↔ `file://` URI (in `hume-lsp`)
- [ ] **P6** — `ChangeSet` → `TextDocumentContentChangeEvent[]` (in `hume-lsp`)
- [ ] **P7** — event-loop timer wheel
- [ ] **P8** — boundary-cost spike (measure, then decide; updates this hub)

## Step 1 — LSP core (Rust data plane)

The `hume-lsp` crate plus editor glue: spawn, handshake, document sync, diagnostics in memory. After this step nothing is visible except `:lsp-status` and the message log — but the data plane is complete. Cards: `docs/lsp/step-1.md`.

- [ ] **C1** — crate scaffold (`hume-lsp/`, deps: `lsp-types` + serde + `hume-editing` only)
- [ ] **C2** — JSON-RPC codec (framing, message enum, id allocation/correlation)
- [ ] **C3** — server process management (reader/writer/stderr threads, `ServerHandle`, kill-on-drop)
- [ ] **C4** — `LspBackend` trait + `InlineLspBackend` scripted double
- [ ] **C5** — lifecycle (initialize handshake, capability storage, shutdown, crash detection)
- [ ] **C6** — request bookkeeping (deadlines, staleness by `text_gen`, `$/cancelRequest`, server→client request dispatch)
- [ ] **C7** — document sync glue (didOpen/didChange/didSave/didClose from `ChangeSet`s)
- [ ] **C8** — server registration (`register-lsp-server!`, root resolution, spawn-on-first-open)
- [ ] **C9** — diagnostics store (Rust-ingested, P4-converted, P2-remapped; bulk never reaches Steel)
- [ ] **C10** — observability + lifecycle commands (`:lsp-status` / `:lsp-stop` / `:lsp-restart`, stderr → log)

## Step 2 — Steel platform

The bridge and primitives that make Steel the feature layer. After it, a plugin can reach any LSP method and any editor surface. Cards: `docs/lsp/step-2.md`.

- [ ] **B1** — JSON↔SteelVal codec (null ↔ void; sized by P8)
- [ ] **B2** — generic LSP bridge (`lsp-request` / `lsp-notify` / `on-lsp-notification`)
- [ ] **B3** — introspection builtins (capabilities, status, generation, server-for-buffer)
- [ ] **B4** — Steel timers (`after`, `debounce`, debounced hook variants)
- [ ] **B5** — decoration stores + setters + diagnostics pull
- [ ] **B6** — edit + navigation primitives (`apply-text-edits!`, `apply-workspace-edit!`, `goto-location!`)
- [ ] **B7** — new hooks (`on-lsp-attach`, `on-diagnostics-changed`, `on-viewport-change`, `on-trigger-char`)
- [ ] **B8** — completion orchestration API (Rust store + filter, Steel session driver)
- [ ] **B9** — Steel minibuffer prompt (`prompt!` — F5 needs it; no prompt primitive exists today)

## Step 3 — UI surfaces

Generic, Steel-scriptable widgets plus store-fed render wiring. The engine primitives mostly exist (reserved `HighlightTier::Diagnostic`, `SignColumn`/`SignSource`, `VirtualLineSource`, `InlineDecoration`, `OverlayProvider`) — LSP is their first client, not their owner. Cards: `docs/lsp/step-3.md`.

- [ ] **U1** — diagnostic underlines (third `SharedHighlighter`, Diagnostic tier) + extra-highlights wiring
- [ ] **U2** — diagnostic gutter signs (first real `SignColumn` registration)
- [ ] **U3** — statusline diagnostics element (Rust element over the C9 store)
- [ ] **U4** — cursor-anchored popup widget (`show-popup!`)
- [ ] **U5** — selection menu widget (`show-menu!`)
- [ ] **U6** — Class B bottom drawer (minimal) + location list (`show-drawer-list!`)
- [ ] **U7** — in-buffer completion menu + dispatch (insert-mode flow over B8 + U4)
- [ ] **U8** — inline diagnostics (`VirtualLineSource`) + the deferred scroll/cursor rewiring
- [ ] **U9** — inlay-hint rendering (`InlineDecoration` over B5's store)

## Step 4 — Features (Steel, `core:lsp`)

Each feature is Steel code composing Steps 2–3 primitives; all are testable against the C4 double with scripted responses. **Rust changes in this step must be zero** — a needed Rust change means a missing platform primitive: stop and report. Cards: `docs/lsp/step-4.md`. (Tags omit B3 — every feature also uses its capability checks and params helpers.)

- [ ] **F1** — hover *(B2, U4, U6)*
- [ ] **F2** — goto definition family *(B2, B6, U6)*
- [ ] **F3** — completions *(B2, B7, B8, U7)*
- [ ] **F4** — diagnostics navigation *(B5, B6, U6)*
- [ ] **F5** — rename *(B2, B6, B9)*
- [ ] **F6** — references *(B2, U6)*
- [ ] **F7** — signature help *(B2, B4, B7, U4)*
- [ ] **F8** — formatting *(B2, B6)*
- [ ] **F9** — code actions *(B2, B5, B6, U5)*
- [ ] **F10** — inlay hints *(B2, B4, B5, B7, U9)*
- [ ] **F11** — `core:lsp` packaging + docs

## Deferred / Future

- **Semantic tokens** — delta decode (integer-array crunching) in Rust feeding a highlight store; orchestration Steel. Interacts with tree-sitter highlight priority.
- **Snippet placeholder navigation** — parsing maybe Steel; insert-mode tabstop state machine with multi-cursor selections likely Rust (see open question).
- **`workspace/symbol` + document symbols** — becomes plugin-writable via B2 + drawer once the M12 fuzzy finder lands.
- **Code lens** — plugin-writable via B2 + B5 stores + `workspace/executeCommand`.
- **Multiple servers per language / per buffer** — Rust routing + result merging; store already keys by server.
- **Bounded auto-restart** for crashed servers (policy knob Steel-side).
- **`workspace/didChangeWatchedFiles`** via a real file watcher (existing ROADMAP Future item).
- **DAP** — entirely separate protocol; out of scope, noted only as the same platform pattern (Rust transport, Steel behavior) applied again.
