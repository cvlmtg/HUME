# Language Server API

Reference for the builtins a plugin can call to register, configure, and talk to language servers. An LSP plugin registers and drives a server through these rather than wiring its own protocol client. Conventions (0-based positions, `!` suffix, pane values, symbols, [JSON handles](#json-handles)) are those of the [Plugin API](plugin-api.md#conventions).

Most of this page is lookup tables of signatures and one-line effects; [Sending a custom request](#sending-a-custom-request) walks through using them together. For setting up and using language servers, see [Language Servers](lsp.md).

## Language servers

| Call | Effect |
|------|--------|
| `(register-lsp-server! name #:command #:args #:init-options #:settings #:env)` | Register (or replace) the server called `name`. A registration only says how to start the server; a language uses it once that language's list names it (`set-language-servers!`). `name` is non-empty with no whitespace |
| `(unregister-lsp-server! name)` | Queue removing `name`'s registration; its buffers detach and a server left with no buffer stops; idempotent. Registering the same name again right after, in the same call, starts a fresh server |
| `(set-language-servers! language entries)`, `(set-default-language-servers! language entries)` | Set the ordered server list for `language`: `entries` is a list of server names or `(hash 'name n 'only-features '(...))` / `(hash 'name n 'except-features '(...))`, or `#f` to clear. The first is the user's list and always wins; the second is a plugin's default, used only while there is no user list. Raises on an unknown feature, on both filters in one entry, or on a name listed twice |
| `(lsp-language-servers language)` | The servers `language` uses, in order: one `(hash 'name n)` each, with `'only-features` or `'except-features` when its entry limits it. A name that is not registered is left out |
| `(lsp-server-registered? name)` | `#t` if a server is registered under `name` |
| `(lsp-stop! target)`, `(lsp-restart! target)` | Queue stopping / stopping-then-respawning a server: `target` is a pane value (every server attached to that buffer) or a server-name string (every running copy of it) |
| `(lsp-show-status! pane)` | Open the `[lsp-status]` read-only view, only while `pane` is still the one you're looking at |
| `(lsp-request! pane method params callback #:feature #:to #:unavailable #:allow-stale #:supersede #:require-focus #:tracked)` | Send a raw request to one server attached to `pane`'s buffer: the first that is running and that the method (a standard one such as `textDocument/hover` is tied to its feature, as named in `set-language-servers!`), `#:feature` (the feature of any other method; naming one for a standard method is an error) and `#:to` (a server value) allow. The choice is made when the request is sent. `callback` is `(lambda (err result) ...)`. `err` is `#f` or `(hash 'kind k 'message m)`: `k` is `'server` (the server's own error, with its `'code` too), `'timeout`, `'stopped` (the server stopped or crashed before it answered), `'unavailable` (no server could take the request: "hover is not supported by rust-analyzer", "rust-analyzer still starting") or `'unsent` (the params could not be sent). `#:unavailable 'empty` makes a request no server can take call back with no error and a void result instead, for a request the user did not ask for. A real response delivers as a JSON handle; read it with `json-ref`/`json-contains?`/`json-list`, or put it in the list `completion-emit!` takes. `#:supersede key` cancels the request still waiting under the same key, without calling it back. `#:allow-stale #t` calls back even if the buffer has changed since the request was sent; by default that answer is dropped. `#:require-focus #t` drops the callback unless `pane` is still the exact pane you were looking at, still showing the same buffer, when the response arrives. It needs `pane` to carry a pane, not just a buffer. `#:tracked token` hands the request a `track-position!` token: it is forgotten once the callback has run, even if it raised, or once the request ends without calling it, unless the callback calls `keep-tracked-position!` |
| `(lsp-request-all! pane method params callback #:feature #:unavailable #:allow-stale #:supersede #:require-focus #:tracked)` | Send the request to every server the method and `#:feature` allow, and call `callback` once, as `(lambda (err results) ...)`, when all of them have answered: `results` holds one `(hash 'server s 'err e 'result r)` per server, in the buffer's server order. `params` is one hash for all of them, or a non-empty list of `(server . params)` pairs, each server once, giving each named server its own; the method and `#:feature` still decide whether each is sent it, and when none is the request is one no server can take. `#:unavailable 'empty` calls back with an empty `results` when no server can take the request, instead of an error. `#:allow-stale`, `#:supersede`, `#:require-focus`, and `#:tracked` work as for `lsp-request!` |
| `(lsp-notify! pane method params #:feature #:to)` | Fire-and-forget notification to every running server attached to `pane`'s buffer that the method (a standard one is tied to its feature), `#:feature` and `#:to` allow, no callback |
| `(lsp-servers pane #:feature #:method)` | The servers attached to `pane`'s buffer, as server values, in order. With neither keyword, all of them, running or not; with either, the ones a request of that feature or method would reach now |
| `(lsp-server-name server)` | The name `server` was registered under |
| `(register-lsp-notification-hook! methods proc)` | Call `proc` as `(lambda (server method params) ...)` only for server notifications whose method is `methods` (a string) or one of `methods` (a list of strings), so one `proc` can serve several methods. Any other method is logged as unhandled, unless a plain `register-hook!` handler takes it. Like `register-hook!`: init or plugin load only, and removed if your plugin fails to load |
| `(lsp-capabilities server)` | A JSON handle onto `server`'s `ServerCapabilities` (read with `json-ref`/`json-contains?`), or `#f` while it is starting or after `lsp-stop!`. A crashed server keeps answering with its last capabilities |
| `(lsp-capability server #:feature f)`, `(lsp-capability server #:method m)` | What `server` advertises for the feature, or for the capability the method needs: `#t`, a JSON handle onto the provider's options (for example its `"triggerCharacters"`), or `#f` when it advertises none, is starting, or was stopped with `lsp-stop!` (a crashed server keeps its last capabilities). Give exactly one of the two keywords |
| `(lsp-server-status)` | List of hashmaps with keys `'name`, `'languages`, `'root`, `'state` (`'starting`, `'running`, `'crashed`, or `'dead`), and `'pending`, one per server instance in any state, crashed ones included |
| `(lsp-document-params pane)` | `{"textDocument" {"uri"}}` for the buffer `pane` names, or `#f` if the buffer has no file. Needs no pane, so a value from `(buffers)` works |
| `(lsp-position-params pane)` | `{"textDocument" {"uri"} "position" p}` for the primary cursor in `pane`'s own pane, or `#f` if the buffer has no file. `p` is a position value each server receives in its own column units when the request is sent; pass the hash to `lsp-request!` as is, or with keys added, before the buffer changes: a position taken before an edit is refused with an `'unsent` error |
| `(track-position! pane)` | Remember the primary cursor in `pane`'s own pane through every edit, and return a token for it |
| `(tracked-position-params token)` | The `lsp-position-params` shape for where the remembered position is now, or `#f` if it was released, its buffer closed or was replaced, or the buffer has no file. A `#f` or released token answers `#f` |
| `(untrack-position! token)` | Forget the position; a released token is a no-op |
| `(keep-tracked-position! token)` | Keep a position handed to `lsp-request!` with `#:tracked` after its callback; forget it later with `untrack-position!`. A released token is a no-op |
| `(lsp-primary-range-params pane)` | Same shape, a `"range"` value for the primary selection alone |
| `(lsp-linewise-ranges-params pane)` | `{"textDocument" {"uri"} "ranges" [...]}`: one range value per linewise selection in `pane`'s own pane (a run of touching selections coalesces into one), `"ranges"` empty if none are linewise; `#f` only for the same reasons `lsp-primary-range-params` returns `#f` |
| `(lsp-position->char-offset pane position)` | The buffer's char offset for a wire `{"line" "character"}` position, a JSON handle from a server response (decoded with the encoding of the server that sent it; anything else raises), or `#f` when it lands past the buffer's last line |
| `(lsp-range->char-range pane range)` | `(hash 'start-char s 'end-char e)` char offsets for a wire `{"start" ... "end" ...}` range, a JSON handle from a server response (decoded with the encoding of the server that sent it; anything else raises). A column past the end of its line clamps to it, and an end may sit at the end of the document, `(line count, 0)`. `#f` when a position is on a line past the end of the document |
| `(lsp-label-offsets->text label offsets)` | The slice of `label` a `ParameterInformation`-style `(start end)` wire offset pair names; `offsets` decodes with the encoding of the server that sent it |
| `(lsp-location uri range)` | A `Location` made of the `uri` string and `range`, a range taken from a language server's answer (a `DocumentSymbol`'s `selectionRange`, say). It decodes with the encoding of the server that sent the range, so it works with `goto-location!` and `lsp-locations->places`. A range you built yourself raises, since it has no encoding |
| `(lsp-locations->places locs)` | One entry per raw `Location`/`LocationLink` in `locs`, in order: `(hash 'path p 'line l 'grapheme-col-or-wire c 'buffer b 'location loc)`, each decoding wire positions with the encoding of the server that sent it, or `#f` for a location HUME cannot jump to: a URI that is not a `file://` one, or a line past the end of the file's open buffer. `'buffer` is the open buffer the location is in, or `#f` when the file is not open, and `'location` is the location itself, for `goto-location!`. Entries are not merged, so two servers naming one place give two entries; `lsp-locations->merged-places` merges them |
| `(lsp-locations->merged-places locs)` | `(hash 'places l 'skipped n)`: `l` is `lsp-locations->places` without the `#f` entries and with only the first entry for each path, line and column, and `n` counts the `#f` entries left out. Use it for a list of places to show or jump to, where several servers' answers merge |
| `(apply-workspace-edit! pane wsedit #:expect-generation)` | Apply an LSP `WorkspaceEdit` (a JSON handle onto one, e.g. straight from an `lsp-request!` response) across every buffer it touches, mapping the hunk for the buffer `pane` shows through `pane`'s selections and editing every other buffer for no pane; logs how many buffers changed and returns that count. `#:expect-generation` works as for [`apply-text-edits!`](plugin-api.md#editing-navigation) |
| `(set-attachment-hook-triggers! source pane server feature chars)` | Like [`set-hook-triggers!`](plugin-api.md#completion), for `pane`'s buffer attached to `server`, a server value as `on-lsp-attach` passes it, and a feature symbol such as `'signature-help`. Nothing is set unless the language's server list lets `server` handle `feature` for the buffer and `server` supports it, so `on-lsp-attach` can call this for every server. The set goes when the buffer detaches from the server, and when a change to the language's server list alters what `server` handles for the buffer (`on-lsp-attach` then fires again, so register there). Does nothing when the buffer is not attached to `server` |
| `(set-attachment-completion-triggers! source pane server feature chars)` | As `set-attachment-hook-triggers!`, for a `'buffer` completion source's own trigger characters; see [`set-completion-triggers!`](plugin-api.md#completion) |

`register-lsp-server!`, `lsp-request!`, `lsp-request-all!` and `lsp-notify!` are covered with examples in [Registering a language server](lsp.md#registering-a-language-server) and [Sending a custom request](#sending-a-custom-request). `lsp-position->char-offset`/`lsp-range->char-range`/`lsp-label-offsets->text` convert LSP wire units (UTF-16 or byte offsets, depending on the server's negotiated encoding) to editor-native char offsets. Always go through these rather than assuming a 1:1 mapping. `lsp-locations->places`'s column is an exact grapheme column when the target has an open buffer; otherwise it's the location's own wire `character` verbatim, since refining it would mean reading a file the user may never open.

## JSON handles

An `lsp-request!` response, `lsp-capabilities`, a `diagnostics-for-buffer`
entry's `'raw` field, the `on-lsp-notification` hook's params, `on-completion-accept`'s
item, and `json-parse`'s result are all opaque JSON handles rather than
decoded hashmaps. Read one with these instead of `hash-ref`/`hash?`/`list?`:

Other values stay ordinary hashmaps: `lsp-request!`'s `err`, `lsp-server-status`,
and a `diagnostics-for-buffer` entry itself (outside its `'raw` field) are
built by HUME, not decoded from server JSON, and read with `hash-ref` as
usual; see [Sending a custom request](#sending-a-custom-request) for
the `err`/`res` distinction in practice.

| Call | Effect |
|------|--------|
| `(json-parse str)` | Decode a JSON string: an object/array becomes a JSON handle (read with `json-ref`/`json-contains?`/`json-list`), a scalar crosses natively, and top-level `null` is void |
| `(json-ref j seg ...)` | Look up a path of string keys / integer indices inside handle `j`. A nested object or array field comes back as another handle; a scalar field comes back as a native string/number/boolean; a `null` field comes back as void. Errors, naming the full path, on a missing key, an out-of-range index, or indexing into the wrong container kind |
| `(json-ref-or j default seg ...)` | `json-ref`, but `default` in place of erroring when the path doesn't resolve or the value there is `null` |
| `(json-contains? j seg ...)` | `#t` iff the path resolves; a `null` value at the end still counts as present |
| `(json-list j)` | `j`, a JSON array handle, as a Steel list of its elements (each one funneled through the same handle/native-value rule as `json-ref`). Errors if `j` isn't an array |
| `(json-array? v)`, `(json-object? v)` | `#t` if `v` is a JSON handle onto an array/object, `#f` for anything else (including a non-handle value) |

## Sending a custom request

`lsp-request!` isn't limited to `core:lsp`'s own commands: any plugin can call it to reach
a server extension the built-in feature set doesn't cover. This is how you'd add a command
for rust-analyzer's `rust-analyzer/expandMacro`, which expands the macro under the cursor and
returns its generated code:

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

The shape is always the same three steps: send a request built from `lsp-position-params` or
`lsp-primary-range-params`, transform the server's response, and hand the result to a UI or store
builtin (`show-popup!`, `show-menu!`, `show-drawer-list!`, `apply-text-edits!`,
`apply-workspace-edit!`, …). `err` and `res` are never both set. Check `err` first and stop
on it, the way every built-in feature does. `res` is a JSON handle: read a field with
`json-ref`/`json-contains?`/`json-list`, not `hash-ref`; a `null` response arrives as void, not `#f`.
`err`, when set, is a hashmap with a `'kind` and a `'message`; read it with `hash-ref`, not
`json-ref`. `'kind` says what went wrong: `'server` when the server answered with an error (its
`'code` is then set too), `'timeout` when no answer came in time, `'unavailable` when no server
could take the request (none supports it, or they are all still starting), `'stopped` when the
server stopped or crashed before it answered, and `'unsent` when the params could not be sent to
the server. The built-in features report `'unavailable` and `'stopped` as a plain message and
every other kind as an error.

When a buffer has several servers, the request goes to the first one in the language's server
order that is running and can answer it. A request for a standard method such as
`textDocument/hover` or `textDocument/formatting` is tied to that method's feature, using the same
feature names as `set-language-servers!` (`'hover`, `'goto-definition`, `'format`, …): a server
whose entry leaves that feature out, or that does not advertise it, is passed over. For any other
method, name the feature with `#:feature`; giving it for a standard method is an error. `(lsp-servers pane #:method "textDocument/hover")` lists
the servers such a request would reach. To reach one server in particular, such as the one an earlier
answer came from, get its value from `(lsp-servers pane)` and pass `#:to server`.
`(lsp-server-name server)` gives its name, `(lsp-capabilities server)` its capabilities, and
`(lsp-capability server #:feature 'completion)` what it advertises for one feature.

`lsp-request-all!` asks every server that can answer and calls back once, when all of them have,
with `(err results)`: one `(hash 'server s 'err e 'result r)` per server, in order. It takes the same
keywords except `#:to`; its params can also be a list of `(server . params)` pairs, giving each
named server its own. That list must not be empty or name a server twice, and the method and
`#:feature` still decide whether each of those servers is sent the request. When none of them
can take it, the request counts as one no server can take.

A request the user did not ask for (completion, inlay hints) should pass `#:unavailable 'empty`:
a request no server can take then calls back with no error and an empty result, instead of an
`'unavailable` error.

The position in `lsp-position-params` (and the ranges in its siblings) is not a line and column
pair you can read: servers count columns in different units, so each server receives the position
in its own units when the request is sent. Add your own keys to the params hash, but take the
position from these builtins rather than building one.

`lsp-request!` also takes four keyword args for requests that fire more than once, or whose answer might arrive after the moment it was asked for has passed. `#:supersede "<key>"` cancels the caller's own previous still-pending request filed under the same key (every server it went to gets `$/cancelRequest` and the old callback never fires), which is how completion's re-request of an incomplete list avoids piling up stale requests as you type. `#:allow-stale #t` lets the callback run even if the buffer has changed since the request was sent, for requests where a slightly-out-of-date answer is still useful. `#:require-focus #t` drops the callback entirely unless the exact pane you called it from is still the one you're looking at, still showing the same buffer, by the time the answer arrives. Hover, signature help, and code actions use it, so a slow answer never pops up over whatever you've moved on to, even if that's just a different split on the same file. `#:tracked token` takes a `track-position!` token for the position the request was asked about: the token follows edits made while the request is pending, and is forgotten once the callback has run, unless the callback calls `keep-tracked-position!`.

A server's response sometimes carries its own position or range rather than the one you sent, such as a related location returned inside `res`, say. Convert it back into a plain buffer offset with `lsp-position->char-offset`/`lsp-range->char-range` before using it with any editing command; `lsp-position->char-offset` returns `#f` for a position at or past the end of the text, and `lsp-range->char-range` for a range reaching a line past the end of the document.
