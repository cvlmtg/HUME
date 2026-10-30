# Language Server API

The editor functions a plugin uses to register, configure and talk to language servers, rather than writing its own protocol client. Conventions (0-based positions, `!` suffix, pane values, symbols, [JSON handles](#json-handles)) follow the [Plugin API](plugin-api.md#conventions). To set up and use language servers, see [Language Servers](lsp.md).

## Language servers

| Call | Pane | Effect |
|------|------|--------|
| `register-lsp-server!` |  | `(register-lsp-server! name #:command cmd [#:args list] [#:init-options hash] [#:settings hash] [#:env pairs])`{.sig} Register or replace the server called `name` (non-empty, no whitespace). See [Registrations and server lists](#registrations-and-server-lists) |
| `unregister-lsp-server!` |  | `(unregister-lsp-server! name)`{.sig} Queue removing `name`'s registration; removing a missing one is no error. Its buffers detach, and a server left with no buffer stops |
| `set-language-servers!`<br>`set-default-language-servers!` |  | `(set-language-servers! language entries)`{.sig} `(set-default-language-servers! language entries)`{.sig} Set the ordered server list for `language`, or clear it with `#f`. See [Registrations and server lists](#registrations-and-server-lists) |
| `lsp-language-servers` |  | `(lsp-language-servers language)`{.sig} The servers `language` uses, in order, one `(hash 'name n)` each, plus `'only-features` or `'except-features` when its entry limits it. Unregistered names are left out |
| `lsp-server-registered?` |  | `(lsp-server-registered? name)`{.sig} `#t` if a server is registered under `name` |
| `lsp-stop!`<br>`lsp-restart!` |  | `(lsp-stop! target)`{.sig} `(lsp-restart! target)`{.sig} Queue stopping, or stopping then restarting, a server. `target` is a server value (that running server), a pane value (every server attached to that buffer) or a server-name string (every running copy of it) |
| `lsp-show-status!` | focused | `(lsp-show-status! pane)`{.sig} Open the `[lsp-status]` read-only view on `pane` once the call returns, if `pane` is still the one you're looking at; otherwise report an error in `:messages` |
| `lsp-request!` |  | `(lsp-request! pane method params callback [#:feature f] [#:to server] [#:unavailable 'empty] [#:allow-stale #t] [#:supersede key] [#:require-focus #t] [#:tracked token])`{.sig} Send a request to the first running server attached to `pane`'s buffer that can take it, and call `callback` as `(lambda (err result) ...)`. See [Sending a custom request](#sending-a-custom-request) |
| `lsp-request-all!` |  | `(lsp-request-all! pane method params callback [#:feature f] [#:unavailable 'empty] [#:allow-stale #t] [#:supersede key] [#:require-focus #t] [#:tracked token])`{.sig} Send the request to every server that can take it, and call `callback` once, as `(lambda (err results) ...)`, when all have answered. See [Asking every server](#asking-every-server) |
| `lsp-supersede-key` |  | `(lsp-supersede-key)`{.sig} A new key for `#:supersede`, equal only to itself. Works from `init.scm` and a plugin's top level too |
| `lsp-notify!` |  | `(lsp-notify! pane method params [#:feature f] [#:to server])`{.sig} Send a notification, with no callback, to every running server attached to `pane`'s buffer that the method (a standard one is tied to its feature), `#:feature` and `#:to` allow |
| `lsp-servers` |  | `(lsp-servers pane [#:feature f] [#:method m])`{.sig} The servers attached to `pane`'s buffer, as server values, in order. With neither keyword, all of them, running or not; with either, the ones a request of that feature or method would reach now |
| `lsp-server-name` |  | `(lsp-server-name server)`{.sig} The name `server` was registered under |
| `register-lsp-notification-hook!` |  | `(register-lsp-notification-hook! methods proc)`{.sig} Call `proc` as `(lambda (server method params) ...)` for server notifications whose method is `methods` (a string) or one of `methods` (a list of strings). Other methods are logged as unhandled unless a plain `register-hook!` handler takes them. As with `register-hook!`, call it at init or plugin load only; it is removed if your plugin fails to load |
| `lsp-capabilities` |  | `(lsp-capabilities server)`{.sig} A JSON handle onto `server`'s `ServerCapabilities`, or `#f` while it is starting or after `lsp-stop!`. A crashed server keeps its last capabilities |
| `lsp-capability` |  | `(lsp-capability server #:feature f)`{.sig} `(lsp-capability server #:method m)`{.sig} What `server` advertises for the feature, or for the capability the method needs: `#t`, a JSON handle onto the provider's options (its `"triggerCharacters"`, say), or `#f`, under the same conditions as `lsp-capabilities`. Give one of the two keywords |
| `lsp-server-status` |  | `(lsp-server-status)`{.sig} A list with one hash per server instance, in any state, with keys `'name`, `'languages`, `'root`, `'state` (`'starting`, `'running`, `'crashed` or `'dead`) and `'pending` |
| `lsp-document-params` |  | `(lsp-document-params pane)`{.sig} `{"textDocument" {"uri"}}` for the buffer `pane` names, or `#f` if the buffer has no file. Needs no pane, so a value from `(buffers)` works |
| `lsp-position-params` | shown | `(lsp-position-params pane)`{.sig} `{"textDocument" {"uri"} "position" p}` for the primary cursor in `pane`'s own pane, or `#f` if the buffer has no file. Pass it to `lsp-request!` before the buffer changes: a position taken before an edit is refused with an `'unsent` error. See [Positions](#positions) |
| `track-position!` | shown | `(track-position! pane)`{.sig} Remember the primary cursor in `pane`'s own pane through every edit, and return a token for it |
| `tracked-position-params` |  | `(tracked-position-params token)`{.sig} The `lsp-position-params` shape for where the remembered position is now. `#f` when `token` is `#f` or released, when its buffer closed or was replaced, or when the buffer has no file |
| `untrack-position!` |  | `(untrack-position! token)`{.sig} Forget the position; a released token is a no-op |
| `keep-tracked-position!` |  | `(keep-tracked-position! token)`{.sig} Keep a position handed to `lsp-request!` with `#:tracked` after its callback; forget it later with `untrack-position!`. A released token is a no-op |
| `lsp-primary-range-params` | shown | `(lsp-primary-range-params pane)`{.sig} Same shape, with a `"range"` for the primary selection alone |
| `lsp-linewise-ranges-params` | shown | `(lsp-linewise-ranges-params pane)`{.sig} `{"textDocument" {"uri"} "ranges" [...]}`: one range per linewise selection in `pane`'s own pane (a run of touching selections becomes one). `"ranges"` is empty if none are linewise. `#f` only when `lsp-primary-range-params` would return `#f` |
| `lsp-position->char-offset` |  | `(lsp-position->char-offset pane position)`{.sig} The buffer's char offset for a `{"line" "character"}` position from a server's answer, or `#f` when it lands past the buffer's last line, at or past the end of the text |
| `lsp-range->char-range` |  | `(lsp-range->char-range pane range)`{.sig} `(hash 'start-char s 'end-char e)` for a `{"start" ... "end" ...}` range from a server's answer. A column past the end of its line clamps to it, and an end may sit at the end of the document, `(line count, 0)`. `#f` when a position is on a line past the end of the document |
| `lsp-label-offsets->text` |  | `(lsp-label-offsets->text label offsets)`{.sig} The slice of `label` that a `ParameterInformation`-style `(start end)` offset pair from a server's answer names |
| `lsp-location` |  | `(lsp-location uri range)`{.sig} A `Location` made of the `uri` string and a `range` from a server's answer (a `DocumentSymbol`'s `selectionRange`, say), for `goto-location!` and `lsp-locations->places` |
| `lsp-locations->places` |  | `(lsp-locations->places locs)`{.sig} One place hash per `Location`/`LocationLink` in `locs`, in order, or `#f` for one HUME cannot jump to. See [Locations](#locations) |
| `lsp-locations->merged-places` |  | `(lsp-locations->merged-places locs)`{.sig} `(hash 'places l 'skipped n)`: the places without the `#f` entries and without repeats. See [Locations](#locations) |
| `apply-workspace-edit!` |  | `(apply-workspace-edit! pane wsedit [#:expect-generation gen])`{.sig} Apply an LSP `WorkspaceEdit` to every buffer it touches, and return how many buffers changed. See [Workspace edits](#workspace-edits) |
| `set-attachment-hook-triggers!` |  | `(set-attachment-hook-triggers! pane source server feature chars)`{.sig} Like [`set-hook-triggers!`](plugin-api.md#completion), for `pane`'s buffer attached to `server` and `feature`, a symbol such as `'signature-help`. See [Attachment triggers](#attachment-triggers) |
| `set-attachment-completion-triggers!` |  | `(set-attachment-completion-triggers! pane source server feature chars)`{.sig} As `set-attachment-hook-triggers!`, for a `'buffer` completion source's own trigger characters; see [`set-completion-triggers!`](plugin-api.md#completion) |

### Registrations and server lists

A registration only says how to start a server: a language uses it once that language's server list names it. The later registration of a name wins, except that a plugin's own code (its body and hooks) never replaces or removes one made in `init.scm` or by a command you run, whatever the order. Registering a name again after unregistering it in the same run of your code starts a fresh server.

A server list's `entries` are server names or `(hash 'name n 'only-features '(...))` / `(hash 'name n 'except-features '(...))`, with the feature names [Language Servers](lsp.md#choosing-which-server-a-language-uses) lists. A list from `init.scm` or a command you run beats a plugin's, whatever the order; of two lists from the same place, the later replaces the earlier. A `set-default-language-servers!` list applies only while nobody has set a `set-language-servers!` list. `#f` clears only the caller's own list, so the next lower one applies again. Both raise on an unknown feature, on both filters in one entry, or on a name listed twice.

Registrations and lists apply once the code that made them finishes, but `lsp-server-registered?` and `lsp-language-servers` already see the ones made earlier in the same run.

### Positions

Servers count columns in different units, so a position in a server's answer means something only to the server that sent it. HUME keeps track of which server sent each position:

- To send a position, take it from `lsp-position-params`, `tracked-position-params`, `lsp-primary-range-params` or `lsp-linewise-ranges-params`. Each server receives it in its own units. Add your own keys to the params hash, but don't build positions yourself.
- To use a position from an answer, convert it with `lsp-position->char-offset`, `lsp-range->char-range`, `lsp-label-offsets->text` or `lsp-location`. They take positions from a server's answer only: `lsp-position->char-offset`, `lsp-range->char-range` and `lsp-location` raise for anything else, such as a range you built yourself.

### Locations

Each `lsp-locations->places` entry is `(hash 'path p 'line l 'grapheme-col-or-wire c 'buffer b 'location loc)`. `'buffer` is the open buffer the location is in, or `#f` when the file is not open; `'location` is the location itself, for `goto-location!`. When the file is open, the column is a character column; otherwise it is the column the server sent, in its own units. An entry is `#f` for a location HUME cannot jump to: a URI that is not a `file://` one, or a line past the end of the file's open buffer.

Two servers naming one place give two entries. `lsp-locations->merged-places` keeps only the first entry for each path, line and column, leaves out the `#f` entries and counts them in `'skipped`. Use it for places to show or jump to when several servers answered.

### Workspace edits

`apply-workspace-edit!` takes a JSON handle onto a `WorkspaceEdit`, such as an `lsp-request!` response. The edit for `pane`'s buffer maps through `pane`'s selections, and every other buffer is edited for no pane. A file with no edits is left alone, not opened. The number of buffers changed also shows on the statusline unless none did. `#:expect-generation` works as for [`apply-text-edits!`](plugin-api.md#editing-navigation).

### Attachment triggers

`set-attachment-hook-triggers!` and `set-attachment-completion-triggers!` take `server` as `on-lsp-attach` passes it. Nothing is set unless the language's server list lets `server` handle `feature` for the buffer and `server` supports it, so `on-lsp-attach` can call them for every server. They do nothing when the buffer is not attached to `server`.

The triggers go when the buffer detaches from `server`. They also go when a change to the language's server list alters what `server` handles for the buffer; `on-lsp-attach` then fires again, so register there.

## JSON handles

An `lsp-request!` response, `lsp-capabilities`, a `diagnostics-for-buffer` entry's `'raw` field, the `on-lsp-notification` hook's params, `on-completion-accept`'s item and `json-parse`'s result are opaque JSON handles, not decoded hashes. Read them with these calls instead of `hash-ref`/`hash?`/`list?`:

| Call | Effect |
|------|--------|
| `json-parse` | `(json-parse str)`{.sig} Decode a JSON string: an object or array becomes a JSON handle, a scalar a native value, and top-level `null` is void |
| `json-ref` | `(json-ref j seg ...)`{.sig} Look up a path of string keys and integer indices inside handle `j`. A nested object or array comes back as another handle, a scalar as a native string, number or boolean, and `null` as void. Raises, naming the full path, on a missing key, an out-of-range index, or indexing into the wrong kind of container |
| `json-ref-or` | `(json-ref-or j default seg ...)`{.sig} `json-ref`, but `default` in place of raising when the path doesn't resolve or the value there is `null` |
| `json-contains?` | `(json-contains? j seg ...)`{.sig} `#t` if the path resolves; a `null` value at the end still counts as present |
| `json-list` | `(json-list j)`{.sig} The elements of `j`, a JSON array handle, as a Steel list, each converted as `json-ref` would. Raises if `j` isn't an array |
| `json-array?`<br>`json-object?` | `(json-array? v)`{.sig} `(json-object? v)`{.sig} `#t` if `v` is a JSON handle onto an array/object, `#f` for anything else, including a value that isn't a handle |

Values HUME builds itself, rather than decoding from a server's JSON, are ordinary hashes read with `hash-ref`: `lsp-request!`'s `err`, `lsp-server-status`, and a `diagnostics-for-buffer` entry outside its `'raw` field.

## Sending a custom request

Any plugin can call `lsp-request!`, not only `core:lsp`'s commands, to reach a server extension `core:lsp` doesn't use. This command uses rust-analyzer's `rust-analyzer/expandMacro`, which expands the macro under the cursor and returns its generated code:

```scheme
(define-command! "rust-expand-macro" "Show the expansion of the macro under the cursor."
  (lambda (pane)
    (lsp-request! pane "rust-analyzer/expandMacro" (lsp-position-params pane)
      (lambda (err res)
        (cond
          (err (log! 'error (string-append "expand macro: " (hash-ref err 'message))))
          ((void? res) (log! 'info "Not inside a macro"))
          (else (show-popup! pane (json-ref res "expansion"))))))))
```

The pattern has three steps: send a request built from `lsp-position-params` or `lsp-primary-range-params`, transform the server's response, and hand the result to a UI or store function (`show-popup!`, `show-menu!`, `show-drawer-list!`, `apply-text-edits!`, `apply-workspace-edit!`, …).

`err` and `res` are never both set. `res` is a JSON handle: read a field with `json-ref`/`json-contains?`/`json-list`, not `hash-ref`. A `null` response arrives as void, not `#f`.

`err`, when set, is a hash with a `'kind` and a `'message`; read it with `hash-ref`, not `json-ref`. `'kind` says what went wrong:

- `'server` when the server answered with an error (its `'code` is then set too)
- `'timeout` when no answer came in time
- `'unavailable` when no server could take the request: none supports it, or they are all still starting. The message says which, such as "hover is not supported by rust-analyzer" or "rust-analyzer still starting"
- `'stopped` when the server stopped or crashed before it answered
- `'unsent` when the params could not be sent to the server

### Choosing the server

With several servers, the request goes to the first in the language's server order that is running and can answer it. A standard method such as `textDocument/hover` or `textDocument/formatting` is tied to its feature, named as in `set-language-servers!` (`'hover`, `'goto-definition`, `'format`, …). A server whose entry leaves that feature out, or that does not advertise it, is skipped. For any other method, name the feature with `#:feature`; giving it for a standard method is an error. `(lsp-servers pane #:method "textDocument/hover")` lists the servers such a request would reach. The server is chosen when the request is sent.

To reach one server in particular, such as the one an earlier answer came from, get its value from `(lsp-servers pane)` and pass `#:to server`. `(lsp-server-name server)` gives its name, `(lsp-capabilities server)` its capabilities, and `(lsp-capability server #:feature 'completion)` what it advertises for one feature.

### Asking every server

`lsp-request-all!` sends the request to every server the method and `#:feature` allow, and calls back once all have answered. `results` holds one `(hash 'server s 'err e 'result r)` per server, in the buffer's server order. `params` is one hash for all servers, or a non-empty list of `(server . params)` pairs, each server once, giving each its own; the method and `#:feature` still decide which of them get it. It takes the same keywords as `lsp-request!` except `#:to`.

### Keywords

`#:unavailable 'empty` (default `'error`) is for a request the user did not ask for, such as completion or inlay hints. A request no server can take then calls back with no error instead of an `'unavailable` error: a void result for `lsp-request!`, an empty `results` list for `lsp-request-all!`.

`#:supersede key` cancels the pending request filed under the same key: every server it went to gets `$/cancelRequest`, and its callback never fires. Each `(lsp-supersede-key)` call returns a new key that no other plugin's request, nor any of HUME's own, can share. Make one at your plugin's top level and pass it to every request it should replace.

`#:allow-stale #t` runs the callback even if the buffer changed after the request was sent; by default that answer is dropped. Its positions describe the text the request was sent for, but `lsp-position->char-offset`/`lsp-range->char-range` read the buffer as it is now, so after an edit they may land on different text. Before using one, compare `buffer-generation` (with `equal?`) against the value you read when sending, and resend if it moved.

`#:require-focus #t` drops the callback unless `pane` is still the pane you're looking at, showing the same buffer, when the answer arrives. `pane` must carry a pane, not only a buffer.

`#:tracked token` takes a `track-position!` token for the position the request asked about. The token follows edits while the request is pending. It is forgotten once the callback has run (even if it raised) or the request ends without calling it, unless the callback calls `keep-tracked-position!`.
