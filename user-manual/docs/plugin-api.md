# Plugin API

Reference for every function a plugin or `init.scm` can call directly — as opposed to a command reached through a key binding or [`call!`](plugins.md#calling-other-commands). Two layers make up this surface:

- **Builtins** — native to the editor, always available, called as plain Scheme: `(buffer-text bid)`, `(bind-key! ...)`. Some are thin Scheme wrappers (keyword arguments, defaults) over a Rust primitive; the wrapper is what's documented here.
- **[Standard Library](standard-library.md)** — `core:stdlib`, an optional bundled *plugin*. Its commands are reached through `call!`, like any other plugin's: `(call! "stdlib/find" pred? lst)`.

This page is a lookup reference — tables of signatures and one-line effects. For narrative walkthroughs and worked examples, see [Plugins](plugins.md), [Language Servers](lsp.md), [Configuration](configuration.md), and the other pages linked throughout.

## Settings & statusline

| Call | Effect |
|------|--------|
| `(set-option! key value)` | Set a global option |
| `(set-buffer-option! bid key value)` | Set an option on one buffer only |
| `(get-option key)`, `(get-option bid key)` | Read the effective value of an option — global default, or `bid`'s override if given |
| `(configure-statusline! left center right)` | Configure the three statusline sections — each a list of element name strings |
| `(set-statusline-text! source bid text)` | Push `text` for a `"steel:<source>"` statusline element, scoped to `bid`; empty string clears it |

See [Reading options from Steel](plugins.md#reading-options-from-steel) for `get-option`'s fallback rules, [Statusline](configuration.md#statusline) for the built-in element names, and [Custom elements](configuration.md#custom-elements) for `configure-statusline!`/`set-statusline-text!` together.

## Key bindings

| Call | Effect |
|------|--------|
| `(bind-key! mode key-string cmd-name)` | Bind a key in `'normal`, `'insert`, or `'extend` mode |
| `(bind-key-extend! mode key-string cmd-name)` | Same, but the binding always extends the selection |
| `(unbind-key! mode key-string)` | Remove a binding |
| `(bind-wait-char! mode key-string cmd-name)` | Bind a key sequence that captures the *next* keypress instead of looking it up — read it back with `(pending-char)` |
| `(bind-keys! mode (key cmd) ...)`, `(bind-keys-extend! mode (key cmd) ...)`, `(unbind-keys! mode key ...)` | Batched forms of the three above |
| `(set-register-prefix! name)` | Target a specific register (`0`–`9`, `k`, `c`, `b`) for the rest of the current command body's `call!`s |

See [Key bindings](configuration.md#key-bindings) for the key-string grammar and full examples, and [Register prefix](plugins.md#register-prefix) for `set-register-prefix!`.

## Commands

| Call | Effect |
|------|--------|
| `(define-command! name doc proc #:repeatable #:inline-output)` | Register `:name` as a typed command |
| `(call! name args ...)` | Dispatch any key-bindable command (built-in or Steel-defined), activating its plugin on demand |
| `(request-wait-char! cmd-name)` | From inside a running command, dispatch `cmd-name` once the user types a character |
| `(pending-char)` | Read the character captured by a `WaitChar` binding or `request-wait-char!`, or `#f` outside that context |
| `(command-plugin name)` | The id string of the plugin that registered command `name` — `"user"` for a top-level `init.scm` definition, `"hume"` for a built-in |
| `(hume/yield!)` | Check the interrupt/step-budget flag inside a long loop, aborting the script if it's set |

`define-command!`, `call!`, `request-wait-char!`, and `pending-char` are covered with examples in [Defining commands](plugins.md#defining-commands), [Calling other commands](plugins.md#calling-other-commands), and [Pending character input](plugins.md#pending-character-input). `hume/yield!` only matters for a script doing real work in a loop — without it, a script that runs past `steel-init-budget-ms`/`steel-command-budget-ms` (see [Global options](configuration.md#global-options)) still runs to completion; interruption is cooperative, not preemptive.

## Plugin lifecycle

| Call | Effect |
|------|--------|
| `(declare-plugin name #:commands #:events #:languages #:config)` | Lazy plugin registration |
| `(load-plugin name #:config)` | Eager plugin registration |
| `(resolve-plugin-path name)` | The plugin's resolved file path if it exists on disk, else `#f`; raises for a malformed name |
| `(loaded-plugins)` | List of plugin names that have finished activating |
| `(declared-plugins)` | List of every declared plugin name, `core:*` included |
| `(plugin-config)` | The calling plugin's own `#:config` value, or an empty hash |

Full picture — activation timing, `#:config` semantics, dependency checks — in [Plugins](plugins.md), particularly [How plugins are loaded](plugins.md#how-plugins-are-loaded) and [Depending on another plugin](plugins.md#depending-on-another-plugin).

## Hooks

| Call | Effect |
|------|--------|
| `(register-hook! name proc)` | Register `proc` for lifecycle event `name`. Top-level/plugin-body only, not inside a command |

See [Hooks](plugins.md#hooks) for the full table of hook names and their lambda signatures.

## Logging

| Call | Effect |
|------|--------|
| `(log! severity message)` | Push `message` to the editor's message log, tagged `severity` |

`severity` is one of `'trace`, `'info`, `'warn`, `'error` — anything else raises. Where a message ends up depends on severity: `'info` only flashes in the statusline (never kept in `:messages`); `'warn` and `'error` do both; `'trace` goes to `:messages` only, never the statusline.

## Buffers, panes & selections

| Call | Effect |
|------|--------|
| `(current-buffer)` | BufferId of the focused buffer |
| `(current-pane)` | PaneId of the focused pane |
| `(buffers)` | List of every open BufferId, in open-order |
| `(panes)` | List of every open PaneId |
| `(buffer-path bid)` | Absolute path string, or `#f` for an unsaved buffer |
| `(buffer-display-path bid)` | Display-ready path (absolutized, `~`-collapsed) — print it, never use it for filesystem I/O; `#f` for an unsaved buffer |
| `(buffer-name bid)` | Display name — filename, or `"*scratch*"` |
| `(buffer-dirty? bid)` | `#t` if `bid` has unsaved edits |
| `(buffer-text bid)` | Full live content as a string |
| `(buffer-lines bid #:start #:end)` | Content as a list of lines, each with its ending stripped |
| `(current-line-number)` | 1-indexed line of the primary cursor, or `#f` |
| `(current-selections)` | List of `(anchor head primary?)` triples for the focused buffer |
| `(char-index->line idx)` | 1-indexed line number containing 0-indexed char offset `idx` |
| `(line->offset bid line)` | 0-based char offset where 0-based content line `line` starts |
| `(viewport-range bid)` | `(first-line . end-line)` currently visible, 0-based end-exclusive, or `#f` if `bid` isn't shown in any pane |
| `(open-buffer! path)` | Open `path`, returning its BufferId |
| `(close-buffer! bid)` | Close a buffer |
| `(switch-to-buffer! bid)` | Focus a buffer |
| `(buffer-language bid)` | Language name string, or `#f` |
| `(set-buffer-language! bid lang)` | Set (or clear, with `#f`) a buffer's language override |
| `(buffer-generation bid)` | Int, bumped by every mutation to `bid` — a staleness token for comparing against a stored snapshot |
| `(selections-linewise? bid)` | `#t` if every one of `bid`'s selections covers whole lines |
| `(selections-charwise? bid)` | `#t` if none of `bid`'s selections covers whole lines |
| `(symbol-under-cursor bid)` | The identifier under `bid`'s primary cursor, as a string |
| `(buffer-id? v)`, `(pane-id? v)` | `#t` if `v` is an opaque BufferId/PaneId |
| `(buffer-id=? a b)`, `(pane-id=? a b)` | Value-equality for two BufferId/PaneId handles |

`buffer-text`, `buffer-lines`, `current-selections`, `char-index->line`, `line->offset`, and `viewport-range` are covered with examples in [Reading selections](plugins.md#reading-selections) and [Reading buffer text](plugins.md#reading-buffer-text). Every `bid`/pane argument here is an opaque id from one of these functions — there's no "current buffer" shortcut baked into the builtin itself; pass `(current-buffer)` explicitly when that's what you mean.

## Editing & navigation

| Call | Effect |
|------|--------|
| `(apply-text-edits! bid edits #:expect-generation)` | Apply a list of `((start-line . start-char) (end-line . end-char) text)` wire-position edits to `bid` |
| `(apply-workspace-edit! wsedit)` | Apply a decoded LSP `WorkspaceEdit` hashmap across every buffer it touches; returns the count of buffers modified |
| `(goto-location! loc)` | Jump to `loc` — a raw LSP `Location`/`LocationLink` hashmap, or `(list target line char-col)` with `target` a BufferId, path, or `file://` URI and `line`/`char-col` char-indexed |

`#:expect-generation` guards against applying a stale edit: pass a `buffer-generation` snapshot and the call fails if the buffer has mutated since. `apply-text-edits!`/`apply-workspace-edit!` exist to apply LSP responses, but take already-decoded shapes — nothing here is LSP-transport-specific.

## Registers

| Call | Effect |
|------|--------|
| `(write-register! name values)` | Store `values` — a list of strings, one per selection — in register `name` |
| `(read-register name)` | Contents of register `name` as a list of strings, or `#f` if it's empty |

Both ends speak the same list shape, so `(write-register! "3" (read-register "3"))` round-trips. Valid names are `0`–`9`, `k` (kill-ring head), `c` (system clipboard), and `b` (black hole) — the same set the [`"` register prefix](copy-and-paste.md#register-prefix) accepts. Writing `k` behaves like a yank to the kill ring; writing `b` discards silently; reading an unwritten register, `b`, or a register holding a recorded macro all answer `#f`.

## Language & syntax

| Call | Effect |
|------|--------|
| `(define-language! name exts globs shebangs #:language-id)` | Define or override a language identity |
| `(register-grammar! name grammar-path symbol highlights-path [injections-path])` | Register an already-compiled tree-sitter grammar |
| `(language-has-grammar? name)` | `#t` if `name` has an attached grammar |

`define-language!`/`register-grammar!` are covered with examples in [Teach HUME a new language](syntax-highlighting.md#teach-hume-a-new-language).

## Language servers

These are editor-builtin commands any LSP plugin can drive — an LSP plugin registers and talks to a server through them rather than wiring its own protocol client.

| Call | Effect |
|------|--------|
| `(register-lsp-server! language #:command #:args #:root-markers #:init-options #:settings #:env)` | Register (or replace) the server for `language` |
| `(unregister-lsp-server! language)` | Queue removing `language`'s registration and shutting down its running clients; idempotent |
| `(lsp-stop! language)`, `(lsp-restart! language)` | Queue stopping / stopping-then-respawning the server for `language`, or `#f` for the focused buffer's attached server |
| `(lsp-show-status!)` | Open the `[lsp-status]` read-only view |
| `(lsp-request server method params callback #:allow-stale #:supersede)` | Send a raw request to `server` (a registered language name, or `#f` for the focused buffer's server); `callback` is `(lambda (err result) ...)` |
| `(lsp-notify server method params)` | Fire-and-forget notification, no callback |
| `(on-lsp-notification method handler)` | Register `handler` — `(lambda (server params) ...)` — for every `method` notification HUME doesn't already special-case (`window/logMessage`, `window/showMessage`, `$/progress`, `publishDiagnostics`) |
| `(lsp-capabilities server)` | Decoded `ServerCapabilities` hashmap, or `#f` if unresolved or mid-handshake |
| `(lsp-server-status)` | List of `{"language" "root" "state" "pending"}` hashmaps, one per registered server |
| `(lsp-server-for-buffer bid)` | Registered language name attached to `bid`, or `#f` |
| `(lsp-registered-for-language? language)` | `#t` if a server is registered for `language` |
| `(lsp-position-params bid)` | `{"textDocument" {"uri"} "position" {"line" "character"}}` from `bid`'s primary cursor, or `#f` |
| `(lsp-primary-range-params bid)` | Same shape, `"range"` from `bid`'s primary selection alone |
| `(lsp-linewise-ranges-params bid)` | `{"textDocument" {"uri"} "ranges" [...]}` — one wire range per linewise selection in `bid`'s buffer (a run of touching selections coalesces into one), `"ranges"` empty if none are linewise; `#f` only for the same reasons `lsp-primary-range-params` returns `#f` |
| `(lsp-position->offset bid position)` | `bid`'s char offset for a wire `{"line" "character"}` hashmap, or `#f` |
| `(lsp-range->offsets bid range)` | `(start . end)` char offsets for a wire `{"start" ... "end" ...}` range, or `#f` |
| `(lsp-label-offsets->text bid label offsets)` | The slice of `label` a `ParameterInformation`-style `(start end)` wire offset pair names, or `#f` |
| `(lsp-locations->display-parts locs)` | One `(path line grapheme-col-or-wire)` list per raw `Location`/`LocationLink` in `locs` |

`register-lsp-server!`, `lsp-request`, and `lsp-notify` are covered with examples in [Registering a language server](lsp.md#registering-a-language-server) and [Advanced: custom requests](lsp.md#advanced-custom-requests). `lsp-position->offset`/`lsp-range->offsets`/`lsp-label-offsets->text` convert LSP wire units (UTF-16 or byte offsets, depending on the server's negotiated encoding) to editor-native char offsets — always go through these rather than assuming a 1:1 mapping. `lsp-locations->display-parts`'s column is an exact grapheme column when the target has an open buffer; otherwise it's the location's own wire `character` verbatim, since refining it would mean reading a file the user may never open.

## Diagnostics & decorations

Not LSP-specific — any plugin can populate these — but LSP diagnostics and inlay hints are the heaviest client.

| Call | Effect |
|------|--------|
| `(diagnostics-for-buffer bid #:severity #:range)` | Diagnostics for `bid`, optionally floored by severity symbol or restricted to a `(start . end)` char range |
| `(diagnostic-counts bid)` | `(errors . warnings)` pair for `bid` |
| `(set-inlay-hints! source bid hints)` | Replace `source`'s inlay hints for `bid` — `hints`: list of `(offset text 'before\|'after)` |
| `(register-sign-source! name bid priority)` | Reserve a gutter sign slot for `name` on `bid`, ranked by `(priority desc, name asc)` among every source registered for that buffer |
| `(set-signs! source bid signs)` | Replace `source`'s gutter signs for `bid` — `signs`: list of `(line text scope)`; `source` must already be registered |
| `(set-virtual-lines! source bid lines)` | Replace `source`'s virtual (ghost) lines for `bid` — `lines`: list of hashmaps with `'line`/`'text` required, optional `'anchor` (`'before`/`'after`), `'scope`, `'segments` |
| `(set-eol-text! source bid lines)` | Replace `source`'s end-of-line text for `bid` — `lines`: list of `(line text scope)` |
| `(set-extra-highlights! source bid spans)` | Replace `source`'s extra syntax highlights for `bid` — `spans`: list of `(start end scope)` char ranges |
| `(set-line-backgrounds! source bid entries)` | Replace `source`'s full-row background tints for `bid` — `entries`: list of `(line scope)` |

`diagnostics-for-buffer` and the hook that feeds it are shown in [Hooks](plugins.md#hooks). A sign source's gutter slot is reserved the first time it registers for a buffer — even before placing any sign — which is what keeps the gutter's width stable as signs come and go; there's no `unregister-sign-source!`, and re-registering the same `name` for the same `bid` just replaces its priority. Line backgrounds have no priority: same-line entries from different sources break ties by source name instead.

## Completion

These are editor-builtin commands any completion plugin can drive — a source registers its triggers and feeds candidates through them rather than rendering its own UI.

| Call | Effect |
|------|--------|
| `(register-trigger-chars! source language chars)` | Register 1-char trigger strings `chars` for `(source, language)` — feeds the `on-trigger-char` hook |
| `(completion-begin! bid items #:incomplete)` | Open a completion session for `bid` with a list of decoded `CompletionItem` hashmaps |
| `(completion-update-filter! text)` | Re-filter the open session against `text` |
| `(completion-top n)` | The top `n` ranked/filtered items |
| `(completion-accept! idx)` | Accept item `idx` from `completion-top`'s (ranked) order — fires the `on-completion-accept` hook |
| `(completion-dismiss!)` | Close the open session |

A completion source registers its trigger characters, then reacts to the `on-trigger-char` hook by fetching candidates and calling `completion-begin!`; `on-completion-refilter` fires as the user keeps typing, and `on-completion-accept` once they pick a result. See [Hooks](plugins.md#hooks) for those three hooks' lambda signatures.

## Pickers

These are editor-builtin commands any plugin can drive — a plugin opens a picker and pushes items through them rather than building its own fuzzy-finder.

| Call | Effect |
|------|--------|
| `(picker! items on-select #:prompt #:pending #:query #:truncate)` | Open a fuzzy-finder panel over a fixed `items` list of `(display . payload)` pairs |
| `(live-picker! on-select #:command #:prompt #:query #:debounce-ms #:cwd #:nul #:ok-exit-codes #:truncate)` | Open a picker whose query re-spawns `#:command`'s subprocess on every keystroke, debounced |
| `(picker-push! token items)` | Append a batch of `(display . payload)` items to an open picker |
| `(picker-replace! token items)` | Replace an open picker's items wholesale |
| `(picker-source-spawn! token cmd args #:cwd #:nul #:ok-exit-codes)` | Stream a subprocess's stdout lines into an open picker as items |
| `(picker-source-stop! token)` | Kill a picker's still-running spawned source |
| `(picker-close! #:token)` | Close a picker; `#:token` makes the close a no-op if that picker has already closed or been replaced |

Full walkthroughs — batch vs. streaming population, truncation direction, exit-code handling, live requery — are in [Custom pickers](plugins.md#custom-pickers) and [Live requery (live grep)](plugins.md#live-requery-live-grep).

## Other UI widgets

| Call | Effect |
|------|--------|
| `(prompt! label on-confirm #:prefill)` | Open a minibuffer text prompt; `on-confirm` fires once, later, with the confirmed text or `#f` on cancel |
| `(show-popup! text #:anchor #:kind #:lang)` | Show a text popup — `#:anchor` `'cursor` (default, floats near the cursor) or `'bottom` (docks above the statusline); `#:kind` `'sticky` (default) or `'scrollable`; `#:lang` for syntax highlighting |
| `(close-popup!)` | Close the open popup |
| `(show-menu! items on-select)` | Show a selection menu over `items`, a list of strings |
| `(close-menu!)` | Close the open menu |
| `(show-drawer-list! items on-select)` | Show a list in the Class B bottom drawer, over `items`, a list of strings |
| `(close-drawer!)` | Close the open drawer |

## Timers

| Call | Effect |
|------|--------|
| `(after ms thunk)` | Call `thunk` with no args once `ms` milliseconds pass; returns a timer id |
| `(cancel-timer! id)` | Cancel a pending timer; idempotent — a no-op if `id` already fired, was cancelled, or never existed |
| `(debounce ms proc)` | Wrap `proc` so each call reschedules it `ms` out, cancelling any still-pending call from a prior invocation |
| `(debounce-by ms proc)` | Same, but keyed per first-argument value — a call keyed one way never cancels a call keyed another |

## Async & subprocesses

| Call | Effect |
|------|--------|
| `(spawn-async! cmd args cwd callback)` | Run `cmd` in the background; `callback` — `(lambda (stdout stderr exit-code) ...)` — fires exactly once, later |
| `(cancel-async! id)` | Kill a still-running `spawn-async!` job and drop its callback; idempotent |
| `(run-inline-output! cmd args #:cwd)` | Run `cmd`, streaming output to the terminal inside an `#:inline-output` command; raises on nonzero exit |

Covered with examples in [Filesystem and processes](plugins.md#filesystem-and-processes).

## Diffing

| Call | Effect |
|------|--------|
| `(diff-lines old-text new-text)` | Line-level hunks where `old-text`/`new-text` differ |
| `(diff-buffer-lines bid ref-text)` | Same, but against `bid`'s current unsaved content — avoids pulling the whole buffer through `buffer-text` first |
| `(diff-words old-text new-text)` | `(hunks . too-long?)` — word-level hunks within a single changed line |

Covered with examples, including hunk shapes, in [Comparing text](plugins.md#comparing-text).

## Filesystem & directories

| Call | Effect |
|------|--------|
| `(data-dir)` | HUME's data directory, or `#f` if unavailable |
| `(runtime-dir)` | HUME's runtime directory, or `#f` if unavailable |
| `(path-join seg ...)` | Join path segments with the OS-native separator |
| `(path->display path)` | Run an absolute `path` string through HUME's display-form pipeline (Windows `\\?\` stripping, `~`-collapse); no filesystem access |
| `(json-parse str)` | Decode a JSON string into hashmaps/lists/strings/numbers/booleans |
| `(hume-target)` | Install-target identifier for the current platform — one of `"darwin-arm64"`, `"darwin-x64"`, `"linux-x64"`, `"windows-x64"` — or `#f` on any other platform |

`json-parse` and the pattern for reading a plugin's own files are covered in [Filesystem and processes](plugins.md#filesystem-and-processes).

## Grammar & install pipeline

These back `:plum-*` and `:lsp-install`/`:lsp-uninstall` — full-trust primitives most plugins won't call directly unless they're building an installer of their own.

| Call | Effect |
|------|--------|
| `(compile-grammar! src out)` | Compile the tree-sitter grammar source at `src` to `out` |
| `(sha256-file path)` | Lowercase hex sha256 digest of `path` |
| `(unpack-gz src dest)` | Decode a single-file gzip archive into `dest`; chmod's it executable on Unix |
| `(unpack-zip src dest-dir bin-path)` | Extract a zip archive into `dest-dir`, then verify `bin-path` exists and chmod it executable on Unix |
| `(acquire-install-lock!)`, `(release-install-lock!)` | Cross-process install lock guarding concurrent `:lsp-install`/`:lsp-uninstall` runs |

## Standard Library

`core:stdlib` is a bundled plugin of helpers for plugin authors — filesystem, subprocess, selection, and config-validation commands, all reached through `call!`. Its full reference lives on the [Standard Library](standard-library.md) page.
