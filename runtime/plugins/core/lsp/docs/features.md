# core:lsp — Request-driven features

Each section covers one feature file: what it asks the server, how it decodes the answer,
and what it does with it. The flags every request passes to `lsp-request!` are collected
in [Request flags](#request-flags) at the end. The diagnostics commands and inlay hints
are in `decorations.md`.

## Goto and references

`goto.scm` serves the four goto commands (definition, declaration, type definition,
implementation) and `lsp/references` through one response handler. Each asks every server
offering the feature, and their locations merge into one list in which rows naming the
same path, line and column appear once (`lsp-locations->merged-places`):

1. A server's error is reported; the others' locations still count.
2. No location at all logs "No definition found" (or "No references found"). A location
   HUME cannot jump to is left out and one warning says how many: its URI is not a file,
   such as `jdt://`, or its line is past the end of the file's open buffer. When that
   leaves nothing, the warning is the whole report. The drawer's refresh leaves such
   locations out without warning again.
3. A single location jumps to it, so two servers agreeing on a definition still jump.
4. Several locations open the [locations drawer](architecture.md#locations-drawer-locationsscm).

`lsp/references` always opens the drawer, even for one result, because "where is this
used" expects a list. Its request adds `context.includeDeclaration`. Every jump lands on
`(focused-pane)`, so it goes where the user is when the response arrives. Goto, the
drawer's rows and the symbol pickers all jump through `lsp/jump-to!`, which warns when
`goto-location!` answers `#f` because the line has gone from its file.

## Symbols

`symbols.scm` serves `lsp/document-symbols` and `lsp/workspace-symbols`. Both list their
rows as `name  kind  detail` and jump to the accepted row in `(focused-pane)`.

`lsp/document-symbols` asks every server offering `documentSymbolProvider` and shows one
`picker!` over every server's answers. A `DocumentSymbol[]` answer is walked pre-order, a
symbol's detail being its parent's name and its jump target its `selectionRange`. A
`SymbolInformation[]` answer keeps its own `location` and uses `containerName` as the
detail. Since a `DocumentSymbol` carries no `uri`, its row holds the buffer's uri and the
`selectionRange`, and `lsp-location` pairs them only for the row that is accepted, which
keeps the server's position encoding. No symbol at all logs "No symbols found".

`lsp/workspace-symbols` opens an empty `live-picker!`. Each query, once the debounce
window passes, sends `workspace/symbol` to every server offering the feature with
one `#:supersede` key, so a newer query replaces one still in flight. Every
answer replaces the rows with every server's symbols, and an error or an answer with nothing
in it replaces them with none, so the picker stops refreshing whatever the outcome. An
answer that arrives after the user typed on is dropped, since its token is for the earlier
query. An empty query clears the rows. The detail is the symbol's
`path:line:col`. A symbol whose `location` has no `range` is left out, since the client
does not advertise `workspaceSymbol/resolve` and such a symbol names no place to jump to.
A symbol HUME cannot jump to, such as one in a `jdt://` file or on a line past the
end of an open file, is left out.

## Hover

The response's `contents` is decoded to text: a bare string as is, an array of
`MarkedString`s joined with blank lines, a `{language, value}` `MarkedString` re-fenced as
a code block so the popup's markdown injection can highlight it, and a
`MarkupContent`'s `value` as is. The text is shown as markdown unless the contents is a
`MarkupContent` with `kind: "plaintext"`.

The popup floats near the cursor when its line count is at most a third of the viewport
height and docks at the bottom otherwise. Each `lsp/hover` closes the previous hover popup
before sending its request. An empty response logs "No hover info".

## Signature help

Typing a trigger character the server advertises, or `)`, reaches the plugin through
`on-trigger-char`. `)` closes the popup. Any other trigger character runs a request
debounced by 150 ms, which first checks that the pane is still live, since the pane may
have closed or changed buffer during the debounce window. A null response, an empty
signature list or an error closes the popup.

The server's `activeSignature` and `activeParameter` are clamped into range. A parameter
label is either a string or a `[start, end)` offset pair into the signature's label. The
offsets count code units in the server's negotiated encoding, so the host slices them
(`lsp-label-offsets->text`). The popup has no styling API, so the active parameter is
marked with `⟨…⟩` on a second line. A response with no `activeParameter` shows the label
alone.

## Completion

The plugin registers a completion source, `"lsp"`, and the editor drives it. The source
is a buffer-target source with `#:priority 10` and `#:resolve #t`: its token is the
identifier before the cursor, and `#:resolve` licenses `completionItem/resolve` on accept
because its items come from the buffer's own attached server. The editor calls it on
`Ctrl-Space`, on a server trigger character (registered at attach as the source's own
trigger characters, with no hook round trip), and again after each keystroke while the
last answer was incomplete.

The source answers with an empty list when no server of the buffer offers completion.
Otherwise it asks every server that does with `lsp-request-all!` under one `#:supersede`
key, so a newer request replaces an older one still in flight, and passes every
non-null response to `completion-emit!` as one entry each, so their items merge into one
menu. A server's error is reported; the others' items still show. A server's own trigger
characters invoke the source in the buffers attached to that server.

The rest happens in the editor. Each item's own edit range, or the list's default
`itemDefaults.editRange`, says where its token starts, and the item is filtered against
the text from there to the cursor. Snippet stripping happens as items enter the store.
Accepting an item applies its main edit, its `additionalTextEdits` and
`completionItem/resolve` together, so the plugin has no accept handler.

## Code actions

`lsp/code-actions` asks every server offering code actions, captures the pane and
`(buffer-generation pane)`, and threads both through the menu selection and any
`codeAction/resolve` round trip without re-reading focus, since each round trip is
asynchronous. The menu lists every server's actions in server order; when more than one
server offered some, each title ends with its server's name. A chosen action's resolve
and command go back to the server that offered it.

Each server's `context.diagnostics` echoes the raw wire `Diagnostic` objects in the
primary selection's range that this server published (the `'raw` field of the
diagnostics store entries whose `'server` is it), and `triggerKind` is `1`. Actions with a
truthy `"disabled"` field are dropped from the menu.

Running a chosen action applies its `edit`, then runs its `command`. An action with
neither is resolved once through `codeAction/resolve` when the server advertises
`resolveProvider` (the editor sends no resolve request to a server that does not), and logs "Code action has no edit or command" if the resolved action is
still empty. The resolve step runs at most once, so a server that keeps returning an empty
action cannot loop. A bare legacy `Command` (a string `command` and no `edit`) is passed to
`workspace/executeCommand` as the whole action object. The edit is applied with
`#:expect-generation`, so an edit computed against text that has since changed fails
instead of applying to the wrong text.

## Formatting

`lsp/format-source` and `:format-source` run the same function, which picks ranges from the
selections and hands them to `lsp/format-document`'s implementation. `lsp/format-document` takes
a `done` and optional ranges: with no ranges it formats the whole document, and it calls
`done` once the edits are applied or the attempt has failed (no path, no server, a server
error, stale text). It works on a buffer
no pane shows. Format-on-save is not wired by default; the user manual's "Format on save"
section has the recipe.

A buffer with no path logs a message and formats nothing. Formatting the whole buffer
goes to the first running server with the `format` feature and a
`documentFormattingProvider`; when there is none, the error names why. Formatting ranges
goes to the first server with a `documentRangeFormattingProvider`, and every range of one
command goes to that same server, so the edits joined below all come from one server. No
such server logs a message and formats nothing.

### Selection classification

| Selection set | Result |
|---|---|
| All selections linewise | Formats those ranges. Touching selections coalesce into one range, and disjoint ones stay separate, since an LSP range is one contiguous span |
| None linewise | Formats the whole buffer |
| A mix | Logs "mixed whole-line and partial selections" and formats nothing |

### Range requests

Every range goes to the first server advertising range formatting, so the joined edits all
come from one server.

Several disjoint ranges go out as one `textDocument/rangesFormatting` request (LSP 3.18)
when that server advertises `rangesSupport`. Otherwise each range is its own
`textDocument/rangeFormatting` request, and more than the `format-max-ranges` config value
ranges log a message and format nothing, the same refusal a mixed selection gets. The cap
keeps a stray select-all-matches with hundreds of cursors from bursting hundreds of
requests at the server, and refusing beats narrowing to one selection without a word.
A server with `rangesSupport` takes every range in one request, so the cap doesn't apply.

The per-range fan-out keeps three boxes: requests still in flight, edits collected so far,
and whether it has aborted. All edits are applied together when the in-flight count
reaches zero. After an error the count never reaches zero, so no partial format is applied.
The abort flag only suppresses duplicate error lines when several ranges fail. Responses
can land in any order.

## Rename

`lsp/rename` prompts with the symbol under the cursor prefilled, captures the buffer's
generation when the name is accepted, and sends `textDocument/rename`. A null response
logs "Nothing to rename". Otherwise the workspace edit is applied with
`#:expect-generation`. A buffer with no server that supports renaming gets a "not supported"
message before any prompt opens. The request itself names the `rename-symbol` feature, so
it goes to whichever server supports it when the name is accepted.

## Request flags

| Request | Routed by | `#:require-focus` | `#:allow-stale` | `#:supersede` |
|---|---|---|---|---|
| `lsp/hover` | `'hover` | yes | yes | |
| `lsp-goto-*` (four), every server | `'goto-definition` and siblings | | | |
| `lsp/references`, every server | `'goto-reference` | yes | | |
| Signature help | `'signature-help` | yes | | |
| `lsp/document-symbols`, every server | `'document-symbols` | yes | | |
| `lsp/workspace-symbols`, every server | `'workspace-symbols` | | yes | one key |
| Code actions (menu request), every server | `'code-action` | yes | | |
| `codeAction/resolve`, `workspace/executeCommand` | `#:to` the server that offered the action | | yes | |
| Completion, every server | `'completion` | | | one key |
| Whole-buffer formatting | `'format` and `documentFormattingProvider` | | yes | |
| Range formatting (both shapes) | `#:to` the first range-formatting server | | yes | |
| Rename | `'rename-symbol` | | yes | |
| Inlay hints, every server | `'inlay-hints` | | | one key per buffer |
| Locations drawer refresh | the drawer's own feature | | yes | one key |

"Routed by" is what picks the server among a buffer's servers when the request is sent:
a feature name sends it to the first running server whose list entry admits that feature
and that advertises it, or to every such server where the row says so, and `#:to` sends
it to one server chosen earlier.

`#:require-focus` drops the callback unless the invoking pane is still the focused pane
and still shows the same buffer when the response arrives. It fits requests whose result
is a popup or menu anchored to that pane. `#:allow-stale` runs the callback even if the
buffer has changed since the request was sent. It fits requests whose result is checked
another way: edits carry `#:expect-generation`, and the locations refresh re-requests at
the tracked position. The goto commands use neither, so a navigation the user asked for
completes even if they looked elsewhere while waiting. `#:supersede` cancels the
pending request under the same key. Each key comes from `lsp-supersede-key`, minted once at the
plugin's top level or once per buffer for inlay hints, so no other caller can cancel these
requests.
