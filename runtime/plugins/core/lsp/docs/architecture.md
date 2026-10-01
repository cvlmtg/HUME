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
- **Errors** go through `lsp/report-error!`, which takes a hashmap with a `'message` key or
  a bare string such as `"timeout"` and logs one `'error` line.
- **Capability guards** read `(lsp-capabilities pane)`, a `JsonHandle` onto the attached
  server's provider capabilities. `lsp/supports?` is true when the handle exists, contains
  the key, and the value is neither `#f` nor JSON null. A provider can be declared and
  then disabled with `#f`, which differs from never being declared.
  `lsp/guard-capability` runs a thunk when the capability is present and otherwise logs
  `'info` "not supported by <server>". `lsp/cap-field` and `lsp/cap-flag?` read a nested
  field off a capability that is either the bare `#t` or an options object (for example
  `codeActionProvider.resolveProvider` or
  `documentRangeFormattingProvider.rangesSupport`), returning a caller-supplied default on
  every kind of miss.

## Shared helpers (`lib.scm`)

- **Trigger characters.** `lsp/setup-trigger-chars!` registers `on-lsp-attach` and
  `on-lsp-detach` handlers for a feature. On attach it reads the server's
  `triggerCharacters` for the feature's capability, adds the feature's own extra
  characters, and registers the set; on detach it registers an empty set. Both tables are
  keyed by the source name and the server name the attach hook passes, so a second server
  attaching under the same source gets its own entry. A feature with a handler (signature
  help) registers through `set-hook-triggers!` and also gets an `on-trigger-char`
  dispatcher that filters on its source name. A feature without one (completion)
  registers through `set-completion-triggers!`, and the editor invokes the source
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

`:lsp-status` shows every running server and its state, plus attached buffers' diagnostic
counts. `:lsp-stop [lang]` and `:lsp-restart [lang]` stop, or stop and respawn, a running
server, defaulting to the focused buffer's. All three wrap Rust builtins and keep no
Scheme-side state beyond the argument default.
