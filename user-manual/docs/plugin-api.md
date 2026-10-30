# Plugin API

Reference for the editor functions a plugin or `init.scm` can call. The editor commands keys run are on [Editor Commands](editor-commands.md); [Commands and functions](#commands-and-functions) explains the difference. This surface has three layers:

- **Editor functions**: always available, called by name as plain Scheme: `(buffer-text pane)`, `(bind-key! ...)`.
- **[Language Server API](lsp-api.md)**: the editor functions for registering, configuring, and sending requests to language servers.
- **[Standard Library](standard-library.md)**: `core:stdlib`, an optional core plugin. Reach its commands through `call!`, like any plugin's: `(call! "stdlib/find" pred? lst)`.

For walkthroughs and worked examples, see [Writing Plugins](writing-plugins.md), [Language Servers](lsp.md), [Configuration](configuration.md), and the pages linked throughout.

## Commands and functions

A script can run three kinds of things:

| Kind | Examples | Run from | Bound to a key | `call!` | Listed in |
|------|----------|----------|----------------|---------|-----------|
| **Editor command** | `move-left`, `completion-trigger`, one a plugin defines with `define-command!` | A key, or a script | Yes | Yes | [Editor Commands](editor-commands.md) for the built-in ones, a plugin's own docs for its commands |
| **Typed command** | `:write`, one defined with `define-typed-command!` | The `:` prompt | No | No | [Command mode](command-mode.md), a plugin's own docs |
| **Editor function** | `buffer-text`, `edit-text!`, `bind-key!` | A script | No | No | This page and the [Language Server API](lsp-api.md) |

An editor command takes what a key press supplies: the pane, then an optional count and extend flag. Call a built-in one by name: `(move-left pane)` is the same as `(call! "move-left" pane)`. A few built-in commands also act with a character or pattern that the key press asks for, and a script passes it as an argument: `(call! "search-forward" pane "foo")` searches as if you had typed `foo` at the `/` prompt. [Calling other commands](writing-plugins.md#calling-other-commands) lists them. To search without moving anything, use [`buffer-search`](#buffers-panes-selections).

An editor function takes the arguments its entry lists. Something is an editor function rather than a command when it needs arguments a key press can't supply, or only makes sense as a step inside your own command.

Steel's library functions and your own `define`s are plain Scheme functions, not editor functions.

## Conventions

- A function whose call changes something (editor state, a registration, a process, a file) ends in `!`. Reads, and functions that only build a value like `debounce`, don't.
- A call on a pane takes the pane first; one on a whole language takes the language first, then what it names there: `(set-signs! pane source signs)`, `(set-hook-triggers! language source chars)`.
- Lines, columns, and char offsets are 0-based everywhere. Add 1 only to show a line number to the user.
- `'start-char`/`'end-char` are char offsets into a whole text: the buffer, or a text you passed in, such as the new side of a `diff-lines` word span. `'start-char-col`/`'end-char-col` are char columns within one line, such as a virtual line's `'segments`.
- A char offset runs from 0 to the length of the text. The length is where a range ends, and a jump to it lands on the last character; something attached to a character, like an inlay hint, needs an offset below it.
- One character can span several char offsets: a letter with its combining marks, an emoji sequence. An offset inside one moves to the whole character: a jump or a `'before` inlay hint lands on its start, an `'after` inlay hint after it, and a range widens to cover it.
- A position your plugin computes for a buffer raises when out of range. A position from outside, such as a language server's answer or a file on disk, may be out of date: a column past the end of its line moves to the line's end, and a line past the end of the text gives `#f` (`lsp-position->char-offset`, `lsp-range->char-range`, `goto-location!`) or makes the whole call raise (`apply-text-edits!`, `apply-workspace-edit!`).
- In a signature, `[#:kw value]` is an optional keyword argument, and its entry gives the default; a keyword shown without brackets is required. Optional values are keywords, never a positional `#f` placeholder. The exception is `define-language!`, whose extensions, globs, and shebangs are positional lists you can drop from the end.
- A structured value you pass in or get back (a decoration entry, a diagnostic, a request error, a plugin's `#:config`) is a hash with symbol keys: `(hash 'line 0 'text "!" 'scope "error")`, read with `(hash-ref d 'message)`. The exceptions:
  - Values decoded from a language server's JSON are JSON handles (see [JSON handles](lsp-api.md#json-handles)).
  - The `lsp-*-params` results (`(hash-ref (lsp-position-params pane) "position")`) and the items `completion-top` returns (source: `(hash-ref item "source")`) have string keys, as in the server's JSON.
  - Picker items `(display . payload)` and `register-lsp-server!`'s `#:env` `("KEY" . "VALUE")` are pairs, not records. A picker's keymap is a hash with key-spec string keys.
  - A selection from `buffer-selections` is not a record either: read it with the `selection-*` calls.
- Every UI opener (`show-popup!`, `show-menu!`, `show-drawer-list!`, `picker!`, `live-picker!`) returns a token, which every call that closes or changes that widget takes. A stale token (the widget already closed or was replaced) makes the call a no-op, or makes it return `#f` where its entry says so, so a late callback can never touch someone else's widget. An opener returns `#f` when the open was dropped before it could happen, and `#f` is a stale token too.
- Where a call may run depends on what it touches:
  - Calls that read or change a live buffer, pane, selection, or widget raise at the top level of `init.scm` and in a plugin body loaded at startup, since nothing is on screen yet. So do calls that start background work that calls you back later (`after!`, `spawn-async!`, `path-programs`) and the calls that stop it (`cancel-timer!`, `cancel-async!`). Call them from a command, hook, timer, or callback.
  - Registrations (`bind-key!` and its variants without `#:keymap`, `define-command!`, `define-typed-command!`, `register-hook!`, `register-lsp-notification-hook!`, `register-completion-source!`, `define-language!`, `configure-language!`) raise inside a command, hook, timer, or callback. Call them at the top level of `init.scm` or in a plugin body.
  - `load-plugin!` works only at the top level of `init.scm`. `declare-plugin!` works there and in a plugin's own `manifest.scm`.
- A procedure you hand over to be called later (a hook handler, a callback, an `on-select`, a `#:render`) must take the arguments its call's entry lists, and the call that takes it raises at once when it can't. A lambda with a fitting rest parameter is accepted, as is an editor function, except as an `on-before-buffer-save` handler, which must be a lambda of one or two parameters and no rest parameter. A command's procedure is checked when the command runs, not by `define-command!`.
- Settings, key bindings, language definitions and grammars rank by whose behalf they are made: the user's beat a plugin's, and a plugin's beat HUME's own. Code acts for the user in `init.scm`, at `:set`, and in any command run from a key or the `:` prompt, including one a plugin defined, along with the hooks, timers and callbacks that code sets up. A plugin's body acts for the plugin, as do the hooks, timers and callbacks the body sets up. The entries below say which of two values from the same rank wins.
- A value from a fixed set of names (a mode, a hook name, a log level, an enum option's value) is a symbol, like `'insert`. Compare symbols with `equal?`: Steel's `eq?` checks identity, so a symbol the editor hands you is never `eq?` to one you wrote.

## Settings & statusline

| Call | Effect |
|------|--------|
| `set-option!` | `(set-option! key value)`{.sig} Set a global option, in the type `get-option` returns for it. Of two values from the same rank, the later wins. Raises for the per-buffer `language` and `line-ending` |
| `set-buffer-option!` | `(set-buffer-option! pane key value)`{.sig} Set an option on `pane`'s buffer only, ranked against the buffer's own overrides as `set-option!` is against the global value |
| `get-option` | `(get-option key)`{.sig} Read an option's global value, ignoring any buffer override. Raises for the per-buffer `language` and `line-ending` |
| `get-buffer-option` | `(get-buffer-option pane key)`{.sig} Read `pane`'s buffer's effective value: its own override if set, else the global default (`language` and `line-ending` have no global default) |
| `configure-statusline!` | `(configure-statusline! left center right)`{.sig} Set the three statusline sections, each a list of element name strings; `(get-option "statusline")` reads them back as `(hash 'left 'center 'right)` |
| `set-statusline-text!` | `(set-statusline-text! pane source text)`{.sig} Push `text` for a `"steel:<source>"` statusline element, scoped to `pane`'s buffer; an empty string clears it |

See [Reading options from Scheme](writing-plugins.md#reading-options-from-scheme) for the two option readers' fallback rules, [Statusline](settings.md#statusline) for the built-in element names, and [Custom elements](settings.md#custom-elements) for the two statusline calls together.

## Key bindings

| Call | Effect |
|------|--------|
| `bind-key!` | `(bind-key! mode key cmd [#:keymap name])`{.sig} Bind `key` to editor command `cmd` in `'normal`, `'insert`, `'extend`, or `'command` mode; an `'insert` or `'command` binding is a single key. With `#:keymap`, the binding goes in keymap `name`, created by its first binding. Of two bindings from the same rank, the later wins |
| `bind-key-extend!` | `(bind-key-extend! mode key cmd [#:keymap name])`{.sig} Same, but the binding always extends the selection; `'normal` only |
| `unbind-key!` | `(unbind-key! mode key [#:keymap name])`{.sig} Unbind `key` for the calling code and everything ranked below it: a plugin's unbind removes HUME's binding but leaves the user's, and the user's keeps any plugin from binding it again |
| `bind-keys!`<br>`bind-keys-extend!`<br>`unbind-keys!` | `(bind-keys! mode [#:keymap name] (key cmd) ...)`{.sig} `(bind-keys-extend! mode [#:keymap name] (key cmd) ...)`{.sig} `(unbind-keys! mode [#:keymap name] key ...)`{.sig} Batched forms of the three above |
| `attach-keymap!` | `(attach-keymap! pane name)`{.sig} Turn on keymap `name` in `pane`'s buffer, ahead of the keymaps attached before it (an already attached one keeps its place). A name with nothing bound into it logs a warning and does nothing |
| `detach-keymap!` | `(detach-keymap! pane name)`{.sig} Turn off keymap `name` in `pane`'s buffer; a name with nothing bound into it logs a warning |
| `set-register-prefix!` | `(set-register-prefix! name)`{.sig} Target a specific register (`0`–`9`, `k`, `c`, `b`) for the rest of the current command body's `call!`s |

See [Key bindings](configuration.md#key-bindings) for the key-string grammar and full examples, [Keys for one buffer](configuration.md#keys-for-one-buffer) for keymaps, and [Register prefix](writing-plugins.md#register-prefix) for `set-register-prefix!`.

## Commands

| Call | Effect |
|------|--------|
| `define-command!` | `(define-command! name doc proc [#:repeatable bool] [#:inline-output bool])`{.sig} Register `name` as an editor command. Both keywords default to `#f` |
| `define-typed-command!` | `(define-typed-command! name doc proc [#:inline-output bool] [#:complete source])`{.sig} Register `name` as a typed command, reachable as `:name`. `#:complete` names a completion source for its argument |
| `call!` | `(call! name args ...)`{.sig} Run an editor command (built-in or Scheme-defined), activating its plugin on demand. A built-in command takes `pane`, an optional count and extend flag, then the character or pattern for the few that need one; see [Calling other commands](writing-plugins.md#calling-other-commands). Raises for a command that runs only from a key, such as `pipe` |
| `on-next-char!` | `(on-next-char! callback)`{.sig} Call `callback` with the next key typed as a character, or with `#f` when that key isn't one |
| `command-plugin` | `(command-plugin name)`{.sig} The id string of the plugin that registered command `name`: `"user"` for a top-level `init.scm` definition, `"hume"` for a built-in, one HUME's runtime files define, or a name that isn't a command |
| `command-exists?` | `(command-exists? name)`{.sig} `#t` if your code can use `name`: a command `call!` reaches (built-in, Scheme-defined, or a not-yet-loaded plugin's, but not a key-only command such as `pipe`), or a procedure bound at the top level (an editor function, a Steel library function, or a `define` at the top level of `init.scm`). `#f` otherwise, including for a plugin's own `define`s and for macros such as `call!` |

[Defining commands](writing-plugins.md#defining-commands), [Calling other commands](writing-plugins.md#calling-other-commands), and [Reading characters](writing-plugins.md#reading-characters) show `define-command!`, `define-typed-command!`, `call!`, and `on-next-char!` with examples.

## Supporting several HUME versions

| Call | Effect |
|------|--------|
| `hume-version` | `(hume-version)`{.sig} The running editor's version as `(hash 'major 'minor 'patch 'commit)`. `'commit` is the short git hash on a development build and `#f` on a release |
| `hume-version>=?` | `(hume-version>=? major minor patch)`{.sig} `#t` if the running editor is at least that version |

A development build reports the release it is heading toward: `(hume-version>=? 0 15 0)` is `#t` on a nightly of the 0.15.0 cycle. Use it to gate on a release.

To use something added after the latest release, check for it directly instead of comparing versions:

```scheme
(if (command-exists? "set-inlay-hints!")
    (set-inlay-hints! pane source hints)
    (fallback))
```

Don't compare `'commit` values: hashes have no order.

## Plugin lifecycle

| Call | Effect |
|------|--------|
| `declare-plugin!` | `(declare-plugin! name [#:entry file] [#:commands names] [#:typed-commands names] [#:events events] [#:languages languages])`{.sig} Declare the triggers that load a plugin lazily: in its own `manifest.scm`, or in `init.scm` for a local `./file.scm` or to give an installed plugin custom triggers. `#:entry` names another file of the plugin to load on its own triggers. Raise cases are listed below the table |
| `load-plugin!` | `(load-plugin! name [#:config hash] [#:ref ref])`{.sig} Bring in an installed or core plugin: lazily when it ships a `manifest.scm`, at startup otherwise. `#:config`, a hash with symbol keys, is the only way to pass configuration; `#:ref` pins an installed plugin to a tag, branch or commit (see [Pinning a version](plugins.md#pinning-a-version)). Raise cases are listed below the table |
| `resolve-plugin-path` | `(resolve-plugin-path name)`{.sig} The path of an installed plugin's `plugin.scm` if it exists on disk, else `#f`; raises for a malformed name and for a local `./file.scm` name |
| `loaded-plugins` | `(loaded-plugins)`{.sig} A list of the names of the plugins whose `plugin.scm` has finished loading, local files included; a plugin with only another `#:entry` file loaded is not listed |
| `declared-plugins` | `(declared-plugins)`{.sig} A list of the names of the plugins `load-plugin!` or `declare-plugin!` named, `core:*` included, installed or not; local `./file.scm` entries are not listed |
| `declared-plugin-ref` | `(declared-plugin-ref name)`{.sig} The `#:ref` the last `load-plugin!` of `name` passed, or `#f` |
| `plugin-config` | `(plugin-config)`{.sig} The `#:config` hash the user passed to `load-plugin!`, or an empty hash. Works in the plugin's body and in every command, hook and callback it defines; raises anywhere else |
| `plugin-dir` | `(plugin-dir)`{.sig} The directory holding the calling plugin's own files, or `#f` when HUME has no directory for that kind of plugin (a user plugin with no home directory set). Works where `(plugin-config)` does; raises anywhere else |

`declare-plugin!` raises and records nothing for:

- a declaration with no trigger at all (no commands, typed commands, events or languages);
- `#:entry` on a local `./file.scm`, or `#:config` (configuration comes only from `load-plugin!`);
- an entry declared twice, or a declaration after the plugin's `load-plugin!`;
- a command name already taken, or listed twice;
- an installed plugin that has no file for the entry;
- in a `manifest.scm`, a declaration of any plugin but the manifest's own.

`load-plugin!` raises for a local `./file.scm` name (declare it with `declare-plugin!` instead), for a `#:config` that is not a hash with symbol keys, for `#:ref` on a `core:` plugin, for a call from a `manifest.scm`, and for an installed plugin directory holding neither `manifest.scm` nor `plugin.scm`.

For the full picture (load timing, `#:config` semantics, dependency checks), see [Plugins](plugins.md), especially [How plugins are loaded](plugins.md#how-plugins-are-loaded), and [Depending on another plugin](writing-plugins.md#depending-on-another-plugin).

## Hooks

| Call | Effect |
|------|--------|
| `register-hook!` | `(register-hook! name proc)`{.sig} Register `proc` for lifecycle event `name`; raises for a lambda that can't take the event's arguments |

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

Report from a plugin with `log!`, and write output the user watches with `print!`. Steel's own print functions (`display`, `displayln`, `newline`, …) write straight to the terminal and draw over the editor's screen.

## Buffers, panes & selections

Almost every call on this page takes a **pane** value: an opaque handle naming a buffer and, where relevant, the pane it's associated with. You get one as a command's or hook's leading argument, from `(focused-pane)`, or from `(buffers)`/`(panes)`/`(buffer-panes pane)` below. Compare two pane values with `equal?`.

A pane value that carries no pane (most hook arguments, and every entry `(buffers)` returns) still works for anything that only needs the buffer (`buffer-text`, `buffer-path`, the decoration setters, and so on).

The **Pane** column of the tables on this page and the [Language Server API](lsp-api.md) page marks the calls that need the pane itself:

- *shown*: raises if the value carries no pane, if that pane has closed, or if it has moved to another buffer.
- *focused*: also raises when that pane isn't the focused one.

A malformed argument is reported before either refusal. A call with no mark works with any pane value, including one that carries no pane.

A command body receives the pane it was invoked through as its leading parameter (see [Defining commands](writing-plugins.md#defining-commands)). A debounced or async continuation can fire after focus has moved, so capture the pane it acts on up front: check `(equal? pane (focused-pane))` before acting on a response when `pane` was captured before the request went out. To pick a specific pane of a buffer shown in several, list them with `(buffer-panes pane)`.

A value naming a closed buffer raises for almost every call:

- every call marked in the Pane column;
- every call that writes to a buffer;
- the reads of its bookkeeping: `buffer-path`, `buffer-text`, `buffer-generation`, `buffer-undo-tree`, `get-buffer-option`, the decoration setters, `apply-text-edits!`, `lsp-request!`, and the like.

Only `buffer-key`, `buffer-live?`, `pane-live?`, `diagnostic-counts`, and `diagnostics-for-buffer` answer for one. The last two answer with zero counts and an empty list, as for a buffer with no diagnostics; check `buffer-live?` first to tell the two apart.

| Call | Pane | Effect |
|------|------|--------|
| `focused-pane` |  | `(focused-pane)`{.sig} The pane focused right now, paired with its buffer: a live read, for code with no pane of its own, such as a response callback or a timer |
| `buffers` |  | `(buffers)`{.sig} A list of every open buffer, as pane-less pane values, in the order they were opened |
| `buffer-live?` |  | `(buffer-live? pane)`{.sig} `#t` if `pane`'s buffer is still open, `#f` otherwise; never raises. A timer, debounce, or async callback whose captured value may have closed should check this before calling anything that raises on a closed buffer |
| `pane-live?` |  | `(pane-live? pane)`{.sig} `#t` if `pane` names a pane that still exists and still shows its own buffer, `#f` otherwise (including for a value that carries no pane); never raises. Use it like `buffer-live?`, in a callback that calls a pane-aware editor function such as `lsp-request!` |
| `panes` |  | `(panes)`{.sig} A list of every open pane, including panes in other tabs |
| `buffer-panes` |  | `(buffer-panes pane)`{.sig} Every pane showing `pane`'s buffer: the focused pane first if it shows that buffer, then the rest of the active tab, then other tabs. `(car (buffer-panes pane))` is the default pick when you need *some* pane on the buffer |
| `buffer-key` |  | `(buffer-key pane)`{.sig} An opaque, comparable value naming only `pane`'s buffer: pane values for the same buffer give equal keys, whatever pane they carry, if any. Use it, not the pane value, to key a hash table of per-buffer plugin state |
| `buffer-path` |  | `(buffer-path pane)`{.sig} The buffer's absolute path, or `#f` for an unsaved buffer |
| `buffer-display-path` |  | `(buffer-display-path pane [#:relative dir])`{.sig} A path for display (absolute, home directory as `~`), or `#f` for an unsaved buffer. With `#:relative dir`, relative to `dir` when the file is inside it, such as `#:relative (cwd)` for the working directory. For printing only, never for file access |
| `buffer-name` |  | `(buffer-name pane)`{.sig} The display name: a view buffer's label (such as `[messages]`), else the filename, else `"*scratch*"` |
| `buffer-dirty?` |  | `(buffer-dirty? pane)`{.sig} `#t` if the buffer has unsaved edits |
| `buffer-text` |  | `(buffer-text pane)`{.sig} The buffer's live content as a string, unsaved edits included. It always ends with a line break, and its line endings are always `\n`, whatever the file uses |
| `buffer-lines` |  | `(buffer-lines pane [#:range range])`{.sig} The content as a list of lines, each with its ending stripped. `range` is `(hash 'start-line s 'end-line e)`, end-exclusive, the shape `viewport-range` returns; a range missing either end, ending past the last line, or starting after it ends raises instead of being clamped |
| `buffer-line-count` |  | `(buffer-line-count pane)`{.sig} The number of lines `buffer-lines` returns, without building the list |
| `buffer-char-count` |  | `(buffer-char-count pane)`{.sig} The length in characters, the final line break included: the largest offset `char-offset->line` accepts |
| `buffer-search` |  | `(buffer-search pane pattern)`{.sig} `(hash 'start-char s 'end-char e)` for the first match of `pattern` in the buffer, or `#f` when nothing matches. `pattern` is what you'd type at the `/` prompt: a regex, matching either case when it is all lowercase, or a literal after `v/`. Raises for an empty or invalid pattern. Moves no selection and leaves the current search as it was, unlike `(call! "search-forward" pane pattern)` |
| `word-search-pattern` |  | `(word-search-pattern pane word)`{.sig} The pattern `*` would search for on `word` in this buffer: `word` taken literally, between word boundaries, ready for `buffer-search` or the `/` prompt. Raises for an empty `word` |
| `buffer-cursor-line` | shown | `(buffer-cursor-line pane)`{.sig} The line of the primary cursor in `pane`'s own pane; never the empty line after the final line break |
| `buffer-selections` | shown | `(buffer-selections pane)`{.sig} A list of opaque selections in `pane`'s own pane, one per selection, sorted by start; read each with the `selection-*` calls below. Anchor and head each sit at the start of a character, and start..end (exclusive) is what the selection covers |
| `selection-anchor`<br>`selection-head` |  | `(selection-anchor sel)`{.sig} `(selection-head sel)`{.sig} The char offset a `buffer-selections` selection is anchored at, and its head's: the cursor |
| `selection-start-char`<br>`selection-end-char` |  | `(selection-start-char sel)`{.sig} `(selection-end-char sel)`{.sig} The first char offset `sel` covers, and the one past the last |
| `selection-primary?` |  | `(selection-primary? sel)`{.sig} `#t` if `sel` is the primary selection |
| `char-offset->line` |  | `(char-offset->line pane idx)`{.sig} The line containing char offset `idx`; the offset at the end of the text is on the last line. Raises when `idx` is past the end of the text |
| `line->char-offset` |  | `(line->char-offset pane line)`{.sig} The char offset where content line `line` starts; raises when `line` is at or past the line count |
| `viewport-range` | shown | `(viewport-range pane)`{.sig} `(hash 'start-line first-line 'end-line end-line)` of the lines `pane`'s own pane shows, end-exclusive; with wrapping on, `'end-line` may run past the bottom edge |
| `open-buffer!` |  | `(open-buffer! path)`{.sig} Open `path`, returning a pane-less pane value for it |
| `close-buffer!` |  | `(close-buffer! pane [#:discard-unsaved bool])`{.sig} Close a buffer. Raises when it has unsaved changes, unless `#:discard-unsaved` is `#t` (default `#f`). Also raises while a `\|` runs in it: stop that first with `(call! "cancel-pipe" pane)`, which returns whether one was running |
| `switch-to-buffer!` | shown | `(switch-to-buffer! pane target)`{.sig} Redirect `pane`'s own pane to `target`'s buffer, and return the pane value now showing it; `pane` still names the buffer the pane left |
| `show-read-only-buffer!` | shown | `(show-read-only-buffer! pane name text)`{.sig} Show `text` in `pane`'s own pane, in a read-only buffer named `[name]` with the cursor on the first line; a second call with the same `name` replaces its text. Returns the pane value now showing the view. Raises for an empty `name` and for `messages`, `plugin-status` and `lsp-status`, which HUME's own commands use |
| `buffer-generation` |  | `(buffer-generation pane)`{.sig} An opaque value naming the buffer's current text. Two compare unequal with `equal?` once the buffer changes. Pass it to `#:expect-generation` or `buffer-changes-since` |
| `buffer-undo-tree` |  | `(buffer-undo-tree pane)`{.sig} The buffer's undo history, as a list with one `(hash 'id 'parent 'age-secs 'current? 'saved?)` per revision, in `'id` order, so a parent comes before its children. `'parent` is `#f` for the root, `'age-secs` is whole seconds since the revision was made, `'current?` marks the revision the buffer is on and `'saved?` the one last saved |
| `selections-linewise?` | shown | `(selections-linewise? pane)`{.sig} `#t` if every selection in `pane`'s own pane covers whole lines. A cursor on an empty line doesn't count either way: it neither satisfies this nor breaks it when a real whole-line selection is also present. `#f` if every selection is such a cursor |
| `selections-charwise?` | shown | `(selections-charwise? pane)`{.sig} `#t` if no selection in `pane`'s own pane covers whole lines, with the same empty-line exception; `#t` if every selection is a cursor on an empty line |
| `symbol-under-cursor` | shown | `(symbol-under-cursor pane)`{.sig} The identifier under the primary cursor in `pane`'s own pane, as a string; `""` when the cursor is not on a word character |
| `pane?` |  | `(pane? v)`{.sig} `#t` if `v` is an opaque pane value |

[Reading selections](writing-plugins.md#reading-selections) and [Reading buffer text](writing-plugins.md#reading-buffer-text) show these calls with examples.

## Editing & navigation

| Call | Pane | Effect |
|------|------|--------|
| `apply-text-edits!` |  | `(apply-text-edits! pane edits [#:expect-generation gen])`{.sig} Apply a list of LSP `TextEdit`s, as JSON handles from a server's response (such as a `textDocument/formatting` result), to `pane`'s buffer as one edit. See below |
| `edit-text!` |  | `(edit-text! pane edits [#:expect-generation gen])`{.sig} Replace char ranges of `pane`'s buffer as one edit, each entry a `(hash 'start-char s 'end-char e 'text t)`: an empty range inserts, empty text deletes. See below |
| `goto-location!` | shown | `(goto-location! pane loc)`{.sig} Move `pane`'s own pane to `loc`, one of: an LSP `Location`/`LocationLink` JSON handle from a server's response; `(hash 'target path 'line l 'char-col c)`, with `path` a file path (never a URI such as `file://…`; write a relative path whose first part holds a colon as `./part:x`) and `l`/`c` char-indexed; or `(hash 'target buffer 'char-offset o)`, with `buffer` a pane value.<br><br>Returns the pane value now showing the buffer it jumped into, or `#f`, moving nothing, when the line `loc` names is past the end of its file. A `'char-offset` past the end of the buffer raises |
| `goto-revision!` | shown | `(goto-revision! pane id)`{.sig} Move `pane`'s buffer to revision `id` of its undo history (a number from `buffer-undo-tree`), across branches, carrying `pane`'s selections with it. Raises when the buffer has no such revision (never made, or dropped by the `undo-levels` limit) and when the buffer is read-only. Redo then follows the branch it entered |
| `insert-key!` | focused | `(insert-key! pane key)`{.sig} Run `key`'s normal Insert-mode behaviour (indent-style-aware Tab, auto-pairs, auto-indented Enter, …) on `pane`, as if it had no Insert-mode binding; `key` is one chord in `bind-key!`'s syntax. Raises for a sequence of more than one key, and for a key with no default Insert-mode behaviour |
| `paste-text!` | focused | `(paste-text! pane text)`{.sig} Paste `text`, a string or a list of strings, where you're typing, as a terminal paste would. See below |

### Text edits

`edit-text!` makes a change that differs from one selection to the next, which commands and `paste-text!` can't, since they do the same thing at every selection (see [Example: Markdown headings](writing-plugins.md#example-markdown-headings)). Every range refers to the text before the call, the entries can come in any order, and two insertions at the same offset go in list order. One `u` undoes the whole list.

`apply-text-edits!` does the same with edits a language server sent. It decodes their positions with the encoding of the server that sent them, as [`apply-workspace-edit!`](lsp-api.md#language-servers) does, so a hash you build by hand (not pulled from a response through `json-ref`/`json-list`) has no encoding and is rejected.

Selections in every pane showing the buffer follow the text. A cursor where text is inserted moves past it; one on replaced or deleted text lands where the new text starts. A range edge inside a character made of several char offsets widens to cover the character, and an insertion inside one moves to its start. Text inserted at the very end of the buffer gets its own line break, and the buffer's last line break is never deleted.

Both edit through `pane` while that pane shows the buffer. When `pane` names no pane, or a pane that now shows another buffer, the buffer is edited for no pane: every pane's selections are carried through, and the call raises while you are typing in Insert mode or a paste is in progress in that buffer.

Both raise and change nothing for:

- an edit past the end of the buffer (for `apply-text-edits!`, one whose range ends on a line past the end), a range that ends before it starts, or overlapping ranges;
- edits from servers with different position encodings (`apply-text-edits!`);
- a read-only buffer, or one where a `|` is running;
- a buffer whose generation is not `#:expect-generation`.

`#:expect-generation` guards against a stale edit: given a `buffer-generation` value, the call fails if the buffer has changed since, and raises if the value belongs to another buffer or is not a generation. With an empty list of edits, `apply-text-edits!`, `edit-text!` and `apply-workspace-edit!` change nothing and skip the changed-since check, but still raise for a value that is not this buffer's generation.

### Insert keys

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

### Pasting

`paste-text!` does what pasting `text` would do at that moment:

- In Insert mode it goes in at every cursor, with no auto-pairs or auto-indent.
- In Normal and Extend mode it replaces each selection; at a cursor (a one-character selection) it goes in before the cursor.
- On the `:` prompt, in a prompt, a search or a picker, line breaks become spaces.

A list of strings, such as `read-register`'s, is placed the way `p` places a register: one string per selection when the counts match, all of them joined otherwise. A prompt always gets them joined.

Pasting into a buffer raises when the buffer is read-only or busy with a running `|`.

`paste-text!` ends a pending `on-next-char!` read without a character, as a terminal paste does. When `paste-text!` runs from an Insert-mode key's binding, `.` repeats it by running the binding again; from a hook or a timer during Insert mode, `.` repeats the paste.

For example, a key that inserts the working directory, in Insert mode and on the `:` prompt:

```scheme
(define-command! "insert-cwd" "Insert the working directory."
  (lambda (pane) (paste-text! pane (cwd))))
(bind-key! 'insert "ctrl-g" "insert-cwd")
(bind-key! 'command "ctrl-g" "insert-cwd")
```

## Registers

| Call | Effect |
|------|--------|
| `write-register!` | `(write-register! name values)`{.sig} Store `values` (a list of strings, one per selection) in register `name` |
| `read-register` | `(read-register name)`{.sig} The contents of register `name` as a list of strings, or `#f` if it's empty |
| `register-name?` | `(register-name? name)`{.sig} `#t` when `name` is a register these two accept, `#f` for any other string |

Both use the same list shape. A string a script writes pastes as whole lines when it ends in a newline, inline otherwise. So `(write-register! "3" (read-register "3"))` keeps the text but loses the shape of an entry that was yanked inline and happens to end in a newline.

Valid names are `0`–`9`, `k` (kill-ring head), `c` (system clipboard), and `b` (black hole), the set the [`"` register prefix](copy-and-paste.md#register-prefix) accepts. Writing `k` acts like a yank to the kill ring; writing `b` discards the values, with no message. Reading an unwritten register, `b`, or a register holding a recorded macro answers `#f`.

## Language & syntax

| Call | Effect |
|------|--------|
| `define-language!` | `(define-language! name [exts] [globs] [shebangs])`{.sig} Define a language, or replace how an existing one is detected. `exts`, `globs`, and `shebangs` are lists and default to `'()`. Of two definitions from the same rank, the first wins and the later is ignored with a warning |
| `configure-language!` | `(configure-language! name [#:roots files] [#:language-id id] [#:comment-tokens tokens] [#:block-comment-tokens pairs])`{.sig} Set a defined language's project root markers (a list of file names), the identifier language servers know it by when that differs from `name`, its line comment tokens (a list of strings) and its block comment pairs (a list of `(start . end)` string pairs). A keyword left out leaves its setting as it is. Values rank as `define-language!`'s definitions do. A `name` no language defines logs an error and changes nothing |
| `bundled-languages` | `(bundled-languages)`{.sig} Every language HUME ships language data for, sorted |
| `register-grammar!` | `(register-grammar! name grammar-path symbol highlights-path [#:injections path] [#:textobjects path] [#:indents path])`{.sig} Register an already-compiled tree-sitter grammar. During startup (`init.scm` and the plugins it loads at startup) the grammar loads the first time a file or an embedded block needs it; at any other time it loads at once and raises if it can't. Of two grammars from the same rank, the later replaces the earlier |
| `language-has-grammar?` | `(language-has-grammar? name)`{.sig} `#t` if `name` has a registered grammar that hasn't failed to load |

[Teach HUME a new language](syntax-highlighting.md#teach-hume-a-new-language) shows `define-language!`, `configure-language!` and `register-grammar!` with examples.

## Diagnostics & decorations

Any plugin can populate these, though LSP diagnostics and inlay hints are the heaviest users. The LSP-only calls are on the [Language Server API](lsp-api.md) page.

| Call | Effect |
|------|--------|
| `diagnostics-for-buffer` | `(diagnostics-for-buffer pane [#:severity floor] [#:range range])`{.sig} The buffer's first 1000 diagnostics by position, at or above `floor` (default: the `lsp-diagnostics-min-severity` option). `range` restricts them to a `(hash 'start-char s 'end-char e)` char range, and raises if it ends before it starts or past the end of the buffer. Entry keys are listed below the table |
| `diagnostic-counts` | `(diagnostic-counts pane [#:severity floor])`{.sig} `(hash 'error n 'warning n 'info n 'hint n)` for the buffer; a severity below `floor` (default: `lsp-diagnostics-min-severity`) counts 0 |
| `set-inlay-hints!` | `(set-inlay-hints! pane source hints)`{.sig} Replace `source`'s inlay hints for the buffer. `hints`: a list of `(hash 'char-offset o 'text t 'side 'before)`, `'side` `'before` or `'after`. An offset inside a character puts a `'before` hint before the whole character and an `'after` hint after it |
| `register-sign-source!` | `(register-sign-source! pane source priority)`{.sig} Reserve a gutter sign slot for `source` on the buffer, ordered among every source registered for it by priority, higher first, ties broken by source name |
| `set-signs!` | `(set-signs! pane source signs)`{.sig} Replace `source`'s gutter signs for the buffer. `signs`: a list of `(hash 'line l 'text t 'scope s)`; `source` must already be registered, and a control character in `'text` raises |
| `set-virtual-lines!` | `(set-virtual-lines! pane source lines)`{.sig} Replace `source`'s virtual (ghost) lines for the buffer. `lines`: a list of hashes with `'line` and `'text` required, and optional `'side` (`'before` or `'after`, default `'after`), `'scope` and `'segments`. A line break in `'text` raises. `'segments` is a list of `(hash 'start-char-col 'end-char-col 'scope)` char-column ranges in `'text`; an edge inside a character widens the segment to the whole character, and two segments sharing a character raise |
| `set-eol-text!` | `(set-eol-text! pane source lines [#:hide-on-insert-line bool])`{.sig} Replace `source`'s end-of-line text for the buffer. `lines`: a list of `(hash 'line l 'text t 'scope s)`. With `#:hide-on-insert-line #t` (default `#f`), the text hides on the cursor's line while you type in Insert mode |
| `set-extra-highlights!` | `(set-extra-highlights! pane source spans)`{.sig} Replace `source`'s extra syntax highlights for the buffer. `spans`: a list of `(hash 'start-char s 'end-char e 'scope sc)` char ranges; an edge inside a character widens the span to the whole character, and the call raises for an empty or reversed span or one past the end of the buffer |
| `set-line-backgrounds!` | `(set-line-backgrounds! pane source entries)`{.sig} Replace `source`'s full-line background tints for the buffer. `entries`: a list of `(hash 'line l 'scope s)` |

Each `diagnostics-for-buffer` entry is a hash with:

- `'start-char` and `'end-char`: a diagnostic past the last character of the buffer starts on that character;
- `'start-line`, and `'end-line`, one past the last line the diagnostic touches;
- `'grapheme-col`: the 0-based column, counted in characters as you see them;
- `'severity` (`'error`, `'warning`, `'info`, or `'hint`) and `'severity-rank`;
- `'message`, `'code` and `'source`;
- `'raw`, a JSON handle onto the diagnostic as the server sent it, and `'server`, the server that published it.

[Hooks](writing-plugins.md#hooks) shows `diagnostics-for-buffer` and the hook that feeds it.

A sign source's gutter slot is reserved the first time it registers for a buffer, before it places any sign, so the gutter's width stays stable as signs come and go. The slot stays reserved across `:e!` until the buffer closes. There's no `unregister-sign-source!`, and re-registering the same `source` for the same buffer only replaces its priority.

Line backgrounds have no priority: same-line entries from different sources break ties by source name.

Signs, virtual lines, end-of-line text and line backgrounds stay on their line while you edit it, and move with it when lines are added or removed above. They go away when their line is deleted, or joined onto the line above or below. On a join, only the decorations of the line whose end survives stay, on the joined line.

## Completion

A plugin registers a completion *source*; the editor drives it. A request goes to:

- every source registered for the buffer, on `Ctrl-Space` in Insert mode (the `completion-trigger` editor command);
- the source a trigger character was registered for, when you type that character;
- the source a typed command declared, on the first `Tab` on its argument;
- the source for the shell the `shell` option runs, on the first `Tab` on the `|` prompt.

All answers are ranked together in one menu, each against its own source's token; no source renders its own UI.

[Completion sources](writing-plugins.md#completion-sources) has the full walkthrough (a static source, richer items, trigger characters, an asynchronous typed-command argument source) and every option `register-completion-source!` takes. `#:complete` on `define-typed-command!` also accepts a built-in source such as `"path"`. The per-server variants of the two trigger calls are on the [Language Server API](lsp-api.md#language-servers) page, and [Hooks](writing-plugins.md#hooks) has the `on-trigger-char` and `on-completion-accept` lambda signatures.

| Call | Effect |
|------|--------|
| `register-completion-source!` | `(register-completion-source! name proc #:target target [#:match kind] [#:priority n] [#:resolve proc] [#:token-chars chars] [#:cancel proc])`{.sig} Register `proc` as completion source `name` (see [Source options](writing-plugins.md#source-options)) |
| `completion-emit!` | `(completion-emit! id items [#:incomplete bool] [#:more bool] [#:span span] [#:prefix bool])`{.sig} Answer request `id` with `items`, or with part of the answer when `#:more` is `#t`. Returns `#f` if the answer was dropped: it was out of date, or the request was already answered without `#:more`. An `id` of `#f` is out of date the same way |
| `register-shell-dialect!` | `(register-shell-dialect! name #:programs names [#:program-extensions exts] #:parse proc #:write proc)`{.sig} How `Tab` on the `\|` prompt reads and writes a word for the shells `#:programs` names (see [Shell dialects](writing-plugins.md#shell-dialects)). `#:program-extensions` defaults to `'(".exe")` |
| `set-completion-triggers!` | `(set-completion-triggers! language source chars)`{.sig} Characters, a list of chars like `'(#\.)`, that invoke `'buffer` source `source` when typed in a `language` buffer. Replaces the previous set; `'()` removes it. An unknown `source` is reported in the message log |
| `set-hook-triggers!` | `(set-hook-triggers! language source chars)`{.sig} Characters that fire the `on-trigger-char` hook, tagged `source`, when typed in a `language` buffer, for features other than completion such as signature help. Replaces the previous set; `'()` removes it |
| `completion-top` | `(completion-top n)`{.sig} The top `n` ranked items of the open menu, each carrying its `source` |
| `completion-accept!` | `(completion-accept! idx)`{.sig} Accept item `idx` in `completion-top`'s ranked order, firing the `on-completion-accept` hook. Raises when no Insert-mode completion menu is open |
| `completion-dismiss!` | `(completion-dismiss!)`{.sig} Close the open menu; a no-op if none is open |

## Pickers

Any plugin can drive a picker with these editor functions, opening one and pushing items into it instead of building its own fuzzy finder. [Custom pickers](writing-plugins.md#custom-pickers) and [Live requery (live grep)](writing-plugins.md#live-requery-live-grep) have full walkthroughs.

| Call | Pane | Effect |
|------|------|--------|
| `picker!` | focused | `(picker! pane items keymap [#:prompt text] [#:pending bool] [#:query text] [#:truncate side] [#:on-close thunk])`{.sig} Open a fuzzy-finder panel over a fixed `items` list of `(display . payload)` pairs. `keymap` maps each key the picker answers to a `(lambda (token item) ...)`; an unbound plain character types into the query and an unbound `Backspace` deletes from it. `#:pending #t` marks the panel as "results still arriving" until the first `picker-push!` or `picker-replace!`. `#:truncate` is `'head` (default) or `'tail`, the end of an over-long row that is cut. `#:on-close` is called once when the picker closes. Returns a token, or `#f` when a `#:repeatable` command's `.` answers the picker without opening it |
| `live-picker!` | focused | `(live-picker! pane keymap #:source proc [#:prompt text] [#:query text] [#:debounce-ms ms] [#:truncate side] [#:on-close thunk])`{.sig} Open a picker whose query runs `proc`, a function of the picker token and the query, on every keystroke, after `ms` milliseconds without typing (default 150). `keymap`, `#:truncate` and `#:on-close` as for `picker!`. Returns a token, or `#f` under `.` as `picker!` does |
| `picker-command-source` |  | `(picker-command-source argv-for [#:cwd dir] [#:nul bool] [#:ok-exit-codes codes])`{.sig} A `#:source` for `live-picker!` that runs the subprocess `argv-for` names for the query |
| `picker-push!` |  | `(picker-push! token items)`{.sig} Append a batch of `(display . payload)` items to an open picker. Returns `#t`, or `#f` for a stale token. A `live-picker!` source passes the token it was called with; once the user has typed on, that token stops feeding the picker |
| `picker-replace!` |  | `(picker-replace! token items)`{.sig} Replace an open picker's items. Returns `#t`, or `#f` for a stale token |
| `picker-source-spawn!` |  | `(picker-source-spawn! token cmd args [#:cwd dir] [#:nul bool] [#:ok-exit-codes codes])`{.sig} Stream a subprocess's stdout lines into an open picker as items. Returns `#t`, or `#f` for a stale token, starting nothing; raises when the program can't be started |
| `picker-source-stop!` |  | `(picker-source-stop! token)`{.sig} Kill a picker's still-running spawned source. Returns `#f` for a stale token |
| `picker-move!` |  | `(picker-move! token count [#:by step])`{.sig} Move the selection of the picker `token` names by `count` rows, pages or half pages (`step`: `'row`, the default, `'page` or `'half-page`); a negative `count` moves up |
| `picker-close!` |  | `(picker-close! token)`{.sig} Close the picker `token` names, calling its `#:on-close` |

## Other UI widgets

| Call | Pane | Effect |
|------|------|--------|
| `prompt!` | focused | `(prompt! pane label callback [#:prefill text])`{.sig} Open a text prompt; `callback` fires once, later, with the confirmed text, or `#f` on cancel. Raises when another `prompt!` prompt is already open. When a `#:repeatable` command opened it, `.` repeats the answer without opening it; see [Dot-repeat](writing-plugins.md#dot-repeat) |
| `show-popup!` | focused | `(show-popup! pane text [#:anchor anchor] [#:kind kind] [#:lang language])`{.sig} Show a text popup, replacing any popup already showing. `#:anchor` is `'cursor` (default, floats near the cursor) or `'bottom` (docks above the statusline); `#:lang` sets syntax highlighting.<br><br>`#:kind` sets how long it lives: `'sticky` (default) closes as soon as you leave the mode you opened it in; `'scrollable` stays open (`Ctrl-u`/`Ctrl-d` scroll it) until any other key, paste, or mouse input.<br><br>Returns a token for `close-popup!`, or `#f` if the popup didn't open |
| `close-popup!` |  | `(close-popup! token)`{.sig} Close the popup `token` names |
| `show-menu!` | focused | `(show-menu! pane items callback)`{.sig} Show a selection menu over `items`, a non-empty list of `(display . payload)` pairs as `picker!` takes (raises on an empty one). `callback` gets the chosen row's payload, or `#f` when the menu is dismissed.<br><br>Returns a token for `close-menu!`, or `#f` if the menu didn't open: the editor is not in Normal or Extend mode, or moved on before the call landed.<br><br>When a `#:repeatable` command opened it, `.` repeats the choice without opening it; see [Dot-repeat](writing-plugins.md#dot-repeat) |
| `close-menu!` |  | `(close-menu! token)`{.sig} Close the menu `token` names without calling its `callback` |
| `show-drawer-list!` | focused | `(show-drawer-list! pane items callback [#:selected row] [#:render proc])`{.sig} Show a list in the bottom drawer, replacing any drawer already open. Returns a token, or `#f`. See [The drawer](#the-drawer) |
| `close-drawer!` |  | `(close-drawer! token)`{.sig} Close the drawer `token` names |
| `update-drawer-list!` |  | `(update-drawer-list! token items callback selected [#:render proc])`{.sig} Replace the open drawer's rows in place (`#:render` as for `show-drawer-list!`), selecting row `selected` (clamped into `items`; pass `(drawer-selected-index token)` to keep the current row). Returns `#t` when applied, `#f` for a stale token. Raises on empty `items`: close the drawer instead of clearing it |
| `drawer-selected-index` |  | `(drawer-selected-index token)`{.sig} The open drawer's selected row, or `#f` for a stale token |

### The drawer

`show-drawer-list!` lists `items`, a non-empty list of `(display . payload)` pairs as a picker takes, in the bottom drawer. It raises on empty `items`: close the drawer, or don't open it, instead. `#:selected` is the row to open on (default 0, clamped into `items`), scrolled into view. `callback` is called with `(payload token)`: the selected row's payload, and the token of the drawer that called it.

With `#:render`, `items` can be any values but `#f`, one per row, each its row's payload. Before showing rows it hasn't drawn yet, the drawer calls `(render start keys)`: `keys` are the items of those rows, starting at row `start`, and `render` returns one string for each. Rows are drawn once per list, so `render` runs again only for rows scrolled into view or after `update-drawer-list!`.

`render` can read buffers, selections and settings but not change the editor; calls that need language servers, diagnostics, completion, timers or background processes raise. An attempt to change the editor, an error, or a result that isn't one string per key closes the drawer, calls `callback` with `#f`, and shows the error.

A new drawer replaces the one already open. The outgoing drawer's `callback` fires with `#f` and its own token, so its owner knows that drawer is gone and can tell this from its new drawer closing.

`show-drawer-list!` returns the token `close-drawer!`, `update-drawer-list!` and `drawer-selected-index` take. It returns `#f`, with a warning, when the editor is not in Normal or Extend mode, and `#f` when the editor moved on before the call landed. `.` never records a drawer pick; see [Dot-repeat](writing-plugins.md#dot-repeat).

## Timers

| Call | Effect |
|------|--------|
| `after!` | `(after! ms thunk)`{.sig} Call `thunk` with no args once `ms` milliseconds pass. Returns a timer for `cancel-timer!`, an opaque value: compare two with `equal?` |
| `cancel-timer!` | `(cancel-timer! timer)`{.sig} Cancel a timer `after!` made: its thunk never runs, even when already due. A no-op once the thunk has run or after an earlier cancel. Raises for anything but a timer, a job included |
| `debounce` | `(debounce ms proc)`{.sig} Wrap `proc` so each call reschedules it `ms` out, cancelling any call still pending from an earlier invocation |
| `debounce-by` | `(debounce-by ms proc [#:key proc])`{.sig} Same, but keyed per `(key . args)`: a call with one key never cancels a call with another. `#:key` defaults to taking the first argument itself. To key by buffer when calls may carry different pane values for the same buffer, pass `#:key (lambda (pane . _) (buffer-key pane))` |

## Async & subprocesses

| Call | Effect |
|------|--------|
| `spawn-async!` | `(spawn-async! cmd args callback [#:cwd dir])`{.sig} Run `cmd` in the background; `callback`, `(lambda (result) ...)`, fires once, later, with the hash `run-capture!` returns. Returns a job for `cancel-async!`, an opaque value: compare two with `equal?`. A program that fails to start doesn't raise: `callback` gets the failure |
| `cancel-async!` | `(cancel-async! job)`{.sig} Kill a still-running `spawn-async!` job, or stop a `path-programs` listing, and drop its callback, even one already due; a no-op for a finished or cancelled job. Raises for anything but a job, a timer included |
| `run-inline-output!` | `(run-inline-output! cmd args [#:cwd dir] [#:env pairs])`{.sig} Run `cmd`, streaming its output to the terminal inside an `#:inline-output` command; `#:env` is a list of `("KEY" . "VALUE")` pairs added to the environment. Raises outside an `#:inline-output` command and on nonzero exit |
| `run-capture!` | `(run-capture! cmd args [#:cwd dir])`{.sig} Run `cmd`, blocking the editor until it exits. Returns the result hash below |

The result hash of `run-capture!` and `spawn-async!` is `(hash 'stdout s 'stderr s 'exit code)`:

- `'stdout`: the program's complete output, as a string.
- `'stderr`: its error output; past 8 KiB, the first and last 4 KiB of it.
- `'exit`: its exit code, or `#f` when it couldn't start (`'stderr` says why) or was killed by a signal. Compare it with `equal?`, not `=`.

A program that writes more than 64 MiB to stdout gets the answer of one that couldn't start.

`spawn-async!`, `run-inline-output!`, `run-capture!` and `picker-source-spawn!` start the program in HUME's working directory, or in `dir` (a relative `dir` is joined onto the working directory), and look it up on the `PATH` HUME started with. They deny git a credential prompt, so a private repository fails instead of waiting for a password.

A script that runs past `steel-init-budget-ms`/`steel-command-budget-ms` (see [Global options](settings.md#global-options)) stops where it is, and none of its error handlers run. HUME's own waits don't count: running a program and waiting for it, deleting or replacing a directory tree (`remove-dir!`, `replace-dir!`), resolving paths with `first-path-outside`, and opening a grammar. `time/sleep-ms` does count, as does waiting in Steel's own calls, such as its `wait`.

[Filesystem and processes](writing-plugins.md#filesystem-and-processes) has examples.

## Diffing

| Call | Effect |
|------|--------|
| `diff-lines` | `(diff-lines old-text new-text)`{.sig} `(hash 'hunks hunks)`: the line-level hunks where `old-text` and `new-text` differ. See [Line hunks](#line-hunks) |
| `diff-buffer-lines` | `(diff-buffer-lines pane ref-text)`{.sig} Same, with `ref-text` as the old side and the buffer's current unsaved content as the new side, faster than passing `(buffer-text pane)` to `diff-lines` |
| `buffer-revision-diff` | `(buffer-revision-diff pane id)`{.sig} `(hash 'hunks hunks)` between revision `id` of the buffer's undo history (a number from `buffer-undo-tree`), as the old side, and the buffer's current content. Raises when the buffer has no such revision |
| `buffer-changes-since` | `(buffer-changes-since pane generation)`{.sig} The hunks from the buffer's text when `generation` (a `buffer-generation` value of the same buffer) was read, as the old side, to its current text, as a bare list of hunks; `#f` when HUME can't say what changed. Raises for another buffer's value |
| `diff-words` | `(diff-words old-text new-text)`{.sig} `(hash 'hunks hunks)`: word-level hunks within a single changed line. See [Word hunks](#word-hunks) |

### Line hunks

Both texts are split into lines the way HUME reads a file: every line ending becomes `\n`, and a missing final line break doesn't count as a change. Unchanged lines are left out. Each hunk is:

```scheme
(hash 'old-start-line s 'old-end-line e 'new-start-line s 'new-end-line e 'old-lines texts 'new-lines texts 'words spans)
```

- `'old-start-line`/`'new-start-line` are 0-based line numbers, and `'old-end-line`/`'new-end-line` the line after the hunk's last one on each side, so a side's lines are `[start, end)`. A pure insertion has an empty old range (`'old-start-line` equals `'old-end-line`); a pure deletion has an empty new range. The empty side's line number is where the change happens, so it feeds straight into `set-signs!` or `set-virtual-lines!`.
- `'old-lines`/`'new-lines` are the text of those lines, without line breaks.
- `'words`, `(hash 'old spans 'new spans)`, marks the words that changed inside the hunk. An old-side span is `(hash 'line 'start-char-col 'end-char-col)`: a line of the old text and char columns in it. A new-side span is `(hash 'start-char 'end-char)`: char offsets into the new text, counting each line ending as one character, ready for `set-extra-highlights!`. Both are end-exclusive, and a span covering a whole line is left out. `'words` is `#f` for a hunk too large to compare word by word.

### Word hunks

`diff-words` returns `(hash 'hunks hunks)`, each hunk `(hash 'old-start-char 'old-end-char 'new-start-char 'new-end-char 'old-text 'new-text)`: 0-based char offsets into `old-text` and `new-text`, with `'old-text`/`'new-text` holding the changed words. A pure insertion has an empty `'old-text`, and `'old-start-char` equal to `'old-end-char`; a pure deletion mirrors that on the new side. It is meant for short texts such as one changed line: for long ones, compare lines with `diff-lines` first.

[Comparing text](writing-plugins.md#comparing-text) has examples.

## Text & strings

Everyday string work (trimming, splitting on a separator, case conversion, prefix/suffix tests, search and replace) uses Steel's own string functions; see Steel's [string reference](https://mattwparas.github.io/steel/book/builtins/steel_strings.html). `string-length`, `substring`, and `string-ref` count Unicode characters (codepoints), not bytes or on-screen columns.

| Call | Effect |
|------|--------|
| `split-words` | `(split-words line word-chars)`{.sig} Every word in `line`, in order, split as the `w`/`b` motions and text objects split words, so a word here is what one of those would select. `word-chars` extends what counts as a word, like the buffer option of the same name (`""` for none), and raises for a value that option would refuse |
| `string-common-prefix` | `(string-common-prefix strings)`{.sig} The longest start every string in `strings` shares, never ending partway through a character the editor shows as one (an accented letter written as two code points, an emoji sequence); `""` for an empty list |

## Filesystem & directories

| Call | Effect |
|------|--------|
| `data-dir` | `(data-dir)`{.sig} HUME's data directory, or `#f` if unavailable |
| `runtime-dir` | `(runtime-dir)`{.sig} HUME's runtime directory, or `#f` if unavailable |
| `cwd` | `(cwd)`{.sig} HUME's working directory (what `:pwd` prints) |
| `set-cwd!` | `(set-cwd! path)`{.sig} Change HUME's working directory, the one `:cd` moves. `path` is taken as written (see below), a relative one joined onto the current directory. Raises if `path` is not a directory. Works from `init.scm` too |
| `replace-dir!` | `(replace-dir! src dest)`{.sig} Move directory `src` to `dest` (both relative to the working directory), replacing the directory at `dest` and creating `dest`'s parent if needed. Either `src` ends up at `dest`, or the call raises and leaves `dest` as it was, even if the script budget runs out during it |
| `try-lock-file!` | `(try-lock-file! path)`{.sig} Take an exclusive lock on file `path` (relative to the working directory), creating the file if needed: `#t` when taken, `#f` when another HUME or program holds it. Taking a lock the same run already holds raises. See below for how long it lasts |
| `unlock-file!` | `(unlock-file! path)`{.sig} Release a lock `try-lock-file!` took; raises when this run doesn't hold it |
| `path-join` | `(path-join seg ...)`{.sig} Join path segments with the operating system's separator |
| `safe-path-segment?` | `(safe-path-segment? name)`{.sig} `#t` if `name` is a string safe to use as one path component on every platform: not empty, not `.` or `..`, and free of `/`, `\`, `:`, `"` and NUL; `#f` for anything else. Use it before joining a user-typed or downloaded name onto a path |
| `remove-dir!` | `(remove-dir! dir)`{.sig} Delete directory `dir` (relative to the working directory) and everything in it; raises when `dir` doesn't exist |
| `find-program` | `(find-program name)`{.sig} The path of the program `name` runs as, looked up on the `PATH` HUME started with (the one every program it starts gets); `#f` when there is none |
| `first-path-outside` | `(first-path-outside dir paths)`{.sig} The first of `paths` that lies outside directory `dir` once every symbolic link in both is resolved, returned as you wrote it; `#f` when every path is inside `dir` or is `dir` itself. All are relative to the working directory. A link to a missing file counts as the path it names. Raises when a path can't be resolved, such as a link that points back to itself |
| `path-entries` | `(path-entries dir prefix)`{.sig} The names in directory `dir` (relative to the working directory) starting with `prefix` (ignoring letter case on Windows), sorted, as `:e` completes them: a directory's name ends in `/` (a link to a directory included), and hidden entries appear only when `prefix` starts with `.`. `'()` when `dir` can't be read |
| `path-programs` | `(path-programs prefix extensions callback)`{.sig} List, in the background, the programs on `PATH` whose names start with `prefix`. `callback`, `(lambda (names done?) ...)`, is called as `PATH` is read with every name found so far, sorted, and a last time with `done?` `#t`. A `callback` that raises stops the listing. Returns a job for `cancel-async!`. On Windows a program is a file ending in one of `extensions` (`'(".exe")`), named without it |
| `home-dir` | `(home-dir)`{.sig} The user's home directory, or `#f` when the environment names none |
| `path->display` | `(path->display path [#:relative dir])`{.sig} `path` as `buffer-display-path` shows a file: absolute with the home directory as `~`, or with `#:relative dir`, relative to `dir` when inside it. A relative `path` or `dir` is taken against the working directory, and a `..` in either is followed on disk, so it raises when the folder before it doesn't exist. The paths are then compared as written, without following symbolic links |
| `read-data-file` | `(read-data-file path)`{.sig} The one Scheme value written in the file at `path`, with `;` comments allowed around it. Raises, naming the file, when its text doesn't parse or holds more or fewer than one value, an empty file included. Use it instead of Steel's `read` for data files: `read` returns end-of-file instead of raising on bad syntax, and leaves the bad text for the next `read` |
| `data-file` | `(data-file dir name)`{.sig} The path of `name.scm` in `dir` when `dir` holds that file, else `#f`. Only a name spelled like one of `dir`'s files, letter case included, matches: `Rust` does not match `rust.scm` on any platform, and `../x` never reaches outside `dir` |
| `data-file-names` | `(data-file-names dir)`{.sig} The names of `dir`'s `.scm` files without the suffix, sorted; `'()` when `dir` doesn't exist |

[Filesystem and processes](writing-plugins.md#filesystem-and-processes) shows how to read a plugin's own files.

A lock from `try-lock-file!` lasts until `unlock-file!`, or until the run that took it ends, or until HUME exits, so no lock is left behind. A run is everything one event starts: the command a key or the `:` prompt runs, along with every command it `call!`s; one hook, timer or callback; or loading `init.scm` and its plugins at startup.

HUME has one working directory: `(cwd)` reads it, and `(set-cwd! path)` and `:cd` move it. A path given to a HUME call is taken as written:

- A relative path joins onto that directory.
- `~` and variables like `$HOME` are not expanded.
- `..` resolves the way the operating system resolves it: after a symlink it leaves the link's target, and after a directory that doesn't exist it raises. Other links stay as typed.

Steel's own file functions resolve a relative path against the same directory. Steel's `change-current-directory!` moves only the directory those functions use, not HUME's; `set-cwd!` moves both.
