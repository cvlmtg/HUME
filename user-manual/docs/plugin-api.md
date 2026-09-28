# Plugin API

Reference for every function a plugin or `init.scm` can call directly, as opposed to a command reached through a key binding or [`call!`](plugins.md#calling-other-commands). Two layers make up this surface:

- **Builtins**: native to the editor, always available, called as plain Scheme: `(buffer-text pane)`, `(bind-key! ...)`. Some are thin Scheme wrappers (keyword arguments, defaults) over a Rust primitive; the wrapper is what's documented here.
- **[Standard Library](standard-library.md)**: `core:stdlib`, an optional bundled *plugin*. Its commands are reached through `call!`, like any other plugin's: `(call! "stdlib/find" pred? lst)`.

This page is a lookup reference: tables of signatures and one-line effects. For narrative walkthroughs and worked examples, see [Plugins](plugins.md), [Language Servers](lsp.md), [Configuration](configuration.md), and the other pages linked throughout.

## Conventions

- A function whose call changes something (editor state, a registration, a process, a file) ends in `!`. Reads and functions that only build a value, like `debounce`, don't.
- Lines, columns, and char offsets are 0-based everywhere. Add 1 only when showing a line number to the user.
- A value from a fixed set of names (a mode, a hook name, a log level) is a symbol, like `'insert`. Compare symbols with `equal?`: Steel's `eq?` checks identity, so a symbol the editor hands you is never `eq?` to one you wrote.

## Settings & statusline

| Call | Effect |
|------|--------|
| `(set-option! key value)` | Set a global option |
| `(set-buffer-option! pane key value)` | Set an option on `pane`'s buffer only |
| `(get-option key)` | Read an option's global value, ignoring any buffer override |
| `(get-buffer-option pane key)` | Read `pane`'s buffer's effective value: its own override if set, else the global default |
| `(configure-statusline! left center right)` | Configure the three statusline sections, each a list of element name strings |
| `(set-statusline-text! source pane text)` | Push `text` for a `"steel:<source>"` statusline element, scoped to `pane`'s buffer; empty string clears it |

See [Reading options from Scheme](plugins.md#reading-options-from-scheme) for `get-option`/`get-buffer-option`'s fallback rules, [Statusline](configuration.md#statusline) for the built-in element names, and [Custom elements](configuration.md#custom-elements) for `configure-statusline!`/`set-statusline-text!` together.

## Key bindings

| Call | Effect |
|------|--------|
| `(bind-key! mode key-string cmd-name)` | Bind a key in `'normal`, `'insert`, or `'extend` mode |
| `(bind-key-extend! mode key-string cmd-name)` | Same, but the binding always extends the selection |
| `(unbind-key! mode key-string)` | Remove a binding |
| `(bind-wait-char! mode key-string cmd-name)` | Bind a key sequence that captures the *next* keypress instead of looking it up; read it back with `(pending-char)` |
| `(bind-keys! mode (key cmd) ...)`, `(bind-keys-extend! mode (key cmd) ...)`, `(unbind-keys! mode key ...)` | Batched forms of the three above |
| `(set-register-prefix! name)` | Target a specific register (`0`–`9`, `k`, `c`, `b`) for the rest of the current command body's `call!`s |

See [Key bindings](configuration.md#key-bindings) for the key-string grammar and full examples, and [Register prefix](plugins.md#register-prefix) for `set-register-prefix!`.

## Commands

| Call | Effect |
|------|--------|
| `(define-command! name doc proc #:repeatable #:inline-output)` | Register `name` as an editor command |
| `(define-typed-command! name doc proc #:inline-output)` | Register `name` as a typed command, reachable as `:name` |
| `(call! name args ...)` | Dispatch any editor command (built-in or Scheme-defined), activating its plugin on demand |
| `(request-wait-char! cmd-name)` | From inside a running command, dispatch `cmd-name` once the user types a character |
| `(pending-char)` | Read the character captured by a `WaitChar` binding or `request-wait-char!`, or `#f` outside that context |
| `(command-plugin name)` | The id string of the plugin that registered command `name`: `"user"` for a top-level `init.scm` definition, `"hume"` for a built-in |
| `(hume/yield!)` | Check the interrupt/step-budget flag inside a long loop, aborting the script if it's set |

`define-command!`, `define-typed-command!`, `call!`, `request-wait-char!`, and `pending-char` are covered with examples in [Defining commands](plugins.md#defining-commands), [Calling other commands](plugins.md#calling-other-commands), and [Pending character input](plugins.md#pending-character-input). `hume/yield!` only matters for a script doing real work in a loop. Without it, a script that runs past `steel-init-budget-ms`/`steel-command-budget-ms` (see [Global options](configuration.md#global-options)) still runs to completion; interruption is cooperative, not preemptive.

## Plugin lifecycle

| Call | Effect |
|------|--------|
| `(declare-plugin! name #:commands #:typed-commands #:events #:languages #:config)` | Lazy plugin registration |
| `(load-plugin! name #:config)` | Eager plugin registration |
| `(resolve-plugin-path name)` | The plugin's resolved file path if it exists on disk, else `#f`; raises for a malformed name |
| `(loaded-plugins)` | List of plugin names that have finished activating |
| `(declared-plugins)` | List of every declared plugin name, `core:*` included |
| `(plugin-config)` | The calling plugin's own `#:config` value, or an empty hash |

Full picture (activation timing, `#:config` semantics, dependency checks) in [Plugins](plugins.md), particularly [How plugins are loaded](plugins.md#how-plugins-are-loaded) and [Depending on another plugin](plugins.md#depending-on-another-plugin).

## Hooks

| Call | Effect |
|------|--------|
| `(register-hook! name proc)` | Register `proc` for lifecycle event `name`. Top-level/plugin-body only, not inside a command |

See [Hooks](plugins.md#hooks) for the full table of hook names and their lambda signatures.

## Logging

| Call | Effect |
|------|--------|
| `(log! severity message)` | Push `message` to the editor's message log, tagged `severity` |

`severity` is one of `'trace`, `'info`, `'warn`, `'error`; anything else raises. Where a message ends up depends on severity: `'info` only flashes in the statusline (never kept in `:messages`); `'warn` and `'error` do both; `'trace` goes to `:messages` only, never the statusline.

## Buffers, panes & selections

Almost every call on this page takes a **pane** value: an opaque handle that names a buffer and, where relevant, the specific pane it's associated with. You get one as a command or hook's own leading argument, from `(focused-pane)`, or from `(buffers)`/`(panes)`/`(buffer-panes pane)` below. Never build one by hand. A pane value that carries no pane (most hook arguments, and every entry `(buffers)` returns) still works for anything that only needs the buffer (`buffer-text`, `buffer-path`, the decoration setters, and so on); a call that needs the pane itself (`buffer-cursor-line`, `buffer-selections`, `viewport-range`, `switch-to-buffer!`, the UI openers) raises if the pane component is missing, if that pane has since closed, or if it no longer shows the buffer the value named.

If a buffer is shown in more than one pane and you need a *specific* one rather than whatever value you already have, use `(buffer-panes pane)` (below) to list them and pick explicitly: a debounced or async continuation that fires after focus has moved to a different pane on the same buffer should capture the pane it cares about up front, not assume one.

A closed buffer behaves differently depending on the call: most reads below (`buffer-cursor-line`, `buffer-selections`, `symbol-under-cursor`, `selections-linewise?`/`selections-charwise?`, `viewport-range`, `diagnostic-counts`, `diagnostics-for-buffer`, the `lsp-*-params` calls) answer `#f`/empty for a value naming a closed buffer, the same as they would for one that just isn't shown anywhere, indistinguishable from "not shown" without checking `buffer-live?` first. Every call that writes to a buffer, or that reads bookkeeping with no "not shown" case of its own (`buffer-path`, `buffer-text`, `buffer-generation`, `get-buffer-option`, the decoration setters, `apply-text-edits!`, `lsp-request!`, …) raises instead.

| Call | Effect |
|------|--------|
| `(focused-pane)` | The pane focused right now, paired with its buffer: a live read. Reach for this from a response callback, a timer, or anything else with no pane of its own to act on, e.g. checking `(equal? pane (focused-pane))` before acting on a response, when `pane` was captured before an async request went out. A command body doesn't need this: it receives the pane it was invoked through as its own leading parameter (see [Defining commands](plugins.md#defining-commands)) |
| `(buffers)` | List of every open buffer, as pane-less pane values, in open-order |
| `(buffer-live? pane)` | `#t` if `pane`'s buffer is still open, `#f` otherwise; never raises. The idiom for a timer, debounce, or async callback whose captured value may have closed by the time it fires: check this before calling anything that would otherwise raise on a closed buffer |
| `(panes)` | List of every open pane, including panes in other tabs |
| `(buffer-panes pane)` | Every pane currently showing `pane`'s buffer: the focused pane first if it shows that buffer, then the rest of the active tab, then other tabs. `(car (buffer-panes pane))` picks the same one a plugin would want by default when it just needs *some* pane on the buffer |
| `(buffer-key pane)` | An opaque, comparable value naming just `pane`'s buffer: two pane values for the same buffer (even with different panes, or no pane at all) produce equal keys. Use this, not the pane value itself, as the key in a hash table a plugin keeps for its own per-buffer state |
| `(buffer-path pane)` | Absolute path string, or `#f` for an unsaved buffer |
| `(buffer-display-path pane)` | Display-ready path (absolutized, `~`-collapsed). Print it, never use it for filesystem I/O; `#f` for an unsaved buffer |
| `(buffer-name pane)` | Display name: filename, or `"*scratch*"` |
| `(buffer-dirty? pane)` | `#t` if the buffer has unsaved edits |
| `(buffer-text pane)` | Full live content as a string |
| `(buffer-lines pane #:start #:end)` | Content as a list of lines, each with its ending stripped |
| `(buffer-line-count pane)` | Content line count, cheaper than `(length (buffer-lines pane))` |
| `(buffer-cursor-line pane)` | 0-based line of the primary cursor in `pane`'s own pane |
| `(buffer-selections pane)` | List of `(anchor head primary?)` triples in `pane`'s own pane |
| `(offset->line pane idx)` | 0-based line containing 0-based char offset `idx` in the buffer's text |
| `(line->offset pane line)` | 0-based char offset where 0-based content line `line` starts |
| `(viewport-range pane)` | `(first-line . end-line)` currently visible in `pane`'s own pane, 0-based end-exclusive |
| `(open-buffer! path)` | Open `path`, returning a pane-less pane value for it |
| `(close-buffer! pane)` | Close a buffer |
| `(switch-to-buffer! pane target)` | Redirect `pane`'s own pane to `target`'s buffer |
| `(buffer-language pane)` | Language name string, or `#f` |
| `(set-buffer-language! pane lang)` | Set (or clear, with `#f`) a buffer's language override |
| `(buffer-generation pane)` | Int, bumped by every mutation to the buffer: a staleness token for comparing against a stored snapshot |
| `(selections-linewise? pane)` | `#t` if every selection in `pane`'s own pane covers whole lines. A cursor sitting alone on a blank line doesn't count either way (it neither satisfies this nor breaks it when a real whole-line selection is also present), and `#f` if every selection is such a cursor |
| `(selections-charwise? pane)` | `#t` if none of the selections in `pane`'s own pane cover whole lines, with the same blank-line-cursor exception as above; `#t` if every selection is such a cursor |
| `(symbol-under-cursor pane)` | The identifier under the primary cursor in `pane`'s own pane, as a string |
| `(pane? v)` | `#t` if `v` is an opaque pane value |

`buffer-text`, `buffer-lines`, `buffer-line-count`, `buffer-selections`, `offset->line`, `line->offset`, and `viewport-range` are covered with examples in [Reading selections](plugins.md#reading-selections) and [Reading buffer text](plugins.md#reading-buffer-text). Every `pane` argument here is an opaque value from one of these functions, or the one a command receives as its own leading parameter (see [Defining commands](plugins.md#defining-commands)); there's no "current buffer" shortcut baked into any builtin itself. Compare two pane values with plain `equal?`; there's no dedicated equality builtin.

## Editing & navigation

| Call | Effect |
|------|--------|
| `(apply-text-edits! pane edits #:expect-generation)` | Apply a list of edits to `pane`'s buffer, mapped through `pane`'s own selections, each entry a JSON handle onto a wire `TextEdit` (e.g. a `textDocument/formatting` response element, passed straight through) |
| `(apply-workspace-edit! pane wsedit)` | Apply an LSP `WorkspaceEdit` (a JSON handle onto one, e.g. straight from an `lsp-request!` response) across every buffer it touches, mapping the hunk for `pane`'s own buffer (if any) through `pane`'s selections; returns the count of buffers modified |
| `(goto-location! pane loc)` | Move `pane`'s own pane to `loc`: an LSP `Location`/`LocationLink` JSON handle, or `(list target line char-col)` with `target` a pane value, path, or `file://` URI and `line`/`char-col` char-indexed |
| `(insert-key! pane key)` | Run `key`'s normal Insert-mode behaviour (tab-style-aware Tab, auto-pairs, auto-indented Enter, …) on `pane`, as if it had no Insert-mode binding; `key` is one chord in `bind-key!`'s own syntax |

`#:expect-generation` guards against applying a stale edit: pass a `buffer-generation` snapshot and the call fails if the buffer has mutated since. `apply-text-edits!`/`apply-workspace-edit!`/`goto-location!`'s wire shape each decode their positions using the handle's own producing-server encoding. A plain hashmap you build by hand (not pulled from a response via `json-ref`/`json-list`) has no such encoding to decode with, and is rejected.

`insert-key!` only works while `pane` is the focused pane and Insert mode is active, from inside a command bound to an Insert-mode key. It exists so a binding can decide, at the moment the key is pressed, whether to override that key's normal behaviour or fall back to it. For example, binding Tab to complete after a letter and insert a tab everywhere else:

```scheme
(define-command! "tab-or-complete" "Complete after a letter, else insert a tab."
  (lambda (pane)
    (if (letter-before-cursor? pane)
        (call! "completion-trigger" pane)
        (insert-key! pane "tab"))))
(bind-key! 'insert "tab" "tab-or-complete")
```

`letter-before-cursor?` here is a helper you write yourself, not a builtin.

## Registers

| Call | Effect |
|------|--------|
| `(write-register! name values)` | Store `values` (a list of strings, one per selection) in register `name` |
| `(read-register name)` | Contents of register `name` as a list of strings, or `#f` if it's empty |

Both ends speak the same list shape, so `(write-register! "3" (read-register "3"))` round-trips. Valid names are `0`–`9`, `k` (kill-ring head), `c` (system clipboard), and `b` (black hole), the same set the [`"` register prefix](copy-and-paste.md#register-prefix) accepts. Writing `k` behaves like a yank to the kill ring; writing `b` discards silently; reading an unwritten register, `b`, or a register holding a recorded macro all answer `#f`.

## Language & syntax

| Call | Effect |
|------|--------|
| `(define-language! name exts globs shebangs #:language-id)` | Define or override a language identity |
| `(register-grammar! name grammar-path symbol highlights-path #:injections #:textobjects)` | Register an already-compiled tree-sitter grammar |
| `(language-has-grammar? name)` | `#t` if `name` has an attached grammar |

`define-language!`/`register-grammar!` are covered with examples in [Teach HUME a new language](syntax-highlighting.md#teach-hume-a-new-language).

## Language servers

These are editor-builtin commands any LSP plugin can drive: an LSP plugin registers and talks to a server through them rather than wiring its own protocol client.

| Call | Effect |
|------|--------|
| `(register-lsp-server! language #:command #:args #:root-markers #:init-options #:settings #:env)` | Register (or replace) the server for `language` |
| `(unregister-lsp-server! language)` | Queue removing `language`'s registration and shutting down its running clients; idempotent |
| `(lsp-stop! target)`, `(lsp-restart! target)` | Queue stopping / stopping-then-respawning a server: `target` is a pane value (that buffer's attached server) or a language-name string (every server registered for it) |
| `(lsp-show-status! pane)` | Open the `[lsp-status]` read-only view, only while `pane` is still the one you're looking at |
| `(lsp-request! pane method params callback #:allow-stale #:supersede #:require-focus)` | Send a raw request to `pane`'s attached server; `callback` is `(lambda (err result) ...)`. A real response delivers as a JSON handle; read it with `json-ref`/`json-contains?`/`json-list`, or pass it straight to `completion-emit!`. `#:require-focus #t` drops the callback unless `pane` is still the exact pane you were looking at, still showing the same buffer, when the response arrives. It needs `pane` to carry a pane, not just a buffer |
| `(lsp-notify! pane method params)` | Fire-and-forget notification to `pane`'s attached server, no callback |
| `(on-lsp-notification method handler)` | Register `handler` (`(lambda (server params) ...)`) for every `method` notification HUME doesn't already special-case (`window/logMessage`, `window/showMessage`, `$/progress`, `publishDiagnostics`) |
| `(lsp-capabilities pane)` | A JSON handle onto `pane`'s attached server's `ServerCapabilities` (read with `json-ref`/`json-contains?`), or `#f` if unresolved or mid-handshake |
| `(lsp-server-status)` | List of `{"language" "root" "state" "pending"}` hashmaps, one per registered server |
| `(lsp-server-for-buffer pane)` | Registered language name attached to the buffer, or `#f` |
| `(lsp-registered-for-language? language)` | `#t` if a server is registered for `language` |
| `(lsp-position-params pane)` | `{"textDocument" {"uri"} "position" {"line" "character"}}` from the primary cursor in `pane`'s own pane, or `#f` |
| `(lsp-primary-range-params pane)` | Same shape, `"range"` from the primary selection alone |
| `(lsp-linewise-ranges-params pane)` | `{"textDocument" {"uri"} "ranges" [...]}`: one wire range per linewise selection in `pane`'s own pane (a run of touching selections coalesces into one), `"ranges"` empty if none are linewise; `#f` only for the same reasons `lsp-primary-range-params` returns `#f` |
| `(lsp-position->offset pane position)` | The buffer's char offset for a wire `{"line" "character"}` hashmap, or `#f` |
| `(lsp-range->offsets pane range)` | `(start . end)` char offsets for a wire `{"start" ... "end" ...}` range, or `#f` |
| `(lsp-label-offsets->text label offsets)` | The slice of `label` a `ParameterInformation`-style `(start end)` wire offset pair names; `offsets` decodes with its own tagged producing-server encoding |
| `(lsp-locations->display-parts locs)` | One `(path line grapheme-col-or-wire)` list per raw `Location`/`LocationLink` in `locs`, each decodes wire positions with its own tagged producing-server encoding |

`register-lsp-server!`, `lsp-request!`, and `lsp-notify!` are covered with examples in [Registering a language server](lsp.md#registering-a-language-server) and [Advanced: custom requests](lsp.md#advanced-custom-requests). `lsp-position->offset`/`lsp-range->offsets`/`lsp-label-offsets->text` convert LSP wire units (UTF-16 or byte offsets, depending on the server's negotiated encoding) to editor-native char offsets. Always go through these rather than assuming a 1:1 mapping. `lsp-locations->display-parts`'s column is an exact grapheme column when the target has an open buffer; otherwise it's the location's own wire `character` verbatim, since refining it would mean reading a file the user may never open.

## Diagnostics & decorations

Not LSP-specific (any plugin can populate these), but LSP diagnostics and inlay hints are the heaviest client.

| Call | Effect |
|------|--------|
| `(diagnostics-for-buffer pane #:severity #:range)` | Diagnostics for the buffer, optionally floored by severity symbol or restricted to a `(start . end)` char range |
| `(diagnostic-counts pane)` | `(errors . warnings)` pair for the buffer |
| `(set-inlay-hints! source pane hints)` | Replace `source`'s inlay hints for the buffer. `hints`: list of `(offset text 'before\|'after)` |
| `(register-sign-source! name pane priority)` | Reserve a gutter sign slot for `name` on the buffer, ranked by `(priority desc, name asc)` among every source registered for it |
| `(set-signs! source pane signs)` | Replace `source`'s gutter signs for the buffer. `signs`: list of `(line text scope)`; `source` must already be registered |
| `(set-virtual-lines! source pane lines)` | Replace `source`'s virtual (ghost) lines for the buffer. `lines`: list of hashmaps with `'line`/`'text` required, optional `'anchor` (`'before`/`'after`), `'scope`, `'segments` |
| `(set-eol-text! source pane lines)` | Replace `source`'s end-of-line text for the buffer. `lines`: list of `(line text scope)` |
| `(set-extra-highlights! source pane spans)` | Replace `source`'s extra syntax highlights for the buffer. `spans`: list of `(start end scope)` char ranges |
| `(set-line-backgrounds! source pane entries)` | Replace `source`'s full-line background tints for the buffer. `entries`: list of `(line scope)` |

`diagnostics-for-buffer` and the hook that feeds it are shown in [Hooks](plugins.md#hooks). A sign source's gutter slot is reserved the first time it registers for a buffer, even before placing any sign, which is what keeps the gutter's width stable as signs come and go; there's no `unregister-sign-source!`, and re-registering the same `name` for the same buffer just replaces its priority. Line backgrounds have no priority: same-line entries from different sources break ties by source name instead.

## Completion

A plugin registers a completion *source*; the editor drives it. `Ctrl-Space` in Insert mode (the built-in `completion-trigger` command) asks every source registered for the buffer, a registered trigger character asks the source registered under its name, and the first `Tab` on a `:` command line asks the source that command declared. Each answer is ranked with every other source's, in one menu, against that source's own token; no source renders its own UI.

| Call | Effect |
|------|--------|
| `(register-completion-source! name proc #:target #:match #:priority #:resolve)` | Register `proc` as the completion source `name`. `#:target 'buffer` serves Insert mode, calling `(proc id pane prefix)`; its token is always the identifier run before the cursor (`prefix` being that text). `#:target 'minibuf` serves the `:` line, calling `(proc id input cursor)`; its token is always the whitespace-delimited argument the cursor is in. Either way, the token is what the source's answers are filtered against and what accepting one replaces. `#:match` (`'fuzzy` default, `'string`, or `'delegated`) picks how items are scored against the token's text; `#:priority` (default `0`) breaks score ties, higher first. `#:resolve #t` (`'buffer` sources only, default `#f`) claims that this source's items are wire completion items from the buffer's attached LSP server, so accepting one may send `completionItem/resolve` for it. Set this only for a source whose items genuinely came from that server, never for one that just builds its own items in an LSP-attached buffer. `'buffer` and `'minibuf` names are separate: registering `name` again under the same `#:target` replaces the earlier source; the same `name` under the *other* target is a second, independent source |
| `(completion-emit! id items #:incomplete)` | `proc`'s answer to the call it received `id` from, sync or from a later callback, exactly once; an empty list means "nothing from me". `items` is a list (each entry either a completion-item hashmap, where `label` is the only required key, or a bare string, which is sugar for a hashmap with just that `label`) or the JSON handle from an `lsp-request!` response, passed straight through. `#:incomplete #t` asks to be called again as the user keeps typing. It applies to a plain `items` list, or a handle onto a bare `CompletionItem[]` array (which has no `isIncomplete` field of its own); combining it with a handle onto a `CompletionList` object errors, since that shape's own `isIncomplete` field is used instead. Any handle that isn't one of those two response shapes errors too. Returns `#f` when `id` is no longer the latest call (a later keystroke re-asked, or the menu closed) and the answer was dropped |
| `(register-trigger-chars! source language chars)` | Register 1-char trigger strings `chars` for `(source, language)`: typing one in Insert mode fires the `on-trigger-char` hook for any listener named `source`. This is *not* how a `'buffer` completion source's own trigger chars are joined; see `completion-set-trigger-chars!` below |
| `(completion-set-trigger-chars! source language chars)` | A `'buffer` completion source's own trigger characters for `language`, replacing that `(source, language)` pair's previous set; an empty `chars` removes it. Typing one of `chars` in Insert mode invokes `source` directly, the same as an explicit trigger's own `#:target 'buffer` invocation. Errors if `source` names no registered `'buffer` source |
| `(completion-top n)` | The top `n` ranked items of the open menu, each carrying its `source` |
| `(completion-accept! idx)` | Accept item `idx` from `completion-top`'s (ranked) order; fires the `on-completion-accept` hook |
| `(completion-dismiss!)` | Close the open menu; a no-op if none is open |

`define-typed-command!` takes `#:complete "name"` to give a `:` command argument completion from source `name`: a source registered with `#:target 'minibuf`, or a built-in one such as `"path"`. See [Hooks](plugins.md#hooks) for `on-trigger-char` and `on-completion-accept`'s lambda signatures.

## Pickers

These are editor-builtin commands any plugin can drive: a plugin opens a picker and pushes items through them rather than building its own fuzzy-finder.

| Call | Effect |
|------|--------|
| `(picker! pane items on-select #:prompt #:pending #:query #:truncate #:actions)` | Open a fuzzy-finder panel over a fixed `items` list of `(display . payload)` pairs, only while `pane` is still the one you're looking at |
| `(live-picker! pane on-select #:command #:prompt #:query #:debounce-ms #:cwd #:nul #:ok-exit-codes #:truncate #:actions)` | Open a picker whose query re-spawns `#:command`'s subprocess on every keystroke, debounced, only while `pane` is still the one you're looking at |
| `(picker-push! token items)` | Append a batch of `(display . payload)` items to an open picker |
| `(picker-replace! token items)` | Replace an open picker's items wholesale |
| `(picker-source-spawn! token cmd args #:cwd #:nul #:ok-exit-codes)` | Stream a subprocess's stdout lines into an open picker as items |
| `(picker-source-stop! token)` | Kill a picker's still-running spawned source |
| `(picker-close! #:token)` | Close a picker; `#:token` makes the close a no-op if that picker has already closed or been replaced |

Full walkthroughs (batch vs. streaming population, truncation direction, exit-code handling, live requery) are in [Custom pickers](plugins.md#custom-pickers) and [Live requery (live grep)](plugins.md#live-requery-live-grep).

## Other UI widgets

| Call | Effect |
|------|--------|
| `(prompt! pane label on-confirm #:prefill)` | Open a minibuffer text prompt, only while `pane` is still the one you're looking at; `on-confirm` fires once, later, with the confirmed text or `#f` on cancel |
| `(show-popup! pane text #:anchor #:kind #:lang)` | Show a text popup, only while `pane` is still the one you're looking at. `#:anchor` `'cursor` (default, floats near the cursor) or `'bottom` (docks above the statusline); `#:lang` for syntax highlighting. `#:kind` also sets how long it lives: `'sticky` (default) closes on its own as soon as you leave whatever mode you opened it in; `'scrollable` stays open (Ctrl-u/Ctrl-d scroll it) until any other key, paste, or mouse input closes it |
| `(close-popup!)` | Close the open popup; idempotent, a no-op if none is open |
| `(show-menu! pane items on-select)` | Show a selection menu over `items`, a list of strings, only while `pane` is still the one you're looking at |
| `(close-menu!)` | Close the open menu; a no-op if none is open |
| `(show-drawer-list! pane items on-select)` | Show a list in the bottom drawer, over `items`, a non-empty list of strings, only while `pane` is still the one you're looking at. Replaces any drawer already open, and the outgoing drawer's `on-select` fires with `#f` so its owner knows the drawer is gone. Errors on empty `items`; close (or never open) instead. Returns a token scoping `close-drawer!`/`update-drawer-list!`/`drawer-selected-index` to this drawer (hold onto it), or `#f` if the drawer didn't open (the editor moved on before the call landed) |
| `(close-drawer! token)` | Close the open drawer; a no-op if none is open or `token` doesn't match its own |
| `(update-drawer-list! token items on-select selected)` | Replace the open drawer's rows in place, keeping the current selection unless `selected` names another row; returns `#t` when applied, `#f` when no drawer is open, `token` doesn't match its own, or `items` is empty. Close instead of clearing through an update |
| `(drawer-selected-index token)` | The open drawer's selected row, or `#f` when no drawer is open or `token` doesn't match its own |

## Timers

| Call | Effect |
|------|--------|
| `(after! ms thunk)` | Call `thunk` with no args once `ms` milliseconds pass; returns a timer id |
| `(cancel-timer! id)` | Cancel a pending timer; idempotent, a no-op if `id` already fired, was cancelled, or never existed |
| `(debounce ms proc)` | Wrap `proc` so each call reschedules it `ms` out, cancelling any still-pending call from a prior invocation |
| `(debounce-by ms proc #:key [key car])` | Same, but keyed per `(key . args)`: a call keyed one way never cancels a call keyed another. `#:key` defaults to the first argument itself; pass `#:key (lambda (pane . _) (buffer-key pane))` to key by buffer when different calls might carry different pane values for the same buffer |

## Async & subprocesses

| Call | Effect |
|------|--------|
| `(spawn-async! cmd args cwd callback)` | Run `cmd` in the background; `callback` (`(lambda (stdout stderr exit-code) ...)`) fires exactly once, later |
| `(cancel-async! id)` | Kill a still-running `spawn-async!` job and drop its callback; idempotent |
| `(run-inline-output! cmd args #:cwd)` | Run `cmd`, streaming output to the terminal inside an `#:inline-output` command; raises on nonzero exit |
| `(run-capture! cmd args cwd)` | Run `cmd`, blocking until it exits; returns `(stdout stderr exit-code)`. `core:stdlib`'s `stdlib/run` (see [Standard Library](standard-library.md)) is this call under its usual name |

Covered with examples in [Filesystem and processes](plugins.md#filesystem-and-processes).

## Diffing

| Call | Effect |
|------|--------|
| `(diff-lines old-text new-text)` | Line-level hunks where `old-text`/`new-text` differ |
| `(diff-buffer-lines pane ref-text)` | Same, but against the buffer's current unsaved content; avoids pulling the whole buffer through `buffer-text` first |
| `(diff-words old-text new-text)` | `(hunks . too-long?)`: word-level hunks within a single changed line |

Covered with examples, including hunk shapes, in [Comparing text](plugins.md#comparing-text).

## Text & strings

Everyday string work (trimming, splitting on a separator, case conversion, prefix/suffix tests, search and replace) uses Steel's own string functions directly; see Steel's [string reference](https://mattwparas.github.io/steel/book/builtins/steel_strings.html). `string-length`, `substring`, and `string-ref` count Unicode characters (codepoints), not bytes and not on-screen columns.

| Call | Effect |
|------|--------|
| `(split-words line word-chars)` | Every word in `line`, in order: the same classification `w`/`b` motions and text objects use, so a word here is exactly what one of those would select. `word-chars` extends what counts as a word, same as the buffer option of the same name (`""` for none) |

## Filesystem & directories

| Call | Effect |
|------|--------|
| `(data-dir)` | HUME's data directory, or `#f` if unavailable |
| `(runtime-dir)` | HUME's runtime directory, or `#f` if unavailable |
| `(path-join seg ...)` | Join path segments with the OS-native separator |
| `(path->display path)` | Run an absolute `path` string through HUME's display-form pipeline (Windows `\\?\` stripping, `~`-collapse); no filesystem access |
| `(hume-target)` | Install-target identifier for the current platform (one of `"darwin-arm64"`, `"darwin-x64"`, `"linux-x64"`, `"windows-x64"`), or `#f` on any other platform |

The pattern for reading a plugin's own files is covered in [Filesystem and processes](plugins.md#filesystem-and-processes).

## JSON handles

An `lsp-request!` response, `lsp-capabilities`, a `diagnostics-for-buffer`
entry's `"raw"` field, `on-lsp-notification`'s params, `on-completion-accept`'s
item, and `json-parse`'s result are all opaque JSON handles rather than
decoded hashmaps. Read one with these instead of `hash-ref`/`hash?`/`list?`:

Other values stay ordinary hashmaps: `lsp-request!`'s `err`, `lsp-server-status`,
and a `diagnostics-for-buffer` entry itself (outside its `"raw"` field) are
built by HUME, not decoded from server JSON, and read with `hash-ref` as
usual; see [Advanced: custom requests](lsp.md#advanced-custom-requests) for
the `err`/`res` distinction in practice.

| Call | Effect |
|------|--------|
| `(json-parse str)` | Decode a JSON string: an object/array becomes a JSON handle (read with `json-ref`/`json-contains?`/`json-list`), a scalar crosses natively, and top-level `null` is void |
| `(json-ref j seg ...)` | Look up a path of string keys / integer indices inside handle `j`. A nested object or array field comes back as another handle; a scalar field comes back as a native string/number/boolean; a `null` field comes back as void. Errors, naming the full path, on a missing key, an out-of-range index, or indexing into the wrong container kind |
| `(json-ref-or j default seg ...)` | `json-ref`, but `default` in place of erroring when the path doesn't resolve; a present `null` still comes back as void, not `default` |
| `(json-contains? j seg ...)` | `#t` iff the path resolves; a `null` value at the end still counts as present |
| `(json-list j)` | `j`, a JSON array handle, as a Steel list of its elements (each one funneled through the same handle/native-value rule as `json-ref`). Errors if `j` isn't an array |
| `(json-array? v)`, `(json-object? v)` | `#t` if `v` is a JSON handle onto an array/object, `#f` for anything else (including a non-handle value) |

## Grammar & install pipeline

These back `:plum-*` and `:lsp-install`/`:lsp-uninstall`: full-trust primitives most plugins won't call directly unless they're building an installer of their own.

| Call | Effect |
|------|--------|
| `(compile-grammar! src out)` | Compile the tree-sitter grammar source at `src` to `out` |
| `(sha256-file path)` | Lowercase hex sha256 digest of `path` |
| `(unpack-gz! src dest)` | Decode a single-file gzip archive into `dest`; chmod's it executable on Unix |
| `(unpack-zip! src dest-dir bin-path)` | Extract a zip archive into `dest-dir`, then verify `bin-path` exists and chmod it executable on Unix |
| `(acquire-install-lock!)`, `(release-install-lock!)` | Cross-process install lock guarding concurrent `:lsp-install`/`:lsp-uninstall` runs |

## Standard Library

`core:stdlib` is a bundled plugin of helpers for plugin authors: filesystem, subprocess, selection, and config-validation commands, all reached through `call!`. Its full reference lives on the [Standard Library](standard-library.md) page.
