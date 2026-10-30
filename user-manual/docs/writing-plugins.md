# Writing Plugins

A plugin is Scheme, like your `init.scm`, so anything you write there you can package and share. This page walks through writing one. [Plugins](plugins.md) covers installing and loading them, and the [Plugin API](plugin-api.md) lists every editor function.

## The smallest plugin

A plugin is a directory with a `plugin.scm`, the entry point HUME loads. Without one the plugin is malformed, even if it ships a `manifest.scm`. HUME doesn't check for this: such a plugin may be reported as not installed, fail to load, or load only in part. An optional `manifest.scm` makes the plugin lazy (see [Shipping a manifest](#shipping-a-manifest)). A plugin installed by PLUM lives in a directory named after its GitHub owner and repo. The simplest `plugin.scm`:

```scheme
(define-typed-command! "hello"
  "Print a greeting from my plugin."
  (lambda ()
    (log! 'info "Hello from my plugin!")))
```

This registers `:hello`, typed at the command mode prompt.

## Defining commands

Two verbs, matching the two kinds of command described in [Command mode](command-mode.md):

```scheme
(define-command! "command-name"
  "One-line description of what the command does."
  (lambda ()
    ...))

(define-typed-command! "typed-command-name"
  "One-line description of what the command does."
  (lambda ()
    ...))
```

`define-command!` registers an editor command: bind it to a key or run it with `call!`. The command mode prompt never reaches it. `define-typed-command!` registers a typed command, reachable only as `:typed-command-name`, never from a key or `call!`. Both take a name and a one-line description first.

Start every `define-command!` name of a plugin with one shared prefix and `/`, such as `git-diff/toggle-inline-diff`, so plugins never collide: `define-command!`, `define-typed-command!` and `declare-plugin!` raise for a name already taken. The prefix is usually the plugin's name, but only has to be unique. Typed command names take no prefix.

Command names ignore case: `:Hello`, `:HELLO` and `:hello` run the same command, and two names that differ only in case collide. An editor command name must be lowercase: `define-command!` and `#:commands` raise for any other. A typed command name may use any case, and `:` Tab completion fills in the spelling it was defined with.

A typed command name starts with a letter, `-` or `_` and continues with letters, digits, `-` or `_`. `b` followed only by digits is taken, since `:b2` switches to buffer 2. `define-typed-command!` and `#:typed-commands` raise for any other name.

A command's lambda declares the leading arguments it wants, `pane` always first:

- An editor command takes `()`, `(pane)`, `(pane count)` or `(pane count extend)`. `pane` is the pane the command was invoked through, `count` the key press's count prefix (`0` means no count was typed), and `extend` whether Extend mode is active.
- A typed command takes `()`, `(pane)`, `(pane arg)` or `(pane arg force)`. `arg` is the text typed after the command name (a string, or `#f` if none), and `force` whether `!` was appended. Without `arg`, the command rejects an argument; without `force`, it rejects `!`. A lambda with a rest parameter receives both.

Declare only as many leading arguments as the command uses: one that only touches selections takes just `(pane)`, not `(pane count extend)`.

A command that writes to the terminal (an installer, a git operation) takes `#:inline-output #t`, with either verb. The alt-screen opens on the command's first real output, so a run that prints nothing (an up-to-date check, a validation error) never flashes an empty screen or waits for a keypress. Once something is printed, HUME waits for a keypress before returning to the editor. This holds however the command is reached: a key binding, `:`, or, for an editor command, `call!` from another command's body, a hook or a timer. A command `call!`'d from inside another `#:inline-output` command shares its open screen and keypress prompt.

Inside such a command, `print!` writes a line and `run-inline-output!` runs a program whose output reaches the terminal. Either one opens the alt-screen, and both raise outside such a command, `init.scm` included. While the output is on screen, Ctrl-c interrupts the running program and never quits HUME. [Filesystem and processes](#filesystem-and-processes) covers the other ways to run a program:

```scheme
(define-command! "fetch-config"
  "Clone the team config repo into the data directory."
  (lambda ()
    (run-inline-output! "git" (list "clone" "--"
                                     "https://github.com/team/hume-config.git"
                                     (path-join (data-dir) "config"))))
  #:inline-output #t)
```

An editor command that `.` should repeat takes `#:repeatable #t`. `define-typed-command!` has no such keyword: dot-repeat has no meaning for a `:` command. `#:repeatable` and `#:inline-output` are mutually exclusive:

```scheme
(define-command! "delete-and-repeat"
  "Delete the current selection; dot-repeatable."
  (lambda (pane)
    (call! "delete-selection" pane))
  #:repeatable #t)
```

## Dot-repeat

`.` repeats a `#:repeatable` command. When the command asks the user for something (`on-next-char!`, `picker!`, `live-picker!`, `prompt!`, `show-menu!`), `.` hands it the same answer instead of asking, and the command decides again what to do with it. This covers questions its callbacks ask, and a `#:repeatable` command bound to an Insert-mode key.

- Anything else that asks the user while a change is recorded (a non-repeatable command bound in Insert mode, a hook, a timer) ends the change there: `.` repeats only what came before.
- A `picker!` pick or `show-menu!` choice is matched by its text in the list the command builds this time, so the second of two "foo" items stays the second. A `live-picker!` pick, or one from a `picker!` that got items after it opened (`#:pending`, `picker-push!`, `picker-source-spawn!`), reuses its original payload, which a configuration reload forgets.
- A picker is repeated through the binding of the key that closed it, called with the item it got the first time. Bindings that left the picker open (moving the selection, say) are not repeated. The closing binding runs again in full, including whatever it did before its `picker-close!`, so call `picker-close!` before anything else. A picker closed by anything but one of its bindings repeats only its `#:on-close`.
- A `show-drawer-list!` pick is never recorded, and its `on-select` is not part of the change.
- `.` raises an error when the re-run asks for something different or more than the first run did, can't find the picked item or a forgotten payload, or opens a picker whose keymap doesn't bind the key that closed it the first time.

## Calling other commands

Use `(call! ...)` to run other commands from a plugin:

```scheme
(define-command! "delete-and-deselect"
  "Delete the selection, then collapse the cursor."
  (lambda (pane)
    (call! "delete-selection" pane)
    (call! "collapse-selection" pane)))
```

`call!` runs any editor command, built in or Scheme-defined, and activates the target plugin on demand. A built-in command needs a pane to run on, so `call!` of one raises while `init.scm` or a plugin loads: run it from a key binding, a hook or a command.

A built-in command has no parameter list of its own to receive a pane, so `call!` requires as its first argument the pane it should act through: `(call! "delete-selection" pane)`, `(call! "move-right" pane 5)`. Most built-in commands act through whatever pane the value names, focused or not, as long as that pane still exists and still shows the buffer the value names. A few (entering insert mode, an open prompt, extend mode, pane focus/split/close, tabs, …) require the focused pane; [Editor Commands](editor-commands.md) has the full breakdown. A Scheme-defined command receives `pane` as its own first parameter, so `call!` passes whatever arguments that command declares, including a `pane` the caller wants to forward.

For a built-in command, `call!` returns `#f` when the command refused (a too-small split, the last pane, …) and `#t` otherwise. `#t` means nothing was refused, not that anything changed. `call!` raises for a name it can't reach, and for a `pane` that doesn't resolve for that command. A Scheme-defined command returns whatever its body returns.

::: warning `call!` can't run typed commands
Typed commands like `write`, `quit`, or `edit` are not reachable through `call!`. Calling one raises, naming the `:` command to type instead, so `(call! "write")` will not save.
:::

A few built-in commands take one more argument after the optional count and extend flag: what the command acts with. A key press asks for it; a script passes it.

- `find-forward`, `find-backward`, `till-forward`, `till-backward`, `replace`, and `surround-add` take a Scheme character: `(call! "find-forward" pane #\x)`, `(call! "find-forward" pane 2 #\x)`. The character is required: to use one the user types, read it with [`on-next-char!`](#reading-characters) and pass it on. Convert a one-character string with `(string-ref s 0)`.
- `search-forward`, `search-backward`, and `sift-within` take a pattern, and work as if you had typed it at their prompt and pressed Enter, without opening the prompt. The pattern is required: to use one the user types, read it with [`prompt!`](plugin-api.md#other-ui-widgets) and pass it on. `call!` returns `#f` when:
  - a search finds no match. The pattern still becomes the current search, as after Enter.
  - the pattern is invalid or empty. This is also reported in `:messages`, and the current search stays as it was.
  - `sift-within` finds no match.

`prompt!` calls its callback with the confirmed text, or with `#f` when the user cancels. Unlike the `/` and `s` prompts, it doesn't preview matches as you type or recall earlier patterns:

```scheme
(define-command! "search-prompt" "Search forward for a pattern you type."
  (lambda (pane)
    (prompt! pane "search:"
      (lambda (pat) (when pat (call! "search-forward" pane pat))))))
```

`call!` raises an error for a string passed to a character command, a character passed to a pattern command, or an argument passed to a command that takes none.

Together, these make a command that removes trailing spaces and tabs from the selected lines, or from every line when no selection covers whole lines:

```scheme
(define-typed-command! "trim-trailing"
  "Remove trailing spaces and tabs."
  (lambda (pane)
    (when (selections-charwise? pane)
      (select-all pane))
    (when (sift-within pane "(?m)[ \t]+$")
      (delete-selection pane)
      (keep-primary-selection pane))))
```

When nothing matches, `sift-within` leaves the selections as they were and returns `#f`, so the `when` keeps the command from deleting them.

When you forward a `count`, `0` means no count was typed, and leaving the count out means one. So `(call! "move-down" pane)` moves one buffer line, while `(call! "move-down" pane 0)` moves like a bare `j`: `move-down`/`move-up` use this to choose between visual-line and buffer-line movement. A key-bound command that forwards its own `count` thus behaves like a native key binding. A negative count raises. `.` repeats a `#:repeatable` command with its first run's `count`, `0` included, unless you type a count before `.`.

## Reading selections

`(buffer-selections pane)` returns `pane`'s selections as a list of opaque values, positioned in char offsets, not grapheme ordinals. Each selection has:

- an anchor and a head, each the start of a character (a letter with its combining marks is one);
- a start and an exclusive end, bounding what it covers;
- a primary flag.

A command body reading its own buffer's selections declares a leading `pane` parameter and passes that. `core:stdlib`'s helpers answer the common questions about the whole list:

```scheme
(define-command! "example" "" (lambda (pane)
  (call! "stdlib/single-selection?" (buffer-selections pane))
  (call! "stdlib/all-single-char?" (buffer-selections pane))
  (call! "stdlib/cursor-char-offset" (buffer-selections pane))))
```

To work with one selection (the primary one, say), pick it out and read it with the `selection-*` calls, which need no plugin:

```scheme
(let ((primary (call! "stdlib/primary-selection" (buffer-selections pane))))
  (selection-anchor primary)
  (selection-head primary)
  (selection-start-char primary)
  (selection-end-char primary)
  (selection-primary? primary))
```

`(char-offset->line pane idx)` converts a char offset to a line number. It's a separate call rather than a field on every selection, since a selection on its own has no line information.

See [Standard Library](standard-library.md#selections) for the full list of selection helpers.

## Depending on another plugin

If your plugin `call!`s another plugin's commands, declare that plugin with `load-plugin!` or `declare-plugin!`. `call!` activates a lazily-declared dependency on demand, whether it runs at your plugin's top level or in a command a key press fires later. A `call!` to a command nothing provides raises, naming the command, so a missing dependency fails where it is first called. In your plugin's body that is at load time: the plugin fails to load and `:messages` reports it.

To name the missing plugin instead, check `(declared-plugins)`, the plugins named by `load-plugin!` or `declare-plugin!`, before the first call:

```scheme
(unless (member "core:stdlib" (declared-plugins))
  (error "my-plugin: requires core:stdlib — add (load-plugin! \"core:stdlib\") before it"))
```

This check fails at load time (startup or `:reload-config`) and names what's missing, even for a dependency used only from a command, which would otherwise fail the first time that command runs.

`(declared-plugins)` includes a plugin that is named but not installed, which `:messages` reports and `:plugin-status` shows as `absent`. A dependency declared with a narrower list than its own manifest may not provide the command you call, and the `call!` then raises the same way.

`(loaded-plugins)` lists the plugins whose code has finished loading. That suits a dependency without a manifest, which loads at startup. A dependency with a manifest is lazy and missing from that list until something triggers it, so checking there would fail for a plugin that works.

## Reading characters

A command that needs characters from the user (a surround operation, say) reads them with `(on-next-char! callback)`. The next key calls `callback` with its character; Enter and Tab count as `#\newline` and `#\tab`. Any other key, such as Esc, a mouse click, or a paste, calls it with `#f` and does nothing else. The mouse wheel scrolls as usual and the read keeps waiting. To read another character, call `on-next-char!` again from the callback:

```scheme
(define-command! "my-surround"
  "Select the pair for the next typed char, then replace it with the one after."
  (lambda (pane)
    (on-next-char!
      (lambda (ch)
        (when (eqv? ch #\()
          (call! "surround-paren" pane)
          (on-next-char!
            (lambda (new)
              (when new (call! "replace" pane new)))))))))
```

Several reads started before a key arrives take the next keys in the order they started. HUME shows nothing while it waits, so make it clear from context that a character is expected.

`.` repeats a `#:repeatable` command with the characters it read, without waiting for keys; see [Dot-repeat](#dot-repeat).

## Register prefix

To make the following `(call! …)`s in a command body target a register, call `set-register-prefix!` with a one-character register name (`0`–`9`, `k`, `c`, `b`; see [Register prefix](copy-and-paste.md#register-prefix) for what each one holds):

```scheme
(define-command! "paste-kill-ring-after"
  "Paste the kill-ring head after the selection (same as \"kp)."
  (lambda (pane)
    (set-register-prefix! "k")
    (call! "paste-after" pane)))
```

The prefix persists for the rest of the command body.

The black hole register (`"b"`) discards a selection without touching the kill ring or clipboard, for a command that throws text away as a side effect:

```scheme
(define-command! "delete-without-clobbering"
  "Delete the selection without overwriting the kill ring (same as \"bd)."
  (lambda (pane)
    (set-register-prefix! "b")
    (call! "delete" pane)))
```

## Hooks

A plugin reacts to editor events by registering a hook handler with `register-hook!`. Call it at the top level or inside a plugin body, not from a command body:

```scheme
(register-hook! 'on-buffer-save
  (lambda (pane)
    (log! 'info (string-append "saved " (buffer-name pane)))))
```

`register-hook!` raises for a lambda that can't take its hook's arguments. Every `pane` below is the same opaque value as a command's leading parameter (see [Defining commands](#defining-commands)): pass it straight to any editor function that takes one. Most hooks are about a buffer with no pane of its own, and their value carries no particular pane. For `on-buffer-enter`, `on-viewport-change`, `on-trigger-char`, and `on-completion-accept`, it names the pane the event happened in:

| Hook | Fires when | Lambda args |
|------|------------|-------------|
| `on-buffer-open` | A buffer is opened | `(pane)` |
| `on-buffer-close` | A buffer is about to close | `(pane)` |
| `on-before-buffer-save` | A buffer is about to be written to its file | `(pane)` or `(pane done)` |
| `on-buffer-save` | A buffer has been written to its file | `(pane)` |
| `on-buffer-enter` | The focused buffer changes | `(pane)` |
| `on-focus-gained` | The terminal regains focus | `()` |
| `on-mode-change` | The editor mode changes | `(old new)`: mode symbols (`'normal`, `'insert`, `'extend`, `'command`, `'search`, `'sift`); compare with `equal?` |
| `on-language-set` | A buffer's language is detected or changed | `(pane lang)`: `lang` is the language name, or `""` when the buffer has none |
| `on-diagnostics-changed` | A buffer's LSP diagnostics change | `(pane)`: pull details with `diagnostics-for-buffer` |
| `on-lsp-attach` | A language server is ready for a buffer, once per server on that buffer | `(pane server)`: `server` is the server value, the same one each time for one running server |
| `on-lsp-detach` | A language server stops serving a buffer that stays open: it was stopped or restarted, or a server list or the buffer's language no longer includes it | `(pane server)` |
| `on-lsp-notification` | A language server sends a notification HUME doesn't handle itself (it handles `window/logMessage`, `window/showMessage`, `$/progress`, and `publishDiagnostics`) | `(server method params)`: `server` is the sending server's value, `method` a string, `params` a JSON handle. A handler registered here receives every method; to handle only specific ones, use `register-lsp-notification-hook!` instead |
| `on-viewport-change` | The visible region of a pane changes | `(pane range)`: `range` is `(hash 'start-line s 'end-line e)`, the value `viewport-range` returns, 0-based and end-exclusive; with wrapping on, `'end-line` may run a few lines past the bottom edge |
| `on-trigger-char` | A registered trigger character is typed | `(pane char source)`: `char` is a Scheme char, like `#\.` |
| `on-completion-accept` | A completion entry is accepted | `(pane item)` |
| `on-option-change` | A global setting is changed (`:set global`, `set-option!`, `:theme`) | `(key value)`: `key` is a string, `value` the option's new value as `get-option` returns it |
| `on-text-changed` | A buffer's text changes | `(pane changes)`: `changes` lists the hunks since the buffer's previous `on-text-changed`, in the shape `diff-lines` returns with the earlier text as the old side, or is `#f` when HUME can't say what changed |
| `on-undo-history-changed` | A buffer's undo history changes: an edit is recorded, an undo, redo or jump moves to another revision, or a write changes which revision is saved | `(pane)`: pull the tree with `buffer-undo-tree` |

`on-before-buffer-save` fires for `:w`, `:wq` and `:wa`, but not for a write that doesn't reach the buffer's own file (a read-only buffer, a buffer with no path). The file is written once every handler has finished:

- A `(pane)` handler finishes when it returns.
- A `(pane done)` handler finishes when it calls `(done)`, so it can wait for a later reply, such as a language server's. Call `done` on every path, failures included.
- A handler that raises an error is reported and counts as finished.
- If a handler hasn't finished after `hook-before-save-timeout-ms`, the file is written anyway and a warning says so.

Closing the buffer while its save waits drops the save; `:reload-config` lets it go ahead. `:wa` reports what it saved and skipped once all its saves are done, and a failed save stops the ones still waiting. `pane` is the pane the save was asked from if it shows the buffer when the handlers run, otherwise a value with no pane (`:wa` on a hidden buffer, or a pane that has switched buffers). [Format on save](lsp.md#format-on-save) is an example.

`on-buffer-open` and `on-buffer-close` always fire as a pair for a given buffer: a buffer opened and closed within one command fires neither.

`on-text-changed` covers edits, undo, redo, `:e!` reload, and refreshes of read-only view buffers (`:messages`, `:plugin-status`). Those buffers have no file, so a handler that looks up a path must handle a missing one. All changes one command makes (a multi-cursor edit, a macro, a paste) fire it once, but each keystroke while typing is its own command and fires on its own. To react only after typing settles, pair it with `debounce`.

`changes` is `#f` after a refresh of a read-only view buffer, and after a long run of edits, or edits that change a lot of text, since the previous fire. Treat `#f` as "anything may have changed" and rebuild from the whole buffer. A debounced handler runs once for several fires and sees only the last one's `changes`. To get everything, keep the `(buffer-generation pane)` value from when you last caught up and pass it to `buffer-changes-since`.

`on-undo-history-changed` also fires once per command, but only when the undo history moves or a write changes the saved revision, so an insert session fires it once, when the session ends. It also fires for an undo whose net change to the text is nothing (undoing an insert together with its later delete), which `on-text-changed` does not.

A lazy plugin declares the events that activate it with `#:events` in its `manifest.scm` instead (see [How plugins are loaded](plugins.md#how-plugins-are-loaded)). LSP-related hooks like `on-lsp-attach` work with `register-hook!` but not as an `#:events` entry: a plugin gated only on `on-lsp-attach` never activates, since nothing attaches to a server until the plugin has loaded and registered it. Gating on `on-text-changed` activates the plugin on the first edit in *any* buffer, not one it cares about.

`set-option!` also works from a hook or command handler. It changes the *global* default, so use it there only when that is what you want. A value the user set wins over your plugin's, whichever comes first, so a plugin's `set-option!` is a default the user can always override. `set-buffer-option!` ranks the same way against the buffer's own overrides, and a buffer override from your plugin still beats the user's global value.

`(set-buffer-option! pane "option" value)` sets an option on `pane`'s buffer only, from hook and command bodies too ([Buffer options](settings.md#buffer-options) lists the settable options). Pass the pane the hook hands you rather than assuming the buffer you're editing: a hook can fire for a buffer that isn't focused. Setting `language` changes the buffer's language; `""` clears it. To read a buffer's options back, see [Reading options from Scheme](#reading-options-from-scheme) below.

One more example:

```scheme
; react to diagnostics
(register-hook! 'on-diagnostics-changed
  (lambda (pane)
    (let ((errs (diagnostics-for-buffer pane #:severity 'error)))
      (log! 'info (string-append (to-string (length errs)) " errors")))))
```

## Reading options from Scheme

```scheme
(get-option "option-name")
(get-buffer-option pane "option-name")
```

`get-option` returns the option's global value, ignoring any buffer override. `get-buffer-option` returns `pane`'s buffer's effective value: its own override if one is set, else the global default. A named value comes back as a string, like `indent-style`'s `"soft"`. Pass the pane you were given (an `on-language-set` handler receives one) rather than assuming the buffer you care about is the focused one. Both raise for an unknown option name.

- `language` and `line-ending` are per-buffer, with no global value, so `get-option` and `set-option!` raise for them. `(get-buffer-option pane "language")` returns the buffer's language name, or `""` if it has none. `(get-buffer-option pane "line-ending")` returns `"lf"` or `"crlf"`.
- `(get-option "shell")` returns a list of strings, the program and its first arguments: `'("sh" "-c")`.
- For `wrap-mode`, `get-buffer-option` reads the buffer and global levels only: a pane pinned with `:set pane wrap-mode=…` can show a different style than it reports.

```scheme
(get-option "indent-width")           ; the global default indent-width
(get-buffer-option pane "indent-width") ; pane's effective indent-width
```

## Shipping a manifest

A plugin that ships a `manifest.scm` next to its main file is lazy. The manifest declares the entry points that wake the plugin, and a user's `(load-plugin! "username/repo-name")` registers them.

```scheme
; manifest.scm
(declare-plugin! "username/repo-name"
  #:commands '("my-cmd" "my-other-cmd"))
```

A plugin with no `manifest.scm` loads eagerly. A user who wants different triggers declares them before their `load-plugin!` line (see [Choosing your own triggers](plugins.md#choosing-your-own-triggers)), and their list replaces the manifest's instead of merging with it.

If your plugin reacts to a language but can't predict which ones a user cares about, `#:languages '("*")` matches any buffer with a detected language:

```scheme
; manifest.scm
(declare-plugin! "username/repo-name"
  #:languages '("*")
  #:commands '("my-cmd"))
```

Keep `manifest.scm` to just the `declare-plugin!` calls. A manifest takes no `#:config`; the user's config comes from `load-plugin!` and you read it with `(plugin-config)`.

## Splitting a plugin into entries

A plugin can load in pieces. Give `declare-plugin!` an `#:entry` naming another `.scm` file in the plugin's directory, with its own activation entries, and that file loads only when one of them fires. A declaration without `#:entry`, or with `#:entry "plugin.scm"`, is for the main file, `plugin.scm`. Spell every `#:entry` as the file is named on disk, case included; a file that doesn't exist makes the declaration fail with an error. `"Commands.scm"` for a file named `commands.scm` is malformed even where the filesystem ignores case, because it may work on one system and fail on another.

```scheme
; manifest.scm
(declare-plugin! "username/repo-name"
  #:languages '("*"))
(declare-plugin! "username/repo-name"
  #:entry "commands.scm"
  #:typed-commands '("my-install"))
```

Here `plugin.scm` runs when a buffer gets a language, keeping startup cheap, and the heavier `commands.scm` runs the first time `:my-install` is used or its argument is completed with Tab. Each file is loaded, rolled back on failure and reported on its own: a failing `commands.scm` leaves what `plugin.scm` registered in place. Files that `require` the same module share it.

The user's `#:config` belongs to the plugin as a whole, and `(plugin-config)` returns the same value in every file.

## Configuring a plugin

`(plugin-config)` returns the `#:config` hash the user passed to `load-plugin!`: a hash with symbol keys such as `(hash 'disable-binding #t)`, or an empty hash if nothing was passed. A plugin can call it in its body, or later from a command, hook or callback it defined. Called from code that isn't a plugin's, it raises. Instead of raw `hash-contains?`/`hash-ref` and a hand-rolled type check, use `core:stdlib`'s config helpers. They default a missing key, and raise an error naming your plugin and the offending key if the resolved value has the wrong type:

```scheme
(unless (call! "stdlib/config-boolean" "my-plugin" (plugin-config) 'disable-binding #f)
  (bind-key! 'normal "C" "my-command"))
```

`stdlib/config-string`, `stdlib/config-enum`, `stdlib/config-integer`, and `stdlib/config-list` cover the other common shapes; see [Standard Library](standard-library.md#plugin-configuration) for their signatures. This call runs in your plugin's body at load time, so without `core:stdlib` declared your plugin fails to load (see "Depending on another plugin" above).

Document the keys your plugin understands so users know what to pass.

`load-plugin!` is the only way to pass configuration; `declare-plugin!` takes no `#:config`.

## Filesystem and processes

Plugins are trusted code: like any Scheme program, they can read and write any file and spawn any process, with no sandboxed subset of the filesystem. Use Scheme's own functions directly (`open-input-file`, `create-directory!`, `delete-file!`, `read-dir`, `path-exists?`, and so on) for file access.

Before hand-rolling one of these, check [Standard Library](standard-library.md): its filesystem and git commands cover what plugins need most often (idempotent delete, sorted subdirectory listing, git work-tree detection and repo-root resolution).

A few extra functions cover what Scheme can't know on its own:

| Function | Description |
|----------|-------------|
| `(data-dir)` | HUME's data directory, or `#f` if unavailable |
| `(runtime-dir)` | HUME's runtime directory, or `#f` if unavailable |
| `(path-join seg…)` | Join path segments with the OS-native separator |
| `(json-parse str)` | Decode a JSON string: an object/array becomes a JSON handle (read with `json-ref`/`json-contains?`/`json-list`), a scalar crosses natively; errors on malformed input |

To run a program, pick the call by what you need from it:

| Need | Call |
|------|------|
| Its output right away, from something quick (`git rev-parse`) | `(run-capture! cmd args)`: blocks until it exits and returns `(hash 'stdout s 'stderr s 'exit code)` |
| Output the user watches, inside an `#:inline-output` command (an installer, a `git clone`) | `(run-inline-output! cmd args)` |
| Its result later, without making the user wait | `(spawn-async! cmd args callback)` |
| Lines that fill a picker as the program prints them | `picker-source-spawn!`, or `picker-command-source` for a picker that reruns it on every query (see [Custom pickers](#custom-pickers)) |

All four start the program in HUME's working directory unless given `#:cwd`, and look it up on the `PATH` HUME started with. They deny git a credential prompt, so a private repository fails instead of hanging HUME. Steel's own `command`/`spawn-process`/`wait` stay available for what these don't cover, such as writing to a program's input, but HUME does nothing to make them safe: a program they start can write over the editor's screen and stop to ask git for credentials, and waiting for it counts against the script budget. `find-program` looks a program up on the `PATH` HUME started with.

`run-inline-output!` also takes `#:env`, a list of `("KEY" . "VALUE")` pairs added to the child's environment. It raises an error if the command exits non-zero, so wrap it in a handler if a failure is expected.

`run-capture!` blocks the whole editor until the command finishes: fine for something instant (`git rev-parse`), not for anything that might take a moment while the user keeps typing. For that, run it in the background:

```scheme
(spawn-async! "git" (list "show" (string-append ref ":" path))
  (lambda (result)
    (if (equal? (hash-ref result 'exit) 0)
        (use-the-output (hash-ref result 'stdout))
        (report-the-failure (hash-ref result 'stderr))))
  #:cwd repo-root)
```

`spawn-async!` starts `cmd` with `args` and returns immediately. `callback` is called once, after the command has finished, with the same hash `run-capture!` returns:

- `'stdout`: its complete output, as a string.
- `'stderr`: its error output; past 8 KiB, the first and last 4 KiB of it.
- `'exit`: its exit code, or `#f` if it was killed by a signal or couldn't even start (a missing binary, say). Compare it with `equal?`, not `=`.

A command that writes more than 64 MiB to stdout gets the answer of one that couldn't start, with the reason in `'stderr`. A command that fails to start or exits non-zero still calls `callback` instead of raising, so there's one place to handle the outcome. `spawn-async!` returns a job: `(cancel-async! job)` kills the command and discards its callback before it fires. That helps when a newer run supersedes a debounced action (a hook firing on every keystroke, say) before the older command has finished.

Install or overwrite files only under `(data-dir)`, unless you have a specific reason to go elsewhere: HUME expects a plugin's own data (installed grammars, downloaded servers, plugin state) there.

## Reading buffer text

```scheme
(buffer-text pane)
```

Returns a buffer's live content as a string, unsaved edits included, not what's on disk. The string always ends with a newline, and its line endings are always `\n`, even for a file saved with `\r\n`.

```scheme
(buffer-lines pane)
(buffer-lines pane #:range (hash 'start-line start 'end-line end))
```

Returns the buffer's lines as a list, line endings stripped: all of them, or with `#:range`, a 0-based, end-exclusive slice. `(buffer-lines pane #:range (hash 'start-line 10 'end-line 40))` returns lines 10 through 39. A range missing either end, ending past the last line, or starting after it ends raises an error instead of being clamped. `(viewport-range pane)` returns a range of the same shape, so this reads only what's on screen:

```scheme
(buffer-lines pane #:range (viewport-range pane))
```

To diff a buffer against another text, use `(diff-buffer-lines pane ref-text)` instead of `(buffer-text pane)`, especially from a hook that fires on every keystroke.

```scheme
(buffer-line-count pane)
```

Returns the buffer's content line count as an integer: the number of lines `buffer-lines` returns. When the count is all you need, use it instead of `(length (buffer-lines pane))`, which builds and throws away a full list of line strings just to count them.

```scheme
(buffer-char-count pane)
```

Returns the buffer's length in characters, the final line break included: the largest offset `char-offset->line` accepts. With `line->char-offset`, it measures a run of lines without fetching them, the length of the last line included.

```scheme
(line->char-offset pane line)
```

Returns the 0-based char offset where content line `line` (0-based) starts. Decoration functions that take char offsets (like `set-extra-highlights!`) need this when all you have is a line number, from a diff hunk say. Raises if `line` is at or past the buffer's content line count.

### Example: Markdown headings

Reading lines and editing char ranges together make a command that turns lines into Markdown headings. `G H` makes a level-one heading, and a count picks the level: `2GH` turns `Title` into `## Title`, and `### Title` into `## Title` too:

```scheme
(define (heading-prefix-length line)
  (let* ((n (string-length line))
         (skip (lambda (i c)
                 (let loop ((i i))
                   (if (and (< i n) (char=? (string-ref line i) c))
                       (loop (+ i 1))
                       i)))))
    (skip (skip 0 #\#) #\space)))

(define (anchor-lines pane)
  (let loop ((sels (buffer-selections pane)) (lines '()))
    (if (null? sels)
        (reverse lines)
        (let ((line (char-offset->line pane (selection-anchor (car sels)))))
          (loop (cdr sels)
                (if (and (pair? lines) (= line (car lines)))
                    lines
                    (cons line lines)))))))

(define-command! "markdown-heading"
  "Turn the line of each selection's anchor into a Markdown heading. The count sets the level."
  (lambda (pane count)
    (let ((prefix (string-append (make-string (min 6 (max 1 count)) #\#) " ")))
      (edit-text! pane
        (filter (lambda (edit) edit)
          (map (lambda (line)
                 (let ((text (car (buffer-lines pane #:range (hash 'start-line line 'end-line (+ line 1))))))
                   (and (> (string-length text) 0)
                        (let ((start (line->char-offset pane line)))
                          (hash 'start-char start
                                'end-char (+ start (heading-prefix-length text))
                                'text prefix)))))
               (anchor-lines pane)))))))

(bind-key! 'normal "G H" "markdown-heading" #:keymap "markdown-keys")

(register-hook! 'on-language-set
  (lambda (pane lang)
    (if (equal? lang "markdown")
        (attach-keymap! pane "markdown-keys")
        (detach-keymap! pane "markdown-keys"))))
```

The keys go into a keymap of their own, which the hook turns on in Markdown buffers only (see [Keys for one buffer](configuration.md#keys-for-one-buffer)).

`anchor-lines` lists the line of each selection's anchor, once per line: selections come in order and never overlap, so two on the same line are adjacent. For each line with text, the command builds one edit that replaces the line's leading `#`s and spaces with the new ones (an empty range when there are none). [`edit-text!`](plugin-api.md#editing-navigation) applies them all as one edit that one `u` undoes. Every selection stays where it was: a cursor on the old `#`s moves to the start of the line, and any other cursor stays on its character.

Each line needs something different here: a heading loses its old `#`s, a plain line only gets new ones, and an empty line nothing. Commands like `around-line` do the same thing to every selection, and `paste-text!` pastes over each selection the way a paste does, so neither can express that. When every selection gets the same change, compose commands with `call!`. When each needs its own, compute the edits and apply them with `edit-text!`.

## Comparing text

Two functions compute a line-level diff, useful for anything that shows what changed between two versions of a file, like a git-status indicator:

```scheme
(diff-lines old-text new-text)
```

Splits both texts into lines the way HUME treats file content: every line ending becomes LF, and a missing trailing newline doesn't count as a change. Returns `(hash 'hunks hunks)`: the hunks where they differ. Unchanged lines are left out. Each hunk is:

```scheme
(hash 'old-start-line s 'old-end-line e 'new-start-line s 'new-end-line e 'old-lines texts 'new-lines texts 'words spans)
```

- `'old-start-line`/`'new-start-line` are 0-based line numbers, and `'old-end-line`/`'new-end-line` the line after the hunk's last one on each side, so a side's lines are `[start, end)`. A pure insertion has an empty old range (`'old-start-line` equals `'old-end-line`); a pure deletion has an empty new range. Either way, the empty side's line number is where the change happens, so it feeds straight into `set-signs!` or `set-virtual-lines!` with no adjustment.
- `'old-lines`/`'new-lines` are the line contents (no trailing newline).
- `'words`, `(hash 'old spans 'new spans)`, names the words that changed. An old-side span is `(hash 'line 'start-char-col 'end-char-col)`: a line of the old text and character columns in it. A new-side span is `(hash 'start-char 'end-char)`: character offsets into the new text, counting each line ending as one character, ready for `set-extra-highlights!`. Both are end exclusive, and a span covering a whole line is left out. `'words` is `#f` for a hunk too large to compare word by word, so a caller can tell that from a change of whole lines.

```scheme
(diff-buffer-lines pane ref-text)
```

Same result, but compares `ref-text` against the current, unsaved content of `pane`'s buffer, without first pulling the buffer through an editor function as one big string. Use this one in a hook that fires on every keystroke.

To compare inside a single changed line, highlighting the words that differ rather than the whole line:

```scheme
(diff-words old-text new-text)
```

Returns `(hash 'hunks hunks)`. `'hunks` is a list of `(hash 'old-start-char 'old-end-char 'new-start-char 'new-end-char 'old-text 'new-text)`: 0-based character positions into `old-text`/`new-text`, with `'old-text`/`'new-text` holding the changed words. A pure insertion has an empty `'old-text` and `'old-start-char` equal to `'old-end-char`; a pure deletion mirrors that on the new side. It is meant for short texts such as one changed line: for long ones, compare lines with `diff-lines` first.

## Drawing a diff

With [`core:git-diff`](core-plugins.md#core-git-diff) loaded, a command can ask `(call! "git-diff/inline-diff?" pane)` whether inline rendering is on, and turn it on against a ref with `(call! "git-diff/show-inline-diff" pane "HEAD~1")`. The latter never turns it off.

A plugin with its own hunks, such as a revision's diff from `buffer-revision-diff`, can draw them the way `core:git-diff` draws its own with `(call! "git-diff/render-diff" pane source hunks)`. `hunks` is the `'hunks` list `diff-buffer-lines` or `buffer-revision-diff` returns. `source` is a name of the plugin's own, so its drawing never mixes with `core:git-diff`'s. Each call replaces what `source` drew for that buffer, and an empty list draws nothing. While a plugin draws, the buffer's own inline diff is hidden so changed lines don't show twice; the gutter signs stay. `(call! "git-diff/release-diff" pane source)` clears the plugin's drawing and brings the inline diff back.

## Completion sources

A plugin can feed the completion menu by registering a *source*: a procedure the editor calls with a request `id`. The source answers by calling `completion-emit!` with that `id` once, right away or later from a callback, or in parts with `#:more`. The smallest useful source offers a fixed list of words in Insert mode:

```scheme
(register-completion-source! "todo-markers"
  (lambda (id pane prefix)
    (completion-emit! id '("TODO" "FIXME" "HACK")))
  #:target 'buffer)
```

Press `Ctrl-Space` after typing `FI` and `FIXME` is offered alongside every other source's answers. The source can ignore `prefix`, the word being typed: the editor filters and ranks the items against it, and replaces it with the item you accept. An empty list means the source has nothing to offer.

A bare string is shorthand for an item with only a label. For more control, pass a hashmap using the keys of an LSP `CompletionItem`:

- `label`: required.
- `detail`, such as a type or signature: shown in the documentation box described below.
- `insertText`: what accepting the item inserts (default: the label).
- `filterText` and `sortText`: change what the typed text is matched and tie-broken against.
- `kind`: an LSP `CompletionItemKind` number.
- `documentation`: a string or a `(hash 'kind "markdown" 'value text)`, shown below `detail` in a box beside the menu while the item is highlighted. `'kind "plaintext"` shows it as plain text.

```scheme
(register-completion-source! "todo-markers"
  (lambda (id pane prefix)
    (completion-emit! id
      (list (hash 'label "TODO" 'detail "work left to do" 'insertText "TODO: ")
            (hash 'label "FIXME" 'detail "known bug" 'insertText "FIXME: "))))
  #:target 'buffer)
```

The documentation box appears only in Insert mode, for an item with a `detail` or `documentation`. It is drawn right of the menu, or left when there is no room on the right. When neither side fits, or `:set global completion-docs=false` turns the box off, each item's `detail` shows next to its label in the menu instead. Give each item its documentation with the rest of the item: only a language server's items can have theirs fetched later, when highlighted.

```scheme
(register-completion-source! "todo-markers"
  (lambda (id pane prefix)
    (completion-emit! id
      (list (hash 'label "TODO"
                  'detail "work left to do"
                  'insertText "TODO: "
                  'documentation (hash 'kind "markdown"
                                       'value "Marks *unfinished* work.\n\nSearch for it before a release."))
            (hash 'label "NOTE"
                  'detail "remark for readers"
                  'insertText "NOTE: "
                  'documentation "Explains something the code alone does not."))))
  #:target 'buffer)
```

Highlighting `TODO` shows `work left to do` above the rendered markdown. Highlighting `NOTE` shows its `detail` above the plain text.

To open the menu without `Ctrl-Space`, give the source trigger characters for a language. This one fires whenever `@` is typed in a Markdown buffer:

```scheme
(set-completion-triggers! "markdown" "todo-markers" '(#\@))
```

A source can also complete the argument of a `:` command. A `'minibuf` source is called with the whole command line and the cursor position, counted in characters. This one answers asynchronously, once `git` has listed the branches. `#:complete` connects it to the command:

```scheme
(register-completion-source! "git-branches"
  (lambda (id input cursor)
    (spawn-async! "git" '("branch" "--format=%(refname:short)")
      (lambda (result)
        (completion-emit! id
          (if (equal? (hash-ref result 'exit) 0)
              (filter (lambda (s) (not (equal? s "")))
                      (split-many (hash-ref result 'stdout) "\n"))
              '())))))
  #:target 'minibuf #:match 'string)

(define-typed-command! "checkout" "Switch to a git branch: :checkout <branch>."
  (lambda (pane arg)
    (when (string? arg)
      (spawn-async! "git" (list "checkout" arg)
        (lambda (result)
          (unless (equal? (hash-ref result 'exit) 0)
            (log! 'error (hash-ref result 'stderr)))))))
  #:complete "git-branches")
```

If the user keeps typing or closes the menu before `git` answers, the request is out of date: `completion-emit!` drops the answer and returns `#f`. To stop `git` as well, register the source with `#:cancel`.

### Source options

- **`#:target`** (required): `'buffer` serves Insert mode and calls `(proc id pane prefix)`; its token is the run of word characters before the cursor. `'minibuf` serves the `:` and `|` lines and calls `(proc id input cursor)`, with `input` the whole line and `cursor` counted in characters; its token is the whitespace-delimited argument the cursor is in, unless the answer names its own with `#:span`. The token is what items are matched against and what accepting one replaces.
- **`#:match`**: how items are scored against the token. `'fuzzy` (default) keeps items containing the typed characters in order, not necessarily adjacent. `'string` keeps only items that start with the token, all scored equally, so once something is typed, fuzzy matches from other sources rank above them. `'delegated` treats the answer as already filtered and keeps the source's own order.
- **`#:priority`** (`'buffer` sources only, default `0`): breaks score ties between sources, higher first.
- **`#:token-chars`** (`'buffer` sources only, default `""`): characters added to the token on top of the buffer's `word-chars`. With `#:token-chars "-$"`, `foo-bar` is one token: it is what `prefix` holds and what accepting an item replaces, and typing `-` keeps the menu open. It cannot contain whitespace.
- **`#:resolve #t`** (`'buffer` sources only, default `#f`): the items come from the buffer's language server, so highlighting one (for its documentation) or accepting one may send `completionItem/resolve` for it. Set it only for a source that forwards that server's items, never for one that builds its own items in a buffer with a server attached.
- **`#:cancel`** (default `#f`): `(cancel id)` is called when HUME stops waiting for request `id` before its last answer: the user typed on, closed the menu or accepted an item, the source was called again, or the source raised an error when HUME called it. Use it to stop work started for that request, such as a `spawn-async!` job, with `cancel-async!`. A request answered in full is never cancelled.
- **Names**: `'buffer` and `'minibuf` sources have separate names. Registering `name` again under the same target replaces the earlier source; the same `name` under the other target is a second, independent source.
- **`#:incomplete #t`** on `completion-emit!` (`'buffer` sources only) asks for the source to be called again as the user keeps typing, instead of filtering the first answer. A server response marked incomplete does the same.
- **`#:more #t`** on `completion-emit!`: more of the answer follows. Each part replaces the one before, so send every item found so far, and send the last part without `#:more`. An answer without it closes the request; a later answer to it is dropped. The menu shows each part as it arrives, and in Insert mode reports that nothing was found only after the last part. On the `:` line, the first `Tab`'s insert and the insert of a lone item wait for the last part, and a row the user picks stays picked while more parts arrive. Send the last part even when the work fails, or the request stays open until the user moves on. Likewise, an error in a source's callback leaves the request open, so answer `'()` when the work fails.
- **`#:span (hash 'start-char s 'end-char e)`** on `completion-emit!` (`'minibuf` sources registered with `#:match 'delegated` only): the part of the line the answer completes, in characters, in place of the argument the cursor is in. It must hold the cursor. Use it when a word can contain spaces, like a quoted shell word.
- **`#:prefix`** on `completion-emit!` (`'minibuf` sources registered with `#:match 'delegated` only): what the first `Tab` inserts over the token when the answer has several items. Without it, `Tab` inserts what the items' insert texts all start with, when that extends what is typed. A string cannot be empty, and is inserted as given only when the cursor is at the end of the token, since the source did not read the text after the cursor. `#f` inserts nothing, and the first `Tab` only opens the menu. Name the text, or send `#f`, when the insert texts have a syntax of their own, like quoted shell words, whose shared start is not something to put on the line.
- `completion-emit!` also takes a server's whole `textDocument/completion` response, as [`lsp-request!` or `lsp-request-all!`](lsp-api.md#language-servers) delivered it, in its `items` list; its items join the others. Any entry that is not a string, a hashmap with a `label`, or such a response is an error.

## Shell dialects

`Tab` on the `|` prompt finds, reads and writes the word under the cursor by the rules of the shell the `shell` option runs. HUME ships rules, called dialects, for `sh`, `bash`, `zsh`, `dash`, `ksh`, `ash`, `mksh`, `yash`, `posh` and `busybox` (one POSIX dialect), `fish`, PowerShell and `cmd`. For another shell, register a dialect from `init.scm` or a plugin. A shell that reads words the way `sh` does can reuse the shipped procedures:

```scheme
(register-shell-dialect! "oksh"
  #:programs '("oksh")
  #:parse shell/posix-parse
  #:write shell/posix-write)
```

A word has two forms: what is on the line, like `"my dir/no` or `my\ dir/no`, and what the shell reads for it, `my dir/no`. A dialect turns the first into the second with `#:parse`, and puts names back on the line with `#:write`. HUME finds and matches file names on what the shell reads, so a dialect only has to get the shell's quoting right.

- **The name**: registering a name again replaces that dialect, a shipped one included.
- **`#:programs`**: the shells the dialect is for, matched against the file name of the `shell` option's program, whatever its directory, letter case or `.exe`. When two dialects list a program, the one registered last is used.
- **`#:program-extensions`** (default `'(".exe")`): on Windows, the file extensions that make a file a program, which is then offered without its extension. Elsewhere a program is a file anyone may run, and this is not used.
- **`#:parse`**: `(parse input cursor)` reads the word around `cursor` in the line `input` and returns `(shell-word start end command? typed dir open prefix)`:
  - `start`, `end`: where the word starts and ends in `input`, in characters. A completion replaces this part of the line, so it must hold `cursor`.
  - `command?`: whether the shell reads the word as a program name.
  - `typed`: the word as typed, from its start to just past its last directory separator before the cursor, together with a quote that opens right after it. `""` when there is none. When the last separator comes from a variable's value and a name written after the variable would read as part of its name (`$TMPDIR`), `typed` ends with that separator added, so it is `$TMPDIR/`.
  - `dir`: what the shell reads for `typed`, with `~` and variables expanded: the directory to list.
  - `open`: the quote still open at the end of `typed`, or `#f`.
  - `prefix`: what the shell reads from the end of `typed` up to the cursor: the start of the name.
- **`#:write`**: `(write word name close?)` returns the string that replaces the word: usually `typed` as it is, then `name` written for the quote `open` names. `name` is a program or a directory entry, a directory's ending in `/`. With `close?` true the name is complete, and the result closes any quote it is in, also after a directory. With `close?` false the name is the start of several names and the user keeps typing, so a quote stays open.

The shipped procedures are `shell/posix-parse` and `shell/posix-write`, and the same two for `fish`, `pwsh` and `cmd`.

## Custom pickers

The modal fuzzy-finder panel behind [Fuzzy Finder](pickers.md) is a generic widget any plugin can drive: `core:pickers` builds its own file and buffer finders from this API alone.

```scheme
(picker! pane items keymap #:prompt "buffers: ")
```

`pane` must still be the focused pane when the picker opens. Pass the pane the enclosing command or hook was given, or `(focused-pane)` if none is in scope.

`items` is a list of `(display . payload)` pairs. `display` is the string shown and matched against. `payload` is anything you like (a path, a number, a hashmap) except `#f`, which means "nothing matches"; HUME never looks inside it.

`keymap` is every key the picker answers besides typing. It's a hashmap from a key, written as `bind-key!` writes one (`"ctrl-v"`, `"alt-o"`, `"f5"`, never a sequence like `"z f"`), to a procedure of two arguments: the picker's token and the selected item's payload, or `#f` when nothing matches. A key the keymap doesn't bind does nothing, except a plain character, which types into the query, and `Backspace`, which deletes from it; bind either one to change that. `picker!` raises when two keys in the keymap are the same key, such as `"ctrl-n"` and `"c-n"`.

A binding never closes the picker by itself: it calls `(picker-close! token)`. To move the selection it calls `(picker-move! token count #:by 'row)`. `#:by` is `'row` (the default), `'page` or `'half-page`, and a negative `count` moves up.

`core:stdlib`'s `stdlib/picker-keymap` is the keymap `core:pickers` uses. Give it the procedure that handles a chosen item, and it binds `Down`/`Ctrl-n` and `Up`/`Ctrl-p` to move the selection, `PageDown`/`PageUp` and `Ctrl-d`/`Ctrl-u` to move it by a page and half a page, `Esc` to close the picker, `Enter`/`Ctrl-o` to close it and call the procedure with the selected item (`#f` when nothing matches), and `Ctrl-t`/`Ctrl-v`/`Ctrl-s` to do the same in a new tab, a side-by-side split or a stacked split:

```scheme
(picker! pane items (call! "stdlib/picker-keymap" open-item!) #:prompt "files: ")
```

To change or add a key, set it in that map:

```scheme
(picker! pane items
  (hash-insert (call! "stdlib/picker-keymap" open-item!)
               "ctrl-y" (lambda (token item) (picker-close! token) (copy-path! item)))
  #:prompt "files: ")
```

`#:on-close` takes a procedure of no arguments, called once when the picker closes, however it closes: a binding's `picker-close!`, a `picker-close!` from anywhere else, or a second picker opening and replacing this one. Use it to clean up after the picker, such as cancelling a `spawn-async!` job still filling it.

`picker!` returns a token naming this picker, which `picker-close!` takes. The close does nothing if the picker has since been closed or replaced, so a late callback (a `spawn-async!` result, say) never tears down a different picker the user has opened since.

For a handful of items (buffers, a plugin's own static list, a quick synchronous command's output), pass the whole list to `picker!`. For large lists (file lists, grep-style output), open the picker empty and stream an external command's output into it:

```scheme
(define token
  (picker! pane '()
    (call! "stdlib/picker-keymap" (lambda (path) (when path (open-buffer! path))))
    #:prompt "files: "))
(picker-source-spawn! token "git" '("ls-files" "-z" "--cached" "--others" "--exclude-standard") #:nul #t)
```

`picker-source-spawn!` runs `cmd` with `args` directly (no shell). It splits stdout into lines (or NUL-delimited fields with `#:nul #t`) and appends each to the picker as a `(line . line)` item. Display and payload are the same raw line, except that a NUL *inside* a line shows as `:` in the display. That happens when a command uses NUL as an in-line field separator rather than a record delimiter, such as `rg --vimgrep --null`; the payload keeps the NUL, so parsing it in a binding still sees the real separator. The output never passes through Scheme, so this stays fast even for tens of thousands of results: parse the selected line in the binding, not up front. Closing or replacing the picker before the command finishes kills the child process. For a source that produces its own results asynchronously (an LSP request, a timer), `(picker-push! token items)` appends a batch of ordinary `(display . payload)` items instead.

`picker-source-spawn!` shows the user that results are still loading. A picker filled by `picker-push!` from a `spawn-async!` callback has no such signal, so pass `#:pending #t` to `picker!` when opening it empty. That marks the panel as "results still arriving" until the first `picker-push!`/`picker-replace!` call lands.

A row too wide for the panel is clipped to fit, with `…` at the dropped end. `#:truncate` picks the end:

- `'head` (default) drops the front and keeps the tail: right for a file path, whose basename sits at the end.
- `'tail` drops the back and keeps the head: right for a row whose distinguishing part sits at the *front*, like a grep match's `path:line:col:` prefix before the line preview, where a head-cut would swallow the path and show only preview text.

A spawned source's nonzero exit is reported as an error. For a command where some nonzero exits are a normal outcome (`rg` exits `1` for "no matches"), pass `#:ok-exit-codes`. The list replaces the usual "zero is success" rule instead of adding to it, so include `0` unless you want a successful run reported as a failure. `rg` also needs an explicit path argument here: with no path and no terminal attached to its input, it searches its (empty) stdin instead of the working directory, and finds nothing:

```scheme
(picker-source-spawn! token "rg" (list "--vimgrep" "--" pattern ".") #:ok-exit-codes '(0 1))
```

`.` repeats a `#:repeatable` command's pick without opening the picker again, through the binding of the key that closed it; see [Dot-repeat](#dot-repeat).

For a query that drives the *source* itself instead of filtering an already-fetched list, see [Live requery](#live-requery-live-grep) below.

## Live requery (live grep)

A picker whose query should re-run an external command with the new pattern on every keystroke (a live grep, say) uses `live-picker!` instead of `picker!`:

```scheme
(define (grep/open! pane seed)
  (live-picker! pane (call! "stdlib/picker-keymap"
                       (lambda (row) (when row (goto-location! pane (grep/parse row)))))
                #:prompt "grep: " #:query seed
                #:source (picker-command-source
                          (lambda (query)
                            (and (not (equal? query ""))
                                 (list "rg" "--vimgrep" "--" query ".")))
                          #:ok-exit-codes '(0 1))
                #:truncate 'tail))
```

`grep/parse` is left to the reader. `live-picker!` takes a keymap and accepts `#:truncate` and `#:on-close` as `picker!` does (see [Custom pickers](#custom-pickers) above). Here `#:truncate 'tail` keeps the path and clips the line preview: `rg --vimgrep`'s rows are `path:line:col:preview`, so the default head-cut would clip the path. `stdlib/picker-keymap`'s `Ctrl-t`/`Ctrl-v`/`Ctrl-s` open a match in a new tab or split instead of the current pane.

[grep.hume](https://github.com/cvlmtg/grep.hume) is this same idea, finished, with `grep/parse` included.

`live-picker!` opens empty, with no `items` argument and no `#:pending`: its own requery fills it, never a caller pushing items. `#:source` is a function of the picker token and the current query. HUME calls it on every keystroke, after the debounce window, and it fills the picker with `picker-replace!`, `picker-push!` or `picker-source-spawn!`. A source that never answers leaves the previous rows up.

Each query gets its own token, the one `#:source` is called with. A source that answers later, from a language server say, keeps that token and fills the picker through it. Once the user has typed on, `picker-replace!`, `picker-push!` and `picker-source-spawn!` on an earlier query's token do nothing and return `#f`, so an old answer never overwrites newer rows. Fill the picker through that token, not the one `live-picker!` returns. That one, as with `picker!`, is for `picker-close!`, `picker-move!` and `picker-source-stop!`, which keep working through it after the user has typed on; it is also the token a binding gets. The picker shows "refreshing" until an answer for the current query arrives, so a source should reply even with nothing to show, with an empty list.

`picker-command-source` builds the common source, one that runs a subprocess. Its first argument maps the current query to a full argv list (the `cmd args...` shape `picker-source-spawn!` takes) or `#f`. An argv is spawned as `picker-source-spawn!` would, with `#:cwd`, `#:nul` and `#:ok-exit-codes` applying the same way. Returning `#f`, as above for an empty query, makes the picker show nothing instead of searching for an empty pattern. That clear also waits out the debounce window, like a real search, since the source runs only once the window has passed.

Every keystroke stops the running search at once, but its rows stay on screen, marked as refreshing, until the new search's results arrive, with no blank flash between. `#:debounce-ms` (default `150`) delays only the new search, not the stop: type fast and the respawn waits until you pause for the window, showing the previous pattern's rows meanwhile. A query that matches nothing clears the list once the search finishes, so stale rows don't stay up.

A non-empty `#:query` (the `seed` above, resuming a search where the last one left off, say) runs the source as soon as the picker opens, before `live-picker!` returns. An empty or omitted `#:query` runs nothing until the user types.

A live picker shows rows in the order the source produced them, with its local fuzzy filter off: the query already selects what the source returns.
