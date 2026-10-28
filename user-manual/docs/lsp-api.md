# Language Server API

The editor functions a plugin uses to register, configure and talk to language servers, rather than wiring its own protocol client. Conventions (0-based positions, `!` suffix, pane values, symbols, [JSON handles](#json-handles)) follow the [Plugin API](plugin-api.md#conventions).

[Sending a custom request](#sending-a-custom-request) shows these calls working together. To set up and use language servers, see [Language Servers](lsp.md).

## Language servers

| Call | Pane | Effect |
|------|------|--------|
| `register-lsp-server!` |  | `(register-lsp-server! name #:command #:args #:init-options #:settings #:env)`{.sig} Register or replace the server called `name` (non-empty, no whitespace). The later registration wins, except that a plugin's own code (its body and hooks) never replaces one made in `init.scm` or by a command you run, whatever the order. A registration only says how to start the server: a language uses it once that language's server list (`set-language-servers!`) names it |
| `unregister-lsp-server!` |  | `(unregister-lsp-server! name)`{.sig} Queue removing `name`'s registration; idempotent. Its buffers detach, and a server left with no buffer stops. A plugin's own code cannot remove a registration `register-lsp-server!` would not let it replace. Registering the same name again later in the same call starts a fresh server |
| `set-language-servers!`<br>`set-default-language-servers!` |  | `(set-language-servers! language entries)`{.sig} `(set-default-language-servers! language entries)`{.sig} Set the ordered server list for `language`. `entries` lists server names or `(hash 'name n 'only-features '(...))` / `(hash 'name n 'except-features '(...))`; `#f` clears.<br><br>A list from `init.scm` or a command the user runs beats a plugin's, whatever the order. Of two lists from the same place, the later replaces the earlier.<br><br>A `set-default-language-servers!` list applies only while nobody has set a `set-language-servers!` list. `#f` clears only the caller's own list, so the next lower one applies again. Raises on an unknown feature, on both filters in one entry, or on a name listed twice |
| `lsp-language-servers` |  | `(lsp-language-servers language)`{.sig} The servers `language` uses, in order, one `(hash 'name n)` each, plus `'only-features` or `'except-features` when its entry limits it. Unregistered names are left out. Like `lsp-server-registered?`, it reflects a `set-language-servers!` or a registration made earlier in the same call |
| `lsp-server-registered?` |  | `(lsp-server-registered? name)`{.sig} `#t` if a server is registered under `name` |
| `lsp-stop!`<br>`lsp-restart!` |  | `(lsp-stop! target)`{.sig} `(lsp-restart! target)`{.sig} Queue stopping, or stopping then respawning, a server. `target` is a server value (that running server), a pane value (every server attached to that buffer) or a server-name string (every running copy of it) |
| `lsp-show-status!` | focused | `(lsp-show-status! pane)`{.sig} Open the `[lsp-status]` read-only view on `pane` once the call returns, if `pane` is still the one you're looking at; otherwise report an error in `:messages` |
| `lsp-request!` |  | `(lsp-request! pane method params callback #:feature #:to #:unavailable #:allow-stale #:supersede #:require-focus #:tracked)`{.sig} Send a raw request to the first running server attached to `pane`'s buffer that the method, `#:feature` (the feature of a non-standard method) and `#:to` (a server value) allow, chosen when the request is sent.<br><br>`callback` is `(lambda (err result) ...)`. `err` is `#f` or `(hash 'kind k 'message m)`, with `k` one of `'server` (which adds `'code`), `'timeout`, `'stopped`, `'unavailable` or `'unsent`. A real response arrives as a JSON handle; read it with `json-ref`/`json-contains?`/`json-list`, or put it in the list `completion-emit!` takes.<br><br>`#:unavailable 'empty` answers a request no server can take with no error and a void result. `#:supersede key` cancels the pending request under the same key. `#:allow-stale #t` keeps an answer that arrives after the buffer changed. `#:require-focus #t` drops the callback unless you are still looking at `pane`, showing the same buffer. `#:tracked token` hands the request a `track-position!` token.<br><br>[Sending a custom request](#sending-a-custom-request) explains each error kind and keyword |
| `lsp-request-all!` |  | `(lsp-request-all! pane method params callback #:feature #:unavailable #:allow-stale #:supersede #:require-focus #:tracked)`{.sig} Send the request to every server the method and `#:feature` allow, and call back once, as `(lambda (err results) ...)`, when all have answered. `results` holds one `(hash 'server s 'err e 'result r)` per server, in the buffer's server order.<br><br>`params` is one hash for all servers, or a non-empty list of `(server . params)` pairs, each server once, giving each its own. The method and `#:feature` still decide which of them get it; if none does, no server can take the request.<br><br>`#:unavailable 'empty` calls back with an empty `results`, not an error, when no server can take the request. The other keywords work as for `lsp-request!` |
| `lsp-supersede-key` |  | `(lsp-supersede-key)`{.sig} A new key for `#:supersede`, equal only to itself. Works from `init.scm` and a plugin's top level too |
| `lsp-notify!` |  | `(lsp-notify! pane method params #:feature #:to)`{.sig} Send a notification, with no callback, to every running server attached to `pane`'s buffer that the method (a standard one is tied to its feature), `#:feature` and `#:to` allow |
| `lsp-servers` |  | `(lsp-servers pane #:feature #:method)`{.sig} The servers attached to `pane`'s buffer, as server values, in order. With neither keyword, all of them, running or not; with either, the ones a request of that feature or method would reach now |
| `lsp-server-name` |  | `(lsp-server-name server)`{.sig} The name `server` was registered under |
| `register-lsp-notification-hook!` |  | `(register-lsp-notification-hook! methods proc)`{.sig} Call `proc` as `(lambda (server method params) ...)` for server notifications whose method is `methods` (a string) or one of `methods` (a list of strings). Other methods are logged as unhandled unless a plain `register-hook!` handler takes them. As with `register-hook!`, call it at init or plugin load only; it is removed if your plugin fails to load |
| `lsp-capabilities` |  | `(lsp-capabilities server)`{.sig} A JSON handle onto `server`'s `ServerCapabilities` (read with `json-ref`/`json-contains?`), or `#f` while it is starting or after `lsp-stop!`. A crashed server keeps its last capabilities |
| `lsp-capability` |  | `(lsp-capability server #:feature f)`{.sig} `(lsp-capability server #:method m)`{.sig} What `server` advertises for the feature, or for the capability the method needs: `#t`, a JSON handle onto the provider's options (its `"triggerCharacters"`, say), or `#f` when it advertises none, is starting, or was stopped with `lsp-stop!`. A crashed server keeps its last capabilities. Give exactly one of the two keywords |
| `lsp-server-status` |  | `(lsp-server-status)`{.sig} A list with one hashmap per server instance, in any state, with keys `'name`, `'languages`, `'root`, `'state` (`'starting`, `'running`, `'crashed` or `'dead`) and `'pending` |
| `lsp-document-params` |  | `(lsp-document-params pane)`{.sig} `{"textDocument" {"uri"}}` for the buffer `pane` names, or `#f` if the buffer has no file. Needs no pane, so a value from `(buffers)` works |
| `lsp-position-params` | shown | `(lsp-position-params pane)`{.sig} `{"textDocument" {"uri"} "position" p}` for the primary cursor in `pane`'s own pane, or `#f` if the buffer has no file. `p` is a position value that each server receives in its own column units. Pass the hash to `lsp-request!`, as is or with keys added, before the buffer changes: a position taken before an edit is refused with an `'unsent` error |
| `track-position!` | shown | `(track-position! pane)`{.sig} Remember the primary cursor in `pane`'s own pane through every edit, and return a token for it |
| `tracked-position-params` |  | `(tracked-position-params token)`{.sig} The `lsp-position-params` shape for where the remembered position is now. `#f` when `token` is `#f` or released, when its buffer closed or was replaced, or when the buffer has no file |
| `untrack-position!` |  | `(untrack-position! token)`{.sig} Forget the position; a released token is a no-op |
| `keep-tracked-position!` |  | `(keep-tracked-position! token)`{.sig} Keep a position handed to `lsp-request!` with `#:tracked` after its callback; forget it later with `untrack-position!`. A released token is a no-op |
| `lsp-primary-range-params` | shown | `(lsp-primary-range-params pane)`{.sig} Same shape, a `"range"` value for the primary selection alone |
| `lsp-linewise-ranges-params` | shown | `(lsp-linewise-ranges-params pane)`{.sig} `{"textDocument" {"uri"} "ranges" [...]}`: one range value per linewise selection in `pane`'s own pane (a run of touching selections coalesces into one). `"ranges"` is empty if none are linewise. `#f` only when `lsp-primary-range-params` would return `#f` |
| `lsp-position->char-offset` |  | `(lsp-position->char-offset pane position)`{.sig} The buffer's char offset for a wire `{"line" "character"}` position, or `#f` when it lands past the buffer's last line, at or past the end of the text. `position` is a JSON handle from a server response, decoded with the encoding of the server that sent it; anything else raises |
| `lsp-range->char-range` |  | `(lsp-range->char-range pane range)`{.sig} `(hash 'start-char s 'end-char e)` char offsets for a wire `{"start" ... "end" ...}` range. `range` is a JSON handle from a server response, decoded with the encoding of the server that sent it; anything else raises. A column past the end of its line clamps to it, and an end may sit at the end of the document, `(line count, 0)`. `#f` when a position is on a line past the end of the document |
| `lsp-label-offsets->text` |  | `(lsp-label-offsets->text label offsets)`{.sig} The slice of `label` a `ParameterInformation`-style `(start end)` wire offset pair names; `offsets` decodes with the encoding of the server that sent it |
| `lsp-location` |  | `(lsp-location uri range)`{.sig} A `Location` made of the `uri` string and a `range` taken from a language server's answer (a `DocumentSymbol`'s `selectionRange`, say). It decodes with the encoding of the server that sent the range, so it works with `goto-location!` and `lsp-locations->places`. A range you built yourself has no encoding, so it raises |
| `lsp-locations->places` |  | `(lsp-locations->places locs)`{.sig} One entry per raw `Location`/`LocationLink` in `locs`, in order: `(hash 'path p 'line l 'grapheme-col-or-wire c 'buffer b 'location loc)`, each decoding wire positions with the encoding of the server that sent it. An entry is `#f` for a location HUME cannot jump to: a URI that is not a `file://` one, or a line past the end of the file's open buffer.<br><br>`'buffer` is the open buffer the location is in, or `#f` when the file is not open. `'location` is the location itself, for `goto-location!`.<br><br>Two servers naming one place give two entries; `lsp-locations->merged-places` merges them |
| `lsp-locations->merged-places` |  | `(lsp-locations->merged-places locs)`{.sig} `(hash 'places l 'skipped n)`. `l` is `lsp-locations->places` without the `#f` entries, keeping only the first entry for each path, line and column; `n` counts the `#f` entries left out. Use it for places to show or jump to, merging several servers' answers |
| `apply-workspace-edit!` |  | `(apply-workspace-edit! pane wsedit #:expect-generation)`{.sig} Apply an LSP `WorkspaceEdit` (a JSON handle onto one, such as an `lsp-request!` response) to every buffer it touches: the hunk for `pane`'s buffer maps through `pane`'s selections, and every other buffer is edited for no pane. Returns how many buffers changed, also shown on the status line unless none did.<br><br>A file with no edits is left alone, not opened. `#:expect-generation` works as for [`apply-text-edits!`](plugin-api.md#editing-navigation) |
| `set-attachment-hook-triggers!` |  | `(set-attachment-hook-triggers! pane source server feature chars)`{.sig} Like [`set-hook-triggers!`](plugin-api.md#completion), for `pane`'s buffer attached to `server` (a server value, as `on-lsp-attach` passes it) and `feature`, a symbol such as `'signature-help`.<br><br>Nothing is set unless the language's server list lets `server` handle `feature` for the buffer and `server` supports it, so `on-lsp-attach` can call it for every server. It does nothing when the buffer is not attached to `server`.<br><br>The triggers go when the buffer detaches from `server`. They also go when a change to the language's server list alters what `server` handles for the buffer; `on-lsp-attach` then fires again, so register there |
| `set-attachment-completion-triggers!` |  | `(set-attachment-completion-triggers! pane source server feature chars)`{.sig} As `set-attachment-hook-triggers!`, for a `'buffer` completion source's own trigger characters; see [`set-completion-triggers!`](plugin-api.md#completion) |

[Registering a language server](lsp.md#registering-a-language-server) and [Sending a custom request](#sending-a-custom-request) show `register-lsp-server!`, `lsp-request!`, `lsp-request-all!` and `lsp-notify!` with examples.

`lsp-position->char-offset`, `lsp-range->char-range` and `lsp-label-offsets->text` convert LSP wire units (UTF-16 or byte offsets, depending on the server's negotiated encoding) to the editor's char offsets. Use them instead of assuming wire units map 1:1 to chars.

`lsp-locations->places`'s column is a grapheme column when the target has an open buffer. Otherwise it is the location's own wire `character` as is, since refining it would mean reading a file the user may never open.

## JSON handles

An `lsp-request!` response, `lsp-capabilities`, a `diagnostics-for-buffer` entry's `'raw` field, the `on-lsp-notification` hook's params, `on-completion-accept`'s item and `json-parse`'s result are opaque JSON handles, not decoded hashmaps. Read them with these calls instead of `hash-ref`/`hash?`/`list?`:

| Call | Effect |
|------|--------|
| `json-parse` | `(json-parse str)`{.sig} Decode a JSON string: an object/array becomes a JSON handle (read with `json-ref`/`json-contains?`/`json-list`), a scalar crosses natively, and top-level `null` is void |
| `json-ref` | `(json-ref j seg ...)`{.sig} Look up a path of string keys and integer indices inside handle `j`. A nested object or array comes back as another handle, a scalar as a native string, number or boolean, and `null` as void. Errors, naming the full path, on a missing key, an out-of-range index, or indexing into the wrong kind of container |
| `json-ref-or` | `(json-ref-or j default seg ...)`{.sig} `json-ref`, but `default` in place of erroring when the path doesn't resolve or the value there is `null` |
| `json-contains?` | `(json-contains? j seg ...)`{.sig} `#t` iff the path resolves; a `null` value at the end still counts as present |
| `json-list` | `(json-list j)`{.sig} The elements of `j`, a JSON array handle, as a Steel list, each converted to a handle or native value as `json-ref` would. Errors if `j` isn't an array |
| `json-array?`<br>`json-object?` | `(json-array? v)`{.sig} `(json-object? v)`{.sig} `#t` if `v` is a JSON handle onto an array/object, `#f` for anything else, including a non-handle value |

Values HUME builds itself, rather than decoding from server JSON, are ordinary hashmaps read with `hash-ref`: `lsp-request!`'s `err`, `lsp-server-status`, and a `diagnostics-for-buffer` entry outside its `'raw` field. [Sending a custom request](#sending-a-custom-request) shows `err` and `res` handled side by side.

## Sending a custom request

Any plugin can call `lsp-request!`, not just `core:lsp`'s own commands, to reach a server extension the built-in features don't cover. This command uses rust-analyzer's `rust-analyzer/expandMacro`, which expands the macro under the cursor and returns its generated code:

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

`err` and `res` are never both set. Check `err` first and stop on it, as every built-in feature does. `res` is a JSON handle: read a field with `json-ref`/`json-contains?`/`json-list`, not `hash-ref`. A `null` response arrives as void, not `#f`.

`err`, when set, is a hashmap with a `'kind` and a `'message`; read it with `hash-ref`, not `json-ref`. `'kind` says what went wrong:

- `'server` when the server answered with an error (its `'code` is then set too)
- `'timeout` when no answer came in time
- `'unavailable` when no server could take the request: none supports it, or they are all still starting. The message says which, such as "hover is not supported by rust-analyzer" or "rust-analyzer still starting"
- `'stopped` when the server stopped or crashed before it answered
- `'unsent` when the params could not be sent to the server

The built-in features report `'unavailable` and `'stopped` as a plain message and every other kind as an error.

With several servers, the request goes to the first in the language's server order that is running and can answer it. A standard method such as `textDocument/hover` or `textDocument/formatting` is tied to its feature, named as in `set-language-servers!` (`'hover`, `'goto-definition`, `'format`, …). A server whose entry leaves that feature out, or that does not advertise it, is skipped. For any other method, name the feature with `#:feature`; giving it for a standard method is an error. `(lsp-servers pane #:method "textDocument/hover")` lists the servers such a request would reach.

To reach one server in particular, such as the one an earlier answer came from, get its value from `(lsp-servers pane)` and pass `#:to server`. `(lsp-server-name server)` gives its name, `(lsp-capabilities server)` its capabilities, and `(lsp-capability server #:feature 'completion)` what it advertises for one feature.

To ask every server that can answer and get one callback when all have, use `lsp-request-all!`. It takes the same keywords except `#:to`, and can give each server its own params.

A request the user did not ask for (completion, inlay hints) should pass `#:unavailable 'empty`. A request no server can take then calls back with no error instead of an `'unavailable` error: a void result for `lsp-request!`, an empty `results` list for `lsp-request-all!`.

The position in `lsp-position-params` (and the ranges in its siblings) is not a readable line and column: servers count columns in different units, and each receives the position in its own units when the request is sent. Add your own keys to the params hash, but take positions from these functions instead of building them.

`lsp-request!` takes four more keywords for requests that fire repeatedly, or whose answer may arrive after its moment has passed.

`#:supersede key` cancels the pending request filed under the same key: every server it went to gets `$/cancelRequest`, and its callback never fires. Each `(lsp-supersede-key)` call returns a new key that no other plugin's request, nor any of HUME's own, can share. Make one at your plugin's top level and pass it to every request it should replace. Completion uses this to re-request an incomplete list as you type without piling up stale requests.

`#:allow-stale #t` runs the callback even if the buffer changed after the request was sent, for when a slightly out-of-date answer is still useful; by default that answer is dropped. Its positions describe the text the request was sent for, but `lsp-position->char-offset`/`lsp-range->char-range` read the buffer as it is now, so after an edit they may land on different text. Before using one, compare `buffer-generation` (with `equal?`) against the value you read when sending, and resend if it moved.

`#:require-focus #t` drops the callback unless `pane` is still the pane you're looking at, showing the same buffer, when the answer arrives. `pane` must carry a pane, not just a buffer. Hover, signature help and code actions use it so a slow answer never pops up over what you've moved on to, even another split on the same file.

`#:tracked token` takes a `track-position!` token for the position the request asked about. The token follows edits while the request is pending. It is forgotten once the callback has run (even if it raised) or the request ends without calling it, unless the callback calls `keep-tracked-position!`.

A response may carry positions or ranges of its own, such as a related location inside `res`. Convert them with `lsp-position->char-offset`/`lsp-range->char-range` before using them with an editing command. The first returns `#f` for a position past the buffer's last line, at or past the end of the text, the second for a range reaching a line past the end of the document.
