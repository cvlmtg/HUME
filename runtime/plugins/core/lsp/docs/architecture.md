# core:lsp — Architecture

## File layout

`plugin.scm` requires one file per feature area and binds the default keys.
Installing servers is `core:lsp-install`'s job.

| File | Owns | Doc |
|---|---|---|
| `plugin.scm` | Requires, default key bindings | [Default keys](#default-keys) |
| `manifest.scm` | Lazy-activation triggers | [README](../README.md#usage) |
| `lib/helpers.scm` | Capability guards, error reporting, multi-server answers, trigger characters, pane and viewport helpers | this file |
| `lib/locations.scm` | Locations drawer and its refresh session | this file |
| `lib/status.scm` | `:lsp-status`, `:lsp-stop`, `:lsp-restart` | this file |
| `lib/diagnostics.scm` | Diagnostics navigation and drawer, EOL summary, gutter signs | `decorations.md` |
| `lib/inlay.scm` | Inlay hints | `decorations.md` |
| `lib/goto.scm` | Goto family, references | `features.md` |
| `lib/symbols.scm` | Document and workspace symbol pickers | `features.md` |
| `lib/hover.scm` | Hover | `features.md` |
| `lib/sighelp.scm` | Signature help | `features.md` |
| `lib/completion.scm` | Completion | `features.md` |
| `lib/actions.scm` | Code actions | `features.md` |
| `lib/format.scm` | Formatting | `features.md` |
| `lib/rename.scm` | Rename | `features.md` |

## Default keys

`plugin.scm` binds every key in Normal mode. Requests that name a destination (the four
`lsp/goto-*` commands and diagnostic navigation) live under `g`, beside HUME's native line
gotos and structural navigation. Requests that answer with a panel rather than a jump
(references, code actions and the two symbol pickers) live under `z`, the view prefix, the
same shape `core:pickers`' finders use. Rename is `G R`: `G` is a prefix for edit commands
that sit near `g` without being gotos. Hover is `K`, the conventional keyword-lookup key,
used often enough that a prefix would cost a keystroke.

None of these collide with a binding in HUME's default keymap, and the `z` keys beside
them belong to `core:pickers`. Every binding is a two-key sequence or a fresh top-level
key, since a single-key bind over an existing prefix hides the whole subtree under it (see
[`core:vim-keybind`'s README](../../vim-keybind/README.md)). A user's own binding of any of
these keys beats this plugin's, even though the plugin binds them when it activates, after
`init.scm`.

## Request pattern

The request-driven files send a request, transform the response, and call a UI or store
builtin. Goto, references, symbols, completion, code actions, inlay hints and the locations
refresh ask every qualifying server with `lsp-request-all!`, whose callback gets one
`'server`/`'err`/`'result` hash per server, in attachment order. Hover, signature help,
rename, formatting and the code-action follow-ups send `lsp-request!` to one server.
`diagnostics.scm` sends no request: it reads the diagnostics store through
`diagnostics-for-buffer`.

```
lsp-request-all! ─┬─▶ transform response ──▶ UI builtin (show-popup!/show-menu!/goto-location!/picker!)
lsp-request!   ───┘                      └─▶ store builtin (set-inlay-hints!/completion-emit!/…)
```

## Response conventions

- **JSON `null`** is Steel `void`; see the index's
  [Steel pitfalls](../../README.md#steel-pitfalls-worth-knowing-before-you-hit-them).
- **Errors** go through `lsp/report-error!`, which takes a request's `err` hash and logs
  one line: at `'info` for `'unavailable` (no server could take the request, which is a fact
  about the setup) and `'stopped` (the server stopped or crashed first, which is reported on
  its own), at `'error` for every other kind.
- **Choosing a server.** A request for a standard method (`textDocument/hover`, …) is tied
  to that method's feature. `lsp-request!` sends it to the first running server attached
  to the buffer whose list entry admits the feature and whose capabilities advertise it,
  and `lsp-request-all!` to every such server. `#:feature` names the feature of a method
  the editor does not know. When none qualifies, the callback's `err` says why ("hover is
  not supported by rust-analyzer", "… still starting"). A request triggered without the
  user asking (completion, signature help, inlay hints) passes `#:unavailable 'empty` and
  gets an empty answer, not an error, when no server qualifies. A code action's resolve
  and execute requests go `#:to` the `'server` of the `lsp-request-all!` entry the action
  came from. Range formatting picks its server with `lsp-servers` and sends every range
  `#:to` it.
- **Capability fields.** `lsp/cap-field` and `lsp/cap-flag?` read a field off one
  server's `(lsp-capability server #:feature f)` (or `#:method m`), which is `#f`, the
  bare `#t` or an options object (for example a range-formatting provider's
  `rangesSupport`). `lsp/cap-field` returns its caller's default on every kind of miss;
  `lsp/cap-flag?` takes no default and is true only when the field is `#t`. The feature or
  method names the capability, so no plugin spells a `ServerCapabilities` key.

## Shared helpers (`lib/helpers.scm`)

- **Multi-server answers.** `lsp/answered` keeps the entries of an `lsp-request-all!`
  answer that are neither an error nor null, in server order. `lsp/answers` is their
  results, and `lsp/answer-items` concatenates every server's array answer into one list.
  `lsp/report-answer-errors!` reports each entry's error through `lsp/report-error!`, so
  one failing server never hides the others' answers. `lsp/none-answered?` is true when
  every entry is an error, which tells a failure apart from an empty answer, so a caller
  can keep what it already shows.
- **Guards.** `lsp/with-params` calls its body with a pane's `lsp-position-params` or
  `lsp-document-params`, or logs "lsp <what>: this buffer has no file" when they are `#f`.
  `lsp/with-servers` calls its body with a non-empty server list, or logs through
  `lsp/report-unsupported` "<what> is not supported by this buffer's language servers".
- **Trigger characters.** `lsp/setup-trigger-chars!` registers an `on-lsp-attach` handler
  for a feature. For every server that attaches it reads the `triggerCharacters` that
  server advertises for the feature's capability, adds the feature's own extra characters,
  and registers the set for that buffer's attachment to the server; the editor ignores the
  set when the server's list entry excludes the feature or the server does not advertise
  it. The set goes when the buffer detaches, or when a list change fires `on-lsp-attach`
  again. A feature with a handler (signature help) registers through
  `set-attachment-hook-triggers!` and also gets an `on-trigger-char` dispatcher that
  filters on its source name. A feature without one (completion) registers through
  `set-attachment-completion-triggers!`, and the editor invokes the source directly with
  no hook round trip.
- **Pane resolution.** `lsp/resolve-pane` is `(car (buffer-panes pane))`, or `#f` when no
  pane shows the buffer. `buffer-panes` answers the given pane when it still shows its
  buffer, otherwise the focused pane, then the rest of the active tab, then other tabs (see
  the [core plugins index](../../README.md#pane-values-vs-pane-less-values)). It serves
  hooks whose value carries no pane. A caller that already holds a live pane has no reason
  to call it, since re-resolving can pick a different pane on the same buffer.
- **Viewport.** `lsp/visible-lines` is the width of `(viewport-range pane)`, which is
  0-based and end-exclusive, so no `+ 1`. `viewport-range` raises for a pane that does not
  show its buffer, so the caller needs a live pane.
- **Position formatting.** `lsp/format-position` renders a 0-based line and column as
  1-based `line:col`, so a position reads the same in every drawer.
- **Payload rows.** `lsp/payload-rows` pairs each item with its display text, the
  `(display . payload)` rows `show-drawer-list!` and `show-menu!` take, so `on-select`
  gets the item itself.

## Locations drawer (`locations.scm`)

Goto and references open a drawer of locations when a response holds more than one
(references always). `lsp/response-locations` normalizes a response to a list: void gives
`'()`, an array gives its elements, and a single `Location` gives a one-element list.
The wire shape (`{uri, range}` versus `{targetUri, targetRange}`) is decoded by the
`lsp-locations->places` and `lsp-locations->merged-places` builtins, shared with
`goto-location!`, so this plugin never reads a location's wire fields.

- **Rows.** `lsp/place-display` renders `path:line:col` with the path run through
  `path->display`, relative to the working directory when inside it. A location in a
  buffer that is not open carries the server's raw wire character offset instead of a
  grapheme column, because counting graphemes would mean reading a file that may never be
  opened.
- **Where it opens.** The drawer opens on `(focused-pane)`, not the request's invocation
  pane. The goto requests carry no `#:require-focus`, so a slow response still completes
  after the user looked elsewhere, and the drawer appears where the user is.
- **Session.** One session exists at a time, since the drawer is one slot. It holds the
  drawer token, the request's `track-position!` token, the request pane, method
  and params shape, the "nothing found" message, a sequence counter, whether the rows are
  stale, and, keyed by `buffer-key`, each listed buffer's line count and the lines its rows
  sit on. The position is recorded before the request is sent and passed with `#:tracked`,
  so the editor forgets it once the callback has run unless the drawer opened and called
  `keep-tracked-position!`.
- **Refresh.** The rows hold the server's positions, which edits make wrong. An
  `on-text-changed` hook in a listed buffer marks the rows stale when the line count
  differs from the stored one, when a hunk covers a row's line, when a hunk that adds or
  removes lines starts at or above one, or when the hook carries no hunks. The line-count
  test also catches a new reference typed below every row. While the rows are stale, every
  change in a listed buffer counts, since the stored lines belong to an older text than
  the one the hunks start from. Each change that counts calls a 300 ms debounced refresh,
  which repeats the request at `tracked-position-params` with `#:allow-stale #t` under the
  drawer's one `#:supersede` key and swaps the rows, keeping the selected index clamped
  into the new list. The swap stores fresh counts and lines and clears the stale mark. An
  answer with no location HUME can jump to closes the drawer, with the skipped-locations
  warning or the original request's "nothing found" message. An error, or an answer in
  which every server failed, keeps the rows and leaves them stale so the next edit asks
  again. A closed or replaced drawer ends the session.
- **Stale callbacks.** The row callback and each refresh response match the session by its
  drawer token, so the `#f` an outgoing drawer receives when another list replaces it ends
  only its own session. Each refresh also carries a sequence number, and a response for an
  older one is ignored.

## Status commands (`status.scm`)

The three status commands (see the [README](../README.md#commands)) wrap Rust builtins and
keep no Scheme-side state beyond the argument default: every server on the focused buffer.
