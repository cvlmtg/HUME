# core:lsp — Architecture

## File layout

One `plugin.scm` entry `require`s a file per feature area, plus `lib.scm` (shared
helpers), `registration.scm` (catalog, receipts, the install scan), and `servers.scm`
(install/uninstall).

| File | Owns | Doc |
|---|---|---|
| `lib.scm` | Capability checks, error reporting | this file |
| `locations.scm` | Locations drawer and its refresh session | this file |
| `registration.scm` | Seeded catalog, receipt/path helpers, the registration scan | `servers.md` |
| `servers.scm` | Install/uninstall pipeline, install lock, discovery hint | `servers.md` |
| `diagnostics.scm` | Diagnostics navigation, EOL summary, gutter signs | `decorations.md` |
| `inlay.scm` | Inlay hints | `decorations.md` |
| `goto.scm` | Goto-definition family, references | `features.md` |
| `hover.scm` | Hover | `features.md` |
| `sighelp.scm` | Signature help | `features.md` |
| `completion.scm` | Completion | `features.md` |
| `actions.scm` | Code actions | `features.md` |
| `format.scm` | Formatting | `features.md` |
| `rename.scm` | Rename | `features.md` |

## Response conventions

Every feature file shares these:

- **JSON `null` decodes to Steel `void`, not `#f`**: every response handler in this
  plugin checks `(void? res)` for "no results", never `(not res)`.
- **`lsp/report-error!`** takes either a `(hash 'code 'message)` hashmap or the bare string
  `"timeout"` and logs one `'error` line either way.
- **Capability guards** (`lsp/supports?`, `lsp/guard-capability`) read
  `(lsp-capabilities pane)`, a `JsonHandle` onto the buffer's attached server's provider
  capabilities. `lsp/supports?` treats a capability as present only when the handle
  exists, contains the key, and that key isn't explicitly `#f`. A provider capability
  can be declared and then disabled with `#f`, which is different from never being
  declared. `lsp/cap-field`/`lsp/cap-flag?` read a nested field off a capability that can
  be the bare `#t` or an options object (e.g. `codeActionProvider.resolveProvider`,
  `completionProvider.triggerCharacters`, `documentRangeFormattingProvider.rangesSupport`),
  returning a caller-supplied default on every kind of miss alike.

## Shared helpers (`lib.scm`)

- **Trigger-char lifecycle**: `lsp/setup-trigger-chars!` wires `on-lsp-attach`/
  `on-lsp-detach` for a feature (completion, signature help), registering the attached
  chars through whichever table the feature actually needs: signature help (which passes
  a handler) goes through `set-hook-triggers!` and gets `on-trigger-char` wired too;
  completion (no handler) goes through `set-completion-triggers!` instead; the
  editor invokes that source directly against its own trigger chars, no hook round trip.
  Both tables are keyed `(source, language)`, so a second language attaching under the
  same source name gets its own entry rather than clobbering the first.
- **Pane resolution**: `lsp/resolve-pane` answers the focused pane if it shows the
  buffer, else the first pane on the active tab, else any other; `#f` if the buffer isn't
  shown anywhere (see the
  [core plugins index](../../README.md#pane-values-vs-pane-less-values)). For a hook
  whose own value carries no pane (`on-diagnostics-changed`, `on-text-changed`, a
  `(buffers)` element) to make an explicit choice before calling a pane-needing builtin. A
  caller already holding a real pane (`on-viewport-change`, `on-trigger-char`, a
  command's own leading pane) has no reason to call this. Re-resolving risks silently
  picking a *different* pane on the same buffer.
- **Viewport**: `lsp/visible-lines` wraps the synchronous `viewport-range` builtin,
  which is 0-based end-exclusive, so the visible-line count is just the range's width (no
  `+ 1`). `viewport-range` raises rather than answering `#f` for a pane that doesn't show
  its buffer, so every caller either already holds a real, live pane (hover and
  signature help both pass their own request's invocation pane, confirmed still focused
  by `#:require-focus`) or resolves one explicitly first via `lsp/resolve-pane` (inlay
  hints, for the hooks whose own pane value carries none). Its two callers are hover's
  popup-docking threshold and inlay hints' refresh trigger.
- **Location display**: a raw `Location`/`LocationLink` hashmap's `{uri, range}`-vs-
  `{targetUri, targetRange}` shape dispatch lives in one place, shared by
  `goto-location!` (the jump) and `lsp-locations->display-parts` (the drawer row);
  nothing in this plugin reads a location's wire fields directly. Every "L:C" position
  HUME shows a user goes through the one `lsp/format-position` formatter, so
  `:diagnostics`'s drawer rows and the goto/references drawer agree on what a position
  reads as. `lsp/location-display` additionally collapses `~` and strips a Windows UNC
  prefix (the only formatting still done here) and falls back to bare `path:line` when
  there's no column. The one exception: a goto/references target with no open buffer
  renders the location's own raw wire character offset rather than reading the file to
  convert it to a grapheme column, since the file may never be opened and counting
  graphemes in it isn't worth a disk read. `lsp/show-locations!` (`locations.scm`) opens the drawer on
  `(focused-pane)`, not the request's own invocation pane: the goto family skips
  `#:require-focus` so a slow response still completes after the user looked elsewhere,
  and the drawer opens wherever the user is once it lands. Nothing on that path reads the
  invocation pane live: the request records its position with `track-position!` before
  it is sent and hands it over with `#:tracked`, so the editor forgets it once the
  callback is done, whatever the outcome, unless the drawer opened and called
  `keep-tracked-position!`.
- **Locations session**: the drawer's rows hold the server's wire positions, which edits
  make wrong, so the plugin never keeps them across an edit. `locations.scm` keeps one
  session (the drawer is one slot): the request, the `track-position!` token recorded when
  it was sent, and each listed buffer's last line count, keyed by `buffer-key` and
  found through the `'buffer` key of `lsp-locations->display-parts` because matching URIs
  to buffers in Scheme is not reliable. An `on-text-changed` hook compares line counts and
  a debounced refresh re-asks at `tracked-position-params`. A row callback carries its
  session's id, so the `#f` an outgoing drawer receives when another list replaces it
  cannot end the new session.
