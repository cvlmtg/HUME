# core:lsp — Architecture

## File layout

`plugin.scm` checks that `core:stdlib` is declared, requires one file per feature area,
and binds the default keys. Installing servers is `core:lsp-install`'s job.

| File | Owns | Doc |
|---|---|---|
| `plugin.scm` | Requires, `core:stdlib` check, default key bindings | [README](../README.md#key-layout) |
| `manifest.scm` | Lazy-activation triggers | [README](../README.md#usage) |
| `lib.scm` | Capability guards, error reporting, trigger characters, pane and viewport helpers | this file |
| `locations.scm` | Locations drawer and its refresh session | this file |
| `status.scm` | `:lsp-status`, `:lsp-stop`, `:lsp-restart` | this file |
| `diagnostics.scm` | Diagnostics navigation and drawer, EOL summary, gutter signs | `decorations.md` |
| `inlay.scm` | Inlay hints | `decorations.md` |
| `goto.scm` | Goto family, references | `features.md` |
| `hover.scm` | Hover | `features.md` |
| `sighelp.scm` | Signature help | `features.md` |
| `completion.scm` | Completion | `features.md` |
| `actions.scm` | Code actions | `features.md` |
| `format.scm` | Formatting | `features.md` |
| `rename.scm` | Rename | `features.md` |

## Request pattern

The request-driven files (hover, goto, signature help, completion, code actions,
formatting, rename, inlay hints) send an `lsp-request!`, transform the response, and call
a UI or store builtin. `diagnostics.scm` sends no request: it reads the diagnostics store
through `diagnostics-for-buffer`.

```
lsp-request! ──▶ transform response ──▶ UI builtin (show-popup!/show-menu!/goto-location!)
                                    └─▶ store builtin (set-signs!/set-inlay-hints!/…)
```

## Response conventions

- **JSON `null` decodes to Steel `void`, not `#f`.** Every response handler checks
  `(void? res)` for "no results", never `(not res)`.
- **Errors** go through `lsp/report-error!`, which takes a request's `err` hash and logs
  one line: at `'info` for `'unavailable` (no server could take the request, which is a fact
  about the setup) and `'stopped` (the server stopped or crashed first, which is reported on
  its own), at `'error` for every other kind.
- **Choosing a server.** A request for a standard method (`textDocument/hover`, …) is
  tied to that method's feature, and the editor sends it to the first server attached to
  the buffer that is running, whose list entry admits the feature and whose capabilities
  advertise it. `#:feature` names the feature of a method the editor does not know. When none qualifies, the
  callback's `err` says why ("hover is not supported by rust-analyzer", "… still
  starting"). A request triggered without the user asking (completion, signature help,
  inlay hints) passes `#:unavailable 'empty` and gets an empty answer, not an error, when
  no server qualifies. A follow-up that must reach the server an earlier answer came from (code-action
  resolve and execute, range formatting) picks that server with `lsp-servers` and sends
  `#:to` it.
  A command the user asked for reports an empty answer through `lsp/with-servers`, which
  logs "… is not supported by this buffer's language servers" and calls its body only with servers.
- **Capability fields.** `lsp/cap-field` and `lsp/cap-flag?` read a nested field off one
  server's `(lsp-capability server #:feature f)` (or `#:method m`), which is either the
  bare `#t` or an options object (for example a code-action provider's `resolveProvider`
  or a range-formatting provider's `rangesSupport`), returning a caller-supplied default
  on every kind of miss. The feature or method names the capability, so no plugin spells
  a `ServerCapabilities` key.

## Shared helpers (`lib.scm`)

- **Trigger characters.** `lsp/setup-trigger-chars!` registers an `on-lsp-attach` handler
  for a feature. For every server that attaches it reads the
  `triggerCharacters` that server advertises for the feature's capability, adds the
  feature's own extra characters, and registers the set for that buffer's attachment to
  the server; the editor ignores the set when the server's list entry excludes the
  feature or the server does not advertise it. The set goes when the buffer detaches, or when a list change fires
  `on-lsp-attach` again. A feature with a handler (signature
  help) registers through `set-attachment-hook-triggers!` and also gets an `on-trigger-char`
  dispatcher that filters on its source name. A feature without one (completion)
  registers through `set-attachment-completion-triggers!`, and the editor invokes the source
  directly with no hook round trip.
- **Pane resolution.** `lsp/resolve-pane` is `(car (buffer-panes pane))`, or `#f` when no
  pane shows the buffer. `buffer-panes` answers the given pane when it still shows its
  buffer, otherwise the focused pane, then the rest of the active tab, then other tabs (see
  the [core plugins index](../../README.md#pane-values-vs-pane-less-values)). Inlay hints
  use it for hooks whose value carries no pane. A caller that already holds a live pane
  has no reason to call it, since re-resolving can pick a different pane on the same
  buffer.
- **Viewport.** `lsp/visible-lines` is the width of `(viewport-range pane)`, which is
  0-based and end-exclusive, so no `+ 1`. `viewport-range` raises for a pane that does not
  show its buffer, so the caller needs a live pane. Hover is the only caller, for its
  popup-docking threshold.
- **Position formatting.** `lsp/format-position` renders a 0-based line and column as
  1-based `line:col`. The diagnostics drawer and the locations drawer both use it, so a
  position reads the same in both.

## Locations drawer (`locations.scm`)

Goto and references open a drawer of locations when a response holds more than one
(references always). `lsp/response-locations` normalizes a response to a list: void gives
`'()`, an array gives its elements, and a single `Location` gives a one-element list.
The wire shape (`{uri, range}` versus `{targetUri, targetRange}`) is decoded by the
`lsp-locations->display-parts` builtin, shared with `goto-location!`, so this plugin never
reads a location's wire fields.

- **Rows.** `lsp/location-display` renders `path:line:col` with the path run through
  `path->display`. A location in a buffer that is not open carries the server's raw wire
  character offset instead of a grapheme column, because counting graphemes would mean
  reading a file that may never be opened. With no column, the row is `path:line`.
- **Where it opens.** The drawer opens on `(focused-pane)`, not the request's invocation
  pane. The goto requests carry no `#:require-focus`, so a slow response still completes
  after the user looked elsewhere, and the drawer appears where the user is.
- **Session.** One session exists at a time, since the drawer is one slot. It holds an
  id, the drawer token, the request's `track-position!` token, the request pane, method
  and params shape, the "nothing found" message, a sequence counter, and the last known
  line count of each listed buffer keyed by `buffer-key`. The position is recorded before
  the request is sent and passed with `#:tracked`, so the editor forgets it once the
  callback has run unless the drawer opened and called `keep-tracked-position!`.
- **Refresh.** The rows hold the server's positions, which edits make wrong, so the
  session never keeps them across a line-count change. An `on-text-changed` hook compares
  each listed buffer's line count with the stored one. On a change, a 300 ms debounced
  refresh repeats the request at `tracked-position-params` with
  `#:allow-stale #t #:supersede "lsp-locations"` and swaps the rows, keeping the selected
  index clamped into the new list. An empty answer closes the drawer with the original
  request's "nothing found" message, an error keeps the rows, and a closed or replaced
  drawer ends the session. Edits that keep the line count are ignored, so a row can sit a
  few columns off until the next line-count change.
- **Stale callbacks.** A row callback carries its session's id, and each refresh carries a
  sequence number. The `#f` an outgoing drawer receives when another list replaces it
  ends only the session whose id it carries, and a response for an older sequence is
  ignored.

## Status commands (`status.scm`)

`:lsp-status` shows every running server and its state, plus each attached buffer's servers
and diagnostic counts, and the servers a stop left stopped. `:lsp-stop [name]` and `:lsp-restart [name]` stop, or stop and respawn,
a running server by registration name, defaulting to every server on the focused buffer. All three wrap Rust builtins and keep no
Scheme-side state beyond the argument default.
