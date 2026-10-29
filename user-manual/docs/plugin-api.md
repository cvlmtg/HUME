# Plugin API

Reference for the editor functions a plugin or `init.scm` can call. The editor commands keys run are on [Editor Commands](editor-commands.md); [Commands and functions](#commands-and-functions) explains the difference. This surface has three layers:

- **Editor functions**: native, always available, called by name as plain Scheme: `(buffer-text pane)`, `(bind-key! ...)`. Some are thin Scheme wrappers that add keyword arguments and defaults; this page documents the wrapper's signature.
- **[Language Server API](lsp-api.md)**: the editor functions for registering, configuring, and sending requests to language servers.
- **[Standard Library](standard-library.md)**: `core:stdlib`, an optional bundled *plugin*. Reach its commands through `call!`, like any plugin's: `(call! "stdlib/find" pred? lst)`.

For walkthroughs and worked examples, see [Writing Plugins](writing-plugins.md), [Language Servers](lsp.md), [Configuration](configuration.md), and the pages linked throughout.

## Commands and functions

A script can run three kinds of things:

| Kind | Examples | Run from | Bound to a key | `call!` | Listed in |
|------|----------|----------|----------------|---------|-----------|
| **Editor command** | `move-left`, `completion-trigger`, one a plugin defines with `define-command!` | A key, or a script | Yes | Yes | [Editor Commands](editor-commands.md) for the built-in ones, a plugin's own docs for its commands |
| **Typed command** | `:write`, one defined with `define-typed-command!` | The `:` prompt | No | No | [Command mode](command-mode.md), a plugin's own docs |
| **Editor function** | `buffer-text`, `edit-text!`, `bind-key!` | A script | No | No | This page and the [Language Server API](lsp-api.md) |

An editor command takes what a key press supplies: the pane, then an optional count and extend flag. Call a built-in one by name: `(move-left pane)` is the same as `(call! "move-left" pane)`. Nine built-in commands also act with a character or pattern that the key press asks for, and a script passes it as an argument: `(call! "search-forward" pane "foo")` searches as if you had typed `foo` at the `/` prompt, moving the cursor to the match. To search without moving anything, use [`buffer-search`](#buffers-panes-selections). [Calling other commands](writing-plugins.md#calling-other-commands) lists them.

An editor function takes the arguments its entry lists. Something is an editor function rather than a command when it needs arguments a key press can't supply, or only makes sense as a step inside your own command.

Steel's library functions and your own `define`s are plain Scheme functions, not editor functions.

## Conventions

- A function whose call changes something (editor state, a registration, a process, a file) ends in `!`. Reads, and functions that only build a value like `debounce`, don't.
- A call on a pane takes the pane first; one on a whole language takes the language first, then what it names there: `(set-signs! pane source signs)`, `(set-hook-triggers! language source chars)`.
- Lines, columns, and char offsets are 0-based everywhere. Add 1 only to show a line number to the user.
- `'start-char`/`'end-char` are char offsets into a whole text: the buffer, or a text you passed in, as with `diff-words`. `'start-char-col`/`'end-char-col` are char columns within one line, such as a virtual line's `'segments`.
- A char offset runs from 0 to the length of the text. The length is where a range ends, and a jump to it lands on the last character; something attached to a character, like an inlay hint, needs an offset below it.
- One character can span several char offsets: a letter with its combining marks, an emoji sequence. An offset inside one moves to the whole character: a jump or a `'before` inlay hint lands on its start, an `'after` inlay hint after it, and a range widens to cover it.
- A position your plugin computes for a buffer raises when out of range. A position from outside, such as a language server's answer or a file on disk, may be out of date: a column past the end of its line moves to the line's end, and a line that no longer exists gives `#f` (`lsp-position->char-offset`, `lsp-range->char-range`, `goto-location!`) or makes the whole call raise (`apply-text-edits!`, `apply-workspace-edit!`).
- Optional arguments are keywords with a default, like `#:cwd`, never a positional `#f` placeholder. The exception is `define-language!`, whose extensions, globs, and shebangs are positional lists you can drop from the end.
- A structured value you pass in or get back (a decoration entry, a diagnostic, a request error, a plugin's `#:config`) is a hashmap with symbol keys: `(hash 'line 0 'text "!" 'scope "error")`, read with `(hash-ref d 'message)`. The exceptions:
  - Values decoded from server JSON are JSON handles (see [JSON handles](lsp-api.md#json-handles)).
  - The `lsp-*-params` results (`(hash-ref (lsp-position-params pane) "position")`) and the items `completion-top` returns (source: `(hash-ref item "source")`) have string keys, the wire shape.
  - Picker items `(display . payload)`, picker `#:actions` `(key-spec . proc)`, and `register-lsp-server!`'s `#:env` `("KEY" . "VALUE")` are pairs, not records.
  - A selection from `buffer-selections` is not a record either: read it with the `selection-*` calls.
- Every UI opener (`show-popup!`, `show-menu!`, `show-drawer-list!`, `picker!`, `live-picker!`) returns a token, which every call that closes or changes that widget takes. A stale token (the widget already closed or was replaced) is a no-op, so a late callback can never touch someone else's widget. An opener returns `#f` when the open was dropped before it could happen, and `#f` is a stale token too.
- Where a call may run depends on what it touches:
  - Calls that read or change a live buffer, pane, selection, or widget raise at the top level of `init.scm` and in a plugin body loaded at startup, since nothing is on screen yet. So do calls that start background work that calls you back later (`after!`, `spawn-async!`, `path-programs`) and the calls that stop it (`cancel-timer!`, `cancel-async!`). Call them from a command, hook, timer, or callback.
  - Registrations (`bind-key!` and its variants without `#:keymap`, `define-command!`, `define-typed-command!`, `register-hook!`, `register-lsp-notification-hook!`, `register-completion-source!`, `define-language!`, `configure-language!`) raise inside a command, hook, timer, or callback. Call them at the top level of `init.scm` or in a plugin body.
  - `load-plugin!` and `declare-plugin!` work only at the top level of `init.scm`.
- A procedure you hand over to be called later (a hook handler, a callback, an `on-select`, a `#:render`) must take the arguments its call's entry lists. The call that takes it raises at once when it can't, not later when it runs. A lambda with a fitting rest parameter is accepted, as is an editor function.
- A value from a fixed set of names (a mode, a hook name, a log level, an enum option's value) is a symbol, like `'insert`. Compare symbols with `equal?`: Steel's `eq?` checks identity, so a symbol the editor hands you is never `eq?` to one you wrote.
- Steel's `hashset-difference` returns the elements in either set but not in both, not the elements of the first set missing from the second. To remove `b`'s elements from `a`, intersect first: `(hashset-difference a (hashset-intersection a b))`.

## Settings & statusline

| Call | Effect |
|------|--------|
| `set-option!` | `(set-option! key value)`{.sig} Set a global option, in the type `get-option` returns for it. A value from `init.scm`, a hook it registered, or `:set` beats a plugin's in either order; of two from the same place, the later wins. Raises for the per-buffer `language` and `line-ending` |
| `set-buffer-option!` | `(set-buffer-option! pane key value)`{.sig} Set an option on `pane`'s buffer only, ranked against the buffer's own overrides as `set-option!` is against the global value |
| `get-option` | `(get-option key)`{.sig} Read an option's global value, ignoring any buffer override. Raises for the per-buffer `language` and `line-ending` |
| `get-buffer-option` | `(get-buffer-option pane key)`{.sig} Read `pane`'s buffer's effective value: its own override if set, else the global default (`language` and `line-ending` have no global default) |
| `configure-statusline!` | `(configure-statusline! left center right)`{.sig} Set the three statusline sections, each a list of element name strings; `(get-option "statusline")` reads them back as `(hash 'left 'center 'right)` |
| `set-statusline-text!` | `(set-statusline-text! pane source text)`{.sig} Push `text` for a `"steel:<source>"` statusline element, scoped to `pane`'s buffer; an empty string clears it |

See [Reading options from Scheme](writing-plugins.md#reading-options-from-scheme) for the two option readers' fallback rules, [Statusline](settings.md#statusline) for the built-in element names, and [Custom elements](settings.md#custom-elements) for the two statusline calls together.

## Key bindings

| Call | Effect |
|------|--------|
| `bind-key!` | `(bind-key! mode key-string cmd-name [#:keymap name])`{.sig} Bind a key in `'normal`, `'insert`, `'extend`, or `'command` mode; an `'insert` or `'command` binding is a single key. With `#:keymap`, the binding goes in keymap `name`, created by its first binding. A binding from `init.scm` beats a plugin's for the same keys in either order, and a plugin's beats HUME's own; of two from the same place, the later wins |
| `bind-key-extend!` | `(bind-key-extend! mode key-string cmd-name [#:keymap name])`{.sig} Same, but the binding always extends the selection; `'normal` only |
| `unbind-key!` | `(unbind-key! mode key-string [#:keymap name])`{.sig} Unbind the keys for the calling code and everything ranked below it: a plugin's unbind removes HUME's binding but leaves the user's, and the user's keeps any plugin from binding them again |
| `bind-keys!`<br>`bind-keys-extend!`<br>`unbind-keys!` | `(bind-keys! mode [#:keymap name] (key cmd) ...)`{.sig} `(bind-keys-extend! mode [#:keymap name] (key cmd) ...)`{.sig} `(unbind-keys! mode [#:keymap name] key ...)`{.sig} Batched forms of the three above |
| `attach-keymap!` | `(attach-keymap! pane name)`{.sig} Turn on keymap `name` in `pane`'s buffer, ahead of the keymaps attached before it (an already attached one keeps its place). A name with nothing bound into it logs a warning and does nothing |
| `detach-keymap!` | `(detach-keymap! pane name)`{.sig} Turn off keymap `name` in `pane`'s buffer; a name with nothing bound into it logs a warning |
| `set-register-prefix!` | `(set-register-prefix! name)`{.sig} Target a specific register (`0`–`9`, `k`, `c`, `b`) for the rest of the current command body's `call!`s |

See [Key bindings](configuration.md#key-bindings) for the key-string grammar and full examples, [Keys for one buffer](configuration.md#keys-for-one-buffer) for keymaps, and [Register prefix](writing-plugins.md#register-prefix) for `set-register-prefix!`.

## Commands

| Call | Effect |
|------|--------|
| `define-command!` | `(define-command! name doc proc #:repeatable #:inline-output)`{.sig} Register `name` as an editor command |
| `define-typed-command!` | `(define-typed-command! name doc proc #:inline-output #:complete)`{.sig} Register `name` as a typed command, reachable as `:name` |
| `call!` | `(call! name args ...)`{.sig} Run any editor command (built-in or Scheme-defined), activating its plugin on demand. A built-in command takes `pane`, an optional count and extend flag, then the character or pattern for the few that need one; see [Calling other commands](writing-plugins.md#calling-other-commands) |
| `on-next-char!` | `(on-next-char! callback)`{.sig} Call `callback` with the next key typed as a character, or with `#f` when that key isn't one |
| `command-plugin` | `(command-plugin name)`{.sig} The id string of the plugin that registered command `name`: `"user"` for a top-level `init.scm` definition, `"hume"` for a built-in or a name that isn't a command |
| `command-exists?` | `(command-exists? name)`{.sig} `#t` if your plugin can use `name`: a command `call!` reaches (built-in, Scheme-defined, or a not-yet-loaded plugin's, but not a key-only command such as `pipe`), or any procedure bound at the top level (an editor function, a Steel library function, or your own `define`). `#f` otherwise, including for macros such as `call!` |

[Defining commands](writing-plugins.md#defining-commands), [Calling other commands](writing-plugins.md#calling-other-commands), and [Reading characters](writing-plugins.md#reading-characters) show `define-command!`, `define-typed-command!`, `call!`, and `on-next-char!` with examples.

A script that runs past `steel-init-budget-ms`/`steel-command-budget-ms` (see [Global options](settings.md#global-options)) stops where it is, and none of its error handlers run. Waiting on the operating system doesn't count: running a program and waiting for it, compiling a grammar, or deleting a directory tree with `remove-dir!`. `time/sleep-ms` does count, as does waiting in Steel's own calls, such as its `wait`.

## Supporting several HUME versions

| Call | Effect |
|------|--------|
| `hume-version` | `(hume-version)`{.sig} The running editor's version as `(hash 'major 'minor 'patch 'commit)`. `'commit` is the short git hash on a development build and `#f` on a release |
| `hume-version>=?` | `(hume-version>=? major minor patch)`{.sig} `#t` if the running editor is at least that version |

A development build reports the release it is heading toward: `(hume-version>=? 0 15 0)` is `#t` on a nightly of the 0.15.0 cycle. Use it to gate on a release.

To use something not yet shipped in a release, check for it directly instead of comparing versions:

```scheme
(if (command-exists? "set-inlay-hints!")
    (set-inlay-hints! pane source hints)
    (fallback))
```

`command-exists?` works for commands and functions alike, including another plugin's. Show the commit in a bug report if you like, but don't compare it: hashes have no order.

## Plugin lifecycle

| Call | Effect |
|------|--------|
| `declare-plugin!` | `(declare-plugin! name #:entry #:commands #:typed-commands #:events #:languages)`{.sig} Declare the triggers that load a plugin lazily: in a `manifest.scm`, or in `init.scm` for a local `./file.scm` or to give an installed plugin custom triggers. `#:entry` names another file of the plugin to load on its own triggers. Raises for an entry declared twice, a declare after the plugin's `load-plugin!`, and a command name already taken or listed twice |
| `load-plugin!` | `(load-plugin! name #:config #:ref)`{.sig} Bring in a plugin: lazily when it ships a `manifest.scm`, at startup otherwise. `#:config` is the only way to pass configuration; `#:ref` pins an installed plugin to a tag, branch or commit (see [Pinning a version](plugins.md#pinning-a-version)) |
| `resolve-plugin-path` | `(resolve-plugin-path name)`{.sig} The path of an installed plugin's `plugin.scm` if it exists on disk, else `#f`; raises for a malformed name and for a local `./file.scm` name |
| `loaded-plugins` | `(loaded-plugins)`{.sig} A list of the names of the plugins whose `plugin.scm` has finished loading, local files included; a plugin with only another `#:entry` file loaded is not listed |
| `declared-plugins` | `(declared-plugins)`{.sig} A list of the names of the plugins `load-plugin!` or `declare-plugin!` named, `core:*` included, installed or not; local `./file.scm` entries are not listed |
| `declared-plugin-ref` | `(declared-plugin-ref name)`{.sig} The `#:ref` the last `load-plugin!` of `name` passed, or `#f` |
| `plugin-config` | `(plugin-config)`{.sig} The `#:config` hash the user passed to `load-plugin!`, or an empty hash. Works in the plugin's body and in every command, hook and callback it defines; raises anywhere else |
| `plugin-dir` | `(plugin-dir)`{.sig} The directory holding the calling plugin's own files, or `#f` when HUME has no directory for that kind of plugin (a user plugin with no home directory set). Works where `(plugin-config)` does; raises anywhere else |

For the full picture (load timing, `#:config` semantics, dependency checks), see [Plugins](plugins.md), especially [How plugins are loaded](plugins.md#how-plugins-are-loaded), and [Depending on another plugin](writing-plugins.md#depending-on-another-plugin).

## Hooks

| Call | Effect |
|------|--------|
| `register-hook!` | `(register-hook! name proc)`{.sig} Register `proc` for lifecycle event `name`; raises for a lambda that can't take the event's arguments. Top level or plugin body only, not inside a command |

See [Hooks](writing-plugins.md#hooks) for every hook name and its lambda signature.

## Logging

| Call | Effect |
|------|--------|
| `log!` | `(log! severity message)`{.sig} Report `message` at `severity`: in the statusline, in `:messages`, or both (see below) |
| `print!` | `(print! text)`{.sig} Write `text` and a line break to the terminal, inside an `#:inline-output` command; raises outside one |

`severity` is one of `'trace`, `'info`, `'warning`, `'error`; anything else raises. It decides where the message goes:

- `'info` only flashes in the statusline and is never kept in `:messages`.
- `'warning` and `'error` go to both.
- `'trace` goes to `:messages` only.

Report from a plugin with `log!`, and write output the user watches with `print!`. Steel's own print functions (`display`, `displayln`, `newline`, …) write straight to the terminal, over the editor's screen; HUME does not stop them.

## Buffers, panes & selections

Almost every call on this page takes a **pane** value: an opaque handle naming a buffer and, where relevant, the pane it's associated with. You get one as a command's or hook's leading argument, from `(focused-pane)`, or from `(buffers)`/`(panes)`/`(buffer-panes pane)` below. Never build one by hand. No editor function has a "current buffer" shortcut. Compare two pane values with plain `equal?`; there's no dedicated equality function.

A pane value that carries no pane (most hook arguments, and every entry `(buffers)` returns) still works for anything that only needs the buffer (`buffer-text`, `buffer-path`, the decoration setters, and so on).

The **Pane** column of the tables on this page and the [Language Server API](lsp-api.md) page marks the calls that need the pane itself:

- *shown*: raises if the value carries no pane, if that pane has closed, or if it no longer shows the value's buffer.
- *focused*: also raises when that pane isn't the focused one.

A malformed argument is reported before either refusal. A call with no mark works with any pane value, including one that carries no pane.

`lsp-show-status!` opens its view, and checks the pane, once your call returns: if that pane is not focused by then, nothing opens and the refusal goes to `:messages` instead of raising.

A debounced or async continuation can fire after focus has moved to another pane on the same buffer. Such a continuation should capture the pane it cares about up front, not assume one. To pick a specific pane of a buffer shown in several, list them with `(buffer-panes pane)`.

A value naming a closed buffer raises for almost every call: every call marked in the Pane column, every call that writes to a buffer, and the reads of its bookkeeping (`buffer-path`, `buffer-text`, `buffer-generation`, `buffer-undo-tree`, `get-buffer-option`, the decoration setters, `apply-text-edits!`, `lsp-request!`, …). Only `buffer-key`, `buffer-live?`, `pane-live?`, `diagnostic-counts`, and `diagnostics-for-buffer` answer for one. The last two answer with zero counts and an empty list, as for a buffer with no diagnostics; check `buffer-live?` first to tell the two apart.

| Call | Pane | Effect |
|------|------|--------|
| `focused-pane` |  | `(focused-pane)`{.sig} The pane focused right now, paired with its buffer: a live read, for code with no pane of its own, such as a response callback or a timer. E.g. check `(equal? pane (focused-pane))` before acting on a response when `pane` was captured before the async request went out. A command body doesn't need it: it receives the pane it was invoked through as its leading parameter (see [Defining commands](writing-plugins.md#defining-commands)) |
| `buffers` |  | `(buffers)`{.sig} A list of every open buffer, as pane-less pane values, in open order |
| `buffer-live?` |  | `(buffer-live? pane)`{.sig} `#t` if `pane`'s buffer is still open, `#f` otherwise; never raises. A timer, debounce, or async callback whose captured value may have closed should check this before calling anything that raises on a closed buffer |
| `pane-live?` |  | `(pane-live? pane)`{.sig} `#t` if `pane` names a pane that still exists and still shows its own buffer, `#f` otherwise (including for a value that carries no pane); never raises. Use it like `buffer-live?`, in a callback that calls a pane-aware editor function such as `lsp-request!` |
| `panes` |  | `(panes)`{.sig} A list of every open pane, including panes in other tabs |
| `buffer-panes` |  | `(buffer-panes pane)`{.sig} Every pane showing `pane`'s buffer: the focused pane first if it shows that buffer, then the rest of the active tab, then other tabs. `(car (buffer-panes pane))` is the default pick when you need *some* pane on the buffer |
| `buffer-key` |  | `(buffer-key pane)`{.sig} An opaque, comparable value naming only `pane`'s buffer: pane values for the same buffer give equal keys, whatever pane they carry, if any. Use it, not the pane value, to key a hash table of per-buffer plugin state |
| `buffer-path` |  | `(buffer-path pane)`{.sig} Absolute path string, or `#f` for an unsaved buffer |
| `buffer-display-path` |  | `(buffer-display-path pane [#:relative dir])`{.sig} Display-ready path (absolutized, `~`-collapsed), or `#f` for an unsaved buffer. With `#:relative dir`, relative to `dir` when the file is inside it, such as `#:relative (cwd)` for the working directory. For printing only, never for filesystem I/O |
| `buffer-name` |  | `(buffer-name pane)`{.sig} Display name: a view buffer's label (e.g. `[messages]`), else the filename, else `"*scratch*"` |
| `buffer-dirty?` |  | `(buffer-dirty? pane)`{.sig} `#t` if the buffer has unsaved edits |
| `buffer-text` |  | `(buffer-text pane)`{.sig} Full live content as a string |
| `buffer-lines` |  | `(buffer-lines pane #:range)`{.sig} Content as a list of lines, each with its ending stripped; `#:range` is `(hash 'start-line s 'end-line e)`, end-exclusive, the shape `viewport-range` returns |
| `buffer-line-count` |  | `(buffer-line-count pane)`{.sig} Content line count, cheaper than `(length (buffer-lines pane))` |
| `buffer-char-count` |  | `(buffer-char-count pane)`{.sig} Length in characters, the final line break included, cheaper than `(string-length (buffer-text pane))` |
| `buffer-search` |  | `(buffer-search pane pattern)`{.sig} `(hash 'start-char s 'end-char e)` for the first match of `pattern` in the buffer, or `#f` when nothing matches. `pattern` is what you'd type at the `/` prompt: a regex, all-lowercase ignoring case, `v/` for a literal pattern. Raises for an empty or invalid pattern. Moves no selection and leaves the current search as it was, unlike `(call! "search-forward" pane pattern)` |
| `word-search-pattern` |  | `(word-search-pattern pane word)`{.sig} The pattern `*` would search for on `word` in this buffer: `word` taken literally, as a whole word under the buffer's `word-chars`, ready for `buffer-search` or the `/` prompt. Raises for an empty `word` |
| `buffer-cursor-line` | shown | `(buffer-cursor-line pane)`{.sig} Line of the primary cursor in `pane`'s own pane; never the empty line after the final line break |
| `buffer-selections` | shown | `(buffer-selections pane)`{.sig} A list of opaque selections in `pane`'s own pane, one per selection, sorted by start; read each with the `selection-*` calls below. Anchor and head each sit at the start of a character, and start..end (exclusive) is what the selection covers |
| `selection-anchor`<br>`selection-head` |  | `(selection-anchor sel)`{.sig} `(selection-head sel)`{.sig} The char offset a `buffer-selections` selection is anchored at, and its head's: the cursor |
| `selection-start-char`<br>`selection-end-char` |  | `(selection-start-char sel)`{.sig} `(selection-end-char sel)`{.sig} The first char offset `sel` covers, and the one past the last |
| `selection-primary?` |  | `(selection-primary? sel)`{.sig} `#t` if `sel` is the primary selection |
| `char-offset->line` |  | `(char-offset->line pane idx)`{.sig} The line containing char offset `idx`; the offset at the end of the text is on the last line. Raises when `idx` is past the end of the text |
| `line->char-offset` |  | `(line->char-offset pane line)`{.sig} The char offset where content line `line` starts; raises when `line` is past the last line |
| `viewport-range` | shown | `(viewport-range pane)`{.sig} `(hash 'start-line first-line 'end-line end-line)` of the lines `pane`'s own pane shows, end-exclusive; with wrapping on, `'end-line` may run a few lines past the bottom edge |
| `open-buffer!` |  | `(open-buffer! path)`{.sig} Open `path`, returning a pane-less pane value for it |
| `close-buffer!` |  | `(close-buffer! pane #:discard-unsaved [#f])`{.sig} Close a buffer. Raises when it has unsaved changes, unless `#:discard-unsaved` is `#t`. Also raises while a `\|` runs in it: stop that first with `(call! "cancel-pipe" pane)`, which returns whether one was running |
| `switch-to-buffer!` | shown | `(switch-to-buffer! pane target)`{.sig} Redirect `pane`'s own pane to `target`'s buffer, and return the pane value now showing it; `pane` still names the buffer the pane left |
| `show-read-only-buffer!` | shown | `(show-read-only-buffer! pane name text)`{.sig} Show `text` in `pane`'s own pane, in a read-only buffer named `[name]` with the cursor on the first line; a second call with the same `name` replaces its text. Returns the pane value now showing the view. Raises for an empty `name` and for `messages`, `plugin-status` and `lsp-status`, which HUME's own commands use |
| `buffer-generation` |  | `(buffer-generation pane)`{.sig} An opaque value naming the buffer's current text. Two compare unequal with `equal?` once the buffer changes. Pass it to `#:expect-generation` or `buffer-changes-since` |
| `buffer-undo-tree` |  | `(buffer-undo-tree pane)`{.sig} The buffer's undo history, as a list with one `(hash 'id 'parent 'age-secs 'current? 'saved?)` per revision, in `'id` order, so a parent comes before its children. `'parent` is `#f` for the root, `'age-secs` is whole seconds since the revision was made, `'current?` marks the revision the buffer is on and `'saved?` the one last saved |
| `selections-linewise?` | shown | `(selections-linewise? pane)`{.sig} `#t` if every selection in `pane`'s own pane covers whole lines. A cursor alone on a blank line doesn't count either way: it neither satisfies this nor breaks it when a real whole-line selection is also present. `#f` if every selection is such a cursor |
| `selections-charwise?` | shown | `(selections-charwise? pane)`{.sig} `#t` if no selection in `pane`'s own pane covers whole lines, with the same blank-line-cursor exception; `#t` if every selection is such a cursor |
| `symbol-under-cursor` | shown | `(symbol-under-cursor pane)`{.sig} The identifier under the primary cursor in `pane`'s own pane, as a string; `""` when the cursor is not on a word character |
| `pane?` |  | `(pane? v)`{.sig} `#t` if `v` is an opaque pane value |

[Reading selections](writing-plugins.md#reading-selections) and [Reading buffer text](writing-plugins.md#reading-buffer-text) show `buffer-text`, `buffer-lines`, `buffer-line-count`, `buffer-char-count`, `buffer-selections`, `char-offset->line`, `line->char-offset`, and `viewport-range` with examples.

## Editing & navigation

| Call | Pane | Effect |
|------|------|--------|
| `apply-text-edits!` |  | `(apply-text-edits! pane edits #:expect-generation)`{.sig} Apply a list of edits to `pane`'s buffer, mapped through `pane`'s own selections. Each edit is a JSON handle onto a wire `TextEdit` (e.g. a `textDocument/formatting` response element, passed straight through). When `pane` names no pane, or a pane that now shows another buffer, the buffer is edited for no pane: every pane's selections are carried through, and the call raises while an insert or paste session is open on the buffer.<br><br>Raises on edits from servers with different position encodings, and on an edit whose range ends on a line past the end of the document, in which case no edit is applied |
| `goto-location!` | shown | `(goto-location! pane loc)`{.sig} Move `pane`'s own pane to `loc`, one of: an LSP `Location`/`LocationLink` JSON handle from a server response; `(hash 'target path 'line l 'char-col c)`, with `path` a file path (never a URI such as `file://…`; write a relative path whose first part holds a colon as `./part:x`) and `l`/`c` char-indexed; or `(hash 'target buffer 'char-offset o)`, with `buffer` a pane value.<br><br>Returns the pane value now showing the buffer it jumped into, or `#f`, moving nothing, when the line `loc` names no longer exists in its file. A `'char-offset` past the end of the buffer raises |
| `goto-revision!` | shown | `(goto-revision! pane id)`{.sig} Move `pane`'s buffer to revision `id` of its undo history (a number from `buffer-undo-tree`), across branches, carrying `pane`'s selections with it. Raises when the buffer has no such revision (never made, or dropped by the `undo-levels` limit) and when the buffer is read-only. Redo then follows the branch it entered |
| `insert-key!` | focused | `(insert-key! pane key)`{.sig} Run `key`'s normal Insert-mode behaviour (indent-style-aware Tab, auto-pairs, auto-indented Enter, …) on `pane`, as if it had no Insert-mode binding; `key` is one chord in `bind-key!`'s syntax. Raises for a sequence of more than one key, and for a key with no default Insert-mode behaviour |
| `paste-text!` | focused | `(paste-text! pane text)`{.sig} Paste `text`, a string or a list of strings, where you're typing, as a terminal paste would. Raises when the buffer is read-only or busy with a running `\|` |
| `edit-text!` |  | `(edit-text! pane edits #:expect-generation)`{.sig} Replace char ranges of `pane`'s buffer as one edit, each entry a `(hash 'start-char s 'end-char e 'text t)`: an empty range inserts, empty text deletes |

`#:expect-generation` guards against a stale edit: given a `buffer-generation` value, the call fails if the buffer has changed since, or if the value belongs to another buffer. With an empty list of edits, `apply-text-edits!`, `edit-text!` and `apply-workspace-edit!` change nothing and check nothing, `#:expect-generation` included.

`apply-text-edits!` and `goto-location!` decode a wire value's positions with the encoding of the server that sent it, as [`apply-workspace-edit!`](lsp-api.md#language-servers) does. A wire-shaped hashmap you build by hand (not pulled from a response via `json-ref`/`json-list`) has no encoding to decode with, and is rejected.

`insert-key!` works only in Insert mode, from inside a command bound to an Insert-mode key. It lets a binding decide, when the key is pressed, whether to override the key's normal behaviour or fall back to it. For example, binding Tab to complete after a letter and insert a tab everywhere else:

```scheme
(define-command! "tab-or-complete" "Complete after a letter, else insert a tab."
  (lambda (pane)
    (if (letter-before-cursor? pane)
        (call! "completion-trigger" pane)
        (insert-key! pane "tab"))))
(bind-key! 'insert "tab" "tab-or-complete")
```

`letter-before-cursor?` is a helper you write yourself, not an editor function.

`paste-text!` does what pasting `text` would do at that moment:

- In Insert mode it goes in at every cursor, with no auto-pairs or auto-indent.
- In Normal mode it replaces the selections.
- On the command line, in a prompt, a search or a picker, line breaks become spaces.

A list of strings, such as `read-register`'s, is placed the way `p` places a register: one string per selection when the counts match, all of them joined otherwise. A prompt always gets them joined.

`paste-text!` ends a pending `on-next-char!` read without a character, as a terminal paste does. When `paste-text!` runs from an Insert-mode key's binding, `.` repeats it by running the binding again; from a hook or a timer during Insert mode, `.` repeats the paste.

For example, a key that inserts the working directory, in Insert mode and on the command line:

```scheme
(define-command! "insert-cwd" "Insert the working directory."
  (lambda (pane) (paste-text! pane (cwd))))
(bind-key! 'insert "ctrl-g" "insert-cwd")
(bind-key! 'command "ctrl-g" "insert-cwd")
```

`edit-text!` makes a change that differs from one selection to the next, which commands and `paste-text!` can't, since they do the same thing at every selection (see [Example: Markdown headings](writing-plugins.md#example-markdown-headings)). Every range refers to the text before the call, the entries can come in any order, and two insertions at the same offset go in list order. One `u` undoes the whole list.

Selections in every pane showing the buffer follow the text. A cursor where text is inserted moves past it; one on replaced or deleted text lands where the new text starts. A range edge inside a character made of several char offsets widens to cover the character, and an insertion inside one moves to its start. Text inserted at the very end of the buffer gets its own line break, and the buffer's last line break is never deleted.

Like `apply-text-edits!`, it edits through `pane` while that pane shows the buffer and for no pane otherwise, so a value with no pane works too.

`edit-text!` raises and changes nothing for an offset past the end of the buffer, a range that ends before it starts, overlapping ranges, a read-only buffer, or a buffer whose generation is not `#:expect-generation`.

## Registers

| Call | Effect |
|------|--------|
| `write-register!` | `(write-register! name values)`{.sig} Store `values` (a list of strings, one per selection) in register `name` |
| `read-register` | `(read-register name)`{.sig} Contents of register `name` as a list of strings, or `#f` if it's empty |
| `register-name?` | `(register-name? name)`{.sig} `#t` when `name` is a register these two accept, `#f` for any other string |

Both use the same list shape. A string a script writes pastes as whole lines when it ends in a newline, inline otherwise. So `(write-register! "3" (read-register "3"))` keeps the text but loses the shape of an entry that was yanked inline and happens to end in a newline.

Valid names are `0`–`9`, `k` (kill-ring head), `c` (system clipboard), and `b` (black hole), the set the [`"` register prefix](copy-and-paste.md#register-prefix) accepts. Writing `k` acts like a yank to the kill ring; writing `b` discards the values, with no message. Reading an unwritten register, `b`, or a register holding a recorded macro answers `#f`.

## Language & syntax

| Call | Effect |
|------|--------|
| `define-language!` | `(define-language! name exts globs shebangs)`{.sig} Define a language, or replace how an existing one is detected. `exts`, `globs`, and `shebangs` default to `'()`. A definition from `init.scm` beats a plugin's in either order; of two from `init.scm`, or two from plugins, the first wins and the later is ignored with a warning |
| `configure-language!` | `(configure-language! name #:roots #:language-id #:comment-tokens #:block-comment-tokens)`{.sig} Set a defined language's project root markers (a list of file names), the identifier language servers know it by when that differs from `name`, its line comment tokens (a list of strings) and its block comment pairs (a list of `(start . end)` string pairs). A keyword left out leaves its setting as it is.<br><br>Values rank as `define-language!`'s definitions do |
| `bundled-language-grammar` | `(bundled-language-grammar name)`{.sig} The grammar HUME ships for language `name`, as `(hash 'url 'rev 'symbol 'subpath)`, or `#f` when it ships none |
| `bundled-grammar-languages` | `(bundled-grammar-languages)`{.sig} Every language HUME ships a grammar for, sorted |
| `register-grammar!` | `(register-grammar! name grammar-path symbol highlights-path #:injections #:textobjects #:indents)`{.sig} Register an already-compiled tree-sitter grammar. From `init.scm` it loads the first time a file or an embedded block needs it; from a command it loads at once and raises if it can't. A grammar from `init.scm` or a command the user runs beats a plugin's in either order; of two from the same place, the later replaces the earlier |
| `language-has-grammar?` | `(language-has-grammar? name)`{.sig} `#t` if `name` has a registered grammar that hasn't failed to load |

[Teach HUME a new language](syntax-highlighting.md#teach-hume-a-new-language) shows `define-language!`, `configure-language!` and `register-grammar!` with examples.

## Diagnostics & decorations

Any plugin can populate these, though LSP diagnostics and inlay hints are the heaviest users. The LSP-only calls are on the [Language Server API](lsp-api.md) page.

| Call | Effect |
|------|--------|
| `diagnostics-for-buffer` | `(diagnostics-for-buffer pane #:severity #:range)`{.sig} The buffer's first 1000 diagnostics by position, at or above the `#:severity` symbol (default: the `lsp-diagnostics-min-severity` option). `#:range` optionally restricts them to a `(hash 'start-char s 'end-char e)` char range, and raises if it ends before it starts or past the end of the buffer.<br><br>Each entry is a hash with `'start-char` (a diagnostic past the last character of the buffer starts on that character), `'end-char`, `'start-line`, `'end-line` (one past the last line the diagnostic touches), `'grapheme-col`, `'severity` (`'error`, `'warning`, `'info`, or `'hint`), `'severity-rank`, `'message`, `'code`, `'source`, `'raw` (a JSON handle onto the wire diagnostic), and `'server` (the server that published it).<br><br>When a call matches more than 1000, `:messages` notes it at trace level, once until a call over the whole buffer at that floor matches 1000 or fewer |
| `diagnostic-counts` | `(diagnostic-counts pane #:severity floor)`{.sig} `(hash 'error n 'warning n 'info n 'hint n)` for the buffer, keyed by the severity names `diagnostics-for-buffer` uses; a severity below `floor` (default: `lsp-diagnostics-min-severity`) counts 0 |
| `set-inlay-hints!` | `(set-inlay-hints! pane source hints)`{.sig} Replace `source`'s inlay hints for the buffer. `hints`: list of `(hash 'char-offset o 'text t 'side 'before)`, `'side` `'before` or `'after`. An offset inside a character puts a `'before` hint before the whole character and an `'after` hint after it |
| `register-sign-source!` | `(register-sign-source! pane source priority)`{.sig} Reserve a gutter sign slot for `source` on the buffer, ranked by `(priority desc, source asc)` among every source registered for it |
| `set-signs!` | `(set-signs! pane source signs)`{.sig} Replace `source`'s gutter signs for the buffer. `signs`: list of `(hash 'line l 'text t 'scope s)`; `source` must already be registered |
| `set-virtual-lines!` | `(set-virtual-lines! pane source lines)`{.sig} Replace `source`'s virtual (ghost) lines for the buffer. `lines`: list of hashmaps with `'line`/`'text` required, optional `'side` (`'before`/`'after`), `'scope`, `'segments` (a list of `(hash 'start-char-col 'end-char-col 'scope)` char-column ranges in `'text`; an edge inside a character widens the segment to the whole character, and two segments sharing a character raise) |
| `set-eol-text!` | `(set-eol-text! pane source lines #:hide-on-insert-line #f)`{.sig} Replace `source`'s end-of-line text for the buffer. `lines`: list of `(hash 'line l 'text t 'scope s)`. With `#:hide-on-insert-line #t`, the text hides on the cursor's line while you type in Insert mode |
| `set-extra-highlights!` | `(set-extra-highlights! pane source spans)`{.sig} Replace `source`'s extra syntax highlights for the buffer. `spans`: list of `(hash 'start-char s 'end-char e 'scope sc)` char ranges; an edge inside a character widens the span to the whole character, and the call raises for an empty or reversed span or one past the end of the buffer |
| `set-line-backgrounds!` | `(set-line-backgrounds! pane source entries)`{.sig} Replace `source`'s full-line background tints for the buffer. `entries`: list of `(hash 'line l 'scope s)` |

[Hooks](writing-plugins.md#hooks) shows `diagnostics-for-buffer` and the hook that feeds it.

A sign source's gutter slot is reserved the first time it registers for a buffer, before it places any sign, so the gutter's width stays stable as signs come and go. The slot stays reserved across `:e!` until the buffer closes. There's no `unregister-sign-source!`, and re-registering the same `source` for the same buffer only replaces its priority.

Line backgrounds have no priority: same-line entries from different sources break ties by source name.

Signs, virtual lines, end-of-line text and line backgrounds stay on their line while you edit it, and move with it when lines are added or removed above. They go away when their line is deleted, or joined onto the line above or below. On a join, only the decorations of the line whose end survives stay, on the joined line.

## Completion

A plugin registers a completion *source*; the editor drives it. A request goes to:

- every source registered for the buffer, on `Ctrl-Space` in Insert mode (the built-in `completion-trigger` command);
- the source a trigger character was registered for, when you type that character;
- the source a `:` command declared, on the first `Tab` on its command line;
- the source for the shell the `shell` option runs, on the first `Tab` on the `|` prompt.

All answers are ranked together in one menu, each against its own source's token; no source renders its own UI.

[Completion sources](writing-plugins.md#completion-sources) has the full walkthrough (a static source, richer items, trigger characters, an asynchronous `:` command argument source) and every option `register-completion-source!` takes. `#:complete` on `define-typed-command!` also accepts a built-in source such as `"path"`. The per-server variants of the two trigger calls are on the [Language Server API](lsp-api.md#language-servers) page, and [Hooks](writing-plugins.md#hooks) has the `on-trigger-char` and `on-completion-accept` lambda signatures.

| Call | Effect |
|------|--------|
| `register-completion-source!` | `(register-completion-source! name proc #:target #:match #:priority #:resolve #:token-chars #:cancel)`{.sig} Register `proc` as completion source `name` (see [Source options](writing-plugins.md#source-options)) |
| `completion-emit!` | `(completion-emit! id items #:incomplete #:more #:span #:prefix)`{.sig} Answer request `id` with `items`, or with part of the answer when `#:more` is `#t`. Returns `#f` if the answer was dropped: it was out of date, or the request was already answered without `#:more`. An `id` of `#f` is out of date the same way |
| `register-shell-dialect!` | `(register-shell-dialect! name #:programs #:program-extensions #:parse #:write)`{.sig} How `Tab` on the `\|` prompt reads and writes a word for the shells `#:programs` names (see [Shell dialects](writing-plugins.md#shell-dialects)) |
| `set-completion-triggers!` | `(set-completion-triggers! language source chars)`{.sig} Characters, a list of chars like `'(#\.)`, that invoke `'buffer` source `source` when typed in a `language` buffer. Replaces the previous set; `'()` removes it. An unknown `source` is reported in the message log |
| `set-hook-triggers!` | `(set-hook-triggers! language source chars)`{.sig} Characters that fire the `on-trigger-char` hook, tagged `source`, when typed in a `language` buffer, for features other than completion such as signature help. Replaces the previous set; `'()` removes it |
| `completion-top` | `(completion-top n)`{.sig} The top `n` ranked items of the open menu, each carrying its `source` |
| `completion-accept!` | `(completion-accept! idx)`{.sig} Accept item `idx` in `completion-top`'s ranked order, firing the `on-completion-accept` hook. Raises when no Insert-mode completion menu is open |
| `completion-dismiss!` | `(completion-dismiss!)`{.sig} Close the open menu; a no-op if none is open |

## Pickers

Any plugin can drive a picker with these editor functions, opening one and pushing items into it instead of building its own fuzzy-finder. [Custom pickers](writing-plugins.md#custom-pickers) and [Live requery (live grep)](writing-plugins.md#live-requery-live-grep) have full walkthroughs (batch vs. streaming population, truncation direction, exit-code handling, live requery).

| Call | Pane | Effect |
|------|------|--------|
| `picker!` | focused | `(picker! pane items on-select #:prompt #:pending #:query #:truncate #:actions)`{.sig} Open a fuzzy-finder panel over a fixed `items` list of `(display . payload)` pairs. Returns a token, or `#f` when a `#:repeatable` command's `.` answers the picker without opening it |
| `live-picker!` | focused | `(live-picker! pane on-select #:source #:prompt #:query #:debounce-ms #:truncate #:actions)`{.sig} Open a picker whose query runs `#:source` (a function of the picker token and the query) on every keystroke, debounced. Returns a token, or `#f` under `.` as `picker!` does |
| `picker-command-source` |  | `(picker-command-source argv-for #:cwd #:nul #:ok-exit-codes)`{.sig} A `#:source` for `live-picker!` that runs the subprocess `argv-for` names for the query |
| `picker-push!` |  | `(picker-push! token items)`{.sig} Append a batch of `(display . payload)` items to an open picker. A `live-picker!` source passes the token it was called with; once the user has typed on, that token no longer feeds the picker and the call returns `#f` |
| `picker-replace!` |  | `(picker-replace! token items)`{.sig} Replace an open picker's items wholesale |
| `picker-source-spawn!` |  | `(picker-source-spawn! token cmd args #:cwd #:nul #:ok-exit-codes)`{.sig} Stream a subprocess's stdout lines into an open picker as items |
| `picker-source-stop!` |  | `(picker-source-stop! token)`{.sig} Kill a picker's still-running spawned source |
| `picker-close!` |  | `(picker-close! token)`{.sig} Close the picker `token` names; a no-op if that picker has already closed or been replaced |

## Other UI widgets

| Call | Pane | Effect |
|------|------|--------|
| `prompt!` | focused | `(prompt! pane label on-confirm #:prefill)`{.sig} Open a minibuffer text prompt; `on-confirm` fires once, later, with the confirmed text, or `#f` on cancel. Raises when another `prompt!` prompt is already open. When a `#:repeatable` command opened it, `.` repeats the answer without opening it; see [Dot-repeat](writing-plugins.md#dot-repeat) |
| `show-popup!` | focused | `(show-popup! pane text #:anchor #:kind #:lang)`{.sig} Show a text popup, replacing any popup already showing. `#:anchor` is `'cursor` (default, floats near the cursor) or `'bottom` (docks above the statusline); `#:lang` sets syntax highlighting.<br><br>`#:kind` sets how long it lives: `'sticky` (default) closes as soon as you leave the mode you opened it in; `'scrollable` stays open (Ctrl-u/Ctrl-d scroll it) until any other key, paste, or mouse input.<br><br>Returns a token for `close-popup!`, or `#f` if the popup didn't open |
| `close-popup!` |  | `(close-popup! token)`{.sig} Close the popup `token` names; a no-op if it has already closed or been replaced |
| `show-menu!` | focused | `(show-menu! pane items on-select)`{.sig} Show a selection menu over `items`, a non-empty list of `(display . payload)` pairs as `picker!` takes (raises on an empty one). `on-select` gets the chosen row's payload, or `#f` when the menu is dismissed.<br><br>Returns a token for `close-menu!`, or `#f` if the menu didn't open: the editor is not in Normal or Extend mode, or moved on before the call landed.<br><br>When a `#:repeatable` command opened it, `.` repeats the choice without opening it; see [Dot-repeat](writing-plugins.md#dot-repeat) |
| `close-menu!` |  | `(close-menu! token)`{.sig} Close the menu `token` names without calling its `on-select`; a no-op if it has already closed or been replaced |
| `show-drawer-list!` | focused | `(show-drawer-list! pane items on-select #:selected [0] #:render [#f])`{.sig} Show a list in the bottom drawer over `items`, a non-empty list of `(display . payload)` pairs as a picker takes. Raises on empty `items`: close the drawer (or never open it) instead. `#:selected` is the row to open on (clamped into `items`), scrolled into view. `on-select` is called with `(payload token)`: the selected row's payload, and the token of the drawer that called it.<br><br>With `#:render`, `items` can be any values but `#f`, one per row, each its row's payload. Just before showing rows it hasn't drawn yet, the drawer calls `(render start keys)`: `keys` are the items of those rows, starting at row `start`, and `render` returns one string for each. Rows are drawn once per list, so `render` runs again only for rows scrolled into view or after `update-drawer-list!`.<br><br>`render` can read buffers, selections and settings but not change the editor; calls that need language servers, diagnostics, completion, timers or background processes raise. An attempt to change the editor, an error, or a result that isn't one string per key closes the drawer, calls `on-select` with `#f`, and shows the error.<br><br>Replaces any drawer already open. The outgoing drawer's `on-select` fires with `#f` and its own token, so its owner knows that drawer is gone and can tell this from its new drawer closing.<br><br>Returns a token scoping `close-drawer!`/`update-drawer-list!`/`drawer-selected-index` to this drawer (hold onto it), or `#f` if the drawer didn't open because the editor moved on before the call landed. `.` never records a drawer pick; see [Dot-repeat](writing-plugins.md#dot-repeat) |
| `close-drawer!` |  | `(close-drawer! token)`{.sig} Close the drawer `token` names; a no-op if it has already closed or been replaced |
| `update-drawer-list!` |  | `(update-drawer-list! token items on-select selected #:render [#f])`{.sig} Replace the open drawer's rows in place (`#:render` as for `show-drawer-list!`), selecting row `selected` (clamped into `items`; pass `(drawer-selected-index token)` to keep the current row). Returns `#t` when applied, `#f` when no drawer is open or `token` isn't its token. Raises on empty `items`: close the drawer instead of clearing it |
| `drawer-selected-index` |  | `(drawer-selected-index token)`{.sig} The open drawer's selected row, or `#f` when no drawer is open or `token` isn't its token |

## Timers

| Call | Effect |
|------|--------|
| `after!` | `(after! ms thunk)`{.sig} Call `thunk` with no args once `ms` milliseconds pass. Returns a timer for `cancel-timer!`, an opaque value: compare two with `equal?` |
| `cancel-timer!` | `(cancel-timer! timer)`{.sig} Cancel a timer `after!` made: its thunk never runs, even when already due. Idempotent: a no-op once the thunk has run or after an earlier cancel. Raises for anything but a timer, a job included |
| `debounce` | `(debounce ms proc)`{.sig} Wrap `proc` so each call reschedules it `ms` out, cancelling any call still pending from an earlier invocation |
| `debounce-by` | `(debounce-by ms proc #:key)`{.sig} Same, but keyed per `(key . args)`: a call with one key never cancels a call with another. `#:key` defaults to the first argument itself. To key by buffer when calls may carry different pane values for the same buffer, pass `#:key (lambda (pane . _) (buffer-key pane))` |

## Async & subprocesses

| Call | Effect |
|------|--------|
| `spawn-async!` | `(spawn-async! cmd args callback #:cwd dir)`{.sig} Run `cmd` in the background, in `dir` (default: HUME's working directory; a relative `dir` is joined onto it). `callback`, `(lambda (result) ...)`, fires once, later, with the hash `run-capture!` returns. Returns a job for `cancel-async!`, an opaque value: compare two with `equal?`.<br><br>A failed spawn doesn't raise: `callback` gets empty `'stdout`, the error in `'stderr`, and `'exit` `#f`, which a signal-killed command also gets. A command that writes more than 64 MiB to stdout gets the same answer as a failed spawn. Past 8 KiB, `'stderr` keeps the first and last 4 KiB a command writes |
| `cancel-async!` | `(cancel-async! job)`{.sig} Kill a still-running `spawn-async!` job, or stop a `path-programs` listing, and drop its callback, even one already due; idempotent. Raises for anything but a job, a timer included |
| `run-inline-output!` | `(run-inline-output! cmd args #:cwd #:env)`{.sig} Run `cmd`, streaming output to the terminal inside an `#:inline-output` command; `#:env` is a list of `("KEY" . "VALUE")` pairs added to the environment. Raises outside an `#:inline-output` command and on nonzero exit |
| `run-capture!` | `(run-capture! cmd args #:cwd dir)`{.sig} Run `cmd` in `dir` (defaulting as for `spawn-async!`), blocking until it exits. Returns `(hash 'stdout s 'stderr s 'exit code)`; `'exit` is `#f` when the spawn failed (`'stderr` says why) or a signal killed the process. Output is capped as for `spawn-async!` |

[Filesystem and processes](writing-plugins.md#filesystem-and-processes) has examples.

## Diffing

| Call | Effect |
|------|--------|
| `diff-lines` | `(diff-lines old-text new-text)`{.sig} `(hash 'hunks …)`: the line-level hunks where `old-text` and `new-text` differ.<br><br>Each hunk has `'old-start-line`/`'old-end-line` and `'new-start-line`/`'new-end-line` (end exclusive), `'old-lines`/`'new-lines` (the text of those lines), and `'words`, `(hash 'old spans 'new spans)`. An old-side span is `(hash 'line 'start-char-col 'end-char-col)`: a line of the old text and character columns in it. A new-side span is `(hash 'start-char 'end-char)`: character offsets into the new text, counting each line ending as one character.<br><br>Spans mark the words that changed inside the hunk; a span covering a whole line is left out. `'words` is `#f` for a hunk too large to compare word by word |
| `diff-buffer-lines` | `(diff-buffer-lines pane ref-text)`{.sig} Same, but against the buffer's current unsaved content, without pulling the whole buffer through `buffer-text` first |
| `buffer-revision-diff` | `(buffer-revision-diff pane id)`{.sig} Hunks between revision `id` of the buffer's undo history (a number from `buffer-undo-tree`) and the buffer's current content, in `diff-buffer-lines`'s shape with the revision as the old side. Raises when the buffer has no such revision |
| `buffer-changes-since` | `(buffer-changes-since pane generation)`{.sig} The hunks from the buffer's text when `generation` (a `buffer-generation` value of the same buffer) was read, as the old side, to its current text, as a list of hunks shaped like those `diff-lines` returns; `#f` when HUME can't say what changed. Raises for another buffer's value |
| `diff-words` | `(diff-words old-text new-text)`{.sig} `(hash 'hunks …)`: word-level hunks within a single changed line |

[Comparing text](writing-plugins.md#comparing-text) has examples, including hunk shapes.

## Text & strings

Everyday string work (trimming, splitting on a separator, case conversion, prefix/suffix tests, search and replace) uses Steel's own string functions; see Steel's [string reference](https://mattwparas.github.io/steel/book/builtins/steel_strings.html). `string-length`, `substring`, and `string-ref` count Unicode characters (codepoints), not bytes or on-screen columns.

| Call | Effect |
|------|--------|
| `split-words` | `(split-words line word-chars)`{.sig} Every word in `line`, in order, split as the `w`/`b` motions and text objects split words, so a word here is what one of those would select. `word-chars` extends what counts as a word, like the buffer option of the same name (`""` for none) |
| `string-common-prefix` | `(string-common-prefix strings)`{.sig} The longest start every string in `strings` shares, never ending partway through a character the editor shows as one (an accented letter written as two code points, an emoji sequence); `""` for an empty list |

## Filesystem & directories

| Call | Effect |
|------|--------|
| `data-dir` | `(data-dir)`{.sig} HUME's data directory, or `#f` if unavailable |
| `runtime-dir` | `(runtime-dir)`{.sig} HUME's runtime directory, or `#f` if unavailable |
| `cwd` | `(cwd)`{.sig} HUME's working directory (what `:pwd` prints) |
| `set-cwd!` | `(set-cwd! path)`{.sig} Change HUME's working directory, the one `:cd` moves. `path` is taken as written (see below), a relative one joined onto the current directory. Raises if `path` is not a directory. Works from `init.scm` too |
| `replace-dir!` | `(replace-dir! src dest)`{.sig} Move directory `src` to `dest` (both relative to the working directory), replacing the directory at `dest` and creating `dest`'s parent if needed. Either `src` ends up at `dest`, or the call raises and leaves `dest` as it was, even if the script budget runs out during it |
| `try-lock-file!` | `(try-lock-file! path)`{.sig} Take an exclusive lock on file `path` (relative to the working directory), creating the file if needed: `#t` when taken, `#f` when another HUME or program holds it. The lock lasts until `unlock-file!`, until the command, hook or callback that took it returns or is stopped, or until HUME exits, so no lock is left behind. Taking a lock the same command already holds raises |
| `unlock-file!` | `(unlock-file! path)`{.sig} Release a lock `try-lock-file!` took; raises when this command doesn't hold it |
| `path-join` | `(path-join seg ...)`{.sig} Join path segments with the OS-native separator |
| `safe-path-segment?` | `(safe-path-segment? name)`{.sig} `#t` iff `name` is a string safe to use as one path component: not empty, not `.` or `..`, and free of path separators, `:`, `"` and NUL; `#f` for anything else. Use it before joining a user-typed or downloaded name onto a path |
| `remove-dir!` | `(remove-dir! dir)`{.sig} Delete directory `dir` (relative to the working directory) and everything in it; raises when `dir` doesn't exist |
| `find-program` | `(find-program name)`{.sig} The path of the program `name` runs as, looked up on the `PATH` HUME started with (the one every program it starts gets); `#f` when there is none |
| `first-path-outside` | `(first-path-outside dir paths)`{.sig} The first of `paths` that lies outside directory `dir` once every symbolic link in both is resolved, returned as you wrote it; `#f` when every path is inside `dir` or is `dir` itself. All are relative to the working directory. A link to a missing file counts as the path it names. Raises when a path can't be resolved, such as a link that points back to itself |
| `path-entries` | `(path-entries dir prefix)`{.sig} The names in directory `dir` (relative to the working directory) starting with `prefix` (ignoring letter case on Windows), sorted, as `:e` completes them: a directory's name ends in `/` (a link to a directory included), and hidden entries appear only when `prefix` starts with `.`. `'()` when `dir` can't be read |
| `path-programs` | `(path-programs prefix extensions callback)`{.sig} List, in the background, the programs on `PATH` whose names start with `prefix`. `callback`, `(lambda (names done?) ...)`, is called as `PATH` is read with every name found so far, sorted, and a last time with `done?` `#t`. A `callback` that raises stops the listing. Returns a job for `cancel-async!`. On Windows a program is a file ending in one of `extensions` (`'(".exe")`), named without it |
| `home-dir` | `(home-dir)`{.sig} The user's home directory, or `#f` when the environment names none |
| `path->display` | `(path->display path [#:relative dir])`{.sig} `path` as `buffer-display-path` shows a file: absolute with the home directory as `~`, or with `#:relative dir`, relative to `dir` when inside it. A relative `path` or `dir` is taken against the working directory. Compares the paths as written, without following symbolic links |
| `read-data-file` | `(read-data-file path)`{.sig} The one Scheme value written in the file at `path`, with `;` comments allowed around it. Raises, naming the file, when its text doesn't parse or holds more than one value. Use it instead of Steel's `read` for data files: `read` returns end-of-file instead of raising on bad syntax, and leaves the bad text for the next `read` |
| `data-file` | `(data-file dir name)`{.sig} The path of `name.scm` in `dir` when `dir` holds that file, else `#f`. Only a name spelled like one of `dir`'s files, letter case included, matches: `Rust` does not match `rust.scm` on any platform, and `../x` never reaches outside `dir` |
| `data-file-names` | `(data-file-names dir)`{.sig} The names of `dir`'s `.scm` files without the suffix, sorted; `'()` when `dir` doesn't exist |

[Filesystem and processes](writing-plugins.md#filesystem-and-processes) shows how to read a plugin's own files.

HUME has one working directory: `(cwd)` reads it, and `(set-cwd! path)` and `:cd` move it. A path given to a HUME call is taken as written:

- A relative path joins onto that directory.
- `~` and variables like `$HOME` are not expanded.
- `..` resolves the way the operating system resolves it: after a symlink it leaves the link's target, and after a directory that doesn't exist it raises. Other links stay as typed.

Steel's own file functions resolve a relative path against the same directory. Steel's `change-current-directory!` moves only the directory those functions use, not HUME's; `set-cwd!` moves both.

## Grammar compilation

| Call | Effect |
|------|--------|
| `compile-grammar!` | `(compile-grammar! src out)`{.sig} Compile the tree-sitter grammar source at `src` to `out`. Raises on failure with the compiler's error output, except while `init.scm` loads, where it logs a warning instead. The compiler writes nothing to the screen |
