# HUME — LSP Support

The architecture of HUME's Language Server Protocol support and the decisions that span
more than one module. A decision with one implementing site is documented at that site.

| Topic | Where |
|---|---|
| Transport, JSON-RPC codec, client state, wire decoding | `hume-lsp/src/` |
| Document sync, diagnostics, registry, routing, the request bridge | `hume-editor/src/editor/lsp/` |
| Steel builtins | `hume-scripting/src/builtins/` (`lsp.rs`, `lsp/`, `decorations.rs`, `edits.rs`, `json.rs`) |
| The feature plugin | `runtime/plugins/core/lsp/` (`README.md`, `docs/`) |
| Installing servers | `docs/LSP-INSTALL.md`, `runtime/plugins/core/lsp-install/` |
| Plugin author's API | `user-manual/docs/lsp-api.md`, `plugin-api.md` |
| End-user setup, keys, settings | `user-manual/docs/lsp.md` |
| The protocol and HUME's split, as concepts | `docs/learning/lsp.md` |

## Architecture

**Steel on the control plane, Rust on the data plane.** The goals are, in order:
community-extensible features, and a snappy editor on large workloads (rust-analyzer on a
big crate, 100k-line files, bursts of thousands of diagnostics, 1k-item completion lists).

The split is a frequency cut, not a per-feature assignment:

- Work that runs **per user intent** (a command, a server response arriving, a menu
  selection) may be Steel.
- Work that runs **per frame, per keystroke, per scrolled line, or over an unbounded
  collection** (rendering, diagnostic bursts, completion filtering, position mapping
  through edits) is Rust.

Two guardrails hold this through the shape of the API:

- **Bulk data never crosses the Rust↔Steel boundary on a recurring path.** Diagnostics and
  completion items live in Rust stores; Steel sees signals, bounded subsets and policy
  knobs. A one-time ingest per user intent is the exception.
- **Steel never runs on the render path.** Steel writes data stores; Rust providers render
  from them every frame.

Rust therefore ships a platform: transport, document sync, stores, render providers, a
generic request bridge, and widgets (popup, menu, drawer) that Rust renders and Steel
feeds. LSP is the widgets' first client, not their owner. The features are Steel code in
`core:lsp`, written against the same public surface any plugin gets.

**Not supported:** semantic tokens, snippet placeholder navigation, code lens, DAP.

## Steel boundary conventions

- A **pane value** is a buffer paired with the pane it was observed through, or with none.
  Two values compare and hash equal when both halves match; `(buffer-key pane)` narrows to
  the buffer for per-buffer plugin state. A builtin that needs the pane itself raises when
  the value carries none, is stale, or shows another buffer. It never guesses a pane.
- **Positions HUME builtins take and return are 0-based char offsets, char columns and
  lines**, crossing the host traits as plain `usize`. They are addressing units, never
  shown to a user: the shown column is a grapheme column.
- **Wire positions stay inside an undecoded response handle.** Only `goto-location!`, the
  response-shaped setters and the named converters (`lsp-position->char-offset`,
  `lsp-range->char-range`, `lsp-label-offsets->text`, `lsp-locations->places`) read them,
  each in the encoding the handle carries. Steel does no encoding arithmetic.
- **Callbacks take `(err result)`**, with one of the two `#f`.

## Decisions

| Decision | Choice | Reason |
|---|---|---|
| Feature delivery | A generic bridge (`lsp-request!`, `lsp-request-all!`, `lsp-notify!`, `on-lsp-notification`) plus the `core:lsp` plugin, no hardcoded Rust features | Any protocol method is reachable from Steel, and shipping the built-in features on the public surface proves the bridge complete. The Rust platform costs more up front; each further feature is a Steel file. |
| Response representation | Every LSP-shaped value crosses as one opaque JSON handle, read with `json-ref` and its siblings | No deep decode for data a plugin passes straight on. A handle carries its producing server's encoding, so a builtin reads a position's encoding off the value in hand and not off whichever server the pane has now. A value with no such tag is refused, not guessed at. |
| Transport concurrency | std threads and mpsc channels per server, behind the `LspBackend` trait; no async runtime | Transport is never the bottleneck: JSON parsing lands on reader threads, and an incremental `didChange` is proportional to the edit. A runtime would split a synchronous codebase into two concurrency idioms. The cost is hand-rolled request timeouts. |
| Position encoding | Negotiate `utf-8`, fall back to UTF-16 conversion | HUME counts chars and graphemes, so both paths convert. A byte is never assumed to be a column. |
| Server→client requests | Answered in Rust, never surfaced to Steel | JSON-RPC requires a response to every request. Keeping the obligation in Rust means no plugin can cause a server-side timeout. Steel handles notifications only. |
| Pull diagnostics | A server is asked with `textDocument/diagnostic` only until it first publishes; the client does not declare `textDocument.diagnostic` | The capability cannot tell a pull-only server from one that advertises pull and also pushes, and a server that offers both stops pushing once the client declares pull. So the client observes which one a server does. |
| Server crash policy | No auto-restart; `:lsp-restart` only. A stop is remembered per (name, root) until a restart, a registration change or a config reload | A stop is the user's decision, so a registry change or a new buffer does not undo it. Bounded auto-restart is in `docs/ROADMAP.md`. |
| WorkspaceEdit on unopened files | Open each as a buffer; validate every file before committing any | Every mutation goes through the undo tree. A direct file write would be atomic but not undoable. |
| Registration identity | A registration name, ranked by whose behalf it is made (user, then plugin, then HUME's runtime files); last wins within one rank | The user must be able to override a plugin's registration in either order, since a lazy plugin can run before or after `init.scm`. A command the user runs is the user's choice for the session. Last-wins within a rank keeps unregister-then-register working. |
| Per-language server list | A language's servers are the registered entries of the list the highest origin set, in the explicit tier if it has one, else the default tier | One list per origin makes the result independent of call order. The default tier lets an installer write lists that never beat anyone's explicit one. A registration says only how to launch a server; a list says which languages it serves. |
| Attachments and instance lifetime | A buffer's attachments are a function of its language, its path, the registry and the lists. One instance per (registration name, workspace root), alive while some buffer is attached | Recomputing from inputs at a few chokepoints leaves no reaper and no tombstones. Keying by root gives the right `rootUri` when two projects share a session. |
| Routing at send time | Steel states the policy (`#:feature`, `#:to`); Rust picks the servers when the request goes out | A server chosen before an async boundary (a prompt, a debounce) can be stale by the time it is used. `#:to` is for a follow-up to the server an earlier answer came from. |
| Deliveries | One request to N servers is one delivery with N slots; the callback runs once, when the last slot fills | A caller joins nothing by hand, and each slot's error says whether no server could take the request or one failed. |
| Supersede-cancel | Opt-in `#:supersede <key>`, with an opaque key scoped to the buffer | Cancelling by method name would cancel two features' concurrent requests that never raced. A string key would be shared by every plugin. |
| Opaque positions | Positions HUME builds for a request stay opaque until serialized for a server | A buffer's servers may count columns in different encodings, so the bridge encodes per routed server. Each position carries the text version it was built at, so a stale one fails its slot. |
| Fan-out vs pick-first | A feature whose answers combine asks every server; one whose answers conflict asks the first | Completion items, code actions, locations, hints and symbols merge. Hover, signature help, rename and formatting show or edit one thing. |
| Trigger characters | A server's trigger characters live on each buffer's attachment to it | A table kept per server would serve every buffer attached to that server, whatever each buffer's own list says about the feature. |
| Root markers and `languageId` | Properties of the language, not of a server | A language's markers are the same whichever server serves it, and its wire `languageId` can differ from HUME's name (`tsx` is `typescriptreact`). |
