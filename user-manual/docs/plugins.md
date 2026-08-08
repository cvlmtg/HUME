# Plugins

Plugins are written in the same language as your config, so the line between configuring HUME and extending it is thin — a plugin is mostly just `init.scm` code that lives somewhere reusable.

The first half of this page covers using plugins other people wrote; [Writing a plugin](#writing-a-plugin) covers making your own.

Plugins are installed and updated by **PLUM**, a bundled plugin — see [Core Plugins](core-plugins.md#core-plum) to enable it.

## Installing a plugin

Add a `declare-plugin` or `load-plugin` call to your `init.scm`:

```scheme
(declare-plugin "username/repo-name" #:commands '("my-cmd"))
(load-plugin "username/my-theme")
```

Then run `:plum-install` to clone it from GitHub. PLUM never installs anything on its own, so nothing is fetched behind your back at startup; once the plugin is on disk, its commands and key bindings are available from the next launch.

See [How plugins are loaded](#how-plugins-are-loaded) for the difference between the two verbs.

If a plugin supports configuration, pass it with `#:config`:

```scheme
(load-plugin "core:vim-keybind" #:config (hash "change-to-eol" 'off))
```

See [Configuring a plugin](#configuring-a-plugin) for what a plugin does with this value, and the plugin's own docs for which keys it understands.

## Plugin status

```
:plugin-status
```

Shows all declared plugins, whether they loaded successfully, and which commands they registered.

## Reloading configuration

```
:reload-config
```

Reloads `init.scm` from scratch. Useful after editing your config without restarting the editor.

## How plugins are loaded

There are two ways to bring a plugin into the editor from `init.scm`:

| Verb | Timing |
|------|--------|
| `(declare-plugin "name" #:commands ...)` | **Lazy** — body deferred until first use |
| `(load-plugin "name")` | **Eager** — body runs during startup |

**Lazy plugins** (`declare-plugin`) record a *manifest* of what the plugin offers, but don't evaluate the body until the first activation entry is exercised. This keeps startup fast, and is the recommended default: a language-server or formatting plugin whose commands you might never call costs nothing until you do.

**Eager plugins** (`load-plugin`) evaluate their body immediately. Use this for plugins that install options, hooks, or key bindings that must be in place from the first keystroke — themes, paste-style overrides, or anything without a natural "first use" trigger.

A lazy plugin needs at least one activation entry, or it could never activate. Declare them yourself:

- **`#:commands`** — command names the plugin provides. HUME creates placeholder stubs so the names appear in `:` Tab completion immediately; the first dispatch triggers real definition.
- **`#:events`** — lifecycle hooks that trigger loading, as a list of symbols (e.g., `'(on-buffer-open)`).
- **`#:languages`** — buffer language names that trigger loading.

...or, if the plugin ships its own defaults, leave all three off:

```scheme
(declare-plugin "username/repo-name")
```

A bare `declare-plugin` with no activation entries asks the plugin for its own defaults instead of erroring — see [Default activation](#default-activation) if you're writing a plugin and want to support this.

## Writing a plugin

A plugin is a directory containing a `plugin.scm` — that file is the entry point HUME loads. For a plugin installed by PLUM, the directory is named after its GitHub owner and repo. The simplest `plugin.scm`:

```scheme
(define-command! "hello"
  "Print a greeting from my plugin."
  (lambda ()
    (log! 'info "Hello from my plugin!")))
```

This registers `:hello` as a typed command.

### Defining commands

```scheme
(define-command! "command-name"
  "One-line description shown in command help."
  (lambda ()
    ...))
```

Registers a typed command available as `:command-name`. The second argument is a doc string shown in command help; the function is called when the command is dispatched.

For commands that stream subprocess output to the terminal (installers, git operations), add the `#:inline-output #t` keyword. The alt-screen opens on the command's first real output — not eagerly at the start — so a run that produces no output (an already-up-to-date check, a validation error) never flashes an empty screen or waits on an unneeded keypress. Once something is printed, HUME waits for a keypress before returning to the editor, so the output stays on screen until you've read it.

Plugins run with the same privileges as HUME itself, so any Scheme process/filesystem function is available — there's no separate "shell builtin" layer. The one exception: inside an `#:inline-output` command, spawn subprocesses whose output should reach the terminal via `run-inline-output!` rather than a raw `spawn-process`/`command` call — it isolates the child into its own process group so a Ctrl+C meant to interrupt the subprocess doesn't kill HUME too, and it's the trigger that opens the alt-screen:

```scheme
(define-command! "fetch-config"
  "Clone the team config repo into the data directory."
  (lambda ()
    (run-inline-output! "git" (list "clone" "--"
                                     "https://github.com/team/hume-config.git"
                                     (path-join (data-dir) "config"))))
  #:inline-output #t)
```

For commands that should support dot-repeat (`.`), add `#:repeatable #t`. `#:repeatable` and `#:inline-output` are mutually exclusive:

```scheme
(define-command! "delete-and-repeat"
  "Delete the current selection; dot-repeatable."
  (lambda ()
    (call! "delete-selection"))
  #:repeatable #t)
```

### Calling other commands

Use `(call! ...)` to dispatch other commands from within a plugin:

```scheme
(define-command! "delete-and-deselect"
  "Delete the selection, then collapse the cursor."
  (lambda ()
    (call! "delete-selection")
    (call! "collapse-selection")))
```

`call!` dispatches any command that can be bound to a key — built-in and Steel-defined alike — activating the target plugin on demand.

::: warning `call!` can't run `:` commands
Typed commands like `write`, `quit`, or `edit` are not reachable through `call!`. Calling one logs an error and does nothing, so `(call! "write")` will not save. Only key-bindable commands work here.
:::

When forwarding a `count` argument to another command, a count of `0` means "as if no count was typed" — this is how `move-down`/`move-up` decide between visual-row and buffer-line movement, and it lets a key-bound command that forwards its own `count` behave the same way a native keybinding would.

### Reading selections

`(current-selections)` returns the focused buffer's selections as a list of opaque `(anchor head primary?)` triples — char offsets, not grapheme ordinals. Don't index into the tuple directly; go through `core:stdlib`'s helpers instead, which is what they're for:

```scheme
(call! "stdlib/single-selection?" (current-selections))
(call! "stdlib/all-single-char?" (current-selections))
(call! "stdlib/cursor-char-index" (current-selections))
```

`(char-index->line idx)` converts a char offset to a line number when you need one — it's a separate call rather than a field on every selection, since deriving it needs rope access a plain tuple doesn't have.

### Depending on another plugin

::: warning
`call!` with an unknown command name logs an error and no-ops instead of aborting the command body — a missing plugin dependency shows up as an error in `:messages`, not as a crash, so check dependencies up front rather than relying on the error to be noticed.
:::

If your plugin calls another plugin's commands via `call!`, check that the other plugin is available before you rely on it. Which check to use depends on when you need the dependency:

- **Needed immediately** — your plugin body calls into the dependency at the top level, before any key is pressed. Check `(loaded-plugins)`: a lazily-declared plugin that hasn't activated yet won't show up in it, and there's no key press coming to trigger that activation.

  ```scheme
  (unless (member "core:stdlib" (loaded-plugins))
    (error "my-plugin: requires core:stdlib — load it before my-plugin"))
  ```

- **Needed later, inside a command** — the dependency is only called from within a `lambda` that a key press fires. `call!` activates a lazily-declared plugin on demand, so it doesn't matter yet whether the dependency has activated — only that it was declared at all. Check `(declared-plugins)` instead:

  ```scheme
  (unless (member "core:stdlib" (declared-plugins))
    (error "my-plugin: requires core:stdlib — declare or load it before my-plugin"))
  ```

Either check fails loudly at load time (startup or `:reload-config`), naming exactly what's missing, instead of leaving the bug to surface later at whatever moment the dependent command actually runs.

### Pending character input

Some commands need a character argument from the user (like surround operations). `(request-wait-char! cmd-name)` dispatches `cmd-name` once the user types a character; `(pending-char)` then reads that char inside the dispatched command:

```scheme
(define-command! "my-surround"
  "Select the surrounding pair, then replace it with the next typed char."
  (lambda ()
    (call! "surround-paren")
    (request-wait-char! "replace")))
```

HUME shows nothing while it waits, so make it obvious from context that a character is expected.

### Register prefix

To make subsequent `(call! …)` invocations in a command body target a specific register, call `set-register-prefix!` with a single-character register name (`0`–`9`, `k`, `c`, `b` — see [Register prefix](copy-and-paste.md#register-prefix) for what each one holds):

```scheme
(define-command! "paste-kill-ring-after"
  "Paste the kill-ring head after the selection (same as \"kp)."
  (lambda ()
    (set-register-prefix! "k")
    (call! "paste-after")))
```

The prefix persists for the rest of the command body.

Target the black hole register (`"b"`) to discard a selection without touching the kill ring or clipboard — useful when a command needs to throw text away as a side effect of its own logic:

```scheme
(define-command! "delete-without-clobbering"
  "Delete the selection without overwriting the kill ring (same as \"bd)."
  (lambda ()
    (set-register-prefix! "b")
    (call! "delete")))
```

### Hooks

Plugins react to editor lifecycle events by registering a hook handler with `register-hook!`. It must be called at the top level or inside a plugin body — not from a command body:

```scheme
(register-hook! 'on-buffer-save
  (lambda (buffer-id)
    (log! 'info (string-append "saved buffer " (to-string buffer-id)))))
```

Available hooks and their lambda signatures:

| Hook | Fires when | Lambda args |
|------|------------|-------------|
| `on-buffer-open` | A buffer is opened | `(buffer-id)` |
| `on-buffer-close` | A buffer is about to close | `(buffer-id)` |
| `on-buffer-save` | A buffer is saved | `(buffer-id)` |
| `on-buffer-enter` | The focused buffer changes | `(buffer-id)` |
| `on-focus-gained` | The terminal regains focus | `()` |
| `on-mode-change` | The editor mode changes | `(old new)` — mode strings |
| `on-language-set` | A buffer's language is detected or changed | `(buffer-id lang)` — `lang` is a string or `#f` |
| `on-diagnostics-changed` | A buffer's LSP diagnostics change | `(buffer-id)` — pull details with `diagnostics-for-buffer` |
| `on-lsp-attach` | A language server attaches to a buffer | `(buffer-id server-name)` |
| `on-lsp-detach` | A language server detaches from a buffer | `(buffer-id server-name)` |
| `on-viewport-change` | The visible region of a pane changes | `(buffer-id first-line end-line)` — 0-based, end-exclusive |
| `on-trigger-char` | A registered trigger character is typed | `(buffer-id char source)` |
| `on-completion-accept` | A completion entry is accepted | `(buffer-id item)` |
| `on-completion-refilter` | Completion input changes | `(buffer-id text)` |
| `on-option-change` | A global setting is changed (`:set global`, `set-option!`, `:theme`) | `(key value)` — both strings |
| `on-text-changed` | A buffer's text changes | `(buffer-id)` |

`on-buffer-open` and `on-buffer-close` always fire as a pair for a given buffer: a buffer opened and closed within the same command never announces either one.

`on-text-changed` covers edits, undo, redo, `:e!` reload, and refreshes of read-only view buffers (`:messages`, `:ls`, `:plugin-status`) alike — those buffers have no file, so a handler that looks up a path must handle it being absent. It coalesces multiple mutations made by a single command (a multi-cursor edit, a macro, a paste) into one fire, but each keystroke while typing is its own command and so fires on its own — pair it with `debounce` if you want to react only after typing settles rather than on every character.

For lazy plugins, declare the events that should trigger activation via `#:events` on `declare-plugin` instead (see [How plugins are loaded](#how-plugins-are-loaded)). LSP-related hooks like `on-lsp-attach` work fine with `register-hook!`, but can't be used as an `#:events` activation entry — a plugin gated only on `on-lsp-attach` never activates, since nothing attaches to a server until the plugin has already loaded and registered it. The same caveat applies to `on-text-changed`: gating a lazy plugin on it activates on the first edit in *any* buffer, not a buffer the plugin specifically cares about.

`set-option!` works from a hook or command handler too, not just at the top level of your plugin — it changes the *global* default, so use it there when that's really what you want.

For a per-buffer override, `(set-buffer-option! buffer-id "option" value)` sets an option just on the buffer named by `buffer-id`, which also works from hook and command bodies (see [Buffer options](configuration.md#buffer-options) for the list of settable options). Pass the buffer id the hook itself hands you rather than assuming the buffer you're editing — a hook can fire for a buffer other than the one you're currently focused on. `language` isn't an option; set it with `set-buffer-language!` instead. To read a specific buffer's options back the same way, see [Reading options from Steel](#reading-options-from-steel) below.

A few more examples:

```scheme
; format on save
(register-hook! 'on-buffer-save
  (lambda (bid) (call! "lsp-fmt")))

; react to diagnostics
(register-hook! 'on-diagnostics-changed
  (lambda (bid)
    (let ((errs (diagnostics-for-buffer bid #:severity 'error)))
      (log! 'info (string-append (to-string (length errs)) " errors")))))
```

### Reading options from Steel

```scheme
(get-option "option-name")
(get-option bid "option-name")
```

Returns the effective value of an option: called with just an option name, the focused buffer's override if one is set, else the global default. Pass a buffer id first (e.g. inside an `on-language-set` hook, whose handler receives the buffer id as an argument) to read that buffer's value instead of the focused one. Errors on an unknown option name; `language` has no getter — read it with `(buffer-language bid)` instead. For `wrap-mode`, this reads the buffer/global level only — a pane pinned with `:set pane wrap-mode=…` can show a different style than what `get-option` reports.

```scheme
(get-option "tab-width")       ; the focused buffer's effective tab-width
(get-option bid "tab-width")   ; bid's effective tab-width
```

### Default activation

If most users would activate your plugin the same way, give them a one-liner: put a `declare-plugin` call for your own plugin in a `manifest.scm` file next to your plugin's main file.

```scheme
; manifest.scm
(declare-plugin "username/repo-name"
  #:commands '("my-cmd" "my-other-cmd"))
```

A user who writes `(declare-plugin "username/repo-name")` with no `#:commands`/`#:events`/`#:languages` gets your manifest's entries instead of an error. Passing any activation entry explicitly skips your manifest entirely — the user's list is authoritative, not merged with yours. A plugin with no `manifest.scm` can't be declared this way; users who want to use it lazily must list its activation entries themselves (or you can add one).

If your plugin reacts to a language but can't predict which ones a given user cares about, `#:languages '("*")` matches any buffer with a detected language:

```scheme
; manifest.scm
(declare-plugin "username/repo-name"
  #:languages '("*")
  #:commands '("my-cmd"))
```

`#:config` behaves the same as elsewhere: if the user passes `#:config` to their zero-argument `declare-plugin`, that value wins over anything your manifest passes — read it back the usual way with `(plugin-config)`.

Keep `manifest.scm` to just the `declare-plugin` call — it runs whenever a user's bare `declare-plugin` resolves it, which is not a signal that your plugin is about to load.

### Configuring a plugin

A plugin can read the `#:config` value its user passed to `load-plugin` or `declare-plugin` with `(plugin-config)`. It returns whatever was passed — typically a hash — or an empty hash if nothing was passed:

```scheme
(define cfg (plugin-config))
(unless (and (hash-contains? cfg "disable-binding") (hash-ref cfg "disable-binding"))
  (bind-key! 'normal "C" "my-command"))
```

Document the keys your plugin understands so users know what to pass.

The two verbs treat `#:config` differently: with `declare-plugin` the first declaration wins, so a later one can't quietly change it, while `load-plugin` always applies the config it's given. That means a bare `(load-plugin "name")` after a configured `declare-plugin` resets the plugin to its defaults.

### Filesystem and processes

Plugins are trusted code: they can read and write any file, and spawn any process, just like any other Scheme program. There's no separate sandboxed subset of the filesystem — use Scheme's own functions directly (`open-input-file`, `create-directory!`, `delete-file!`, `read-dir`, `path-exists?`, and so on) for file access, and `command`/`spawn-process`/`wait` for running external tools.

A few extra functions cover things Scheme has no way to know on its own:

| Function | Description |
|----------|-------------|
| `(data-dir)` | HUME's data directory, or `#f` if unavailable |
| `(runtime-dir)` | HUME's runtime directory, or `#f` if unavailable |
| `(path-join seg…)` | Join path segments with the OS-native separator |
| `(json-parse str)` | Decode a JSON string into hashmaps/lists/strings/numbers/booleans — errors on malformed input |

`run-inline-output!` also takes a `#:cwd` keyword to set the working directory, and raises an error if the command exits non-zero — wrap it in a handler if a failure is expected.

`command`/`spawn-process`/`wait` all block the whole editor until the command finishes — fine for something instant (`git rev-parse`), but not for anything that might take a moment while the user keeps typing. For that, run it in the background instead:

```scheme
(spawn-async! "git" (list "show" (string-append ref ":" path)) repo-root
  (lambda (stdout stderr exit-code)
    (if (= exit-code 0)
        (use-the-output stdout)
        (report-the-failure stderr))))
```

`spawn-async!` starts `cmd` with `args` (in `cwd`, or `#f` for HUME's own working
directory) and returns immediately — nothing blocks. `callback` is called exactly once,
later, once the command has finished: `stdout` and `stderr` are its complete output as
strings, `exit-code` is its exit code (`-1` if it was killed by a signal, or if the
command couldn't even be started — a missing binary, say). A command that fails to
start or exits non-zero still calls `callback` rather than raising an error, so there's
only one place to handle the outcome. `spawn-async!` returns an id; call
`(cancel-async! id)` to kill the command and discard its callback before it fires —
useful when a debounced action (a hook firing on every keystroke, say) ends up
superseded by a newer one before the older command has finished.

Only install or overwrite files under `(data-dir)` unless you have a specific reason to go elsewhere — that's where HUME expects a plugin's own data (installed grammars, downloaded servers, plugin state) to live.

### Reading buffer text

```scheme
(buffer-text bid)
```

Returns a buffer's full live content as a string — including any unsaved edits, not what's on disk. The string always ends with a trailing newline. Line endings in the returned string are always `\n`, even for a file saved with `\r\n`.

```scheme
(buffer-lines bid)
(buffer-lines bid #:start start #:end end)
```

Returns the buffer's content as a list of lines, each with its line ending stripped. With no range, every line is returned; `#:start`/`#:end` select a 0-based, end-exclusive slice (`(buffer-lines bid #:start 10 #:end 40)` returns lines 10 through 39). An out-of-range `#:end`, or a `#:start` past `#:end`, raises an error rather than silently clamping. Compose with `(viewport-range bid)` to read only what's currently on screen — it returns the same 0-based, end-exclusive range shape, so its pair passes straight through as `#:start`/`#:end`. Guard against `#f`, which `viewport-range` returns for a buffer not currently shown in any pane:

```scheme
(let ((vr (viewport-range bid)))
  (and vr (buffer-lines bid #:start (car vr) #:end (cdr vr))))
```

If you're about to diff a buffer's content against another text, reach for `(diff-buffer-lines bid ref-text)` instead of `(buffer-text bid)`, especially from a hook that fires on every keystroke.

```scheme
(line->offset bid line)
```

Returns the 0-based char offset where content line `line` (0-based) starts — the conversion decoration builtins that take char offsets (like `set-extra-highlights!`) need when all you have is a line number, e.g. from a diff hunk. Raises if `line` is at or past the buffer's content line count.

### Comparing text

Two functions compute a line-level diff — useful for anything that shows what changed between two versions of a file, like a git-status indicator:

```scheme
(diff-lines old-text new-text)
```

Splits both `old-text` and `new-text` into lines the same way HUME treats file content — CRLF line endings become LF, and a missing trailing newline doesn't count as a change — then returns the list of hunks where they differ. Unchanged lines are left out entirely. Each hunk is:

```scheme
(old-start old-count new-start new-count old-lines new-lines)
```

`old-start`/`new-start` are 0-based line numbers, `old-count`/`new-count` are how many lines the hunk covers on each side, and `old-lines`/`new-lines` are the line contents themselves (no trailing newline). A pure insertion has `old-count` `0`; a pure deletion has `new-count` `0` — either way, the zero-count side's line number is exactly where the change happens, so it feeds straight into `set-signs!` or `set-virtual-lines!` with no adjustment.

```scheme
(diff-buffer-lines bid ref-text)
```

Same result, but compares `ref-text` against the current, unsaved content of the buffer named by `bid` — the buffer never has to be pulled through a builtin as one big string first. This is the one to use in a hook that fires on every keystroke.

For a finer-grained comparison inside a single changed line — highlighting exactly which words differ rather than the whole line:

```scheme
(diff-words old-text new-text)
```

Returns `(hunks . too-long?)`. `hunks` is a list of `(old-start old-end new-start new-end old-text new-text)` tuples — 0-based character positions into `old-text`/`new-text`, with `old-text`/`new-text` on each hunk holding the actual changed words. A pure insertion has an empty `old-text` and `old-start` equal to `old-end`; a pure deletion mirrors that on the new side. `too-long?` is `#t` when the two texts were too large to compare word-by-word in time — treat that as a signal to fall back to highlighting the whole line instead of individual words.

### Custom pickers

The modal fuzzy-finder panel behind [Fuzzy Finder](pickers.md) is a generic widget any
plugin can drive — `core:pickers`' own file and buffer finders are built from nothing
but this API.

```scheme
(picker! items on-select #:prompt "buffers: ")
```

`items` is a list of `(display . payload)` pairs — `display` is the string shown and
matched against, `payload` is anything you like (a path, a buffer id, a hashmap); HUME
never looks inside it. `on-select` fires exactly once: with the chosen item's `payload`
if the user presses `Enter`, or `#f` if they press `Esc`, call `picker-close!`, or open a
second picker while this one is still open (which replaces it).

If you close the picker from an asynchronous callback (a `spawn-async!` result
arriving after the user has moved on, say), pass `picker!`'s return value as
`picker-close!`'s `#:token`: the close then becomes a no-op if the picker has
since been closed or replaced, rather than tearing down whatever different
picker the user has open by then. Called with no token — the usual case, from
a key binding — `picker-close!` closes whatever picker is currently open.

For a handful of items — buffers, a plugin's own static list, the output of a quick
synchronous command — build the whole list up front and pass it to `picker!` directly.
For anything enumeration-scale (file lists, grep-style output), open the picker empty
and stream an external command's output straight into it instead:

```scheme
(define token (picker! '() (lambda (path) (when path (open-buffer! path))) #:prompt "files: "))
(picker-source-spawn! token "git" '("ls-files" "-z" "--cached" "--others" "--exclude-standard") #:nul #t)
```

`picker-source-spawn!` runs `cmd` with `args` directly (no shell), splitting its stdout
into lines (or NUL-delimited fields with `#:nul #t`) and appending each one to the picker
as its own `(line . line)` item — display and payload are the same raw line. Nothing
about the command's output passes through Scheme itself, so this stays fast even for
tens of thousands of results; do any parsing of the selected line inside `on-select`,
not up front. The child process is killed automatically if the picker is closed or
replaced before the command finishes. `(picker-push! token items)` appends a batch of
ordinary `(display . payload)` items instead, for a source that produces its own results
asynchronously (an LSP request, a timer) rather than through a spawned command.

`picker-source-spawn!` already shows the user something is still loading. A picker
populated by `picker-push!` from a `spawn-async!` callback has no such signal of its
own, so pass `#:pending #t` to `picker!` when opening empty this way — it marks the
panel as "results still arriving" until the first `picker-push!` call lands.

## Bundled core plugins

HUME ships several built-in plugins — see [Core Plugins](core-plugins.md) for the full list and what each does.
