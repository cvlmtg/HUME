# Plugin API

Reference for every function a plugin or `init.scm` can call directly, as opposed to a command reached through a key binding or [`call!`](writing-plugins.md#calling-other-commands). Three layers make up this surface:

- **Builtins**: native to the editor, always available, called as plain Scheme: `(buffer-text pane)`, `(bind-key! ...)`. Some are thin Scheme wrappers that add keyword arguments and defaults; the signature documented here is the wrapper's.
- **[Language Server API](lsp-api.md)**: the builtins for registering, configuring, and requesting from language servers.
- **[Standard Library](standard-library.md)**: `core:stdlib`, an optional bundled *plugin*. Its commands are reached through `call!`, like any other plugin's: `(call! "stdlib/find" pred? lst)`.

This page is a lookup reference: tables of signatures and one-line effects. For narrative walkthroughs and worked examples, see [Writing Plugins](writing-plugins.md), [Language Servers](lsp.md), [Configuration](configuration.md), and the other pages linked throughout.

## Commands and builtins

Editor commands and builtins are different things. A command, such as `completion-trigger` or one a plugin defines with `define-command!`, is something a key can run: bind it with `bind-key!`, or run it from Scheme with `call!`. A builtin, such as `insert-key!` or `buffer-text`, is an ordinary Scheme function: call it by name with its arguments, but it can't be bound to a key and `call!` won't find it. Something is a builtin rather than a command when it needs arguments a keypress can't supply, or when it only makes sense as a step inside a command of your own. The [Builtin Commands](builtin-commands.md) page lists every command the editor ships with; this page lists the builtins.

## Conventions

- A function whose call changes something (editor state, a registration, a process, a file) ends in `!`. Reads and functions that only build a value, like `debounce`, don't.
- Lines, columns, and char offsets are 0-based everywhere. Add 1 only when showing a line number to the user.
- A char offset runs from 0 to the length of the text. The length is where a range ends, and a jump to it lands on the last character; something attached to a character, like an inlay hint, needs an offset below it.
- One character can span several char offsets: a letter with its combining marks, an emoji sequence. An offset inside one moves to the whole character: a jump or a `'before` inlay hint lands on its start, an `'after` inlay hint after it, and a range widens to cover it.
- A position your plugin computes for a buffer raises when it is out of range. A position from outside, such as a language server's answer or a file on disk, may be out of date: a column past the end of its line moves to the end of the line, and a line that no longer exists gives `#f` (`lsp-position->char-offset`, `lsp-range->char-range`, `goto-location!`) or makes the whole call raise (`apply-text-edits!`, `apply-workspace-edit!`).
- Optional arguments are keywords with a default, like `#:cwd`, never a positional `#f` placeholder. The exception is `define-language!`, whose extensions, globs, and shebangs are positional lists you can drop from the end.
- A structured value you pass in or get back (a decoration entry, a diagnostic, a request error) is a hashmap with symbol keys: `(hash 'line 0 'text "!" 'scope "error")`, read with `(hash-ref d 'message)`. Values decoded from server JSON are JSON handles instead (see [JSON handles](lsp-api.md#json-handles)). Two kinds of value are hashmaps with string keys, the wire shape: the `lsp-*-params` results (`(hash-ref (lsp-position-params pane) "position")`) and the items `completion-top` returns, whose source is `(hash-ref item "source")`. Picker items `(display . payload)`, picker `#:actions` `(key-spec . proc)`, and `register-lsp-server!`'s `#:env` `("KEY" . "VALUE")` are pairs, not records: the first two are per-row data on a hot path, the last a map.
- Every UI opener (`show-popup!`, `show-menu!`, `show-drawer-list!`, `picker!`, `live-picker!`) returns a token, and every call that closes or changes that widget takes it. A stale token (the widget already closed or was replaced) is a no-op, so a late callback can never touch someone else's widget. `#f` — an opener's own answer when the open was dropped before it could happen — is stale by construction and a no-op the same way.
- Where a call may run depends on what it touches. Calls that read or change a live buffer, pane, selection, or widget raise at the top level of `init.scm` and in a plugin body loaded at startup, since nothing is on screen yet; call them from a command, hook, timer, or callback. So do calls that start background work that calls you back later (`after!`, `spawn-async!`, `path-programs`) and the calls that stop it (`cancel-timer!`, `cancel-async!`). Registrations (`bind-key!` and its variants without `#:keymap`, `define-command!`, `define-typed-command!`, `register-hook!`, `register-lsp-notification-hook!`, `register-completion-source!`, `define-language!`, `configure-language!`) raise inside a command, hook, timer, or callback; call them at the top level of `init.scm` or in a plugin body. `load-plugin!` and `declare-plugin!` work only at the top level of `init.scm`.
- A value from a fixed set of names (a mode, a hook name, a log level, an enum option's value) is a symbol, like `'insert`. Compare symbols with `equal?`: Steel's `eq?` checks identity, so a symbol the editor hands you is never `eq?` to one you wrote.

## Settings & statusline

| Call | Effect |
|------|--------|
| `(set-option! key value)` | Set a global option |
| `(set-buffer-option! pane key value)` | Set an option on `pane`'s buffer only |
| `(get-option key)` | Read an option's global value, ignoring any buffer override |
| `(get-buffer-option pane key)` | Read `pane`'s buffer's effective value: its own override if set, else the global default (`language` and `line-ending` are per-buffer and have no global default) |
| `(configure-statusline! left center right)` | Configure the three statusline sections, each a list of element name strings |
| `(set-statusline-text! source pane text)` | Push `text` for a `"steel:<source>"` statusline element, scoped to `pane`'s buffer; empty string clears it |

See [Reading options from Scheme](writing-plugins.md#reading-options-from-scheme) for `get-option`/`get-buffer-option`'s fallback rules, [Statusline](settings.md#statusline) for the built-in element names, and [Custom elements](settings.md#custom-elements) for `configure-statusline!`/`set-statusline-text!` together.

## Key bindings

| Call | Effect |
|------|--------|
| `(bind-key! mode key-string cmd-name [#:keymap name])` | Bind a key in `'normal`, `'insert`, `'extend`, or `'command` mode; an `'insert` or `'command` binding is a single key. With `#:keymap`, the binding goes in the keymap `name`, created by its first binding |
| `(bind-key-extend! mode key-string cmd-name [#:keymap name])` | Same, but the binding always extends the selection; `'normal` only |
| `(unbind-key! mode key-string [#:keymap name])` | Remove a binding |
| `(bind-keys! mode [#:keymap name] (key cmd) ...)`, `(bind-keys-extend! mode [#:keymap name] (key cmd) ...)`, `(unbind-keys! mode [#:keymap name] key ...)` | Batched forms of the three above |
| `(attach-keymap! pane name)` | Turn on the keymap `name` in `pane`'s buffer, ahead of the keymaps attached before it (one already attached keeps its place); a name nothing has been bound into logs a warning and does nothing |
| `(detach-keymap! pane name)` | Turn off the keymap `name` in `pane`'s buffer; a name nothing has been bound into logs a warning |
| `(set-register-prefix! name)` | Target a specific register (`0`–`9`, `k`, `c`, `b`) for the rest of the current command body's `call!`s |

See [Key bindings](configuration.md#key-bindings) for the key-string grammar and full examples, [Keys for one buffer](configuration.md#keys-for-one-buffer) for keymaps, and [Register prefix](writing-plugins.md#register-prefix) for `set-register-prefix!`.

## Commands

| Call | Effect |
|------|--------|
| `(define-command! name doc proc #:repeatable #:inline-output)` | Register `name` as an editor command |
| `(define-typed-command! name doc proc #:inline-output #:complete)` | Register `name` as a typed command, reachable as `:name` |
| `(call! name args ...)` | Dispatch any editor command (built-in or Scheme-defined), activating its plugin on demand. A built-in takes `pane`, an optional count and extend flag, then the character or pattern the few that need one act with; see [Calling other commands](writing-plugins.md#calling-other-commands) |
| `(on-next-char! callback)` | Call `callback` with the next key typed as a character, or with `#f` when that key isn't one |
| `(command-plugin name)` | The id string of the plugin that registered command `name`: `"user"` for a top-level `init.scm` definition, `"hume"` for a built-in or a name that isn't a command |
| `(command-exists? name)` | `#t` if `name` is something your plugin can use: a command reachable with `call!` (built-in, Scheme-defined, or a not-yet-loaded plugin's), or any procedure bound at the top level (a builtin, a Steel library function, or your own `define`). `#f` otherwise, including for macros such as `call!` |

`define-command!`, `define-typed-command!`, `call!`, and `on-next-char!` are covered with examples in [Defining commands](writing-plugins.md#defining-commands), [Calling other commands](writing-plugins.md#calling-other-commands), and [Reading characters](writing-plugins.md#reading-characters). A script that runs past `steel-init-budget-ms`/`steel-command-budget-ms` (see [Global options](settings.md#global-options)) is stopped where it is, and none of its error handlers run. Time spent waiting on the operating system doesn't count: running a program and waiting for it (Steel's `wait` and `wait->stdout` included), compiling a grammar, or deleting a directory tree (Steel's `delete-directory!` included). `time/sleep-ms` does count.

## Supporting several HUME versions

| Call | Effect |
|------|--------|
| `(hume-version)` | The running editor's version as `(major minor patch commit)`. `commit` is the short git hash on a development build and `#f` on a release |
| `(hume-version>=? major minor patch)` | `#t` if the running editor is at least that version |

A development build reports the release it is heading toward, so `(hume-version>=? 0 15 0)` is `#t` on a nightly of the 0.15.0 cycle. Use it to gate on a release.

To use something newer that has not shipped in a release, check for it directly instead of comparing versions:

```scheme
(if (command-exists? "set-inlay-hints!")
    (set-inlay-hints! source pane hints)
    (fallback))
```

`command-exists?` works the same for a command and for a function, including one provided by another plugin. Show the commit in a bug report if you like, but don't compare it: hashes have no order.

## Plugin lifecycle

| Call | Effect |
|------|--------|
| `(declare-plugin! name #:entry #:commands #:typed-commands #:events #:languages)` | Declares the triggers that load a plugin lazily. Used in a `manifest.scm`, or in `init.scm` for a local `./file.scm` or to give an installed plugin custom triggers; `#:entry` names another file of the plugin to load on its own triggers |
| `(load-plugin! name #:config #:ref)` | Brings in a plugin. It loads lazily when the plugin ships a `manifest.scm` and at startup otherwise; `#:config` is the only way to pass configuration; `#:ref` pins an installed plugin to a tag, branch or commit (see [Pinning a version](plugins.md#pinning-a-version)) |
| `(resolve-plugin-path name)` | The path of an installed plugin's `plugin.scm` if it exists on disk, else `#f`; raises for a malformed name and for a local `./file.scm` name |
| `(loaded-plugins)` | List of plugin names whose `plugin.scm` has finished loading, local files included; a plugin with only another `#:entry` file loaded is not listed |
| `(declared-plugins)` | List of plugin names named by `load-plugin!` or `declare-plugin!`, `core:*` included, whether or not they are installed; local `./file.scm` entries are not listed |
| `(declared-plugin-ref name)` | The `#:ref` the last `load-plugin!` of `name` passed, or `#f` |
| `(plugin-config)` | The `#:config` value the user passed to `load-plugin!`, or an empty hash |
| `(plugin-dir)` | The directory holding the calling plugin's own files, or `#f` outside a plugin body |

Full picture (load timing, `#:config` semantics, dependency checks) in [Plugins](plugins.md), particularly [How plugins are loaded](plugins.md#how-plugins-are-loaded) and [Depending on another plugin](writing-plugins.md#depending-on-another-plugin).

## Hooks

| Call | Effect |
|------|--------|
| `(register-hook! name proc)` | Register `proc` for lifecycle event `name`. Top-level/plugin-body only, not inside a command |

See [Hooks](writing-plugins.md#hooks) for the full table of hook names and their lambda signatures.

## Logging

| Call | Effect |
|------|--------|
| `(log! severity message)` | Report `message` at `severity`: in the statusline, in `:messages`, or both (see below) |

`severity` is one of `'trace`, `'info`, `'warn`, `'error`; anything else raises. Where a message ends up depends on severity: `'info` only flashes in the statusline (never kept in `:messages`); `'warn` and `'error` do both; `'trace` goes to `:messages` only, never the statusline.

Steel's own print functions (`display`, `displayln`, `print`, `println`, `write`, `write-string`, `write-char`, `newline`, `simple-display`, `simple-displayln`) print nothing once the editor is running, except inside an `#:inline-output` command. They work while `init.scm` loads, and always when given an explicit port. Use `log!` to report from a command.

## Buffers, panes & selections

Almost every call on this page takes a **pane** value: an opaque handle that names a buffer and, where relevant, the specific pane it's associated with. You get one as a command or hook's own leading argument, from `(focused-pane)`, or from `(buffers)`/`(panes)`/`(buffer-panes pane)` below. Never build one by hand. A pane value that carries no pane (most hook arguments, and every entry `(buffers)` returns) still works for anything that only needs the buffer (`buffer-text`, `buffer-path`, the decoration setters, and so on); a call that needs the pane itself (`buffer-cursor-line`, `buffer-selections`, `viewport-range`, `switch-to-buffer!`, the UI openers) raises if the pane component is missing, if that pane has since closed, or if it no longer shows the buffer the value named.

If a buffer is shown in more than one pane and you need a *specific* one rather than whatever value you already have, use `(buffer-panes pane)` (below) to list them and pick explicitly: a debounced or async continuation that fires after focus has moved to a different pane on the same buffer should capture the pane it cares about up front, not assume one.

A value naming a closed buffer raises for almost every call: the reads that need a pane (`buffer-cursor-line`, `buffer-selections`, `symbol-under-cursor`, `selections-linewise?`/`selections-charwise?`, `viewport-range`, the `lsp-position-params`, `lsp-primary-range-params` and `lsp-linewise-ranges-params` calls), every call that writes to a buffer, and the reads of its bookkeeping (`buffer-path`, `buffer-text`, `buffer-generation`, `buffer-undo-tree`, `goto-revision!`, `get-buffer-option`, the decoration setters, `apply-text-edits!`, `lsp-request!`, …). Only `buffer-key`, `buffer-live?`, `pane-live?`, `diagnostic-counts`, and `diagnostics-for-buffer` answer for one; the last two answer with zero counts and an empty list, the same as for a buffer with no diagnostics. Check `buffer-live?` first to tell the two apart.

| Call | Effect |
|------|--------|
| `(focused-pane)` | The pane focused right now, paired with its buffer: a live read. Reach for this from a response callback, a timer, or anything else with no pane of its own to act on, e.g. checking `(equal? pane (focused-pane))` before acting on a response, when `pane` was captured before an async request went out. A command body doesn't need this: it receives the pane it was invoked through as its own leading parameter (see [Defining commands](writing-plugins.md#defining-commands)) |
| `(buffers)` | List of every open buffer, as pane-less pane values, in open-order |
| `(buffer-live? pane)` | `#t` if `pane`'s buffer is still open, `#f` otherwise; never raises. The idiom for a timer, debounce, or async callback whose captured value may have closed by the time it fires: check this before calling anything that would otherwise raise on a closed buffer |
| `(pane-live? pane)` | `#t` if `pane` names a pane that still exists and still shows its own buffer, `#f` otherwise, including for a value that carries no pane; never raises. The same idiom as `buffer-live?` for a callback that calls a pane-aware builtin such as `lsp-request!` |
| `(panes)` | List of every open pane, including panes in other tabs |
| `(buffer-panes pane)` | Every pane currently showing `pane`'s buffer: the focused pane first if it shows that buffer, then the rest of the active tab, then other tabs. `(car (buffer-panes pane))` picks the same one a plugin would want by default when it just needs *some* pane on the buffer |
| `(buffer-key pane)` | An opaque, comparable value naming just `pane`'s buffer: two pane values for the same buffer (even with different panes, or no pane at all) produce equal keys. Use this, not the pane value itself, as the key in a hash table a plugin keeps for its own per-buffer state |
| `(buffer-path pane)` | Absolute path string, or `#f` for an unsaved buffer |
| `(buffer-display-path pane #:relative)` | Display-ready path (absolutized, `~`-collapsed). With `#:relative #t`, relative to the working directory when the file is inside it. Print it, never use it for filesystem I/O; `#f` for an unsaved buffer |
| `(buffer-name pane)` | Display name: a view buffer's label (e.g. `[messages]`), else the filename, else `"*scratch*"` |
| `(buffer-dirty? pane)` | `#t` if the buffer has unsaved edits |
| `(buffer-text pane)` | Full live content as a string |
| `(buffer-lines pane #:range)` | Content as a list of lines, each with its ending stripped; `#:range` is `(hash 'start-line s 'end-line e)`, 0-based and end-exclusive, the shape `viewport-range` returns |
| `(buffer-line-count pane)` | Content line count, cheaper than `(length (buffer-lines pane))` |
| `(buffer-char-count pane)` | Length in characters, the final line break included, cheaper than `(string-length (buffer-text pane))` |
| `(buffer-cursor-line pane)` | 0-based line of the primary cursor in `pane`'s own pane; never the empty line after the final line break |
| `(buffer-selections pane)` | List of opaque selections, one per selection in `pane`'s own pane; read each through the [`stdlib/selection-*` accessors](standard-library.md#selections). Anchor and head are each the start of a character (a letter with its combining marks counts as one), and start..end (exclusive) is what the selection covers |
| `(char-offset->line pane idx)` | 0-based line containing 0-based char offset `idx` in the buffer's text; the offset at the end of the text is on the last line. Raises when `idx` is past the end of the text |
| `(line->char-offset pane line)` | 0-based char offset where 0-based content line `line` starts; raises when `line` is past the last line |
| `(viewport-range pane)` | `(hash 'start-line first-line 'end-line end-line)` shown in `pane`'s own pane, 0-based end-exclusive; with wrapping on, `'end-line` may run a few lines past the bottom edge |
| `(open-buffer! path)` | Open `path`, returning a pane-less pane value for it |
| `(close-buffer! pane)` | Close a buffer. Raises while a `\|` runs in it: stop it first with `(call! "cancel-pipe" pane)`, which returns whether one was running |
| `(switch-to-buffer! pane target)` | Redirect `pane`'s own pane to `target`'s buffer |
| `(show-read-only-buffer! pane name text)` | Show `text` in `pane`'s own pane, in a read-only buffer named `[name]` with the cursor on the first line. Calling it again with the same `name` replaces the text in that buffer. Raises for an empty `name` and for `messages`, `plugin-status` and `lsp-status`, which HUME's own commands use |
| `(buffer-generation pane)` | Int, bumped by every mutation to the buffer: a staleness token for comparing against a stored snapshot |
| `(buffer-undo-tree pane)` | The buffer's undo history as a list with one `(hash 'id 'parent 'age-secs 'current? 'saved?)` per revision, in `'id` order, so a parent comes before its children. `'parent` is `#f` for the root, `'age-secs` is whole seconds since the revision was made, `'current?` marks the revision the buffer is on and `'saved?` the one last saved |
| `(selections-linewise? pane)` | `#t` if every selection in `pane`'s own pane covers whole lines. A cursor sitting alone on a blank line doesn't count either way (it neither satisfies this nor breaks it when a real whole-line selection is also present), and `#f` if every selection is such a cursor |
| `(selections-charwise? pane)` | `#t` if none of the selections in `pane`'s own pane cover whole lines, with the same blank-line-cursor exception as above; `#t` if every selection is such a cursor |
| `(symbol-under-cursor pane)` | The identifier under the primary cursor in `pane`'s own pane, as a string; `""` when the cursor is not on a word character |
| `(pane? v)` | `#t` if `v` is an opaque pane value |

`buffer-text`, `buffer-lines`, `buffer-line-count`, `buffer-char-count`, `buffer-selections`, `char-offset->line`, `line->char-offset`, and `viewport-range` are covered with examples in [Reading selections](writing-plugins.md#reading-selections) and [Reading buffer text](writing-plugins.md#reading-buffer-text). Every `pane` argument here is an opaque value from one of these functions, or the one a command receives as its own leading parameter (see [Defining commands](writing-plugins.md#defining-commands)); there's no "current buffer" shortcut baked into any builtin itself. Compare two pane values with plain `equal?`; there's no dedicated equality builtin.

## Editing & navigation

| Call | Effect |
|------|--------|
| `(apply-text-edits! pane edits #:expect-generation)` | Apply a list of edits to `pane`'s buffer, mapped through `pane`'s own selections (when `pane` names no pane, or a pane that now shows another buffer, the buffer is edited for no pane: every pane's selections are carried through, and it raises while an insert or paste session is open on the buffer), each entry a JSON handle onto a wire `TextEdit` (e.g. a `textDocument/formatting` response element, passed straight through). Raises on an empty list, so check a formatting response for `[]` first, on edits from servers with different position encodings, and on an edit whose range ends on a line past the end of the document, in which case no edit is applied |
| `(goto-location! pane loc)` | Move `pane`'s own pane to `loc`: an LSP `Location`/`LocationLink` JSON handle from a server response, `(hash 'target path 'line l 'char-col c)` with `path` a file path (never a URI such as `file://…`; write a relative path whose first part holds a colon as `./part:x`) and `l`/`c` 0-based and char-indexed, or `(hash 'target buffer 'char-offset o)` with `buffer` a pane value. Returns `#t` after the jump, and `#f` when the line `loc` names no longer exists in its file, in which case nothing moves. A `'char-offset` past the end of the buffer raises |
| `(goto-revision! pane id)` | Move `pane`'s buffer to revision `id` of its undo history, a number from `buffer-undo-tree`, across branches. Raises when the buffer has no such revision (it was never made, or the `undo-levels` limit dropped it) and when the buffer is read-only. Redo then follows the branch it entered |
| `(insert-key! pane key)` | Run `key`'s normal Insert-mode behaviour (indent-style-aware Tab, auto-pairs, auto-indented Enter, …) on `pane`, as if it had no Insert-mode binding; `key` is one chord in `bind-key!`'s own syntax. Raises for a sequence of more than one key, and for a key with no default Insert-mode behaviour |
| `(paste-text! pane text)` | Paste `text`, a string or a list of strings, where you're typing, as a terminal paste would. Raises when `pane` isn't the focused pane, or when the buffer is busy with a running `|` |
| `(edit-text! pane edits)` | Replace char ranges of `pane`'s buffer as one edit, each entry a `(hash 'start-char s 'end-char e 'text t)`: an empty range inserts, empty text deletes |

`#:expect-generation` guards against applying a stale edit: pass a `buffer-generation` snapshot and the call fails if the buffer has mutated since. `apply-text-edits!`/`goto-location!`'s wire shape, like [`apply-workspace-edit!`](lsp-api.md#language-servers)'s, each decode their positions with the encoding of the server that sent them. A plain hashmap you build by hand (not pulled from a response via `json-ref`/`json-list`) has no such encoding to decode with, and is rejected.

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

`paste-text!` does whatever pasting `text` would do at that moment. In Insert mode it goes in at every cursor, with no auto-pairs or auto-indent; in Normal mode it replaces the selections; on the command line, in a prompt, a search or a picker, line breaks become spaces. A list of strings, such as `read-register`'s, is placed the way `p` places a register: one string per selection when there are as many of each, and all of them joined otherwise, which is what a prompt always gets. A character an `on-next-char!` callback is waiting for ends without one, as on a paste. Called from an Insert-mode key's binding, `.` repeats it by running the binding again; called from a hook or a timer during Insert mode, `.` repeats the paste. For example, a key that inserts the working directory, in Insert mode and on the command line:

```scheme
(define-command! "insert-cwd" "Insert the working directory."
  (lambda (pane) (paste-text! pane (cwd))))
(bind-key! 'insert "ctrl-g" "insert-cwd")
(bind-key! 'command "ctrl-g" "insert-cwd")
```

`edit-text!` is for a change that differs from one selection to the next, which commands and `paste-text!` can't make since they do the same thing at every selection (see [Example: Markdown headings](writing-plugins.md#example-markdown-headings)). Every range counts against the text as it was before the call, the entries can come in any order, and two insertions at the same offset go in list order. One `u` undoes the whole list. Selections in every pane showing the buffer follow the text: a cursor where text is inserted moves past it, and one on replaced or deleted text lands where the new text starts. A range edge inside a character made of several char offsets widens to cover the character, and an insertion inside one moves to its start. Text inserted at the very end of the buffer gets its own line break, and the buffer's last line break is never deleted. An empty list does nothing. Nothing changes, and the call raises, for an offset past the end of the buffer, a range that ends before it starts, ranges that overlap, and a read-only buffer.

## Registers

| Call | Effect |
|------|--------|
| `(write-register! name values)` | Store `values` (a list of strings, one per selection) in register `name` |
| `(read-register name)` | Contents of register `name` as a list of strings, or `#f` if it's empty |
| `(register-name? name)` | `#t` when `name` is a register these two accept, `#f` for any other string |

Both ends speak the same list shape. A string a script writes pastes as whole lines when it ends in a newline and inline otherwise, so `(write-register! "3" (read-register "3"))` keeps the text but loses the shape of an entry that was yanked inline and happens to end in a newline. Valid names are `0`–`9`, `k` (kill-ring head), `c` (system clipboard), and `b` (black hole), the same set the [`"` register prefix](copy-and-paste.md#register-prefix) accepts. Writing `k` behaves like a yank to the kill ring; writing `b` discards silently; reading an unwritten register, `b`, or a register holding a recorded macro all answer `#f`.

## Language & syntax

| Call | Effect |
|------|--------|
| `(define-language! name exts globs shebangs)` | Define a language, or replace how an existing one is detected. `exts`, `globs`, and `shebangs` default to `'()` |
| `(configure-language! name #:roots #:language-id #:comment-tokens #:block-comment-tokens)` | Set a defined language's project root markers (a list of file names), the identifier language servers know it by when that differs from `name`, its line comment tokens (a list of strings) and its block comment pairs (a list of `(start . end)` string pairs). Each keyword left out leaves that setting as it is. A value from `init.scm` beats a plugin's in either order; of two values from `init.scm`, or two from plugins, the first wins and the later one is ignored with a warning |
| `(bundled-language-grammar name)` | The grammar HUME ships for language `name`, as `(url rev symbol subpath)`, or `#f` when it ships none |
| `(bundled-grammar-languages)` | Every language HUME ships a grammar for, sorted |
| `(grammar-url g)`, `(grammar-rev g)`, `(grammar-symbol g)`, `(grammar-subpath g)` | One field of a grammar `bundled-language-grammar` returns |
| `(register-grammar! name grammar-path symbol highlights-path #:injections #:textobjects #:indents)` | Register an already-compiled tree-sitter grammar. From `init.scm` it loads the first time a file or an embedded block needs it; from a command it loads at once and raises if it can't |
| `(language-has-grammar? name)` | `#t` if `name` has a registered grammar that hasn't failed to load |

`define-language!`/`configure-language!`/`register-grammar!` are covered with examples in [Teach HUME a new language](syntax-highlighting.md#teach-hume-a-new-language).

## Diagnostics & decorations

Not LSP-specific (any plugin can populate these), but LSP diagnostics and inlay hints are the heaviest client. The LSP-only calls are on the [Language Server API](lsp-api.md) page.

| Call | Effect |
|------|--------|
| `(diagnostics-for-buffer pane #:severity #:range)` | Up to 1000 diagnostics for the buffer, floored by severity symbol (default: the `lsp-diagnostics-min-severity` option) and optionally restricted to a `(hash 'start-char s 'end-char e)` char range, which raises if it ends before it starts or past the end of the buffer. Each entry is a hash with `'start-char` (a diagnostic past the last character of the buffer starts on that character), `'end-char`, `'start-line`, `'end-line` (one past the last line the diagnostic touches), `'grapheme-col`, `'severity` (`'error`, `'warning`, `'info`, or `'hint`), `'severity-rank`, `'message`, `'code`, `'source`, `'raw` (a JSON handle onto the wire diagnostic), and `'server` (the server that published it) |
| `(diagnostic-counts pane)` | `(hash 'errors n 'warnings n)` for the buffer |
| `(set-inlay-hints! source pane hints)` | Replace `source`'s inlay hints for the buffer. `hints`: list of `(hash 'char-offset o 'text t 'side 'before)`, `'side` `'before` or `'after`. An offset inside a character (a letter with its combining marks, an emoji sequence) puts a `'before` hint before the whole character and an `'after` hint after it |
| `(register-sign-source! name pane priority)` | Reserve a gutter sign slot for `name` on the buffer, ranked by `(priority desc, name asc)` among every source registered for it |
| `(set-signs! source pane signs)` | Replace `source`'s gutter signs for the buffer. `signs`: list of `(hash 'line l 'text t 'scope s)`; `source` must already be registered |
| `(set-virtual-lines! source pane lines)` | Replace `source`'s virtual (ghost) lines for the buffer. `lines`: list of hashmaps with `'line`/`'text` required, optional `'anchor` (`'before`/`'after`), `'scope`, `'segments` (a list of `(hash 'start-char 'end-char 'scope)` char ranges into `'text`; an edge inside a character widens the segment to the whole character, and two segments sharing a character raise) |
| `(set-eol-text! source pane lines #:hide-on-insert-line #f)` | Replace `source`'s end-of-line text for the buffer. `lines`: list of `(hash 'line l 'text t 'scope s)`. With `#:hide-on-insert-line #t`, the text is hidden on the line the cursor is on while you type in Insert mode |
| `(set-extra-highlights! source pane spans)` | Replace `source`'s extra syntax highlights for the buffer. `spans`: list of `(hash 'start-char s 'end-char e 'scope sc)` char ranges; an edge inside a character widens the span to the whole character, and the call raises for an empty or reversed span or one past the end of the buffer |
| `(set-line-backgrounds! source pane entries)` | Replace `source`'s full-line background tints for the buffer. `entries`: list of `(hash 'line l 'scope s)` |

`diagnostics-for-buffer` and the hook that feeds it are shown in [Hooks](writing-plugins.md#hooks). A sign source's gutter slot is reserved the first time it registers for a buffer, even before placing any sign, which is what keeps the gutter's width stable as signs come and go; there's no `unregister-sign-source!`, and re-registering the same `name` for the same buffer just replaces its priority. Line backgrounds have no priority: same-line entries from different sources break ties by source name instead.

## Completion

A plugin registers a completion *source*; the editor drives it. `Ctrl-Space` in Insert mode (the built-in `completion-trigger` command) asks every source registered for the buffer, a registered trigger character asks the source registered under its name, the first `Tab` on a `:` command line asks the source that command declared, and the first `Tab` on the `|` prompt asks the source for the shell the `shell` option runs. Each answer is ranked with every other source's, in one menu, against that source's own token; no source renders its own UI.

Full walkthrough (a static source, richer items, trigger characters, an asynchronous `:` command argument source) and every option `register-completion-source!` takes are in [Completion sources](writing-plugins.md#completion-sources). `#:complete` on `define-typed-command!` also accepts a built-in source such as `"path"`. The per-server variants of the two trigger calls are on the [Language Server API](lsp-api.md#language-servers) page. See [Hooks](writing-plugins.md#hooks) for the `on-trigger-char` and `on-completion-accept` lambda signatures.

| Call | Effect |
|------|--------|
| `(register-completion-source! name proc #:target #:match #:priority #:resolve #:token-chars #:cancel)` | Register `proc` as completion source `name` (see [Source options](writing-plugins.md#source-options)) |
| `(completion-emit! id items #:incomplete #:more #:span #:prefix)` | Answer request `id` with `items`, or with a part of the answer when `#:more` is `#t`; `#f` if the answer was dropped as out of date |
| `(register-shell-dialect! name #:programs #:program-extensions #:parse #:write)` | How `Tab` on the `\|` prompt reads and writes a word for the shells `#:programs` names (see [Shell dialects](writing-plugins.md#shell-dialects)) |
| `(set-completion-triggers! source language chars)` | Characters that invoke `'buffer` source `source` when typed in a `language` buffer; replaces the previous set, and `'()` removes it. An unknown `source` is reported in the message log |
| `(set-hook-triggers! source language chars)` | Characters that fire the `on-trigger-char` hook, tagged `source`, when typed in a `language` buffer; replaces the previous set, and `'()` removes it. For features other than completion, such as signature help |
| `(completion-top n)` | The top `n` ranked items of the open menu, each carrying its `source` |
| `(completion-accept! idx)` | Accept item `idx` from `completion-top`'s (ranked) order; fires the `on-completion-accept` hook. Raises when no Insert-mode completion menu is open |
| `(completion-dismiss!)` | Close the open menu; a no-op if none is open |

## Pickers

These are editor-builtin commands any plugin can drive: a plugin opens a picker and pushes items through them rather than building its own fuzzy-finder.

Full walkthroughs (batch vs. streaming population, truncation direction, exit-code handling, live requery) are in [Custom pickers](writing-plugins.md#custom-pickers) and [Live requery (live grep)](writing-plugins.md#live-requery-live-grep).

| Call | Effect |
|------|--------|
| `(picker! pane items on-select #:prompt #:pending #:query #:truncate #:actions)` | Open a fuzzy-finder panel over a fixed `items` list of `(display . payload)` pairs, only while `pane` is the focused pane (raises otherwise) |
| `(live-picker! pane on-select #:source #:prompt #:query #:debounce-ms #:truncate #:actions)` | Open a picker whose query runs `#:source` (a function of the picker token and the query) on every keystroke, debounced, only while `pane` is the focused pane (raises otherwise) |
| `(picker-command-source argv-for #:cwd #:nul #:ok-exit-codes)` | A `#:source` for `live-picker!` that runs the subprocess `argv-for` names for the query |
| `(picker-push! token items)` | Append a batch of `(display . payload)` items to an open picker. A `live-picker!` source passes the token it was called with: once the user has typed on, that token no longer feeds the picker and the call returns `#f` |
| `(picker-replace! token items)` | Replace an open picker's items wholesale |
| `(picker-source-spawn! token cmd args #:cwd #:nul #:ok-exit-codes)` | Stream a subprocess's stdout lines into an open picker as items |
| `(picker-source-stop! token)` | Kill a picker's still-running spawned source |
| `(picker-close! token)` | Close the picker `token` names; a no-op if that picker has already closed or been replaced |

## Other UI widgets

| Call | Effect |
|------|--------|
| `(prompt! pane label on-confirm #:prefill)` | Open a minibuffer text prompt, only while `pane` is the focused pane (raises otherwise); `on-confirm` fires once, later, with the confirmed text or `#f` on cancel. Raises when another `prompt!` prompt is already open. When a `#:repeatable` command opened it, `.` repeats the answer without opening it; see [Dot-repeat](writing-plugins.md#dot-repeat) |
| `(show-popup! pane text #:anchor #:kind #:lang)` | Show a text popup, only while `pane` is the focused pane (raises otherwise). `#:anchor` `'cursor` (default, floats near the cursor) or `'bottom` (docks above the statusline); `#:lang` for syntax highlighting. `#:kind` also sets how long it lives: `'sticky` (default) closes on its own as soon as you leave whatever mode you opened it in; `'scrollable` stays open (Ctrl-u/Ctrl-d scroll it) until any other key, paste, or mouse input closes it. Replaces any popup already showing. Returns a token for `close-popup!`, or `#f` if the popup didn't open |
| `(close-popup! token)` | Close the popup `token` names; a no-op if it has already closed or been replaced |
| `(show-menu! pane items on-select)` | Show a selection menu over `items`, a list of strings, only while `pane` is the focused pane (raises otherwise). Returns a token for `close-menu!`, or `#f` if the menu didn't open: the editor is not in Normal or Extend mode, or moved on before the call landed. When a `#:repeatable` command opened it, `.` repeats the choice without opening it; see [Dot-repeat](writing-plugins.md#dot-repeat) |
| `(close-menu! token)` | Close the menu `token` names without calling its `on-select`; a no-op if it has already closed or been replaced |
| `(show-drawer-list! pane items on-select #:selected [0] #:render [#f])` | Show a list in the bottom drawer, over `items`, a non-empty list of strings, only while `pane` is the focused pane (raises otherwise). With `#:render`, `items` can be any values, one per row, and the drawer calls `(render start keys)` just before it shows rows it hasn't drawn yet: `keys` are the items of those rows, starting at row `start`, and `render` returns one string for each. Rows are drawn once per list, so `render` only runs again for rows scrolled into view or after `update-drawer-list!`. `render` can read the editor but not change it: an attempt to change it, an error, or a result that isn't one string per key closes the drawer, calls `on-select` with `#f`, and shows the error. Inside `render`, buffers, selections and settings can be read, while calls that need language servers, diagnostics, completion, timers or background processes raise. `#:selected` is the row to open on (clamped into `items`), scrolled into view. `on-select` is called with `(index token)` — `token` is the token of the drawer that called it. Replaces any drawer already open, and the outgoing drawer's `on-select` fires with `#f` and its own token, so its owner knows the drawer is gone and can tell that from its new drawer closing. Errors on empty `items`; close (or never open) instead. Returns a token scoping `close-drawer!`/`update-drawer-list!`/`drawer-selected-index` to this drawer (hold onto it), or `#f` if the drawer didn't open (the editor moved on before the call landed). `.` never records a drawer pick; see [Dot-repeat](writing-plugins.md#dot-repeat) |
| `(close-drawer! token)` | Close the drawer `token` names; a no-op if it has already closed or been replaced |
| `(update-drawer-list! token items on-select selected #:render [#f])` | Replace the open drawer's rows in place (`#:render` as for `show-drawer-list!`), selecting row `selected` (clamped into `items`; pass `(drawer-selected-index token)` to keep the current row); returns `#t` when applied, `#f` when no drawer is open, `token` doesn't match its own, or `items` is empty. Close instead of clearing through an update |
| `(drawer-selected-index token)` | The open drawer's selected row, or `#f` when no drawer is open or `token` doesn't match its own |

## Timers

| Call | Effect |
|------|--------|
| `(after! ms thunk)` | Call `thunk` with no args once `ms` milliseconds pass; returns a timer id |
| `(cancel-timer! id)` | Cancel a pending timer; idempotent, a no-op if `id` already fired, was cancelled, or never existed |
| `(debounce ms proc)` | Wrap `proc` so each call reschedules it `ms` out, cancelling any still-pending call from a prior invocation |
| `(debounce-by ms proc #:key)` | Same, but keyed per `(key . args)`: a call keyed one way never cancels a call keyed another. `#:key` defaults to the first argument itself; pass `#:key (lambda (pane . _) (buffer-key pane))` to key by buffer when different calls might carry different pane values for the same buffer |

## Async & subprocesses

| Call | Effect |
|------|--------|
| `(spawn-async! cmd args callback #:cwd dir)` | Run `cmd` in the background, in `dir` (default: HUME's working directory; a relative `dir` is joined onto it); `callback` (`(lambda (stdout stderr exit-code) ...)`) fires exactly once, later. Returns a job id for `cancel-async!`. A failed spawn doesn't raise: `callback` fires with empty stdout, the error in stderr, and exit code `#f`, which a signal-killed command also gets |
| `(cancel-async! id)` | Kill a still-running `spawn-async!` job, or stop a `path-programs` listing, and drop its callback, even one already due; idempotent |
| `(run-inline-output! cmd args #:cwd #:env)` | Run `cmd`, streaming output to the terminal inside an `#:inline-output` command; `#:env` is a list of `("KEY" . "VALUE")` pairs added to the environment; raises on nonzero exit |
| `(run-capture! cmd args #:cwd dir)` | Run `cmd` in `dir` (default: HUME's working directory; a relative `dir` is joined onto it), blocking until it exits; returns `(hash 'stdout s 'stderr s 'exit code)`, with `'exit` `#f` when the spawn failed (stderr says why) or a signal killed the process |

Covered with examples in [Filesystem and processes](writing-plugins.md#filesystem-and-processes).

## Diffing

| Call | Effect |
|------|--------|
| `(diff-lines old-text new-text)` | Line-level hunks where `old-text`/`new-text` differ. Each hunk has `'old-start-line`/`'old-end-line` and `'new-start-line`/`'new-end-line` (0-based, end exclusive), `'old-lines`/`'new-lines` (the text of each of those lines), and `'words`, `(hash 'old spans 'new spans)`: an old-side span is `(hash 'line 'start-char-col 'end-char-col)`, a line of the old text and character columns in it; a new-side span is `(hash 'start-char 'end-char)`, character offsets into the new text, counting each line ending as one character. Spans mark the words that changed inside the hunk; a span covering a whole line is left out |
| `(diff-buffer-lines pane ref-text)` | Same, but against the buffer's current unsaved content; avoids pulling the whole buffer through `buffer-text` first |
| `(buffer-revision-diff pane id)` | Hunks between revision `id` of the buffer's undo history, a number from `buffer-undo-tree`, and the buffer's current content, in the shape `diff-buffer-lines` returns with the revision as the old side. Raises when the buffer has no such revision |
| `(diff-words old-text new-text)` | `(hash 'hunks … 'deadline-hit …)`: word-level hunks within a single changed line |

Covered with examples, including hunk shapes, in [Comparing text](writing-plugins.md#comparing-text).

## Text & strings

Everyday string work (trimming, splitting on a separator, case conversion, prefix/suffix tests, search and replace) uses Steel's own string functions directly; see Steel's [string reference](https://mattwparas.github.io/steel/book/builtins/steel_strings.html). `string-length`, `substring`, and `string-ref` count Unicode characters (codepoints), not bytes and not on-screen columns.

| Call | Effect |
|------|--------|
| `(split-words line word-chars)` | Every word in `line`, in order: the same classification `w`/`b` motions and text objects use, so a word here is exactly what one of those would select. `word-chars` extends what counts as a word, same as the buffer option of the same name (`""` for none) |
| `(string-common-prefix strings)` | The longest start every string in `strings` shares, never ending partway through a character the editor shows as one (an accented letter written as two code points, an emoji sequence); `""` for an empty list |

## Filesystem & directories

| Call | Effect |
|------|--------|
| `(data-dir)` | HUME's data directory, or `#f` if unavailable |
| `(runtime-dir)` | HUME's runtime directory, or `#f` if unavailable |
| `(cwd)` | HUME's working directory (what `:pwd` prints) |
| `(set-cwd! path)` | Change HUME's working directory, like `:cd`; a relative `path` is joined onto the current one; raises if `path` is not a directory. Works from `init.scm` too |
| `(replace-dir! src dest)` | Move the directory `src` to `dest` (both relative to the working directory), replacing the directory `dest` held and creating `dest`'s parent if needed. Either `src` ends up at `dest` or the call raises and leaves `dest` as it was, even if the script budget runs out during it |
| `(try-lock-file! path)` | Take an exclusive lock on the file `path` (relative to the working directory), creating the file if needed: `#t` when taken, `#f` when another HUME or program holds it. The lock lasts until `unlock-file!`, until the command, hook or callback that took it returns or is stopped, or until HUME exits, so a lock is never left behind. Taking a lock the same command already holds raises |
| `(unlock-file! path)` | Release a lock `try-lock-file!` took; raises when this command doesn't hold it |
| `(path-join seg ...)` | Join path segments with the OS-native separator |
| `(safe-path-segment? name)` | `#t` iff `name` is a string safe to use as one path component: not empty, not `.` or `..`, and free of a path separator, `:`, `"` and NUL; `#f` for anything else. Use it before joining a user-typed or downloaded name onto a path |
| `(path-entries dir prefix)` | The names in directory `dir` (relative to the working directory) starting with `prefix` (ignoring letter case on Windows), sorted, each directory's ending in `/` (a link to a directory included), as `:e` completes them: hidden entries only when `prefix` starts with `.`. `'()` when `dir` can't be read |
| `(path-programs prefix extensions callback)` | List the programs on `PATH` whose names start with `prefix` in the background. `callback` (`(lambda (names done?) ...)`) is called as `PATH` is read, with every name found so far, sorted, and a last time with `done?` `#t`. A `callback` that raises stops the listing. Returns an id for `cancel-async!`. On Windows a program is a file ending in one of `extensions` (`'(".exe")`), named without it |
| `(home-dir)` | The user's home directory, or `#f` when the environment names none |
| `(path->display path)` | Run an absolute `path` string through HUME's display-form pipeline (Windows `\\?\` stripping, `~`-collapse); no filesystem access |
| `(read-data-file path)` | The one Scheme value written in the file at `path`; `;` comments are allowed around it. Raises an error naming the file when its text doesn't parse or holds more than one value. Use it instead of Steel's `read` for data files: `read` returns end-of-file instead of raising on bad syntax, and leaves the bad text for the next `read` |
| `(data-file dir name)` | The path of `name.scm` in `dir` when `dir` holds that file, else `#f`. Only a name spelled exactly like one of `dir`'s files matches, so `Rust` does not match `rust.scm` on any platform and `../x` never reaches outside `dir` |
| `(data-file-names dir)` | The names of `dir`'s `.scm` files without the suffix, sorted; `'()` when `dir` doesn't exist |

The pattern for reading a plugin's own files is covered in [Filesystem and processes](writing-plugins.md#filesystem-and-processes).

HUME has one working directory: Steel's `current-directory` and `change-current-directory!` are `(cwd)` and `(set-cwd! path)`, and `:cd` moves it too. A relative path given to a HUME call resolves against it the way `:e` resolves a typed one: `..` collapses and links stay as typed. Steel's own file functions resolve a relative path against the same directory.

## Grammar compilation

| Call | Effect |
|------|--------|
| `(compile-grammar! src out)` | Compile the tree-sitter grammar source at `src` to `out`. Raises on failure, except while `init.scm` loads, where it logs a warning instead |
